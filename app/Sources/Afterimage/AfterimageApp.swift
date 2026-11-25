import AfterimageEngine
import AfterimageKit
import SwiftUI

@main struct AfterimageApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @State private var model = AppModel.shared

    var body: some Scene {
        Window("Afterimage", id: "library") {
            RootView().environment(model).environment(model.settings).environment(model.recorder)
                .frame(minWidth: 820, minHeight: 540)
        }
        .defaultSize(width: 1240, height: 820)
        .commands { AppCommands(model: model) }

        Window("Developer Log", id: "devlog") {
            DevLogView().environment(model).environment(model.settings)
        }
        .defaultSize(width: 980, height: 620)

        Settings {
            SettingsView().environment(model).environment(model.settings).environment(model.recorder)
        }

        MenuBarExtra("Afterimage: \(model.statusLine)", systemImage: model.menuSymbol) {
            MenuBarContent().environment(model).environment(model.settings).environment(model.recorder)
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_: Notification) { AppModel.shared.start() }
    func applicationShouldTerminateAfterLastWindowClosed(_: NSApplication) -> Bool { false }
    func applicationWillTerminate(_: Notification) { AppModel.shared.stop() }
}

struct AppCommands: Commands {
    let model: AppModel
    @Environment(\.openWindow) private var openWindow

    var body: some Commands {
        CommandGroup(after: .appInfo) {
            Button("Check for Updates…") { Words.checkForUpdates() }
        }
        CommandGroup(replacing: .newItem) {
            Button("Capture Now") { model.recorder.captureNow() }.keyboardShortcut("n")
        }
        CommandGroup(after: .textEditing) {
            Button("Find") { model.focusSearch += 1 }.keyboardShortcut("f")
        }
        CommandMenu("Library") {
            Button("Show Library") { openWindow(id: "library") }.keyboardShortcut("0")
            Divider()
            Button(model.selectionIsKept ? "Don’t Keep" : "Keep") { model.toggleKept() }
                .keyboardShortcut("k").disabled(model.selection.isEmpty)
            Button("Delete…") { model.askDelete() }
                .keyboardShortcut(.delete).disabled(model.selection.isEmpty)
            Divider()
            Menu("Forget") { ForgetItems(model: model) }
        }
    }
}

struct ForgetItems: View {
    let model: AppModel
    var body: some View {
        Button("The Last 10 Minutes…") { model.confirmForget = .init(title: "the last 10 minutes", seconds: 600) }
        Button("The Last Hour…") { model.confirmForget = .init(title: "the last hour", seconds: 3600) }
        Button("Today…") { model.confirmForget = .init(title: "today", seconds: Date().timeIntervalSince(Calendar.current.startOfDay(for: Date()))) }
        Divider()
        Button("Everything…") { model.confirmForget = .init(title: "everything", seconds: nil) }
    }
}
