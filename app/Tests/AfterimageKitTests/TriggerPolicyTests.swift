import Testing
@testable import AfterimageKit

@Suite struct TriggerPolicyTests {
    @Test func waitsForTheScreenToSettle() {
        var p = TriggerPolicy()
        p.event(.app, at: 100)
        #expect(p.due(at: 100.2) == nil)
        #expect(p.due(at: 100.9) == .app)
    }
    @Test func aBurstBecomesOneCapture() {
        var p = TriggerPolicy()
        p.event(.app, at: 100); p.event(.window, at: 100.3); p.event(.tab, at: 100.5)
        #expect(p.due(at: 100.9) == nil)          // still settling after the last event
        #expect(p.due(at: 101.4) == .app)         // the most telling reason wins
        p.captured(at: 101.4)
        #expect(p.due(at: 110) == nil)
    }
    @Test func endlessEventsCannotPostponeForever() {
        var p = TriggerPolicy()
        var fired = false
        for i in 0..<40 {
            let t = 100 + Double(i) * 0.25
            p.event(.window, at: t)
            if p.due(at: t) != nil { fired = true; break }
        }
        #expect(fired)
    }
    @Test func keepsAMinimumGapButDoesNotDropTheEvent() {
        var p = TriggerPolicy()
        p.captured(at: 100)
        p.event(.app, at: 100.5)
        #expect(p.due(at: 102) == nil)
        #expect(p.due(at: 104.1) == .app)
    }
    @Test func titleChangesAreRateLimited() {
        var p = TriggerPolicy()
        p.event(.tab, at: 100); #expect(p.due(at: 101) == .tab); p.captured(at: 101)
        p.event(.tab, at: 106); #expect(p.due(at: 107) == nil)
        #expect(p.due(at: 116.1) == .tab)
        p.captured(at: 116.1)
        p.event(.tab, at: 117); p.event(.app, at: 121)
        #expect(p.due(at: 122) == .app)           // a switch isn't held back by the title limit
    }
    @Test func intervalCountsFromTheLastCapture() {
        var p = TriggerPolicy()
        p.captured(at: 100)
        #expect(p.due(at: 159) == nil)
        #expect(p.due(at: 160) == .interval)
        p.interval = 0
        #expect(p.due(at: 9999) == nil)
    }
    @Test func disabledTriggersAreIgnored() {
        var p = TriggerPolicy(); p.enabled = [.interval, .manual]
        p.event(.app, at: 100)
        #expect(p.due(at: 102) != .app)
    }
    @Test func manualIsImmediate() {
        var p = TriggerPolicy(); p.captured(at: 100)
        p.event(.manual, at: 100.1)
        #expect(p.due(at: 100.1) == .manual)
    }
    @Test func spinnersAreNotTitleChanges() {
        #expect(TriggerPolicy.stableTitle("⠋ Building") == TriggerPolicy.stableTitle("⠙ Building"))
        #expect(TriggerPolicy.stableTitle("✳ Fix tests") == TriggerPolicy.stableTitle("· Fix tests"))
        #expect(TriggerPolicy.stableTitle("(3) Inbox") == TriggerPolicy.stableTitle("(4) Inbox"))
        #expect(TriggerPolicy.stableTitle("Building 41%") == TriggerPolicy.stableTitle("Building 42%"))
        #expect(TriggerPolicy.stableTitle("Inbox") != TriggerPolicy.stableTitle("Sent"))
        #expect(TriggerPolicy.stableTitle("日本語") != TriggerPolicy.stableTitle("中文"))
    }
}
