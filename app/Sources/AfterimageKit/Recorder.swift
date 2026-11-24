import AfterimageEngine
import AppKit
import Observation

/// Watches the desktop and decides when to look. Owns every reason not to.
@MainActor @Observable public final class Recorder {
    public enum State: Equatable {
        case off, needsPermission, paused(until: Date?), waiting(String), watching
    }
    public private(set) var state: State = .off
    public private(set) var lastCapture: Date?
    public private(set) var lastNote = ""
    public private(set) var flash = 0

    private let settings: AppSettings
    private let engine: Engine
    private let capturer: Capturer
    private var policy = TriggerPolicy()
    private var timer: Timer?
    private var observers: [(NotificationCenter, NSObjectProtocol)] = []
    private var tracker = FrontTracker()
    /// The window the last successful capture was of: its title has passed every privacy check.
    /// AIDEV-NOTE: privacy-critical. Bound to the window and the settings it passed under; withdrawn by any capture that didn't succeed.
    private var approved: Approval?
    private struct Approval: Equatable { let window: CGWindowID, title: String, rules: Exclusions }
    /// Samples reach the engine in the order they were taken.
    private let activityQueue = DispatchQueue(label: "afterimage.activity", qos: .utility)
    private var lastActivity = Date.distantPast
    private var logging = false
    private var pausedUntil: Date?
    private var asleep = false
    private var resumeAfter = Date.distantPast
    private var backoffUntil = Date.distantPast
    private var failures = 0
    private var capturing = false
    private var ready = false
    private var lastWindowPoll = Date.distantPast

    public init(settings: AppSettings, engine: Engine) {
        self.settings = settings
        self.engine = engine
        capturer = Capturer(engine: engine)
    }

    public func start() {
        guard timer == nil else { return }
        let ws = NSWorkspace.shared.notificationCenter
        on(ws, NSWorkspace.didActivateApplicationNotification) { $0.note(.app) }
        on(ws, NSWorkspace.didLaunchApplicationNotification) { $0.note(.launch) }
        on(ws, NSWorkspace.activeSpaceDidChangeNotification) { $0.note(.space) }
        on(ws, NSWorkspace.screensDidSleepNotification) { $0.asleep = true }
        on(ws, NSWorkspace.willSleepNotification) { $0.asleep = true }
        on(ws, NSWorkspace.sessionDidResignActiveNotification) { $0.asleep = true }
        on(ws, NSWorkspace.screensDidWakeNotification) { $0.woke() }
        on(ws, NSWorkspace.didWakeNotification) { $0.woke() }
        on(ws, NSWorkspace.sessionDidBecomeActiveNotification) { $0.woke() }
        on(DistributedNotificationCenter.default(), .init("com.apple.screenIsUnlocked")) { $0.woke() }
        on(DistributedNotificationCenter.default(), .init("com.apple.screenIsLocked")) { $0.policy.clear() }
        let t = Timer(timeInterval: 0.25, repeats: true) { [weak self] _ in MainActor.assumeIsolated { self?.tick() } }
        t.tolerance = 0.08
        RunLoop.main.add(t, forMode: .common)
        timer = t
        Task { [capturer] in
            await capturer.warmUp()
            ready = true
        }
    }

    public func stop() {
        timer?.invalidate()
        timer = nil
        for (c, o) in observers { c.removeObserver(o) }
        observers = []
    }

    public func pause(for seconds: TimeInterval?) {
        pausedUntil = seconds.map { Date().addingTimeInterval($0) } ?? .distantFuture
        policy.clear()
        tick()
    }
    public func resume() {
        pausedUntil = nil
        settings.recording = true
        policy.clear()
        tick()
    }
    public func captureNow() { policy.event(.manual, at: now) }

    private var now: TimeInterval { ProcessInfo.processInfo.systemUptime }
    private func note(_ t: Trigger) { if state == .watching { policy.event(t, at: now) } }

    private func on(_ c: NotificationCenter, _ n: Notification.Name, _ f: @escaping @MainActor (Recorder) -> Void) {
        observers.append((c, c.addObserver(forName: n, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { if let self { f(self) } }
        }))
    }

    private func woke() {
        asleep = false
        // Bursts of ScreenCaptureKit calls right after wake can make macOS ask for permission again.
        resumeAfter = Date().addingTimeInterval(5)
        policy.clear()
    }

    private func current() -> State {
        if !settings.recording { return .off }
        if let p = pausedUntil {
            if p > Date() { return .paused(until: p == .distantFuture ? nil : p) }
            pausedUntil = nil
        }
        if !Permissions.screen { return .needsPermission }
        if asleep || Session.isLocked { return .waiting("Screen locked") }
        if !ready { return .waiting("Getting ready") }
        if Date() < resumeAfter { return .waiting("Just woke up") }
        if Date() < backoffUntil { return .waiting("Capture isn’t available") }
        if settings.skipSecureInput && Session.isSecureInput { return .waiting("A password is being typed") }
        return .watching
    }

    /// Asked again right around each capture: pausing, a password field or a new exclusion counts immediately.
    private func gate() -> (blocked: String?, exclusions: Exclusions) {
        let why: String? = switch current() {
        case .watching: nil
        case .off: "Recording is off"
        case .needsPermission: "Screen Recording isn’t allowed"
        case .paused: "Paused"
        case .waiting(let w): w
        }
        return (why, settings.exclusions)
    }

