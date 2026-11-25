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
    /// Keep a log of which window is in front and what else is open.
    public var trackActivity: Bool { didSet { save("trackActivity", trackActivity) } }
    public var allDisplays: Bool { didSet { save("allDisplays", allDisplays) } }
    /// Photograph each window on its own, covered or not, instead of the whole display. Off unless chosen.
    public var windowsSeparately: Bool { didSet { save("windowsSeparately", windowsSeparately) } }
    /// With windows photographed on their own: how often, in seconds, the whole screen is saved as well. 0: never.
    public var overviewSeconds: Int { didSet { save("overviewSeconds", overviewSeconds) } }
    public var skipSecureInput: Bool { didSet { save("skipSecureInput", skipSecureInput) } }
    public var hidePasswordManagers: Bool { didSet { save("hidePasswordManagers", hidePasswordManagers) } }
    public var hidePrivateWindows: Bool { didSet { save("hidePrivateWindows", hidePrivateWindows) } }
    public var hideTor: Bool { didSet { save("hideTor", hideTor) } }
    public var hideUnreadable: Bool { didSet { save("hideUnreadable", hideUnreadable) } }
    public var excludedApps: [String] {
        didSet {
            save("excludedApps", excludedApps)
            let added = excludedApps.filter { !oldValue.contains($0) }
            if !added.isEmpty { onAppsExcluded?(added) }
        }
    }
    /// Told which apps were just put off-limits, so pictures of them still waiting can be let go.
    // AIDEV-NOTE: privacy-critical. The recorder points this at Engine.dropQueued.
    @ObservationIgnored public var onAppsExcluded: (([String]) -> Void)?
    public var excludedSites: [String] { didSet { save("excludedSites", excludedSites) } }
    public var requireAI: Bool { didSet { save("requireAI", requireAI) } }
    public var model: String { didSet { save("model", model) } }
    public var embedModel: String { didSet { save("embedModel", embedModel) } }
    public var describe: String { didSet { save("describe", describe) } }
    public var port: Int { didSet { save("port", port) } }
    /// The machine LM Studio runs on. Empty: this Mac. Otherwise an address on the person's own network.
    public var lmHost: String { didSet { save("lmHost", lmHost) } }
    public var keepDays: Int { didSet { save("keepDays", keepDays) } }
    public var keepGB: Int { didSet { save("keepGB", keepGB) } }
    /// Let an AI agent on this Mac read the library. Only honoured in developer mode.
    public var agentAccess: Bool { didSet { save("agentAccess", agentAccess) } }
    public var agentPort: Int { didSet { save("agentPort", agentPort) } }
    /// Made the first time access is switched on; changing it cuts off whoever had the old one.
    public var agentToken: String { didSet { save("agentToken", agentToken) } }
    public var developer: Bool { didSet { save("developer", developer) } }
    /// How much a description says: "brief", "detailed", "both" or "custom". Only honoured in developer mode.
    public var describeDetail: String { didSet { save("describeDetail", describeDetail) } }
    /// The custom description: a full account or a few sentences, and the numbers it is made with.
    public var customLong: Bool { didSet { save("customLong", customLong) } }
    public var customTokens: Int { didSet { save("customTokens", customTokens) } }
    public var customChars: Int { didSet { save("customChars", customChars) } }
    public var customEdge: Int { didSet { save("customEdge", customEdge) } }
    /// How much the model thinks before it answers: "off", "low", "medium" or "high". Only honoured in developer mode.
    public var reasoning: String { didSet { save("reasoning", reasoning) } }
    /// Labs: a second, slower pass over saved pictures while nobody is at the Mac.
    public var idlePass: Bool { didSet { save("idlePass", idlePass) } }
    public var idleModel: String { didSet { save("idleModel", idleModel) } }
    public var idleReasoning: String { didSet { save("idleReasoning", idleReasoning) } }
    /// Minutes without a key or the pointer before the Mac counts as left alone.
    public var idleMinutes: Int { didSet { save("idleMinutes", idleMinutes) } }
    public var onboarded: Bool { didSet { save("onboarded", onboarded) } }
    public var recording: Bool { didSet { save("recording", recording) } }

    public init(_ d: UserDefaults = .standard) {
        self.d = d
        func get<T>(_ k: String, _ def: T) -> T { d.object(forKey: k) as? T ?? def }
        onSwitch = get("onSwitch", true)
        onLaunch = get("onLaunch", true)
        onTab = get("onTab", true)
        interval = get("interval", 120)
        trackActivity = get("trackActivity", true)
        allDisplays = get("allDisplays", false)
        windowsSeparately = get("windowsSeparately", false)
        overviewSeconds = get("overviewSeconds", 60)
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
        lmHost = get("lmHost", "")
        keepDays = get("keepDays", 30)
        keepGB = get("keepGB", 20)
        agentAccess = get("agentAccess", false)
        agentPort = get("agentPort", 4319)
        agentToken = get("agentToken", "")
        developer = get("developer", false)
        describeDetail = get("describeDetail", "brief")
        customLong = get("customLong", true)
        customTokens = get("customTokens", 2000)
        customChars = get("customChars", 8000)
        customEdge = get("customEdge", 1600)
        reasoning = get("reasoning", "off")
        idlePass = get("idlePass", false)
        idleModel = get("idleModel", "")
        idleReasoning = get("idleReasoning", "off")
        idleMinutes = get("idleMinutes", 3)
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

    public func newAgentToken() {
        // The system generator can't hand back an unfilled buffer.
        var g = SystemRandomNumberGenerator()
        agentToken = (0..<24).map { _ in String(format: "%02x", UInt8.random(in: .min ... .max, using: &g)) }.joined()
    }
    /// What to paste into a terminal to connect Claude Code. The token itself isn't in it: it is read from a file
    /// only this user can open, so it stays out of the clipboard and shell history.
    public func agentCommand(port: Int, tokenFile: String) -> String {
        "claude mcp add --transport http afterimage http://127.0.0.1:\(port)/mcp --header \"Authorization: Bearer $(cat '\(tokenFile)')\""
    }

    /// Settings for an MCP client that takes JSON (Claude Desktop, Cursor and others).
    public nonisolated static func agentConfig(port: Int, token: String) -> String {
        """
        {
          "mcpServers": {
            "afterimage": {
              "type": "http",
              "url": "http://127.0.0.1:\(port)/mcp",
              "headers": { "Authorization": "Bearer \(token)" }
            }
          }
        }
        """
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
    /// Whatever was typed, as a plain address: no scheme, no path, no name and password, and the port from its own field.
    /// AIDEV-NOTE: the engine decides whether the address is allowed (core/src/lm.rs check_host); this only tidies.
    public nonisolated static func lmURL(host: String, port: Int) -> String {
        var h = host.trimmingCharacters(in: .whitespacesAndNewlines)
        if let r = h.range(of: "://") { h = String(h[r.upperBound...]) }
        if let i = h.firstIndex(where: { "/?#".contains($0) }) { h = String(h[..<i]) }
        if let i = h.lastIndex(of: "@") { h = String(h[h.index(after: i)...]) }
        if h.hasPrefix("[") {
            if let i = h.firstIndex(of: "]") { h = String(h[...i]) }
        } else if h.filter({ $0 == ":" }).count > 1 {
            h = "[\(h)]"
        } else if let i = h.firstIndex(of: ":") {
            h = String(h[..<i])
        }
        return "http://\(h.isEmpty ? "127.0.0.1" : h):\(port)"
    }
    /// LM Studio is on another machine: what it is asked crosses the network.
    public var modelIsElsewhere: Bool { !isThisMac(base: Self.lmURL(host: lmHost, port: port)) }
    /// The detail a description is made with. Outside developer mode the choice is out of sight, so it doesn't count.
    public nonisolated static func detail(_ choice: String, developer: Bool) -> DescribeDetail {
        guard developer else { return .brief }
        return switch choice {
        case "detailed": .detailed
        case "both": .both
        case "custom": .custom
        default: .brief
        }
    }
    /// Whether the Mac has been left alone long enough for the slow pass.
    public nonisolated static func leftAlone(idleSeconds: Double, minutes: Int, enabled: Bool) -> Bool {
        enabled && idleSeconds >= Double(max(1, minutes)) * 60
    }
    public var config: Config {
        Config(lmUrl: Self.lmURL(host: lmHost, port: port), model: model, embedModel: embedModel, requireAi: requireAI,
               describe: describe == "always" ? .always : describe == "never" ? .never : .pluggedIn,
               maxAgeDays: UInt32(max(0, keepDays)), maxBytes: UInt64(max(0, keepGB)) * 1_000_000_000, unloadAfterSecs: 300, developer: developer,
               detail: Self.detail(describeDetail, developer: developer), customLong: customLong,
               customTokens: UInt32(clamping: customTokens), customChars: UInt32(clamping: customChars), customEdge: UInt32(clamping: customEdge),
               reasoning: developer ? reasoning : "off",
               idleModel: idlePass ? idleModel : "", idleReasoning: idleReasoning)
    }
}
