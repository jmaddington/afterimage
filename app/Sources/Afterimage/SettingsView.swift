import AfterimageEngine
import AfterimageKit
import ServiceManagement
import SwiftUI
import UniformTypeIdentifiers

struct SettingsView: View {
    var body: some View {
        TabView {
            Tab("Capture", systemImage: "circle.circle") { CaptureSettings() }
            Tab("Privacy", systemImage: "eye.slash") { PrivacySettings() }
            Tab("Intelligence", systemImage: "brain") { IntelligenceSettings() }
            Tab("Storage", systemImage: "internaldrive") { StorageSettings() }
        }
        .frame(width: 560)
        .frame(minHeight: 420)
    }
}

private struct CaptureSettings: View {
    @Environment(AppSettings.self) private var settings
    @State private var login = SMAppService.mainApp.status == .enabled
    @State private var loginError: String?

    var body: some View {
        @Bindable var settings = settings
        Form {
            Section("Take a picture when you") {
                Toggle("Switch apps, windows or spaces", isOn: $settings.onSwitch)
                Toggle("Open an app", isOn: $settings.onLaunch)
                Toggle("Change tabs or pages", isOn: $settings.onTab)
            }
            Section {
                Picker("And regularly", selection: $settings.interval) {
                    Text("Never").tag(0)
                    Text("Every 30 seconds").tag(30)
                    Text("Every minute").tag(60)
                    Text("Every 2 minutes").tag(120)
                    Text("Every 5 minutes").tag(300)
                    Text("Every 15 minutes").tag(900)
                }
                Picker("Displays", selection: $settings.allDisplays) {
                    Text("The one in use").tag(false)
                    Text("All of them").tag(true)
                }
            } footer: { Text("Identical pictures are never saved twice.").foregroundStyle(.secondary) }
            Section {
                Toggle("Open at login", isOn: $login)
                    .onChange(of: login) { _, on in
                        do { if on { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() } } catch { loginError = error.localizedDescription }
                        login = SMAppService.mainApp.status == .enabled
                    }
                if let e = loginError { Text(e).foregroundStyle(.secondary) }
            }
        }.formStyle(.grouped)
    }
}

private struct PrivacySettings: View {
    @Environment(AppSettings.self) private var settings
    @State private var site = ""
    @State private var badSite = false
    @State private var choosing = false

