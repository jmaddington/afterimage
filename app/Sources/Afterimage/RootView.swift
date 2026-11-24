import AfterimageEngine
import AfterimageKit
import SwiftUI

struct RootView: View {
    @Environment(AppModel.self) private var model
    @Environment(AppSettings.self) private var settings

    var body: some View {
        @Bindable var model = model
        Group {
            if settings.onboarded { LibraryView() } else { WelcomeView() }
        }
        .confirmationDialog(deleteTitle, isPresented: .init(get: { model.confirmDelete != nil }, set: { if !$0 { model.confirmDelete = nil } })) {
            Button("Delete", role: .destructive) { if let d = model.confirmDelete { model.delete(d) } }
        } message: { Text("This can’t be undone.") }
        .confirmationDialog("Forget \(model.confirmForget?.title ?? "")?", isPresented: .init(get: { model.confirmForget != nil }, set: { if !$0 { model.confirmForget = nil } }), presenting: model.confirmForget) { f in
            Button("Forget", role: .destructive) { model.forget(f) }
        } message: { f in
            Text(f.seconds == nil ? "Every picture and everything read from it will be deleted, including the ones you kept. This can’t be undone."
                                  : "Pictures from this time will be deleted, including any you kept. This can’t be undone.")
        }
        .alert("Something went wrong", isPresented: .init(get: { model.problem != nil }, set: { if !$0 { model.problem = nil } })) {
            Button("OK") {}
        } message: { Text(model.problem ?? "") }
    }

    private var deleteTitle: String {
        let n = model.confirmDelete?.count ?? 0
        return n == 1 ? "Delete this picture?" : "Delete \(n) pictures?"
    }
}

struct LibraryView: View {
    @Environment(AppModel.self) private var model
    @Environment(Recorder.self) private var recorder
    @Environment(AppSettings.self) private var settings
    @FocusState private var searchFocused: Bool

    var body: some View {
        @Bindable var model = model
        NavigationSplitView {
            Sidebar().navigationSplitViewColumnWidth(min: 190, ideal: 220, max: 300)
        } detail: {
            ZStack {
                GridView().opacity(model.open == nil ? 1 : 0).allowsHitTesting(model.open == nil)
                if let s = model.openShot {
                    ViewerView(shot: s).transition(.opacity.combined(with: .scale(scale: 0.985)))
                }
            }
            .animation(.smooth(duration: 0.22), value: model.open == nil)
            .safeAreaInset(edge: .top, spacing: 0) { Notice() }
            .navigationTitle(title)
            .navigationSubtitle(subtitle)
            .toolbar { toolbar }
        }
        .searchable(text: $model.query, placement: .toolbar, prompt: "Words, or what you remember")
        .searchFocused($searchFocused)
        .onChange(of: model.focusSearch) { searchFocused = true }
        .onChange(of: model.query) { model.open = nil }
    }

    private var title: String {
        switch model.place {
        case .all: "Library"
        case .kept: "Kept"
        case .hidden: "With Secrets Hidden"
        case .app(let b): model.apps.first { $0.bundleId == b }?.name ?? "App"
        }
    }
    private var subtitle: String {
        if model.isSearching { return model.searching ? "Searching" : Words.count(model.shots.count, "result", "results") }
        return model.statusLine
    }

    @ToolbarContentBuilder private var toolbar: some ToolbarContent {
        @Bindable var model = model
        if model.open != nil {
            ToolbarItem(placement: .navigation) {
                Button("Back", systemImage: "chevron.left") { model.open = nil }.help("Back to the library")
            }
        }
        ToolbarItem {
            Menu {
                Picker("When", selection: $model.scope) { ForEach(DateScope.allCases) { Text($0.rawValue).tag($0) } }.pickerStyle(.inline).labelsHidden()
            } label: {
                Label("When", systemImage: model.scope == .any ? "calendar" : "calendar.badge.checkmark")
            }
            .help("Show pictures from \(model.scope.rawValue.lowercased())")
        }
        ToolbarItem {
            if case .watching = recorder.state {
                pauseMenu
            } else if recorder.state == .off || isPaused {
                Button("Resume", systemImage: "play.fill") { recorder.resume() }.help("Start capturing again")
            } else {
                pauseMenu
            }
        }
        if model.open != nil {
            ToolbarItem {
                Button(model.openShot?.kept == true ? "Don’t Keep" : "Keep", systemImage: model.openShot?.kept == true ? "bookmark.fill" : "bookmark") {
                    if let id = model.open { model.toggleKept([id]) }
                }.help("Kept pictures are never removed automatically")
            }
            ToolbarItem {
                Button("Delete", systemImage: "trash") { if let id = model.open { model.askDelete([id]) } }.help("Delete this picture")
            }
        }
    }
    private var isPaused: Bool { if case .paused = recorder.state { true } else { false } }
    private var pauseMenu: some View {
        Menu {
            PauseItems()
        } label: { Label("Pause", systemImage: "pause.fill") } primaryAction: { recorder.pause(for: nil) }
            .help("Pause capturing")
    }
}

