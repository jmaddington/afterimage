import AfterimageKit
import SwiftUI

struct MenuBarContent: View {
    @Environment(AppModel.self) private var model
    @Environment(Recorder.self) private var recorder
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Text(model.statusLine)
        ForEach(model.missingPermissions, id: \.name) { m in
            Button("Allow \(m.name)…") { m == .screen ? Permissions.openScreenSettings() : Permissions.openAccessibilitySettings() }
        }
        Divider()
        switch recorder.state {
        case .off, .paused: Button("Resume") { recorder.resume() }
        default: Menu("Pause") { PauseItems() }
        }
        Button("Capture Now") { recorder.captureNow() }
        Menu("Forget") { ForgetItems(model: model) }
        Divider()
        Button("Open Library") { show() }
        SettingsLink { Text("Settings…") }
        Divider()
        Text(Words.version)
        Button("Check for Updates…") { Words.checkForUpdates() }
        Button("See Releases…") { NSWorkspace.shared.open(Words.releases) }
        Divider()
        Button("Quit Afterimage") { NSApp.terminate(nil) }
            .onChange(of: model.confirmForget?.id) { if model.confirmForget != nil { show() } }
    }
    private func show() {
        openWindow(id: "library")
        NSApp.activate()
    }
}
