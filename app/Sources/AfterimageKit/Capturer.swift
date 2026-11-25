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
    /// Why the last capture was refused on privacy grounds, for a log that mustn't name the app.
    private var lastRefusal: HideReason?
    /// Part of every window's name, new each time the app starts.
    private let session = WindowShots.newSession()
    /// Windows that have a picture of them in front showing what they show now. Bounded; forgetting costs a repeat.
    private var frontTaken: Set<CGWindowID> = []
    /// What each window said in the last picture saved of it (WindowShots.words). Bounded; forgetting costs a repeat.
    private var lastWords: [CGWindowID: Int] = [:]
    /// When the whole screen was last saved alongside the windows.
    private var lastOverview: Date?
    public init(engine: Engine) { self.engine = engine }

    /// Asked of the recorder each time: why not to look right now (nil to go ahead), and what is off-limits.
    public typealias Gate = @MainActor @Sendable () -> (blocked: String?, exclusions: Exclusions)

    private struct Hidden {
        var pids: Set<pid_t> = []
        var reasons: [pid_t: HideReason] = [:]
        var focusedURL: [pid_t: String] = [:]
        /// Places to frost, by app, in screen points.
        var covers: [pid_t: [Cover]] = [:]
    }

    /// Everything that bears on whether a picture may be kept, at one instant.
    struct Look: Sendable {
        var blocked: String?
        var exclusions: Exclusions
        var front: pid_t?
        var hidden: Set<pid_t>
        var drawn: Set<pid_t>
        var covers: [pid_t: [Cover]] = [:]
    }

    /// The app in front is one that mustn't be photographed, so nothing is. Afterimage itself doesn't count: it is
    /// always among the hidden, its windows are left out of the picture, and what else is on screen is still wanted.
    // AIDEV-NOTE: privacy-critical. Leaving Afterimage out relies on ScreenCaptureKit listing it; if it isn't listed,
    // AIDEV-NOTE: `spoiled` sees a hidden app drawn but not omitted and the picture is dropped.
    static func offLimits(_ front: pid_t, _ hidden: Set<pid_t>, ours: Bool) -> Bool { !ours && hidden.contains(front) }

    /// Whether a browser is left out on account of its page. `answered`: its extension replied in time.
    // AIDEV-NOTE: owner's decision 2026-11-25: a silent extension no longer stops capture (it used to: .noAnswer).
    // AIDEV-NOTE: The browser is then photographed as if the extension weren't installed; rules and the model still check it.
    static func pageHide(planned: HideReason?, answered: Bool) -> HideReason? { planned }

    /// Why a picture taken between these looks can't be kept, or nil.
    static func spoiled(_ before: Look, _ after: Look, during: [Look] = [], omitted: Set<pid_t>) -> CaptureOutcome? {
        let all = [before] + during + [after]
        if let b = all.lazy.compactMap(\.blocked).first { return .blocked(b) }
        if all.contains(where: { $0.exclusions != before.exclusions }) { return .blocked("Your settings changed mid-capture") }
        if all.contains(where: { $0.front != before.front }) { return .skipped("The app changed mid-capture") }
        // A field that moved may have been photographed anywhere in between.
        let covers = all.map { $0.covers.filter { !omitted.contains($0.key) && !$0.value.isEmpty } }
        if covers.contains(where: { $0 != covers[0] }) { return .skipped("The page moved mid-capture") }
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
        return Look(blocked: Session.isLocked ? "The screen is locked" : g.blocked, exclusions: g.exclusions, front: front, hidden: hidden.pids, drawn: Set(drawn), covers: hidden.covers)
    }

    /// Vision's first request loads a model and can take a while; get it over with before it matters.
    public func warmUp() {
        guard let c = CGContext(data: nil, width: 64, height: 64, bitsPerComponent: 8, bytesPerRow: 0, space: Pixels.sRGB, bitmapInfo: Pixels.rgba),
              let i = c.makeImage() else { return }
        _ = try? Recognizer.lines(in: i)
    }

    /// The outcome, and when it was a privacy refusal, which kind (for a log that mustn't name the app).
    /// `windows`: photograph each window on its own instead of the display ("Capture windows separately").
    public func capture(_ trigger: Trigger, allDisplays: Bool, windows: Bool = false, overviewEvery: Int = 0, gate: @escaping Gate) async -> (outcome: CaptureOutcome, refusal: HideReason?) {
        // Actors are re-entrant across awaits; without this two cycles could mix one's pixels with the other's facts.
        guard !busy else { return (.blocked("Busy"), nil) }
        busy = true
        defer { busy = false }
        lastRefusal = nil
        let outcome = windows ? await runWindows(trigger, allDisplays: allDisplays, overviewEvery: overviewEvery, gate: gate) : await run(trigger, allDisplays: allDisplays, gate: gate)
        return (outcome, lastRefusal)
    }

    /// Each window as its own picture, covered or not. Same order as `run`: decide, look, photograph, look again.
    /// AIDEV-NOTE: privacy-critical. Which windows is decided by WindowShots.plan (rules in core/src/windows.rs), before
    /// AIDEV-NOTE: and after; a picture is kept only if both agree. Two brackets: the window in front (with a picture of
    /// AIDEV-NOTE: the displays, read for context only and never saved) in one short one; the windows behind in a second,
    /// AIDEV-NOTE: with a look after every picture. Trouble in the second costs only the windows behind.
    private func runWindows(_ trigger: Trigger, allDisplays: Bool, overviewEvery: Int, gate: @escaping Gate) async -> CaptureOutcome {
        let front = await MainActor.run { NSWorkspace.shared.frontmostApplication.map { ($0.processIdentifier, $0.localizedName ?? "", $0.bundleIdentifier ?? "") } }
        guard let front else { return .skipped("No app is in front") }
        // Afterimage in front: its own windows are left out of every picture, and the rest of the screen is taken.
        let ours = front.0 == getpid()

        let exclusions = await gate().exclusions
        var hiddenBefore = await hidden(exclusions)
        if Self.offLimits(front.0, hiddenBefore.pids, ours: ours) {
            lastRefusal = hiddenBefore.reasons[front.0] ?? .app
            return Self.refused(hiddenBefore.reasons[front.0], front.1)
        }
        let content: SCShareableContent
        do {
            content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false)
        } catch { return .failed(error.localizedDescription) }
        // Listing windows can take long enough for the facts to change.
        hiddenBefore = await hidden(exclusions)
        if Self.offLimits(front.0, hiddenBefore.pids, ours: ours) {
            lastRefusal = hiddenBefore.reasons[front.0] ?? .app
            return Self.refused(hiddenBefore.reasons[front.0], front.1)
        }
        // "Displays: the one in use" holds here too: only windows on the displays a whole-display picture would show.
        let screens = pick(content.displays, front: OnScreenWindow.front(of: front.0, in: OnScreenWindow.all())?.bounds, all: allDisplays)
        guard !screens.isEmpty else { return .failed("No display is available") }
        let desktop = screens.map(\.frame)
        // Minimised windows and windows on another desktop are listed too; they aren't on screen.
        let handles = Dictionary(content.windows.filter(\.isOnScreen).map { ($0.windowID, $0) }, uniquingKeysWith: { a, _ in a })
        let shareable = Set(handles.keys)
        // Who owns what is read at the same instant as the list it is judged with, and the picture is labelled from it.
        func plan(_ hidden: Set<pid_t>) async -> (choices: [WindowChoice], list: [OnScreenWindow], owners: [pid_t: WindowShots.Owner], names: [pid_t: String]) {
            let running = await MainActor.run {
                NSWorkspace.shared.runningApplications.map { ($0.processIdentifier, $0.bundleIdentifier ?? "", $0.activationPolicy == .regular, $0.localizedName ?? "") }
            }
            let owners = Dictionary(running.map { ($0.0, WindowShots.Owner(bundleId: $0.1, regular: $0.2)) }, uniquingKeysWith: { a, _ in a })
            let list = OnScreenWindow.all()
            let choices = WindowShots.plan(exclusions, front: (front.0, front.2), list: list, owners: owners, shareable: shareable, hidden: hidden, own: getpid(), desktop: desktop)
            return (choices, list, owners, Dictionary(running.map { ($0.0, $0.3) }, uniquingKeysWith: { a, _ in a }))
        }
        let planned = await plan(hiddenBefore.pids)
        let wanted = planned.choices.filter { $0.skip == nil }
        guard !wanted.isEmpty else { return .blocked("No window could be photographed") }
        // Nothing hidden is ever chosen, so nothing hidden beforehand can be in a window's picture.
        let omit = content.applications.filter { hiddenBefore.pids.contains($0.processID) }
        let omitted = hiddenBefore.pids
        // The first bracket also photographs the displays, where only apps ScreenCaptureKit lists can be left out:
        // a hidden app that is drawn but not listed spoils that look, as it does for whole-display capture.
        let leftOut = Set(omit.map(\.processID))

        struct Shot { let choice: WindowChoice; let image: CGImage; let at: Date; let content: CGSize? }
        func grab(_ c: WindowChoice) async -> Shot? {
            // The handle must be for the app the verdict was about.
            guard let w = handles[c.id], w.owningApplication?.processID == c.pid else { return nil }
            let filter = SCContentFilter(desktopIndependentWindow: w)
            let cfg = SCStreamConfiguration()
            // What the picture covers, when ScreenCaptureKit says; otherwise the window as it listed it.
            let said = filter.contentRect.isEmpty ? nil : filter.contentRect.size
            guard let size = WindowShots.pixels(said ?? w.frame.size, scale: Double(filter.pointPixelScale)) else { return nil }
            cfg.width = size.width
            cfg.height = size.height
            cfg.showsCursor = false
            cfg.ignoreShadowsSingleWindow = true
            cfg.captureResolution = .best
            // AIDEV-NOTE: stamped before the pixels exist, so "forget the last minutes" can never miss them.
            let at = Date()
            // A window that closed a moment ago gives an error; that is one window less, not a failure.
            do { return Shot(choice: c, image: try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: cfg), at: at, content: said) }
            catch {
                NSLog("a window couldn't be photographed: %@ [AIMG-WIN02]", error.localizedDescription)
                return nil
            }
        }

        // First bracket: the window in front, and the displays as whole-display capture would have seen them.
        let before = await look(exclusions, hiddenBefore, gate)
        if let why = Self.spoiled(before, before, omitted: leftOut) { return why }
        guard before.front == front.0, before.exclusions == exclusions else { return .skipped("The app changed mid-capture") }
        let frontChoice = wanted.first(where: \.front)
        var frontShot: Shot?
        if let frontChoice { frontShot = await grab(frontChoice) }
        // AIDEV-NOTE: privacy-critical. These pictures are read for their text and dropped. Whole-display capture hides a
        // AIDEV-NOTE: secret wherever it shows once it is found anywhere on screen; a window alone must hide no less.
        var around: [(display: SCDisplay, image: CGImage, at: Date)] = []
        for d in screens {
            let filter = SCContentFilter(display: d, excludingApplications: omit, exceptingWindows: [])
            let cfg = SCStreamConfiguration()
            let scale = Double(filter.pointPixelScale)
            cfg.width = Int(Double(d.width) * scale)
            cfg.height = Int(Double(d.height) * scale)
            cfg.showsCursor = false
            cfg.captureResolution = .best
            let at = Date()
            do { around.append((d, try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: cfg), at)) }
            catch { return .failed(error.localizedDescription) }
        }
        let quick = await look(exclusions, hiddenBefore, gate)
        let hiddenAfter = await hidden(exclusions)
        let after = await look(exclusions, hiddenAfter, gate)
        if let why = Self.spoiled(before, after, during: [quick], omitted: leftOut) { return why }
        var everHidden = hiddenBefore.pids.union(hiddenAfter.pids)
        let kept = Set(keepWindows(before: planned.choices, after: await plan(everHidden).choices))
        if let f = frontChoice {
            // The window in front is what this look was for. Changed or not photographed, the look is taken again.
            guard kept.contains(f.id) else { return .skipped("The window changed mid-capture") }
            guard frontShot != nil else { return .skipped("The window in front couldn’t be photographed") }
            // Its address is the one the check vouched for only if it was the same before and after.
            guard hiddenBefore.focusedURL[f.pid] == hiddenAfter.focusedURL[f.pid] else { return .skipped("The page changed mid-capture") }
        }

        let apps = await MainActor.run {
            NSWorkspace.shared.runningApplications.filter { $0.activationPolicy == .regular }
                .map { AppRef(name: $0.localizedName ?? "", bundleId: $0.bundleIdentifier ?? "") }
        }.filter { hideApp(ex: exclusions, bundleId: $0.bundleId) == nil }
        let listed = Dictionary(planned.list.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        let displays = content.displays.map { (id: $0.displayID, frame: $0.frame) }
        // Read once, when the first picture needs it.
        var context: [TextLine]?
        var perDisplay: [[TextLine]] = []
        func surroundings() -> [TextLine]? {
            if let context { return context }
            var each: [[TextLine]] = []
            for a in around {
                guard let l = try? Recognizer.lines(in: a.image) else { return nil }
                each.append(l)
            }
            perDisplay = each
            context = each.flatMap { $0 }
            return context
        }
        var queued = 0
        // Now and then the whole screen is saved too, so there is a picture of how it all sat together.
        // AIDEV-NOTE: the same pictures the first bracket vouched for, saved the way whole-display capture saves them:
        // AIDEV-NOTE: hidden apps left out, the extension's fields placed, deduplicated per display.
        // AIDEV-NOTE: saved after the windows of the same look, so what they gave away is already remembered; and only
        // AIDEV-NOTE: if the address in front was the same before and after, window in front or not.
        func overview() async {
            guard WindowShots.overviewDue(last: lastOverview, now: Date(), every: overviewEvery), !around.isEmpty,
                  hiddenBefore.focusedURL[front.0] == hiddenAfter.focusedURL[front.0], let everything = surroundings() else { return }
            lastOverview = Date()
            let frontWindow = OnScreenWindow.front(of: front.0, in: planned.list)
            for (i, a) in around.enumerated() {
                let key = String(a.display.displayID)
                guard engine.isNew(displayKey: key, luma: Pixels.thumbprint(a.image)), let b = Pixels.bitmap(a.image) else { continue }
                let m = Moment(
                    takenAt: Int64(a.at.timeIntervalSince1970 * 1000), trigger: trigger.rawValue, display: await Self.name(a.display.displayID),
                    appName: front.1, bundleId: front.2, windowTitle: ours ? "" : frontWindow?.title ?? "", url: ours ? "" : hiddenAfter.focusedURL[front.0] ?? "",
                    userName: Session.consoleUser, fullName: NSFullUserName(), host: Session.computerName, sessions: Session.loggedIn, runningApps: apps)
                engine.ingest(frame: Frame(rgba: b.data, width: UInt32(b.width), height: UInt32(b.height), stride: UInt32(b.stride), lines: perDisplay[i], moment: m, displayKey: key,
                                           covers: Self.onDisplay(before.covers.filter { !leftOut.contains($0.key) }.values.flatMap { $0 }, a.display.frame), context: everything))
                queued += 1
            }
        }
        /// Read one picture and hand it over. False when it couldn't be, and its thumbprint has been let go.
        /// `ifWordsChanged`: the picture differs only slightly from the last look, so it is saved only if it reads
        /// differently from the last one saved. Not saving it then is not a failure.
        func send(_ s: Shot, covers: [Cover], url: String, ifWordsChanged: Bool = false) async -> Bool {
            guard let w = listed[s.choice.id], let owner = planned.owners[w.pid] else { return false }
            let key = "win:\(w.id)"
            guard let lines = try? Recognizer.lines(in: s.image) else {
                engine.lost(displayKey: key)
                return false
            }
            let words = WindowShots.words(lines)
            if ifWordsChanged, lastWords[w.id] == words { return true }
            // Without what else was on screen the picture would hide less than a whole-display one: not taken.
            guard let around = surroundings(), let b = Pixels.bitmap(s.image) else {
                engine.lost(displayKey: key)
                return false
            }
            if lastWords.count >= 512 { lastWords.removeAll() }
            lastWords[w.id] = words
            let display = Self.displays(displays, front: w.bounds, all: false, main: CGMainDisplayID()).first
            let m = Moment(
                takenAt: Int64(s.at.timeIntervalSince1970 * 1000), trigger: trigger.rawValue, display: await Self.name(display ?? CGMainDisplayID()),
                appName: planned.names[w.pid] ?? "", bundleId: owner.bundleId, windowTitle: w.title, url: url,
                userName: Session.consoleUser, fullName: NSFullUserName(), host: Session.computerName, sessions: Session.loggedIn, runningApps: apps,
                windowKey: WindowShots.key(session: session, id: w.id), windowFront: s.choice.front)
            engine.ingest(frame: Frame(rgba: b.data, width: UInt32(b.width), height: UInt32(b.height), stride: UInt32(b.stride), lines: lines, moment: m, displayKey: key, covers: covers, context: around))
            queued += 1
            return true
        }
        /// The secret fields on a window, or nil when one can't be placed and the window must be left out.
        func fields(_ s: Shot) -> [Cover]? {
            guard let w = listed[s.choice.id], w.pid == s.choice.pid,
                  let covers = WindowShots.covers(before.covers[w.pid] ?? [], window: w.bounds, content: s.content) else {
                engine.noteCapture(trigger: "window", outcome: "A window was left out: a secret field couldn’t be placed on it")
                NSLog("a window was left out: a secret field couldn't be placed on it [AIMG-WIN01]")
                return nil
            }
            return covers
        }

        if let s = frontShot {
            frontShot = nil
            guard let covers = fields(s) else { return .skipped("A secret field couldn’t be placed on the window in front") }
            let id = s.choice.id
            let verdict = WindowShots.take(front: true, manual: trigger == .manual, print: Pixels.thumbprint(s.image, skipBar: false), queued: queued, hadFront: frontTaken.contains(id)) {
                engine.howNew(displayKey: "win:\(id)", luma: $0)
            }
            if verdict == .take || verdict == .ifWordsChanged {
                guard await send(s, covers: covers, url: hiddenAfter.focusedURL[s.choice.pid] ?? "", ifWordsChanged: verdict == .ifWordsChanged) else { return .failed("The screen couldn't be read, so it wasn't saved") }
                if frontTaken.count >= 512 { frontTaken.removeAll() }
                frontTaken.insert(id)
            }
        }

        // Second bracket: the windows behind, one at a time, with a look after each. Only changed ones are held on to.
        var behind: [Shot] = []
        var slight: Set<CGWindowID> = []
        var looks: [Look] = []
        for c in wanted where !c.front && kept.contains(c.id) {
            guard queued + behind.count < WindowShots.maxQueued else { break }
            guard let s = await grab(c) else { continue }
            looks.append(await look(exclusions, hiddenAfter, gate))
            let id = c.id
            let verdict = WindowShots.take(front: false, manual: false, print: Pixels.thumbprint(s.image, skipBar: false), queued: queued + behind.count, hadFront: false) {
                engine.howNew(displayKey: "win:\(id)", luma: $0)
            }
            guard verdict == .take || verdict == .ifWordsChanged else { continue }
            // What it shows has changed since any picture of it in front.
            if verdict == .take { frontTaken.remove(id) } else { slight.insert(id) }
            behind.append(s)
        }
        if !behind.isEmpty {
            let hiddenLast = await hidden(exclusions)
            let last = await look(exclusions, hiddenLast, gate)
            everHidden.formUnion(hiddenLast.pids)
            let still = Set(keepWindows(before: planned.choices, after: await plan(everHidden).choices))
            let spoiled = Self.spoiled(after, last, during: looks, omitted: omitted) != nil
            for s in behind {
                let key = "win:\(s.choice.id)"
                guard !spoiled, still.contains(s.choice.id), let covers = fields(s) else {
                    engine.lost(displayKey: key)
                    continue
                }
                // One window that can't be read is one window less, not a failed look.
                _ = await send(s, covers: covers, url: "", ifWordsChanged: slight.contains(s.choice.id))
            }
        }
        await overview()
        around = []
        return queued == 0 ? .unchanged : .queued(queued)
    }

    private func run(_ trigger: Trigger, allDisplays: Bool, gate: @escaping Gate) async -> CaptureOutcome {

        let front = await MainActor.run { NSWorkspace.shared.frontmostApplication.map { ($0.processIdentifier, $0.localizedName ?? "", $0.bundleIdentifier ?? "") } }
        guard let front else { return .skipped("No app is in front") }
        // Afterimage in front: its own windows are left out of every picture, and the rest of the screen is taken.
        let ours = front.0 == getpid()

        let exclusions = await gate().exclusions
        var hiddenBefore = await hidden(exclusions)
        if Self.offLimits(front.0, hiddenBefore.pids, ours: ours) {
            lastRefusal = hiddenBefore.reasons[front.0] ?? .app
            return Self.refused(hiddenBefore.reasons[front.0], front.1)
        }
        let frontWindow = OnScreenWindow.front(of: front.0, in: OnScreenWindow.all())

        let content: SCShareableContent
        do {
            content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false)
        } catch { return .failed(error.localizedDescription) }
        // Listing windows can take long enough for the facts to change.
        hiddenBefore = await hidden(exclusions)
        if Self.offLimits(front.0, hiddenBefore.pids, ours: ours) {
            lastRefusal = hiddenBefore.reasons[front.0] ?? .app
            return Self.refused(hiddenBefore.reasons[front.0], front.1)
        }
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
                appName: front.1, bundleId: front.2,
                // Afterimage's own title can hold what was being searched for; the picture doesn't show its window anyway.
                windowTitle: ours ? "" : frontWindow?.title ?? "", url: ours ? "" : hiddenAfter.focusedURL[front.0] ?? "",
                userName: Session.consoleUser, fullName: NSFullUserName(), host: Session.computerName, sessions: Session.loggedIn, runningApps: apps)
            engine.ingest(frame: Frame(rgba: b.data, width: UInt32(b.width), height: UInt32(b.height), stride: UInt32(b.stride), lines: lines, moment: m, displayKey: key, covers: Self.onDisplay(before.covers.filter { !omitted.contains($0.key) }.values.flatMap { $0 }, d.frame)))
            queued += 1
        }
        return queued == 0 ? .unchanged : .queued(queued)
    }

    private func hidden(_ ex: Exclusions) async -> Hidden {
        var h = Hidden()
        // Never photograph our own window: it shows other photographs.
        h.pids.insert(getpid())
        let screens: [(CGRect, Double)] = await MainActor.run {
            // Cocoa's origin is the main screen's bottom left; everything else here counts from its top left.
            let top = NSScreen.screens.first?.frame.maxY ?? 0
            return NSScreen.screens.map { (CGRect(x: $0.frame.minX, y: top - $0.frame.maxY, width: $0.frame.width, height: $0.frame.height), Double($0.backingScaleFactor)) }
        }
        func scale(at r: CGRect) -> Double { screens.max { Self.area($0.0, r) < Self.area($1.0, r) }.map { Self.area($0.0, r) > 0 ? $0.1 : 0 } ?? 0 }
        let running = await MainActor.run { NSWorkspace.shared.runningApplications.map { ($0.processIdentifier, $0.bundleIdentifier ?? "", $0.isHidden) } }
        let visible = OnScreenWindow.all()
        let drawing = Set(visible.map(\.pid))
        for (pid, bundle, _) in running {
            if let r = hideApp(ex: ex, bundleId: bundle) {
                h.pids.insert(pid)
                h.reasons[pid] = r
            } else if isBrowser(bundleId: bundle), drawing.contains(pid) {
                // Chrome describes its page only when asked. Asked only where its extension is in use: that is when
                // the page's place on screen is needed.
                if needsWaking(bundleId: bundle) || engine.extensionHeard(pid: pid) { Accessibility.wake(pid: pid) }
                let reading = Accessibility.read(pid: pid, visible: visible)
                if let r = hideBrowser(ex: ex, bundleId: bundle, windows: reading.windows, complete: reading.complete) {
                    h.pids.insert(pid)
                    h.reasons[pid] = r
                } else {
                    // Measured now, not whenever it last changed: the same is done after the picture and must agree.
                    let answered = engine.refreshPages(pid: pid)
                    let areas = reading.pages.map { PageArea(url: $0.url, rect: Rect(x: $0.frame.minX, y: $0.frame.minY, w: $0.frame.width, h: $0.frame.height), scale: scale(at: $0.frame)) }
                    let plan = planPage(ex: ex, hint: engine.pageHint(pid: pid), areas: areas, complete: reading.complete)
                    if !plan.note.isEmpty { engine.noteCapture(trigger: "page", outcome: "Couldn’t place a field: \(plan.note)") }
                    if !answered { engine.noteCapture(trigger: "page", outcome: "A browser’s extension didn’t answer; captured without its help") }
                    if let r = Self.pageHide(planned: plan.hide, answered: answered) {
                        h.pids.insert(pid)
                        h.reasons[pid] = r
                    } else {
                        if !plan.covers.isEmpty { h.covers[pid] = plan.covers }
                        h.focusedURL[pid] = reading.focusedURL
                    }
                }
            }
        }
        return h
    }

    private func pick(_ all: [SCDisplay], front: CGRect?, all wantAll: Bool) -> [SCDisplay] {
        let ids = Self.displays(all.map { ($0.displayID, $0.frame) }, front: front, all: wantAll, main: CGMainDisplayID())
        return all.filter { ids.contains($0.displayID) }
    }
    /// Every display, or the one holding most of the front window, or the main one when nothing is in front.
    static func displays(_ d: [(id: CGDirectDisplayID, frame: CGRect)], front: CGRect?, all: Bool, main: CGDirectDisplayID) -> [CGDirectDisplayID] {
        if all || d.count <= 1 { return d.map(\.id) }
        if let f = front, let best = d.max(by: { area($0.frame, f) < area($1.frame, f) }), area(best.frame, f) > 0 { return [best.id] }
        return d.map(\.id).filter { $0 == main }
    }
    /// Points on the desktop to fractions of one display; nil for what isn't on it.
    static func onDisplay(_ covers: [Cover], _ d: CGRect) -> [Cover] {
        covers.compactMap { c in
            let r = CGRect(x: c.rect.x, y: c.rect.y, width: c.rect.w, height: c.rect.h)
            guard area(r, d) > 0 else { return nil }
            return Cover(kind: c.kind, rect: Rect(x: (r.minX - d.minX) / d.width, y: (r.minY - d.minY) / d.height, w: r.width / d.width, h: r.height / d.height))
        }
    }
    private static func area(_ a: CGRect, _ b: CGRect) -> CGFloat {
        let r = a.intersection(b)
        return r.isNull ? 0 : r.width * r.height
    }
    @MainActor private static func name(_ id: CGDirectDisplayID) -> String {
        NSScreen.screens.first { ($0.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? CGDirectDisplayID) == id }?.localizedName ?? "Display"
    }
    static func refused(_ r: HideReason?, _ app: String) -> CaptureOutcome {
        r == .unreadable || r == .noAnswer ? .skipped(phrase(r, app)) : .blocked(phrase(r, app))
    }
    /// The same reasons without saying which app: which apps a person excludes is their business.
    public static func logPhrase(_ r: HideReason) -> String {
        switch r {
        case .passwordManager: "A password manager was in front"
        case .site: "An excluded site was open"
        case .privateWindow: "A private window was open"
        case .unreadable: "A browser’s address couldn’t be read"
        case .secretField: "A browser was showing a secret that couldn’t be placed"
        case .noAnswer: "A browser’s extension didn’t answer"
        case .systemPrompt: "macOS was asking for something"
        case .app: "An excluded app was in front"
        }
    }
    static func phrase(_ r: HideReason?, _ app: String) -> String {
        switch r {
        case .passwordManager: "A password manager is open"
        case .site: "A site you excluded is open in \(app)"
        case .privateWindow: "\(app) has a private window open"
        case .unreadable: "\(app)’s address couldn’t be read"
        case .secretField: "\(app) is showing a secret Afterimage can’t place"
        case .noAnswer: "\(app)’s Afterimage extension didn’t answer"
        case .systemPrompt: "macOS is asking for something"
        case .app, nil: "\(app) is excluded"
        }
    }
}