struct PauseItems: View {
    @Environment(Recorder.self) private var recorder
    var body: some View {
        Button("Pause for 15 Minutes") { recorder.pause(for: 900) }
        Button("Pause for 1 Hour") { recorder.pause(for: 3600) }
        Button("Pause Until Tomorrow") {
            let t = Calendar.current.startOfDay(for: Date()).addingTimeInterval(86400)
            recorder.pause(for: t.timeIntervalSinceNow)
        }
        Button("Pause") { recorder.pause(for: nil) }
    }
}

/// One quiet line, only when something needs the person.
struct Notice: View {
    @Environment(AppModel.self) private var model
    @Environment(AppSettings.self) private var settings
    @Environment(Recorder.self) private var recorder
    @State private var starting = false

    var body: some View {
        Group {
            if recorder.state == .needsPermission {
                row("Afterimage needs permission to see the screen.", "Open Settings…") { Permissions.requestScreen(); Permissions.openScreenSettings() }
            } else if let t = model.lmTrouble {
                if case .offline = model.status?.lm {
                    row(settings.requireAI ? "LM Studio isn’t running, so nothing is being saved." : "LM Studio isn’t running. Only built-in rules are hiding secrets.", starting ? "Starting…" : "Start LM Studio") {
                        starting = true
                        Task {
                            _ = await Task.detached { [e = model.engine] in e.startLmServer() }.value
                            await model.probe()
                            starting = false
                        }
                    }
                } else {
                    settingsRow(t)
                }
            } else if !Permissions.accessibility && settings.hideUnreadable {
                row("Browsers are left out until Afterimage can read page addresses.", "Allow…") { Permissions.requestAccessibility() }
            }
        }
    }
    private func row(_ text: String, _ button: String, _ go: @escaping () -> Void) -> some View {
        HStack {
            Image(systemName: "info.circle").foregroundStyle(.secondary)
            Text(text)
            Spacer()
            Button(button, action: go).disabled(starting)
        }
        .padding(.horizontal, 20).padding(.vertical, 9)
        .background(.bar)
    }
    private func settingsRow(_ text: String) -> some View {
        HStack {
            Image(systemName: "info.circle").foregroundStyle(.secondary)
            Text(text)
            Spacer()
            SettingsLink { Text("Choose a Model…") }
        }
        .padding(.horizontal, 20).padding(.vertical, 9).background(.bar)
    }
}

struct Sidebar: View {
    @Environment(AppModel.self) private var model
    var body: some View {
        @Bindable var model = model
        List(selection: .init(get: { Optional(model.place) }, set: { if let p = $0 { model.place = p } })) {
            Section {
                Label("Library", systemImage: "photo.on.rectangle").badge(Int(model.stats.count)).tag(Place.all)
                Label("Kept", systemImage: "bookmark").badge(Int(model.stats.kept)).tag(Place.kept)
                Label("Secrets Hidden", systemImage: "eye.slash").badge(Int(model.stats.hidden)).tag(Place.hidden)
            }
            if !model.apps.isEmpty {
                Section("Apps") {
                    ForEach(model.apps, id: \.bundleId) { a in
                        Label { Text(a.name) } icon: { AppIcon(bundleId: a.bundleId) }
                            .badge(Int(a.count)).tag(Place.app(a.bundleId))
                    }
                }
            }
        }
        .onCommand(#selector(NSResponder.selectAll(_:))) { model.selection = Set(model.shots.map(\.id)) }
        .safeAreaInset(edge: .bottom) {
            HStack {
                Text(Words.bytes(model.stats.bytes)).monospacedDigit()
                Spacer()
                if model.stats.pending > 0 { Text("\(model.stats.pending.formatted()) to describe") }
            }
            .font(.footnote).foregroundStyle(.secondary).padding(.horizontal, 16).padding(.vertical, 10)
        }
    }
}
