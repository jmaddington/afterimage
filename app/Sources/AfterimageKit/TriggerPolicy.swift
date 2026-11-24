import Foundation

/// Why a picture was taken, most telling first.
public enum Trigger: String, CaseIterable, Sendable {
    case manual, launch, app, space, window, tab, interval
    var rank: Int { Self.allCases.firstIndex(of: self)! }
}

/// Decides when a burst of events becomes one capture. Pure, so it can be tested without a screen.
public struct TriggerPolicy: Sendable {
    /// Quiet time after the last event, so animations finish.
    public var settle: TimeInterval = 0.8
    /// A busy stretch is captured at least this often, settled or not.
    public var patience: TimeInterval = 3
    public var minGap: TimeInterval = 4
    /// Title changes are chatty (terminals, players), so they get a longer leash.
    public var titleGap: TimeInterval = 15
    /// Zero turns the timer off.
    public var interval: TimeInterval = 60
    public var enabled: Set<Trigger> = Set(Trigger.allCases)

    private var pending: Trigger?
    private var first: TimeInterval = 0
    private var last: TimeInterval = 0
    private var lastCapture: TimeInterval?
    private var retries = 0
    private var retrying = false

    public init() {}

    public mutating func event(_ t: Trigger, at now: TimeInterval) {
        guard t != .interval, t == .manual || enabled.contains(t) else { return }
        if pending == nil { first = now }
        last = now
        retries = 0
        if retrying { (pending, retrying) = (nil, false) }
        if pending.map({ t.rank < $0.rank }) ?? true { pending = t }
    }

    /// The capture to take now, if any. Stays due until `captured` or `clear`.
    public mutating func due(at now: TimeInterval) -> Trigger? {
        if let p = pending {
            if p == .manual { return p }
            let settled = now - last >= settle || now - first >= patience
            let gap = p == .tab && !retrying ? titleGap : minGap
            if settled && (lastCapture.map { now - $0 >= gap } ?? true) { return p }
        }
        if interval > 0, enabled.contains(.interval), let l = lastCapture, now - l >= interval { return .interval }
        if interval > 0, enabled.contains(.interval), lastCapture == nil, pending == nil { return .interval }
        return nil
    }

    public mutating func captured(at now: TimeInterval) { lastCapture = now; pending = nil; retrying = false }
    /// The capture came to nothing (a menu was open, a page was mid-change). Try again twice, then let it go.
    public mutating func missed(_ t: Trigger, at now: TimeInterval) {
        guard t != .interval, t != .manual, pending == nil, retries < 2 else { return }
        retries += 1
        (pending, first, last, retrying) = (t, now, now, true)
    }
    public mutating func clear() { pending = nil; retrying = false }

    /// Drop spinners, counters and percentages so only real title changes count.
    public static func stableTitle(_ s: String) -> String {
        s.unicodeScalars.filter { CharacterSet.letters.contains($0) || $0 == " " }
            .map(String.init).joined().split(separator: " ").joined(separator: " ")
    }
}
