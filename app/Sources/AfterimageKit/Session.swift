import AppKit
import Carbon.HIToolbox
import IOKit.ps
import SystemConfiguration

/// Who is here, and is the Mac in a state worth recording?
public enum Session {
    public static var isLocked: Bool {
        guard let d = CGSessionCopyCurrentDictionary() as? [String: Any] else { return true }
        if d["CGSSessionScreenIsLocked"] as? Bool == true { return true }
        return d[kCGSessionOnConsoleKey as String] as? Bool == false
    }
    /// A password field (or a terminal's Secure Keyboard Entry) is capturing keystrokes.
    public static var isSecureInput: Bool { IsSecureEventInputEnabled() }
    public static var idleSeconds: Double {
        CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: CGEventType(rawValue: ~0)!)
    }
    /// Whole seconds since the last key or pointer event, for a picture's record.
    public static var idleWhole: UInt32 { whole(idleSeconds) }
    static func whole(_ seconds: Double) -> UInt32 { seconds.isFinite ? UInt32(min(max(seconds, 0), 4_000_000_000)) : 0 }
    public static var onPower: Bool {
        guard let s = IOPSCopyPowerSourcesInfo()?.takeRetainedValue(),
              let t = IOPSGetProvidingPowerSourceType(s)?.takeUnretainedValue() as String? else { return true }
        return t == kIOPMACPowerKey
    }
    public static var calm: Bool {
        let p = ProcessInfo.processInfo
        return !p.isLowPowerModeEnabled && (p.thermalState == .nominal || p.thermalState == .fair)
    }
    public static let computerName: String = (SCDynamicStoreCopyComputerName(nil, nil) as String?) ?? ""
    public static var consoleUser: String { (SCDynamicStoreCopyConsoleUser(nil, nil, nil) as String?) ?? NSUserName() }

    /// Everyone with a login session, e.g. "jr (console)".
    public static var loggedIn: [String] {
        var out: [String] = []
        setutxent()
        defer { endutxent() }
        while let e = getutxent()?.pointee {
            guard e.ut_type == Int16(USER_PROCESS) else { continue }
            let user = withUnsafeBytes(of: e.ut_user) { String(decoding: $0.prefix { $0 != 0 }, as: UTF8.self) }
            let line = withUnsafeBytes(of: e.ut_line) { String(decoding: $0.prefix { $0 != 0 }, as: UTF8.self) }
            let s = line == "console" ? "\(user) (console)" : "\(user) (terminal)"
            if !out.contains(s) { out.append(s) }
        }
        return out
    }
}

/// What the window server says is on screen. Needs no permission for owners and bounds.
public struct OnScreenWindow: Sendable {
    public let id: CGWindowID, pid: pid_t, title: String, layer: Int, bounds: CGRect, alpha: Double
    /// The window a person would say they're in. Suggestion lists and hover cards sit in front of it.
    public static func front(of pid: pid_t, in list: [OnScreenWindow]) -> OnScreenWindow? {
        let mine = list.filter { $0.pid == pid && $0.layer == 0 && $0.alpha > 0 }
        let real = mine.filter { $0.bounds.width > 160 && $0.bounds.height > 160 }
        guard let top = real.first else { return mine.first }
        guard top.title.isEmpty else { return top }
        // Only a popup yields to what's behind it, and only to the window it belongs to.
        return real.first { !$0.title.isEmpty && $0.bounds.contains(top.bounds) } ?? top
    }
    public static func all() -> [OnScreenWindow] {
        let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
        return list.compactMap { d in
            guard let id = d[kCGWindowNumber as String] as? CGWindowID, let pid = d[kCGWindowOwnerPID as String] as? pid_t else { return nil }
            var r = CGRect.zero
            if let b = d[kCGWindowBounds as String] as? NSDictionary { CGRectMakeWithDictionaryRepresentation(b, &r) }
            return OnScreenWindow(id: id, pid: pid, title: d[kCGWindowName as String] as? String ?? "", layer: d[kCGWindowLayer as String] as? Int ?? 0, bounds: r, alpha: d[kCGWindowAlpha as String] as? Double ?? 1)
        }
    }
}
