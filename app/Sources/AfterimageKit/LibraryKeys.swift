import AppKit

/// What a plain key press means in the library window. Decided here, from the raw event, because SwiftUI's own key
/// handling only reaches whichever view happens to hold focus.
public enum LibraryKeys {
    public enum Action: Equatable { case move(dx: Int, dy: Int), open, close, delete, deleteNow }

    /// nil: not ours, let it through.
    /// - typing: a text field has the keyboard. - busy: a sheet or a question is up. - viewing: one picture is open.
    public static func action(keyCode: UInt16, modifiers: NSEvent.ModifierFlags, typing: Bool, busy: Bool, viewing: Bool) -> Action? {
        guard !typing, !busy else { return nil }
        let held = modifiers.intersection([.command, .control, .option, .shift])
        // Shift-Delete: delete without being asked first.
        if held == .shift, keyCode == 51 || keyCode == 117 { return .deleteNow }
        guard held.isEmpty else { return nil }
        switch keyCode {
        case 123: return .move(dx: -1, dy: 0)
        case 124: return .move(dx: 1, dy: 0)
        // With one picture open there are no rows; up and down are left for scrolling a zoomed picture.
        case 126: return viewing ? nil : .move(dx: 0, dy: -1)
        case 125: return viewing ? nil : .move(dx: 0, dy: 1)
        case 36, 76: return viewing ? nil : .open
        case 49: return viewing ? .close : .open
        case 51, 117: return .delete
        // G for gallery: back to the grid.
        case 5: return viewing ? .close : nil
        default: return nil
        }
    }
}
