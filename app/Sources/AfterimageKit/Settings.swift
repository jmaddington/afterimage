import AfterimageEngine
import Foundation
import Observation

/// Everything a person can choose, persisted in UserDefaults.
@MainActor @Observable public final class AppSettings {
    private let d: UserDefaults
    public var changed: (() -> Void)?

    public var onSwitch: Bool { didSet { save("onSwitch", onSwitch) } }
    public var onLaunch: Bool { didSet { save("onLaunch", onLaunch) } }
    public var onTab: Bool { didSet { save("onTab", onTab) } }
    /// Seconds between timed captures; 0 is off.
    public var interval: Int { didSet { save("interval", interval) } }
    public var allDisplays: Bool { didSet { save("allDisplays", allDisplays) } }
    public var skipSecureInput: Bool { didSet { save("skipSecureInput", skipSecureInput) } }
    public var hidePasswordManagers: Bool { didSet { save("hidePasswordManagers", hidePasswordManagers) } }
    public var hidePrivateWindows: Bool { didSet { save("hidePrivateWindows", hidePrivateWindows) } }
    public var hideTor: Bool { didSet { save("hideTor", hideTor) } }
    public var hideUnreadable: Bool { didSet { save("hideUnreadable", hideUnreadable) } }
    public var excludedApps: [String] { didSet { save("excludedApps", excludedApps) } }
    public var excludedSites: [String] { didSet { save("excludedSites", excludedSites) } }
    public var requireAI: Bool { didSet { save("requireAI", requireAI) } }
    public var model: String { didSet { save("model", model) } }
    public var embedModel: String { didSet { save("embedModel", embedModel) } }
    public var describe: String { didSet { save("describe", describe) } }
    public var port: Int { didSet { save("port", port) } }
    public var keepDays: Int { didSet { save("keepDays", keepDays) } }
    public var keepGB: Int { didSet { save("keepGB", keepGB) } }
    public var developer: Bool { didSet { save("developer", developer) } }
    public var onboarded: Bool { didSet { save("onboarded", onboarded) } }
    public var recording: Bool { didSet { save("recording", recording) } }

    public init(_ d: UserDefaults = .standard) {
        self.d = d
        func get<T>(_ k: String, _ def: T) -> T { d.object(forKey: k) as? T ?? def }
        onSwitch = get("onSwitch", true)
        onLaunch = get("onLaunch", true)
        onTab = get("onTab", true)
        interval = get("interval", 120)
        allDisplays = get("allDisplays", false)
        skipSecureInput = get("skipSecureInput", true)
        hidePasswordManagers = get("hidePasswordManagers", true)
        hidePrivateWindows = get("hidePrivateWindows", true)
        hideTor = get("hideTor", true)
        hideUnreadable = get("hideUnreadable", true)
        excludedApps = get("excludedApps", [])
        excludedSites = get("excludedSites", [])
        requireAI = get("requireAI", true)
        model = get("model", "gemma-4-26b-a4b-it-qat-mlx")
        embedModel = get("embedModel", "text-embedding-nomic-embed-text-v1.5")
        describe = get("describe", "pluggedIn")
        port = get("port", 1234)
        keepDays = get("keepDays", 30)
        keepGB = get("keepGB", 20)
        developer = get("developer", false)
        onboarded = get("onboarded", false)
        recording = get("recording", true)
    }

    private func save(_ k: String, _ v: Any) {
        d.set(v, forKey: k)
        changed?()
    }

    /// Pressing Start means start, whatever was chosen before.
    public func finishOnboarding() {
        recording = true
        onboarded = true
    }

    public var exclusions: Exclusions {
        Exclusions(apps: excludedApps, sites: excludedSites, passwordManagers: hidePasswordManagers, privateWindows: hidePrivateWindows, unreadableBrowsers: hideUnreadable, tor: hideTor)
    }
    public var triggers: Set<Trigger> {
        var t: Set<Trigger> = [.manual]
        if onSwitch { t.formUnion([.app, .window, .space]) }
        if onLaunch { t.insert(.launch) }
        if onTab { t.insert(.tab) }
        if interval > 0 { t.insert(.interval) }
        return t
    }
    public var config: Config {
        Config(lmUrl: "http://127.0.0.1:\(port)", model: model, embedModel: embedModel, requireAi: requireAI,
               describe: describe == "always" ? .always : describe == "never" ? .never : .pluggedIn,
               maxAgeDays: UInt32(max(0, keepDays)), maxBytes: UInt64(max(0, keepGB)) * 1_000_000_000, unloadAfterSecs: 300, developer: developer)
    }
}
