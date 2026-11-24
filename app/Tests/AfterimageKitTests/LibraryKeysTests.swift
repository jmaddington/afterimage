import AppKit
import Testing
@testable import AfterimageKit

struct LibraryKeysTests {
    func act(_ k: UInt16, _ m: NSEvent.ModifierFlags = [], typing: Bool = false, busy: Bool = false, viewing: Bool = false) -> LibraryKeys.Action? {
        LibraryKeys.action(keyCode: k, modifiers: m, typing: typing, busy: busy, viewing: viewing)
    }
    @Test func arrowsMoveAndDeleteDeletesInTheGrid() {
        #expect(act(123) == .move(dx: -1, dy: 0)); #expect(act(124) == .move(dx: 1, dy: 0))
        #expect(act(126) == .move(dx: 0, dy: -1)); #expect(act(125) == .move(dx: 0, dy: 1))
        #expect(act(36) == .open); #expect(act(49) == .open)
        #expect(act(51) == .delete); #expect(act(117) == .delete)
        // Arrow keys always carry these two flags; they aren't modifiers the person pressed.
        #expect(act(123, [.numericPad, .function]) == .move(dx: -1, dy: 0))
    }
    @Test func withAPictureOpenSidewaysStepsAndSpaceCloses() {
        #expect(act(123, viewing: true) == .move(dx: -1, dy: 0)); #expect(act(124, viewing: true) == .move(dx: 1, dy: 0))
        #expect(act(126, viewing: true) == nil); #expect(act(49, viewing: true) == .close); #expect(act(51, viewing: true) == .delete)
    }
    @Test func gGoesBackToTheGalleryAndShiftDeleteDoesNotAsk() {
        #expect(act(5, viewing: true) == .close); #expect(act(5) == nil); #expect(act(5, typing: true, viewing: true) == nil)
        #expect(act(51, .shift) == .deleteNow); #expect(act(117, .shift, viewing: true) == .deleteNow)
        #expect(act(51, [.shift, .command]) == nil); #expect(act(51, .shift, typing: true) == nil); #expect(act(51, .shift, busy: true) == nil)
    }
    @Test func typingQuestionsAndShortcutsAreLeftAlone() {
        for k: UInt16 in [123, 124, 125, 126, 36, 49, 51] {
            #expect(act(k, typing: true) == nil); #expect(act(k, busy: true) == nil)
            #expect(act(k, .command) == nil); #expect(act(k, .option) == nil); #expect(act(k, .shift) == (k == 51 ? .deleteNow : nil))
        }
        #expect(act(0) == nil)
    }
    @Test func agentSettingsAreValidJSONWithTheAddressAndToken() throws {
        let j = try #require(JSONSerialization.jsonObject(with: Data(AppSettings.agentConfig(port: 4319, token: "tok-123").utf8)) as? [String: Any])
        let a = try #require((j["mcpServers"] as? [String: Any])?["afterimage"] as? [String: Any])
        #expect(a["url"] as? String == "http://127.0.0.1:4319/mcp")
        #expect((a["headers"] as? [String: String])?["Authorization"] == "Bearer tok-123")
    }
}
