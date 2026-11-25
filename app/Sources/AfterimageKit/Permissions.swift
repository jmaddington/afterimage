import AppKit
import ApplicationServices

public enum Permissions {
    /// AIDEV-NOTE: never probe with SCShareableContent: it can itself raise the system prompt.
    public static var screen: Bool { CGPreflightScreenCaptureAccess() }
    public static var accessibility: Bool { AXIsProcessTrusted() }

    /// A permission that isn't there, and what its absence is costing.
    public enum Missing: Sendable, Equatable {
        case screen, accessibility
        public var name: String { self == .screen ? "Screen Recording" : "Accessibility" }
        public var cost: String {
            self == .screen ? "Screen Recording isn’t allowed, so nothing is being captured" : "Accessibility isn’t allowed, so browsers aren’t being captured"
        }
    }
    /// `needsAddresses`: browsers are left out unless their page address can be read, which takes Accessibility.
    public static func missing(screen: Bool, accessibility: Bool, needsAddresses: Bool) -> [Missing] {
        (screen ? [] : [.screen]) + (accessibility || !needsAddresses ? [] : [.accessibility])
    }

    @discardableResult public static func requestScreen() -> Bool { CGRequestScreenCaptureAccess() }
    public static func requestAccessibility() {
        _ = AXIsProcessTrustedWithOptions(["AXTrustedCheckOptionPrompt": true] as CFDictionary)
    }
    public static func openScreenSettings() { open("Privacy_ScreenCapture") }
    public static func openAccessibilitySettings() { open("Privacy_Accessibility") }
    private static func open(_ pane: String) {
        if let u = URL(string: "x-apple.systempreferences:com.apple.preference.security?\(pane)") { NSWorkspace.shared.open(u) }
    }

    /// Screen Recording only takes effect in a new process. Wait for this one to go, then reopen.
    /// Approach from Dayflow (MIT, (c) 2025 Jerry Liu); see NOTICE.
    @MainActor public static func relaunch() {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/sh")
        p.arguments = ["-c", "for i in $(seq 1 150); do kill -0 \(getpid()) 2>/dev/null || { sleep 0.5; /usr/bin/open \"$1\"; exit 0; }; sleep 0.2; done", "relaunch", Bundle.main.bundlePath]
        try? p.run()
        NSApp.terminate(nil)
    }
}
