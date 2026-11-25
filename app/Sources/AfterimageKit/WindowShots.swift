import AfterimageEngine
import CoreGraphics
import Foundation

/// The parts of "Capture windows separately" that need no screen: what the window server lists, turned into what the
/// engine's policy judges, and what to do with each picture that comes back.
/// AIDEV-NOTE: privacy-critical. The rules are in core/src/windows.rs; nothing here may let through what they refuse.
public enum WindowShots {
    /// Which app a process is, as far as anyone can tell.
    public struct Owner: Sendable, Equatable {
        public let bundleId: String
        /// An ordinary app, not a helper or an agent.
        public let regular: Bool
        public init(bundleId: String, regular: Bool) {
            self.bundleId = bundleId
            self.regular = regular
        }
    }

    /// The most pictures one look may queue: the window in front and a few behind it. The engine's queue holds twelve,
    /// so two looks fit while the model is busy.
    static let maxQueued = 5

    /// `list` is the window server's, front to back. `shareable`: the windows ScreenCaptureKit offers as on screen.
    static func seen(_ list: [OnScreenWindow], owners: [pid_t: Owner], shareable: Set<CGWindowID>) -> [WindowSeen] {
        list.map { w in
            let o = owners[w.pid]
            return WindowSeen(id: w.id, pid: w.pid, bundleId: o?.bundleId ?? "", regular: o?.regular ?? false, layer: Int32(clamping: w.layer), alpha: w.alpha,
                              onScreen: shareable.contains(w.id), bounds: Rect(x: w.bounds.minX, y: w.bounds.minY, w: w.bounds.width, h: w.bounds.height))
        }
    }

    /// The engine's verdict on every listed window. `hidden`: the apps the capture check found off-limits.
    static func plan(_ ex: Exclusions, front: (pid: pid_t, bundleId: String), list: [OnScreenWindow], owners: [pid_t: Owner], shareable: Set<CGWindowID>,
                     hidden: Set<pid_t>, own: pid_t, desktop: [CGRect]) -> [WindowChoice] {
        // `desktop`: the displays being captured. A window elsewhere, or parked off all of them, isn't taken.
        let scene = WindowScene(ownPid: own, frontPid: front.pid, frontBundleId: front.bundleId, frontWindow: OnScreenWindow.front(of: front.pid, in: list)?.id, hidden: hidden.sorted(),
                                desktop: desktop.map { Rect(x: $0.minX, y: $0.minY, w: $0.width, h: $0.height) })
        return planWindows(ex: ex, scene: scene, windows: seen(list, owners: owners, shareable: shareable))
    }

    /// Names one window for one run of the app. The window server starts its numbers again after a restart, so the
    /// run is part of the name: two windows are never taken for one.
    static func key(session: String, id: CGWindowID) -> String { "\(session)-\(id)" }
    static func newSession() -> String {
        var g = SystemRandomNumberGenerator()
        return String(format: "%08x", UInt32.random(in: .min ... .max, using: &g))
    }

    /// Whether it is time to save a picture of the whole screen as well. `every`: seconds between them; 0 is never.
    static func overviewDue(last: Date?, now: Date, every: Int) -> Bool {
        guard every > 0 else { return false }
        guard let last, last <= now else { return true }
        return now.timeIntervalSince(last) >= Double(every)
    }

    enum Take: Equatable {
        case take, blank, full, unchanged
        /// A few cells differ (a cursor blinked, a button changed shade): save it only if it reads differently.
        case ifWordsChanged
    }

    /// What to do with one window's picture. `howNew` records the thumbprint, so it is asked last and only once.
    /// `hadFront`: this window has a picture of it as the window in front, of what it shows now.
    static func take(front: Bool, manual: Bool, print: Data, queued: Int, hadFront: Bool, howNew: (Data) -> Newness) -> Take {
        if Pixels.isBlank(print) { return .blank }
        if queued >= maxQueued { return .full }
        let new = howNew(print)
        // Asked for by hand, or first seen in front, the window in front is taken changed or not. One behind it never is.
        if new == .changed || (front && (manual || !hadFront)) { return .take }
        return new == .slight ? .ifWordsChanged : .unchanged
    }

    /// What a window says, as one number to compare with the last picture saved of it. Spacing doesn't count, nor
    /// does the upright stroke OCR sometimes makes of a text cursor.
    static func words(_ lines: [TextLine]) -> Int {
        var h = Hasher()
        for l in lines { h.combine(l.text.filter { !$0.isWhitespace && $0 != "|" }) }
        return h.finalize()
    }

    /// The secret fields lying on this window, as fractions of its picture. Nil when there are some and the picture
    /// can't be trusted to be exactly the window: then they can't be placed, and the window isn't photographed.
    /// `content`: the size ScreenCaptureKit says the picture covers, in points, if it said.
    static func covers(_ covers: [Cover], window: CGRect, content: CGSize?) -> [Cover]? {
        let on = Capturer.onDisplay(covers, window)
        if on.isEmpty { return [] }
        guard let content, abs(content.width - window.width) <= 1, abs(content.height - window.height) <= 1 else { return nil }
        return on
    }

    /// How many pixels to ask for. Nil for a size that makes no sense.
    static func pixels(_ size: CGSize, scale: Double) -> (width: Int, height: Int)? {
        let w = Double(size.width) * scale, h = Double(size.height) * scale
        guard w.isFinite, h.isFinite, w >= 1, h >= 1, w <= 32768, h <= 32768 else { return nil }
        return (Int(w), Int(h))
    }
}
