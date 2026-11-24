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
    @Test func typingQuestionsAndShortcutsAreLeftAlone() {
        for k: UInt16 in [123, 124, 125, 126, 36, 49, 51] {
            #expect(act(k, typing: true) == nil); #expect(act(k, busy: true) == nil)
            #expect(act(k, .command) == nil); #expect(act(k, .option) == nil); #expect(act(k, .shift) == nil)
        }
        #expect(act(0) == nil)
    }
}