    private func tick() {
        let s = current()
        if s != state {
            // What happened before a pause isn't a reason to look after it.
            if state == .watching { policy.clear(); stopActivity() }
            state = s
        }
        guard s == .watching, !capturing else { return }
        policy.enabled = settings.triggers
        policy.interval = TimeInterval(settings.interval)
        pollWindow()
        logActivity()
        guard let t = policy.due(at: now) else { return }
        // A timed look at a Mac nobody is using is a duplicate.
        if t == .interval && Session.idleSeconds > max(60, Double(settings.interval)) { return }
        policy.captured(at: now)
        capturing = true
        let all = settings.allDisplays, generation = policy.generation
        let shown = frontNow()
        engine.setConditions(onPower: Session.onPower, calm: Session.calm)
        Task {
            let (r, refused) = await capturer.capture(t, allDisplays: all) { self.gate() }
            capturing = false
            if settings.developer {
                let refusal = refused.map(Capturer.logPhrase)
                let what: String = switch r {
                case .queued(let n): n == 1 ? "Taken" : "Taken (\(n) displays)"
                case .unchanged: "Nothing had changed"
                case .skipped(let why): "Skipped, will retry: \(refusal ?? why)"
                case .blocked(let why): "Not taken: \(refusal ?? why)"
                case .failed(let why): "Failed: \(why)"
                }
                Task.detached { [engine] in engine.noteCapture(trigger: t.rawValue, outcome: what) }
            }
            switch r {
            case .queued:
                approved = shown == frontNow() ? shown : nil
                failures = 0
                lastCapture = Date()
                lastNote = ""
                flash += 1
            case .unchanged:
                approved = shown == frontNow() ? shown : nil
                failures = 0
                lastNote = "Nothing has changed"
            case .skipped(let why):
                approved = nil
                lastNote = why
                if current() == .watching { policy.missed(t, at: now, since: generation) }
            case .blocked(let why):
                approved = nil
                lastNote = why
            case .failed(let why):
                approved = nil
                failures += 1
                lastNote = why
                backoffUntil = Date().addingTimeInterval([5, 30, 60, 120, 300][min(failures - 1, 4)])
                NSLog("capture failed: %@ [AIMG-CAPFAIL]", why)
            }
        }
    }

    private func pollWindow() {
        guard Date().timeIntervalSince(lastWindowPoll) >= 0.5 else { return }
        lastWindowPoll = Date()
        guard let app = NSWorkspace.shared.frontmostApplication else { return }
        let w = OnScreenWindow.front(of: app.processIdentifier, in: OnScreenWindow.all())
        if let t = tracker.see(pid: app.processIdentifier, window: w) { policy.event(t, at: now) }
    }

    /// The window in front and the settings in force, as an approval would record them.
    private func frontNow() -> Approval? {
        guard let app = NSWorkspace.shared.frontmostApplication, let w = OnScreenWindow.front(of: app.processIdentifier, in: OnScreenWindow.all()) else { return nil }
        return Approval(window: w.id, title: TriggerPolicy.stableTitle(w.title), rules: settings.exclusions)
    }

    /// Once a second while watching: which window is in front and what else is open. The engine applies the privacy rules.
    private func logActivity() {
        guard settings.trackActivity else { return stopActivity() }
        guard Date().timeIntervalSince(lastActivity) >= 1 else { return }
        lastActivity = Date()
        // Nobody at the keyboard for two minutes: the stretch ends where they left it.
        if Session.idleSeconds > 120 { return stopActivity() }
        guard let app = NSWorkspace.shared.frontmostApplication, app.processIdentifier != getpid() else { return stopActivity() }
        let all = OnScreenWindow.all().filter { $0.layer == 0 && $0.alpha > 0 && $0.bounds.width > 160 && $0.bounds.height > 160 }
        let names = Dictionary(NSWorkspace.shared.runningApplications.map { ($0.processIdentifier, ($0.localizedName ?? "", $0.bundleIdentifier ?? "")) }, uniquingKeysWith: { a, _ in a })
        let front = OnScreenWindow.front(of: app.processIdentifier, in: all)
        func open(_ w: OnScreenWindow) -> OpenWindow { OpenWindow(app: names[w.pid]?.0 ?? "", bundleId: names[w.pid]?.1 ?? "", title: w.title) }
        let sample = ActivitySample(
            at: Int64(Date().timeIntervalSince1970 * 1000),
            front: OpenWindow(app: app.localizedName ?? "", bundleId: app.bundleIdentifier ?? "", title: front?.title ?? ""),
            titleOk: front.map { approved == Approval(window: $0.id, title: TriggerPolicy.stableTitle($0.title), rules: settings.exclusions) } ?? false,
            others: all.filter { $0.id != front?.id && $0.pid != getpid() }.map(open))
        logging = true
        let ex = settings.exclusions
        activityQueue.async { [engine] in
            do { try engine.noteActivity(ex: ex, sample: sample) } catch { NSLog("activity not recorded: %@ [AIMG-ACTIVITY]", error.localizedDescription) }
        }
    }

    private func stopActivity() {
        guard logging else { return }
        logging = false
        activityQueue.async { [engine] in engine.endActivity() }
    }
}
