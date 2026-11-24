import AfterimageEngine
import AppKit
import ApplicationServices

/// Reads page addresses and private-window markers from a browser's accessibility tree.
/// AIDEV-NOTE: AX calls block on the target app; every read has a short timeout and an overall deadline.
/// AIDEV-NOTE: privacy-critical: anything incomplete returns nil, which the engine treats as "leave the browser out".
public enum Accessibility {
    public static var isTrusted: Bool { AXIsProcessTrusted() }

    public struct Reading: Sendable {
        public var windows: [BrowserWindow]
        public var focusedURL: String?
    }

    public static func read(pid: pid_t, visible: [OnScreenWindow], budget: TimeInterval = 1.0) -> Reading? {
        guard isTrusted, pid != getpid() else { return nil }
        let app = AXUIElementCreateApplication(pid)
        AXUIElementSetMessagingTimeout(app, 0.25)
        let deadline = Date().addingTimeInterval(budget)
        var elements = (attr(app, kAXWindowsAttribute) as? [AXUIElement]) ?? []
        // Extension popups and sheets can hang off the app rather than its window list.
        for c in (attr(app, kAXChildrenAttribute) as? [AXUIElement]) ?? [] {
            let role = attr(c, kAXRoleAttribute) as? String ?? ""
            if ["AXWindow", "AXSheet", "AXPopover", "AXGroup"].contains(role), !elements.contains(where: { CFEqual($0, c) }) { elements.append(c) }
        }
        var out: [BrowserWindow] = []
        for w in elements {
            guard Date() < deadline else { return nil }
            AXUIElementSetMessagingTimeout(w, 0.25)
            if attr(w, kAXMinimizedAttribute) as? Bool == true { continue }
            guard let b = window(w, deadline: deadline) else { return nil }
            out.append(b)
        }
        // A window the server is drawing that accessibility didn't show us could hold anything.
        let drawn = visible.filter { $0.pid == pid && $0.layer == 0 && $0.bounds.width > 160 && $0.bounds.height > 160 }
        if drawn.count > out.count { return nil }
        var url: String?
        if let f = attr(app, kAXFocusedWindowAttribute), CFGetTypeID(f) == AXUIElementGetTypeID() {
            url = window(f as! AXUIElement, deadline: deadline)?.urls.first
        }
        return Reading(windows: out, focusedURL: url)
    }

    private static func window(_ w: AXUIElement, deadline: Date) -> BrowserWindow? {
        var b = BrowserWindow(title: attr(w, kAXTitleAttribute) as? String, urls: [], controlLabels: [], fieldDescriptions: [])
        if let d = attr(w, kAXDocumentAttribute) as? String, d.contains(":") { b.urls.append(d) }
        // Breadth-first over the window's furniture. Web content itself is never entered.
        var queue = [w]
        var i = 0
        while i < queue.count {
            guard Date() < deadline, i < 900 else { return nil }
            let e = queue[i]
            i += 1
            let role = attr(e, kAXRoleAttribute) as? String ?? ""
            switch role {
            case "AXWebArea":
                let u = attr(e, kAXURLAttribute)
                if let s = (u as? URL)?.absoluteString ?? (u as? String) {
                    if !b.urls.contains(s) { b.urls.append(s) }
                } else {
                    b.urls.append("unknown:")
                }
                continue
            case "AXButton", "AXPopUpButton", "AXMenuButton", "AXStaticText", "AXImage":
                for k in [kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute] {
                    if let s = attr(e, k) as? String, !s.isEmpty, s.count < 40 { b.controlLabels.append(s) }
                }
            case "AXTextField", "AXComboBox", "AXSearchField":
                if let s = attr(e, kAXDescriptionAttribute) as? String, !s.isEmpty { b.fieldDescriptions.append(s) }
                continue
            default: break
            }
            queue.append(contentsOf: (attr(e, kAXChildrenAttribute) as? [AXUIElement]) ?? [])
        }
        return b
    }

    private static func attr(_ e: AXUIElement, _ name: String) -> CFTypeRef? {
        var v: CFTypeRef?
        return AXUIElementCopyAttributeValue(e, name as CFString, &v) == .success ? v : nil
    }
}