    var body: some View {
        @Bindable var settings = settings
        Form {
            Section {
                Toggle("Password managers", isOn: $settings.hidePasswordManagers)
                Toggle("Private browsing", isOn: $settings.hidePrivateWindows)
                Toggle("Anything while a password is being typed", isOn: $settings.skipSecureInput)
                Toggle("Browsers whose address can’t be read", isOn: $settings.hideUnreadable)
            } header: { Text("Never capture") } footer: {
                Text("Password managers includes Bitwarden, Keeper, 1Password, Passwords and others, along with their websites and browser extensions.").foregroundStyle(.secondary)
            }
            Section("Apps") {
                ForEach(settings.excludedApps, id: \.self) { b in
                    HStack {
                        AppIcon(bundleId: b, size: 20)
                        Text(name(b))
                        Spacer()
                        remove { settings.excludedApps.removeAll { $0 == b } }.accessibilityLabel("Remove \(name(b))")
                    }
                }
                Button("Add an App…") { choosing = true }
            }
            Section {
                ForEach(settings.excludedSites, id: \.self) { s in
                    HStack {
                        Image(systemName: "globe").foregroundStyle(.secondary).frame(width: 20)
                        Text(s)
                        Spacer()
                        remove { settings.excludedSites.removeAll { $0 == s } }.accessibilityLabel("Remove \(s)")
                    }
                }
                HStack {
                    TextField("Site", text: $site, prompt: Text("chase.com, github.com/settings, *.corp.example")).labelsHidden().onSubmit(add)
                    Button("Add", action: add).disabled(site.isEmpty)
                }
                if badSite { Text("That doesn’t look like a web address.").foregroundStyle(.red) }
            } header: { Text("Sites") } footer: {
                Text("If one of these is showing in any window, that browser is left out of the picture.").foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .fileImporter(isPresented: $choosing, allowedContentTypes: [.application], allowsMultipleSelection: true) { r in
            for u in (try? r.get()) ?? [] {
                if let b = Bundle(url: u)?.bundleIdentifier, !settings.excludedApps.contains(b) { settings.excludedApps.append(b) }
            }
        }
        .fileDialogDefaultDirectory(URL(fileURLWithPath: "/Applications"))
    }
    private func add() {
        guard let s = normalizeSite(raw: site) else { badSite = true; return }
        badSite = false
        if !settings.excludedSites.contains(s) { settings.excludedSites.append(s) }
        site = ""
    }
    private func name(_ b: String) -> String {
        NSWorkspace.shared.urlForApplication(withBundleIdentifier: b).map { FileManager.default.displayName(atPath: $0.path) } ?? b
    }
    private func remove(_ go: @escaping () -> Void) -> some View {
        Button("Remove", systemImage: "minus.circle", action: go).labelStyle(.iconOnly).buttonStyle(.borderless)
    }
}

private struct IntelligenceSettings: View {
    @Environment(AppModel.self) private var model
    @Environment(AppSettings.self) private var settings
    @State private var models: [ModelInfo] = []

    var body: some View {
        @Bindable var settings = settings
        Form {
            Section {
                LabeledContent("LM Studio") {
                    HStack(spacing: 6) {
                        Circle().fill(model.status?.lm == .ready ? .green : .orange).frame(width: 8, height: 8)
                        Text(model.lmTrouble ?? (model.status == nil ? "Checking" : "Ready"))
                    }
                }
                if let d = model.status?.linkedDevices, !d.isEmpty {
                    LabeledContent("Linked machines", value: d.joined(separator: ", "))
                }
                ModelPicker(models: models, vision: true, selection: $settings.model)
                LabeledContent("Port") { TextField("Port", value: $settings.port, format: .number.grouping(.never)).labelsHidden().frame(width: 80) }
            } footer: {
                Text(model.status?.linkedDevices.isEmpty == false
                     ? "LM Link may run the model on one of your linked machines. Afterimage never contacts a cloud service."
                     : "Afterimage only talks to LM Studio on this Mac. It never contacts a cloud service.").foregroundStyle(.secondary)
            }
            Section {
                Picker("If the model isn’t available", selection: $settings.requireAI) {
                    Text("Don’t save").tag(true)
                    Text("Use built-in rules").tag(false)
                }
            } footer: {
                Text("Built-in rules catch card numbers, Social Security numbers, keys and labelled passwords. Only the model catches a password with no label.").foregroundStyle(.secondary)
            }
            Section {
                Picker("Describe pictures", selection: $settings.describe) {
                    Text("When plugged in").tag("pluggedIn")
                    Text("Always").tag("always")
                    Text("Never").tag("never")
                }
                ModelPicker(models: models, vision: false, selection: $settings.embedModel).labelsHidden(false)
            } footer: {
                Text("Descriptions and search by meaning are made after secrets are hidden. The model is released after five idle minutes.").foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .task(id: model.status?.lm == .ready) {
            models = (try? await Task.detached { [e = model.engine] in try e.models() }.value) ?? []
        }
        .onChange(of: settings.model) { Task { await model.probe() } }
        .onChange(of: settings.port) { Task { await model.probe() } }
    }
}

private extension View {
    func labelsHidden(_ hide: Bool) -> some View { Group { if hide { labelsHidden() } else { self } } }
}

private struct StorageSettings: View {
    @Environment(AppModel.self) private var model
    @Environment(AppSettings.self) private var settings
    /// A stricter limit waits here until the person has seen what it would remove.
    @State private var pending: (days: Int, gb: Int, count: UInt64)?

    var body: some View {
        Form {
            Section {
                LabeledContent("Pictures", value: model.stats.count.formatted())
                LabeledContent("Space used", value: Words.bytes(model.stats.bytes))
                if let o = model.stats.oldest {
                    LabeledContent("Going back to", value: Date(timeIntervalSince1970: Double(o) / 1000).formatted(date: .abbreviated, time: .omitted))
                }
            }
            Section {
                Picker("Keep pictures for", selection: limit(\.keepDays)) {
                    Text("1 day").tag(1)
                    Text("1 week").tag(7)
                    Text("30 days").tag(30)
                    Text("90 days").tag(90)
                    Text("1 year").tag(365)
                    Text("Forever").tag(0)
                }
                Picker("Use at most", selection: limit(\.keepGB)) {
                    Text("5 GB").tag(5)
                    Text("10 GB").tag(10)
                    Text("20 GB").tag(20)
                    Text("50 GB").tag(50)
                    Text("100 GB").tag(100)
                    Text("No limit").tag(0)
                }
            } footer: { Text("The oldest go first. Pictures you keep are never removed automatically.").foregroundStyle(.secondary) }
            Section {
                Button("Forget Everything…", role: .destructive) { model.confirmForget = .init(title: "everything", seconds: nil); NSApp.activate() }
            }
        }
        .formStyle(.grouped)
        .confirmationDialog("Remove \(Words.count(Int(pending?.count ?? 0), "picture", "pictures")) now?", isPresented: .init(get: { pending != nil }, set: { if !$0 { pending = nil } })) {
            Button("Remove", role: .destructive) { if let p = pending { apply(p.days, p.gb) } }
        } message: { Text("They’re outside the new limit. This can’t be undone.") }
    }

    private func limit(_ key: ReferenceWritableKeyPath<AppSettings, Int>) -> Binding<Int> {
        Binding { settings[keyPath: key] } set: { v in
            let days = key == \.keepDays ? v : settings.keepDays
            let gb = key == \.keepGB ? v : settings.keepGB
            Task {
                let n = (try? await Task.detached { [e = model.engine] in try e.wouldExpire(maxAgeDays: UInt32(days), maxBytes: UInt64(gb) * 1_000_000_000) }.value) ?? 0
                if n > 0 { pending = (days, gb, n) } else { apply(days, gb) }
            }
        }
    }
    private func apply(_ days: Int, _ gb: Int) {
        settings.keepDays = days
        settings.keepGB = gb
        let c = settings.config
        Task {
            _ = try? await Task.detached { [e = model.engine] in e.setConfig(config: c); return try e.applyRetention() }.value
            model.reload()
        }
    }
}
