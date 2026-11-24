import CoreGraphics
import Foundation
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
        #expect(p.due(at: 101) == nil)
        #expect(p.due(at: 101.6) == .app)
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
    @Test func aMissedCaptureIsTriedAgainButNotForever() {
        var p = TriggerPolicy(); p.interval = 0
        p.event(.tab, at: 100); p.captured(at: 101); p.missed(.tab, at: 101.5, since: p.generation)
        #expect(p.due(at: 102) == nil)
        #expect(p.due(at: 105.1) == .tab)         // a retry waits the ordinary gap, not the title one
        p.captured(at: 105.1); p.missed(.tab, at: 106, since: p.generation)
        #expect(p.due(at: 109.2) == .tab)
        p.captured(at: 109.2); p.missed(.tab, at: 110, since: p.generation)
        #expect(p.due(at: 200) == nil)
        p.event(.app, at: 201); p.captured(at: 202); p.missed(.app, at: 202.5, since: p.generation)
        #expect(p.due(at: 206.1) == .app)         // a new event earns new tries
        p.clear(); #expect(p.due(at: 300) == nil)
        p.missed(.interval, at: 301, since: p.generation); #expect(p.due(at: 400) == nil)
    }
    @Test func aMissFromBeforeAPauseIsNotRetried() {
        var p = TriggerPolicy(); p.interval = 0
        p.event(.app, at: 100); p.captured(at: 101)
        let g = p.generation
        p.clear()                                 // paused, locked, or off while the capture was in flight
        p.missed(.app, at: 102, since: g)
        #expect(p.due(at: 200) == nil)
    }
    @Test func aNewerEventOutranksARetry() {
        var p = TriggerPolicy(); p.captured(at: 100)
        p.event(.app, at: 100.5); p.missed(.tab, at: 101, since: p.generation)
        #expect(p.due(at: 104.1) == .app)
    }
}

@Suite struct SwitchingTests {
    /// Switching every two seconds, as someone looking for something does: each stop is captured.
    @Test func everySwitchTwoSecondsApartIsCaptured() {
        var p = TriggerPolicy(); p.interval = 0
        var taken: [Double] = []
        for step in 0..<200 {
            let t = 100 + Double(step) * 0.1
            if step % 20 == 0 { p.event(.window, at: t) }
            if p.due(at: t) != nil { p.captured(at: t); taken.append(t) }
        }
        #expect(taken.count == 10, "\(taken)")
        #expect(zip(taken, 0...).allSatisfy { $0 - (100 + Double($1) * 2) < 1 })
    }
    @Test func rapidFlickingIsOneCaptureOfWhereItEnded() {
        var p = TriggerPolicy(); p.interval = 0
        for i in 0..<6 { p.event(.app, at: 100 + Double(i) * 0.2) }
        #expect(p.due(at: 101.1) == nil)
        #expect(p.due(at: 101.6) == .app)
    }
}

@Suite struct FrontWindowTests {
    func w(_ id: CGWindowID, _ title: String, _ width: Double = 900, _ height: Double = 700, layer: Int = 0, alpha: Double = 1, pid: pid_t = 5) -> OnScreenWindow {
        OnScreenWindow(id: id, pid: pid, title: title, layer: layer, bounds: CGRect(x: 0, y: 0, width: width, height: height), alpha: alpha)
    }
    @Test func popupsAreNotTheFrontWindow() {
        let page = w(9, "apples page")
        #expect(OnScreenWindow.front(of: 5, in: [w(1, "", 838, 218), page])?.id == 9)     // address bar suggestions
        #expect(OnScreenWindow.front(of: 5, in: [w(1, "x", 270, 80), page])?.id == 9)      // tab hover card
        #expect(OnScreenWindow.front(of: 5, in: [w(1, "x", layer: 101), w(2, "x", alpha: 0), w(3, "x", pid: 6), page])?.id == 9)
        #expect(OnScreenWindow.front(of: 5, in: [w(8, "second"), page])?.id == 8)
        #expect(OnScreenWindow.front(of: 5, in: [w(1, "", 1038, 218)])?.id == 1)           // nothing better
        #expect(OnScreenWindow.front(of: 5, in: [w(1, "", 100, 50)])?.id == 1)
        // Untitled but not a popup of the titled window: the titled one isn't what's in front.
        let away = OnScreenWindow(id: 7, pid: 5, title: "", layer: 0, bounds: CGRect(x: 2000, y: 0, width: 800, height: 600), alpha: 1)
        #expect(OnScreenWindow.front(of: 5, in: [away, page])?.id == 7)
        #expect(OnScreenWindow.front(of: 5, in: []) == nil)
    }
}

