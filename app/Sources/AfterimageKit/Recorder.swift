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
    private var front: (pid: pid_t, window: CGWindowID, title: String)?
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
        let t = Timer(timeInterval: 0.5, repeats: true) { [weak self] _ in MainActor.assumeIsolated { self?.tick() } }
        t.tolerance = 0.15
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

    private func tick() {
        let s = current()
        if s != state { state = s }
        guard s == .watching, !capturing else { return }
        policy.enabled = settings.triggers
        policy.interval = TimeInterval(settings.interval)
        pollWindow()
        guard let t = policy.due(at: now) else { return }
        // A timed look at a Mac nobody is using is a duplicate.
        if t == .interval && Session.idleSeconds > max(60, Double(settings.interval)) { return }
        policy.captured(at: now)
        capturing = true
        let ex = settings.exclusions, all = settings.allDisplays
        engine.setConditions(onPower: Session.onPower, calm: Session.calm)
        Task {
            let r = await capturer.capture(t, exclusions: ex, allDisplays: all)
            capturing = false
            switch r {
            case .queued:
                failures = 0
                lastCapture = Date()
                lastNote = ""
                flash += 1
            case .unchanged:
                failures = 0
                lastNote = "Nothing has changed"
            case .skipped(let why):
                lastNote = why
            case .failed(let why):
                failures += 1
                lastNote = why
                backoffUntil = Date().addingTimeInterval([5, 30, 60, 120, 300][min(failures - 1, 4)])
                NSLog("capture failed: %@ [AIMG-CAPFAIL]", why)
            }
        }
    }

    /// No public "tab changed" event exists; the front window's id and title are the tell.
    private func pollWindow() {
        guard Date().timeIntervalSince(lastWindowPoll) >= 1 else { return }
        lastWindowPoll = Date()
        guard let app = NSWorkspace.shared.frontmostApplication,
              let w = OnScreenWindow.all().first(where: { $0.pid == app.processIdentifier && $0.layer == 0 }) else { return }
        let title = TriggerPolicy.stableTitle(w.title)
        if let f = front, f.pid == app.processIdentifier {
            if f.window != w.id { policy.event(.window, at: now) } else if f.title != title { policy.event(.tab, at: now) }
        }
        front = (app.processIdentifier, w.id, title)
    }
}
