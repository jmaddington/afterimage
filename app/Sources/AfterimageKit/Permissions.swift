import AppKit
import ApplicationServices

public enum Permissions {
    /// AIDEV-NOTE: never probe with SCShareableContent: it can itself raise the system prompt.
    public static var screen: Bool { CGPreflightScreenCaptureAccess() }
    public static var accessibility: Bool { AXIsProcessTrusted() }

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
