import AfterimageEngine
import CoreGraphics
import Foundation
import Testing
@testable import AfterimageKit

/// A pretend accessibility tree. `broken` attributes answer with an error, as a busy or dying app does.
final class FakeElement: ScreenElement {
    var a: [String: Any]
    var broken: Set<String> = []
    var reads = 0
    init(_ role: String?, _ a: [String: Any] = [:], children: [FakeElement] = []) {
        self.a = a
        if let role { self.a["AXRole"] = role }
        if !children.isEmpty { self.a["AXChildren"] = children }
    }
    static func == (l: FakeElement, r: FakeElement) -> Bool { l === r }
    func read(_ attribute: String) throws -> Any? {
        reads += 1
        if broken.contains(attribute) { throw Unreadable() }
        return a[attribute]
    }
}

@Suite struct AccessibilityReadingTests {
    let frame = CGRect(x: 10, y: 20, width: 1000, height: 700)
    func web(_ url: Any?) -> FakeElement { FakeElement("AXWebArea", url.map { ["AXURL": $0] } ?? [:]) }
    func window(_ kids: [FakeElement], frame f: CGRect? = nil, title: String = "Page") -> FakeElement {
        let f = f ?? frame
        return FakeElement("AXWindow", ["AXTitle": title, "AXPosition": f.origin, "AXSize": f.size], children: kids)
    }
    func app(_ windows: [FakeElement], focused: FakeElement? = nil) -> FakeElement {
        let a = FakeElement("AXApplication", ["AXWindows": windows])
        if let focused { a.a["AXFocusedWindow"] = focused }
        return a
    }
    func read(_ a: FakeElement, drawn: [CGRect]? = nil) throws -> Accessibility.Reading {
        try Accessibility.read(app: a, drawn: drawn ?? [frame], deadline: .now + 5)
    }

