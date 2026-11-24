import AfterimageEngine
import AppKit
import ScreenCaptureKit

public enum CaptureOutcome: Sendable, Equatable {
    case queued(Int)
    case unchanged
    /// Something in the way that may pass on its own.
    case skipped(String)
    /// A choice or a state that forbids it. Never retried.
    case blocked(String)
    case failed(String)
}

/// One capture at a time: decide what may be seen, photograph it, check again, read it, hand it to the engine.
/// AIDEV-NOTE: privacy-critical. The before/after check closes the gap where a page navigates mid-capture.
public actor Capturer {
    private let engine: Engine
    private var busy = false
    public init(engine: Engine) { self.engine = engine }

    /// Asked of the recorder each time: why not to look right now (nil to go ahead), and what is off-limits.
    public typealias Gate = @MainActor @Sendable () -> (blocked: String?, exclusions: Exclusions)

    private struct Hidden {
        var pids: Set<pid_t> = []
        var reasons: [pid_t: HideReason] = [:]
        var focusedURL: [pid_t: String] = [:]
    }

    /// Everything that bears on whether a picture may be kept, at one instant.
    struct Look: Sendable {
        var blocked: String?
        var exclusions: Exclusions
        var front: pid_t?
        var hidden: Set<pid_t>
        var drawn: Set<pid_t>
    }

    /// Why a picture taken between these looks can't be kept, or nil.
    static func spoiled(_ before: Look, _ after: Look, during: [Look] = [], omitted: Set<pid_t>) -> CaptureOutcome? {
        let all = [before] + during + [after]
        if let b = all.lazy.compactMap(\.blocked).first { return .blocked(b) }
        if all.contains(where: { $0.exclusions != before.exclusions }) { return .blocked("Your settings changed mid-capture") }
        if all.contains(where: { $0.front != before.front }) { return .skipped("The app changed mid-capture") }
        // Off-limits at any instant and drawn at any instant: they needn't be the same one.
        let hidden = all.reduce(into: Set<pid_t>()) { $0.formUnion($1.hidden) }
        let drawn = all.reduce(into: Set<pid_t>()) { $0.formUnion($1.drawn) }
        return hidden.intersection(drawn).subtracting(omitted).isEmpty ? nil : .skipped("Something private appeared")
    }

    private func look(_ ex: Exclusions, _ hidden: Hidden, _ gate: Gate) async -> Look {
        let g = await gate()
        let front = await MainActor.run { NSWorkspace.shared.frontmostApplication?.processIdentifier }
        // Status items and slivers can't show anything worth hiding.
        let drawn = OnScreenWindow.all().filter { $0.bounds.width > 48 && $0.bounds.height > 48 }.map(\.pid)
        return Look(blocked: Session.isLocked ? "The screen is locked" : g.blocked, exclusions: g.exclusions, front: front, hidden: hidden.pids, drawn: Set(drawn))
    }

    /// Vision's first request loads a model and can take a while; get it over with before it matters.
    public func warmUp() {
        guard let c = CGContext(data: nil, width: 64, height: 64, bitsPerComponent: 8, bytesPerRow: 0, space: Pixels.sRGB, bitmapInfo: Pixels.rgba),
              let i = c.makeImage() else { return }
        _ = try? Recognizer.lines(in: i)
    }

    public func capture(_ trigger: Trigger, allDisplays: Bool, gate: @escaping Gate) async -> CaptureOutcome {
        // Actors are re-entrant across awaits; without this two cycles could mix one's pixels with the other's facts.
        guard !busy else { return .blocked("Busy") }
        busy = true
        defer { busy = false }

        let front = await MainActor.run { NSWorkspace.shared.frontmostApplication.map { ($0.processIdentifier, $0.localizedName ?? "", $0.bundleIdentifier ?? "") } }
        guard let front else { return .skipped("No app is in front") }
        if front.0 == getpid() { return .blocked("Afterimage is in front") }

        let exclusions = await gate().exclusions
        var hiddenBefore = await hidden(exclusions)
        if hiddenBefore.pids.contains(front.0) { return Self.refused(hiddenBefore.reasons[front.0], front.1) }
        let frontWindow = OnScreenWindow.front(of: front.0, in: OnScreenWindow.all())

        let content: SCShareableContent
        do {
            content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false)
        } catch { return .failed(error.localizedDescription) }
        // Listing windows can take long enough for the facts to change.
        hiddenBefore = await hidden(exclusions)
        if hiddenBefore.pids.contains(front.0) { return Self.refused(hiddenBefore.reasons[front.0], front.1) }
        let omit = content.applications.filter { hiddenBefore.pids.contains($0.processID) }
        let omitted = Set(omit.map(\.processID))
        let displays = pick(content.displays, front: frontWindow?.bounds, all: allDisplays)
        guard !displays.isEmpty else { return .failed("No display is available") }

        // The last thing before looking.
        let before = await look(exclusions, hiddenBefore, gate)
        if let why = Self.spoiled(before, before, omitted: omitted) { return why }
        guard before.front == front.0, before.exclusions == exclusions else { return .skipped("The app changed mid-capture") }

        var shots: [(display: SCDisplay, image: CGImage, at: Date)] = []
        for d in displays {
            let filter = SCContentFilter(display: d, excludingApplications: omit, exceptingWindows: [])
            let cfg = SCStreamConfiguration()
            let scale = Double(filter.pointPixelScale)
            cfg.width = Int(Double(d.width) * scale)
            cfg.height = Int(Double(d.height) * scale)
            cfg.showsCursor = false
            cfg.captureResolution = .best
            // AIDEV-NOTE: stamped before the pixels exist, so "forget the last minutes" can never miss them.
            let at = Date()
            do { shots.append((d, try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: cfg), at)) }
            catch { return .failed(error.localizedDescription) }
        }

        // Anything that became off-limits while we were looking spoils the whole frame.
        let quick = await look(exclusions, hiddenBefore, gate)
        let hiddenAfter = await hidden(exclusions)
        let after = await look(exclusions, hiddenAfter, gate)
        if let why = Self.spoiled(before, after, during: [quick], omitted: omitted) { return why }

        let apps = await MainActor.run {
            NSWorkspace.shared.runningApplications.filter { $0.activationPolicy == .regular }
                .map { AppRef(name: $0.localizedName ?? "", bundleId: $0.bundleIdentifier ?? "") }
        }.filter { hideApp(ex: exclusions, bundleId: $0.bundleId) == nil }
        var queued = 0
        for (d, image, at) in shots {
            let key = String(d.displayID)
            guard engine.isNew(displayKey: key, luma: Pixels.thumbprint(image)) || trigger == .manual else { continue }
            let lines: [TextLine]
            do { lines = try Recognizer.lines(in: image) } catch {
                engine.lost(displayKey: key)
                return .failed("The screen couldn't be read, so it wasn't saved")
            }
            guard let b = Pixels.bitmap(image) else {
                engine.lost(displayKey: key)
                return .failed("Out of memory")
            }
            let m = Moment(
                takenAt: Int64(at.timeIntervalSince1970 * 1000), trigger: trigger.rawValue, display: await Self.name(d.displayID),
                appName: front.1, bundleId: front.2, windowTitle: frontWindow?.title ?? "", url: hiddenAfter.focusedURL[front.0] ?? "",
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
                if let r = hideBrowser(ex: ex, bundleId: bundle, windows: reading.windows, complete: reading.complete) {
                    h.pids.insert(pid)
                    h.reasons[pid] = r
                } else if let u = reading.focusedURL {
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
    static func refused(_ r: HideReason?, _ app: String) -> CaptureOutcome {
        r == .unreadable ? .skipped(phrase(r, app)) : .blocked(phrase(r, app))
    }
    static func phrase(_ r: HideReason?, _ app: String) -> String {
        switch r {
        case .passwordManager: "A password manager is open"
        case .site: "A site you excluded is open in \(app)"
        case .privateWindow: "\(app) has a private window open"
        case .unreadable: "\(app)’s address couldn’t be read"
        case .systemPrompt: "macOS is asking for something"
        case .app, nil: "\(app) is excluded"
        }
    }
}
