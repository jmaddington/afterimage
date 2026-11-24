import AfterimageEngine
import AppKit
import ApplicationServices

/// The app wouldn't say. Never treated as "there is nothing there".
public struct Unreadable: Error {}

/// One element of an accessibility tree.
public protocol ScreenElement: Equatable {
    /// nil means the element genuinely has no such attribute; anything else that goes wrong throws.
    /// Elements come back as `Self`, lists of elements as `[Self]`, points and sizes as their CG types.
    func read(_ attribute: String) throws -> Any?
}

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
        let app = LiveElement(AXUIElementCreateApplication(pid))
        let drawn = visible.filter { $0.pid == pid && $0.alpha > 0 && $0.bounds.width > 160 && $0.bounds.height > 160 }.map(\.bounds)
        return try? read(app: app, drawn: drawn, deadline: Date().addingTimeInterval(budget))
    }

    static func read<E: ScreenElement>(app: E, drawn: [CGRect], deadline: Date) throws -> Reading {
        var elements = try list(app, kAXWindowsAttribute)
        // Extension popups and sheets can hang off the app rather than its window list.
        for c in try list(app, kAXChildrenAttribute) where ["AXWindow", "AXSheet", "AXPopover", "AXGroup"].contains(try role(c)) {
            if !elements.contains(c) { elements.append(c) }
        }
        var seen: [(element: E, window: BrowserWindow, frame: CGRect?)] = []
        for w in elements {
            guard Date() < deadline else { throw Unreadable() }
            if try w.read(kAXMinimizedAttribute) as? Bool == true { continue }
            seen.append((w, try window(w, deadline: deadline), try frame(w)))
        }
        // A window the server is drawing that accessibility didn't show us could hold anything.
        var frames = seen.compactMap(\.frame)
        for d in drawn {
            guard let i = frames.firstIndex(where: { same($0, d) }) else { throw Unreadable() }
            frames.remove(at: i)
        }
        // Metadata only, so a failure here costs nothing; but never report a page that wasn't inspected.
        let focused = try? app.read(kAXFocusedWindowAttribute) as? E
        return Reading(windows: seen.map(\.window), focusedURL: seen.first { $0.element == focused }?.window.urls.first)
    }

    private static func window<E: ScreenElement>(_ w: E, deadline: Date) throws -> BrowserWindow {
        var b = BrowserWindow(title: try w.read(kAXTitleAttribute) as? String, urls: [], controlLabels: [], fieldDescriptions: [])
        if let d = try w.read(kAXDocumentAttribute) as? String, d.contains(":") { b.urls.append(d) }
        // Breadth-first over the window's furniture. Web content itself is never entered.
        var queue = [w]
        var i = 0
        while i < queue.count {
            guard Date() < deadline, i < 900 else { throw Unreadable() }
            let e = queue[i]
            i += 1
            switch try role(e) {
            case "AXWebArea":
                let u = try e.read(kAXURLAttribute)
                let s = (u as? URL)?.absoluteString ?? (u as? String) ?? "unknown:"
                if !b.urls.contains(s) { b.urls.append(s) }
                continue
            case "AXButton", "AXPopUpButton", "AXMenuButton", "AXStaticText", "AXImage":
                for k in [kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute] {
                    if let s = try e.read(k) as? String, !s.isEmpty, s.count < 40 { b.controlLabels.append(s) }
                }
            case "AXTextField", "AXComboBox", "AXSearchField":
                if let s = try e.read(kAXDescriptionAttribute) as? String, !s.isEmpty { b.fieldDescriptions.append(s) }
                continue
            default: break
            }
            queue.append(contentsOf: try list(e, kAXChildrenAttribute))
        }
        return b
    }

    private static func role<E: ScreenElement>(_ e: E) throws -> String {
        guard let r = try e.read(kAXRoleAttribute) as? String else { throw Unreadable() }
        return r
    }
    private static func list<E: ScreenElement>(_ e: E, _ name: String) throws -> [E] {
        guard let v = try e.read(name) else { return [] }
        guard let l = v as? [E] else { throw Unreadable() }
        return l
    }
    private static func frame<E: ScreenElement>(_ e: E) throws -> CGRect? {
        guard let p = try e.read(kAXPositionAttribute) as? CGPoint, let s = try e.read(kAXSizeAttribute) as? CGSize else { return nil }
        return CGRect(origin: p, size: s)
    }
    private static func same(_ a: CGRect, _ b: CGRect) -> Bool {
        abs(a.minX - b.minX) <= 2 && abs(a.minY - b.minY) <= 2 && abs(a.width - b.width) <= 2 && abs(a.height - b.height) <= 2
    }
}

struct LiveElement: ScreenElement {
    let e: AXUIElement
    init(_ e: AXUIElement) {
        self.e = e
        AXUIElementSetMessagingTimeout(e, 0.25)
    }
    static func == (l: LiveElement, r: LiveElement) -> Bool { CFEqual(l.e, r.e) }

    func read(_ attribute: String) throws -> Any? {
        var v: CFTypeRef?
        switch AXUIElementCopyAttributeValue(e, attribute as CFString, &v) {
        case .success: break
        case .noValue, .attributeUnsupported: return nil
        default: throw Unreadable()
        }
        guard let v else { return nil }
        if CFGetTypeID(v) == AXUIElementGetTypeID() { return LiveElement(v as! AXUIElement) }
        if CFGetTypeID(v) == AXValueGetTypeID() {
            var p = CGPoint.zero, s = CGSize.zero
            if AXValueGetValue(v as! AXValue, .cgPoint, &p) { return p }
            if AXValueGetValue(v as! AXValue, .cgSize, &s) { return s }
            return nil
        }
        if let l = v as? [AnyObject], l.allSatisfy({ CFGetTypeID($0) == AXUIElementGetTypeID() }) { return l.map { LiveElement($0 as! AXUIElement) } }
        return v
    }
}
