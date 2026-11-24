import AfterimageEngine
import AppKit
import ScreenCaptureKit

public enum CaptureOutcome: Sendable, Equatable {
    case queued(Int)
    case unchanged
    case skipped(String)
    case failed(String)
}

/// One capture at a time: decide what may be seen, photograph it, check again, read it, hand it to the engine.
/// AIDEV-NOTE: privacy-critical. The before/after check closes the gap where a page navigates mid-capture.
public actor Capturer {
    private let engine: Engine
    private var busy = false
    public init(engine: Engine) { self.engine = engine }

    private struct Hidden {
        var pids: Set<pid_t> = []
        var reasons: [pid_t: HideReason] = [:]
        var focusedURL: [pid_t: String] = [:]
    }

    /// Vision's first request loads a model and can take a while; get it over with before it matters.
    public func warmUp() {
        guard let c = CGContext(data: nil, width: 64, height: 64, bitsPerComponent: 8, bytesPerRow: 0, space: Pixels.sRGB, bitmapInfo: Pixels.rgba),
              let i = c.makeImage() else { return }
        _ = try? Recognizer.lines(in: i)
    }

    public func capture(_ trigger: Trigger, exclusions: Exclusions, allDisplays: Bool) async -> CaptureOutcome {
        // Actors are re-entrant across awaits; without this two cycles could mix one's pixels with the other's facts.
        guard !busy else { return .skipped("Busy") }
        busy = true
        defer { busy = false }
        guard !Session.isLocked else { return .skipped("The screen is locked") }

        let front = await MainActor.run { NSWorkspace.shared.frontmostApplication.map { ($0.processIdentifier, $0.localizedName ?? "", $0.bundleIdentifier ?? "") } }
        guard let front else { return .skipped("No app is in front") }
        if front.0 == getpid() { return .skipped("Afterimage is in front") }

        let before = await hidden(exclusions)
        if before.pids.contains(front.0) { return .skipped(Self.phrase(before.reasons[front.0], front.1)) }
        let windowsBefore = OnScreenWindow.all()
        let title = windowsBefore.first { $0.pid == front.0 && $0.layer == 0 }?.title ?? ""

        let content: SCShareableContent
        do {
            content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false)
        } catch { return .failed(error.localizedDescription) }
        let omit = content.applications.filter { before.pids.contains($0.processID) }
        let displays = pick(content.displays, front: windowsBefore.first { $0.pid == front.0 && $0.layer == 0 }?.bounds, all: allDisplays)
        guard !displays.isEmpty else { return .failed("No display is available") }

        var shots: [(SCDisplay, CGImage)] = []
        for d in displays {
            let filter = SCContentFilter(display: d, excludingApplications: omit, exceptingWindows: [])
            let cfg = SCStreamConfiguration()
            let scale = Double(filter.pointPixelScale)
            cfg.width = Int(Double(d.width) * scale)
            cfg.height = Int(Double(d.height) * scale)
            cfg.showsCursor = false
            cfg.captureResolution = .best
            do { shots.append((d, try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: cfg))) }
            catch { return .failed(error.localizedDescription) }
        }

        // Anything that became off-limits while we were looking spoils the whole frame.
        let after = await hidden(exclusions)
        let frontAfter = await MainActor.run { NSWorkspace.shared.frontmostApplication?.processIdentifier }
        let omitted = Set(omit.map(\.processID))
        // Status items and slivers can't show anything worth hiding.
        let drawn = Set((OnScreenWindow.all() + windowsBefore).filter { $0.bounds.width > 48 && $0.bounds.height > 48 }.map(\.pid))
        if !after.pids.union(before.pids).intersection(drawn).subtracting(omitted).isEmpty { return .skipped("Something private appeared") }
        guard frontAfter == front.0 else { return .skipped("The app changed mid-capture") }
        guard !Session.isLocked else { return .skipped("The screen is locked") }

        let apps = await MainActor.run {
            NSWorkspace.shared.runningApplications.filter { $0.activationPolicy == .regular }
                .map { AppRef(name: $0.localizedName ?? "", bundleId: $0.bundleIdentifier ?? "") }
        }.filter { hideApp(ex: exclusions, bundleId: $0.bundleId) == nil }
        var queued = 0
        for (d, image) in shots {
            let key = String(d.displayID)
            guard engine.isNew(displayKey: key, luma: Pixels.thumbprint(image)) || trigger == .manual else { continue }
            let lines: [TextLine]
            do { lines = try Recognizer.lines(in: image) } catch { return .failed("The screen couldn't be read, so it wasn't saved") }
            guard let b = Pixels.bitmap(image) else { return .failed("Out of memory") }
            let m = Moment(
                takenAt: Int64(Date().timeIntervalSince1970 * 1000), trigger: trigger.rawValue, display: await Self.name(d.displayID),
                appName: front.1, bundleId: front.2, windowTitle: title, url: after.focusedURL[front.0] ?? "",
                userName: Session.consoleUser, fullName: NSFullUserName(), host: Session.computerName, sessions: Session.loggedIn, runningApps: apps)
            engine.ingest(frame: Frame(rgba: b.data, width: UInt32(b.width), height: UInt32(b.height), stride: UInt32(b.stride), lines: lines, moment: m, displayKey: key))
            queued += 1
        }
        return queued == 0 ? .unchanged : .queued(queued)
    }

    private func hidden(_ ex: Exclusions) async -> Hidden {
        var h = Hidden()
        // Never photograph our own window: it shows other photographs.
        h.pids.insert(getpid())
        let running = await MainActor.run { NSWorkspace.shared.runningApplications.map { ($0.processIdentifier, $0.bundleIdentifier ?? "", $0.isHidden) } }
        let visible = OnScreenWindow.all()
        let drawing = Set(visible.map(\.pid))
        for (pid, bundle, _) in running {
            if let r = hideApp(ex: ex, bundleId: bundle) {
                h.pids.insert(pid)
                h.reasons[pid] = r
            } else if isBrowser(bundleId: bundle), drawing.contains(pid) {
                let reading = Accessibility.read(pid: pid, visible: visible)
                if let r = hideBrowser(ex: ex, bundleId: bundle, windows: reading?.windows) {
                    h.pids.insert(pid)
                    h.reasons[pid] = r
                } else if let u = reading?.focusedURL {
                    h.focusedURL[pid] = u
                }
            }
        }
        return h
    }

    private func pick(_ all: [SCDisplay], front: CGRect?, all wantAll: Bool) -> [SCDisplay] {
        if wantAll || all.count == 1 { return all }
        if let f = front, let d = all.max(by: { area($0.frame, f) < area($1.frame, f) }), area(d.frame, f) > 0 { return [d] }
        return all.filter { $0.displayID == CGMainDisplayID() }
    }
    private func area(_ a: CGRect, _ b: CGRect) -> CGFloat {
        let r = a.intersection(b)
        return r.isNull ? 0 : r.width * r.height
    }
    @MainActor private static func name(_ id: CGDirectDisplayID) -> String {
        NSScreen.screens.first { ($0.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? CGDirectDisplayID) == id }?.localizedName ?? "Display"
    }
    static func phrase(_ r: HideReason?, _ app: String) -> String {
        switch r {
        case .passwordManager: "A password manager is open"
        case .site: "A site you excluded is open in \(app)"
        case .privateWindow: "\(app) has a private window open"
        case .unreadable: "\(app)’s address couldn’t be read"
        case .app, nil: "\(app) is excluded"
        }
    }
}