@Suite struct FrontTrackerTests {
    func w(_ id: CGWindowID, _ title: String) -> OnScreenWindow { OnScreenWindow(id: id, pid: 5, title: title, layer: 0, bounds: CGRect(x: 0, y: 0, width: 900, height: 700), alpha: 1) }
    @Test func noticesWindowsAndTitles() {
        var f = FrontTracker()
        #expect(f.see(pid: 5, window: w(1, "a")) == nil)          // the app switch itself is announced elsewhere
        #expect(f.see(pid: 5, window: w(1, "a")) == nil)
        #expect(f.see(pid: 5, window: w(1, "b")) == .tab)
        #expect(f.see(pid: 5, window: w(2, "b")) == .window)
        #expect(f.see(pid: 6, window: w(3, "c")) == nil)
    }
    @Test func aSlowAppsFirstWindowIsNoticed() {
        var f = FrontTracker()
        _ = f.see(pid: 5, window: w(1, "a"))
        #expect(f.see(pid: 6, window: nil) == nil)
        #expect(f.see(pid: 6, window: w(2, "page")) == .window)
        #expect(f.see(pid: 6, window: nil) == nil)                // closed its last window
    }
}

@MainActor @Suite struct SettingsTests {
    @Test func torIsLeftOutUntilAllowed() throws {
        let name = "afterimage.tests.\(UUID())"
        let d = try #require(UserDefaults(suiteName: name))
        defer { d.removePersistentDomain(forName: name) }
        let s = AppSettings(d)
        #expect(s.exclusions.tor)
        s.hideTor = false
        #expect(!s.exclusions.tor && !AppSettings(d).exclusions.tor && s.exclusions.privateWindows)
    }
    @Test func startingTurnsRecordingOn() throws {
        let name = "afterimage.tests.\(UUID())"
        let d = try #require(UserDefaults(suiteName: name))
        defer { d.removePersistentDomain(forName: name) }
        let s = AppSettings(d)
        s.recording = false
        s.finishOnboarding()
        #expect(s.recording && s.onboarded && AppSettings(d).recording && AppSettings(d).onboarded)
    }
}

@Suite struct BrowserLinkTests {
    let dir = FileManager.default.temporaryDirectory.appending(path: "afterimage-link-\(UUID())")
    func chrome() -> URL { dir.appending(path: "Google/Chrome") }

    @Test func connectingWritesAHostFileTheBrowserWillAccept() throws {
        try FileManager.default.createDirectory(at: chrome(), withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let link = BrowserLink(support: dir, bridge: "/Applications/Afterimage.app/Contents/MacOS/afterimage-bridge")
        let b = try #require(link.browsers.first { $0.name == "Google Chrome" })
        #expect(b.installed && !link.isConnected(b))
        try link.connect(b)
        #expect(link.isConnected(b))
        let f = chrome().appending(path: "NativeMessagingHosts/com.jmaddington.afterimage.json")
        let j = try #require(JSONSerialization.jsonObject(with: Data(contentsOf: f)) as? [String: Any])
        #expect(j["name"] as? String == "com.jmaddington.afterimage" && j["type"] as? String == "stdio")
        #expect(j["path"] as? String == "/Applications/Afterimage.app/Contents/MacOS/afterimage-bridge")
        #expect(j["allowed_origins"] as? [String] == ["chrome-extension://\(BrowserLink.chromeExtensionID)/"])
        // Moved app: the old path no longer counts.
        #expect(!BrowserLink(support: dir, bridge: "/elsewhere/afterimage-bridge").isConnected(b))
        try link.disconnect(b)
        #expect(!link.isConnected(b) && !FileManager.default.fileExists(atPath: f.path))
    }
    @Test func firefoxNamesTheExtensionNotAnOrigin() throws {
        try FileManager.default.createDirectory(at: dir.appending(path: "Mozilla"), withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let link = BrowserLink(support: dir, bridge: "/x/afterimage-bridge")
        let b = try #require(link.browsers.first { $0.name == "Firefox" })
        try link.connect(b)
        let j = try #require(JSONSerialization.jsonObject(with: Data(contentsOf: dir.appending(path: "Mozilla/NativeMessagingHosts/com.jmaddington.afterimage.json"))) as? [String: Any])
        #expect(j["allowed_extensions"] as? [String] == ["afterimage@jmaddington.com"] && j["allowed_origins"] == nil)
    }
    @Test func browsersThatAreNotThereAreNotOffered() {
        let link = BrowserLink(support: dir, bridge: "/x/afterimage-bridge")
        #expect(link.browsers.allSatisfy { !$0.installed })
        #expect(throws: (any Error).self) { try link.connect(link.browsers[0]) }
    }
    @Test func theIDMatchesTheExtensionsKey() throws {
        let m = URL(fileURLWithPath: #filePath).deletingLastPathComponent().appending(path: "../../../extension/host.chrome.json").standardizedFileURL
        let j = try #require(JSONSerialization.jsonObject(with: Data(contentsOf: m)) as? [String: Any])
        #expect(j["allowed_origins"] as? [String] == ["chrome-extension://\(BrowserLink.chromeExtensionID)/"])
    }
}