    @Test func readsAddressesAndLabels() throws {
        let w = window([FakeElement("AXToolbar", children: [FakeElement("AXButton", ["AXTitle": "Incognito"]), FakeElement("AXTextField", ["AXDescription": "Address"])]),
                        web(URL(string: "https://a.com/x"))])
        let r = try read(app([w], focused: w))
        #expect(r.windows.count == 1 && r.windows[0].urls == ["https://a.com/x"])
        #expect(r.windows[0].controlLabels == ["Incognito"] && r.windows[0].fieldDescriptions == ["Address"])
        #expect(r.focusedURL == "https://a.com/x")
    }
    @Test(arguments: ["AXWindows", "AXChildren"]) func anErrorOnTheAppIsUnreadable(attribute: String) {
        let a = app([window([web("https://a.com")])])
        a.broken = [attribute]
        #expect(throws: Unreadable.self) { try read(a) }
    }
    @Test(arguments: ["AXRole", "AXChildren", "AXTitle", "AXMinimized", "AXDocument", "AXPosition", "AXSize"]) func anErrorOnAWindowIsUnreadable(attribute: String) {
        let w = window([web("https://a.com")])
        w.broken = [attribute]
        #expect(throws: Unreadable.self) { try read(app([w])) }
    }
    @Test(arguments: ["AXRole", "AXChildren", "AXURL", "AXTitle", "AXDescription", "AXValue"]) func anErrorOnWhateverInsideIsUnreadable(attribute: String) {
        for role in ["AXButton", "AXWebArea", "AXTextField", "AXGroup"] {
            let e = FakeElement(role, ["AXURL": "https://a.com"])
            e.broken = [attribute]
            let matters = ["AXRole": true, "AXChildren": role == "AXButton" || role == "AXGroup", "AXURL": role == "AXWebArea",
                           "AXTitle": role == "AXButton", "AXDescription": role == "AXButton" || role == "AXTextField", "AXValue": role == "AXButton"][attribute]!
            if matters { #expect(throws: Unreadable.self, "\(role) \(attribute)") { try read(app([window([e])])) } }
        }
    }
    @Test func anElementWithNoRoleOrOddChildrenIsUnreadable() {
        #expect(throws: Unreadable.self) { try read(app([window([FakeElement(nil)])])) }
        let w = window([])
        w.a["AXChildren"] = "nonsense"
        #expect(throws: Unreadable.self) { try read(app([w])) }
        let odd = FakeElement("AXApplication", ["AXWindows": 7])
        #expect(throws: Unreadable.self) { try read(odd) }
    }
    @Test func anAddressThatIsMissingOrOddIsMarkedUnknown() throws {
        #expect(try read(app([window([web(nil)])])).windows[0].urls == ["unknown:"])
        #expect(try read(app([window([web(7)])])).windows[0].urls == ["unknown:"])
    }
    @Test func webContentIsNeverEntered() throws {
        let inner = FakeElement("AXButton", ["AXTitle": "page text"])
        let page = FakeElement("AXWebArea", ["AXURL": "https://a.com"], children: [inner])
        #expect(try read(app([window([page])])).windows[0].controlLabels.isEmpty && inner.reads == 0)
    }
    @Test func tooManyElementsOrTooLongIsUnreadable() {
        let w = window((0..<1000).map { _ in FakeElement("AXGroup") })
        #expect(throws: Unreadable.self) { try read(app([w])) }
        #expect(throws: Unreadable.self) { try Accessibility.read(app: app([window([web("https://a.com")])]), drawn: [frame], deadline: .now - 1) }
    }
    @Test func minimisedWindowsAreSkipped() throws {
        let m = window([FakeElement(nil)])
        m.a["AXMinimized"] = true
        #expect(try read(app([m]), drawn: []).windows.isEmpty)
    }
    @Test func theFocusedPageIsOneThatWasInspected() throws {
        let w = window([web("https://a.com")]), stranger = window([web("https://b.com")])
        #expect(try read(app([w], focused: stranger)).focusedURL == nil)
    }

    // Every window being drawn must be one that was inspected, not merely as many.
    @Test func aDrawnWindowNobodyInspectedIsUnreadable() throws {
        let other = CGRect(x: 400, y: 300, width: 500, height: 400)
        let a = window([web("https://a.com")]), hidden = window([web("https://b.com")], frame: CGRect(x: 2000, y: 0, width: 900, height: 600))
        #expect(throws: Unreadable.self) { try read(app([a, hidden]), drawn: [frame, other]) }
        #expect(throws: Unreadable.self) { try read(app([a]), drawn: [frame, frame]) }
        #expect(try read(app([a, hidden]), drawn: [frame]).windows.count == 2)
        #expect(try read(app([a]), drawn: [frame.offsetBy(dx: 1, dy: -1)]).windows.count == 1)
    }
    @Test func aPopoverHangingOffTheAppCounts() throws {
        let f = CGRect(x: 600, y: 80, width: 360, height: 420)
        let pop = FakeElement("AXPopover", ["AXPosition": f.origin, "AXSize": f.size], children: [web("chrome-extension://nngceckbapebfimnlniiiahkandclblb/popup")])
        let a = app([window([web("https://a.com")])])
        a.a["AXChildren"] = [pop]
        let r = try read(a, drawn: [frame, f])
        #expect(r.windows.count == 2 && r.windows[1].urls.first!.hasPrefix("chrome-extension"))
    }
}

@Suite struct CaptureVerdictTests {
    func look(_ f: (inout Capturer.Look) -> Void = { _ in }) -> Capturer.Look {
        var l = Capturer.Look(blocked: nil, exclusions: Exclusions(apps: [], sites: [], passwordManagers: true, privateWindows: true, unreadableBrowsers: true), front: 5, hidden: [9], drawn: [5, 9])
        f(&l)
        return l
    }
    @Test func nothingChangedIsFine() { #expect(Capturer.spoiled(look(), look(), omitted: [9]) == nil) }
    @Test func anythingThatChangedSpoilsIt() {
        #expect(Capturer.spoiled(look(), look { $0.blocked = "A password is being typed" }, omitted: [9]) == "A password is being typed")
        #expect(Capturer.spoiled(look { $0.blocked = "Paused" }, look(), omitted: [9]) == "Paused")
        #expect(Capturer.spoiled(look(), look { $0.exclusions.sites = ["chase.com"] }, omitted: [9]) != nil)
        #expect(Capturer.spoiled(look(), look { $0.exclusions.apps = ["x"] }, omitted: [9]) != nil)
        #expect(Capturer.spoiled(look(), look { $0.front = 6 }, omitted: [9]) != nil)
        #expect(Capturer.spoiled(look(), look { $0.hidden = [9, 5] }, omitted: [9]) != nil)
        // Private before, gone after, and never left out of the picture.
        #expect(Capturer.spoiled(look { $0.hidden = [9, 7]; $0.drawn = [5, 7] }, look(), omitted: [9]) != nil)
        // Private and on screen, but macOS didn't list it, so it couldn't be left out.
        #expect(Capturer.spoiled(look(), look(), omitted: []) != nil)
        // Private but not on screen.
        #expect(Capturer.spoiled(look { $0.hidden = [9, 7] }, look { $0.hidden = [9, 7] }, omitted: [9]) == nil)
    }
}
