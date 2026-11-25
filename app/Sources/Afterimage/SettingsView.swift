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
                Toggle("Capture windows separately", isOn: $settings.windowsSeparately)
                if settings.windowsSeparately {
                    Picker("Whole screen as well", selection: $settings.overviewSeconds) {
                        Text("Never").tag(0)
                        Text("Every minute").tag(60)
                        Text("Every 5 minutes").tag(300)
                        Text("Every 15 minutes").tag(900)
                    }
                }
            } footer: {
                Text(settings.windowsSeparately
                     ? "Each window is its own picture, including windows behind others, and a window behind is photographed only when it has changed. A browser is photographed only while it is the window in front. Minimised windows and windows on another desktop are skipped. Identical pictures are never saved twice."
                     : "Identical pictures are never saved twice.").foregroundStyle(.secondary)
            }
            Section {
                Toggle("Keep a log of the window in front", isOn: $settings.trackActivity)
            } footer: {
                Text("Which app and window you were in, for how long, and what else was open. No pictures. Apps you exclude show only as private time. A window’s title is kept only once a picture of that window has been checked for secrets and saved.").foregroundStyle(.secondary)
            }
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
                Toggle("Tor Browser", isOn: $settings.hideTor)
                Toggle("Anything while a password is being typed", isOn: $settings.skipSecureInput)
                Toggle("Browsers whose address can’t be read", isOn: $settings.hideUnreadable)
            } header: { Text("Never capture") } footer: {
                Text("“Password managers” covers Bitwarden, Keeper, 1Password, Passwords and others, along with their websites and browser extensions.").foregroundStyle(.secondary)
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
            BrowserSection()
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
    /// What is being typed into Computer. It becomes the setting only when committed.
    @State private var hostDraft = ""
    @Environment(AppModel.self) private var model
    @Environment(AppSettings.self) private var settings
    @Environment(\.openWindow) private var openWindow
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
                // AIDEV-NOTE: privacy-critical. Typed into a draft and used only once committed: a half-typed address is
                // AIDEV-NOTE: a different machine, and screen text must never be sent to it.
                LabeledContent("Computer") {
                    HStack {
                        TextField("This Mac", text: $hostDraft).labelsHidden().frame(width: 200).autocorrectionDisabled()
                            .onSubmit { settings.lmHost = hostDraft.trimmingCharacters(in: .whitespacesAndNewlines) }
                        if hostDraft.trimmingCharacters(in: .whitespacesAndNewlines) != settings.lmHost {
                            Button("Use") { settings.lmHost = hostDraft.trimmingCharacters(in: .whitespacesAndNewlines) }
                        }
                    }
                }
                LabeledContent("Port") { TextField("Port", value: $settings.port, format: .number.grouping(.never)).labelsHidden().frame(width: 80) }
            } footer: {
                if settings.modelIsElsewhere {
                    Text("LM Studio on another computer: the text on your screen is sent to it before anything is hidden, along with saved pictures (for descriptions), their text and titles, and what you search for. Nothing checks that the computer at that address is yours, and nothing is encrypted unless the network encrypts it. Only private addresses are accepted, but on a network you don’t control someone else’s machine can have one. A Tailscale address is the safe choice.").foregroundStyle(.orange)
                } else {
                    Text(model.status?.linkedDevices.isEmpty == false
                         ? "LM Link may run the model on one of your linked machines. Leave Computer empty to use LM Studio on this Mac."
                         : "Afterimage talks to LM Studio on this Mac. To use another computer on your own network, enter its name or address.").foregroundStyle(.secondary)
                }
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
                LabeledContent("Search model") { ModelPicker(models: models, vision: false, selection: $settings.embedModel).labelsHidden() }
            } footer: {
                Text("Descriptions and search by meaning are made after secrets are hidden. The model is released after five idle minutes.").foregroundStyle(.secondary)
            }
            Section {
                Toggle("Second pass when idle", isOn: $settings.idlePass).disabled(settings.describe == "never")
                if settings.idlePass {
                    LabeledContent("Model") { ModelPicker(models: models, vision: true, selection: $settings.idleModel).labelsHidden() }
                    Picker("Thinking", selection: $settings.idleReasoning) {
                        Text("As little as the model allows").tag("off")
                        Text("Low").tag("low")
                        Text("Medium").tag("medium")
                        Text("High").tag("high")
                    }
                    Stepper("After \(settings.idleMinutes) minute\(settings.idleMinutes == 1 ? "" : "s") away", value: $settings.idleMinutes, in: 1...60)
                }
            } header: {
                Text("🧪 Labs")
            } footer: {
                Text("Experiments that may change or go. Second pass: while nobody is at the Mac and it is plugged in, a second model, which can be bigger and slower, writes a fuller account of each saved picture, newest first. It stops when you come back.").foregroundStyle(.secondary)
            }
            Section {
                Toggle("Developer mode", isOn: $settings.developer)
                Button("Show Log") { openWindow(id: "devlog") }
                if settings.developer {
                    Picker("Descriptions", selection: $settings.describeDetail) {
                        Text("Brief").tag("brief")
                        Text("Detailed").tag("detailed")
                        Text("Both").tag("both")
                        Text("Custom").tag("custom")
                    }
                    if settings.describeDetail == "custom" {
                        Picker("Style", selection: $settings.customLong) {
                            Text("A few sentences").tag(false)
                            Text("Full account").tag(true)
                        }
                        Stepper("Answer up to \(settings.customTokens) tokens", value: $settings.customTokens, in: 200...8192, step: 100)
                        Stepper("Text given: \(settings.customChars) characters", value: $settings.customChars, in: 500...40000, step: 500)
                        Stepper("Picture up to \(settings.customEdge) pixels wide", value: $settings.customEdge, in: 640...2880, step: 160)
                    }
                    Text(Words.describeDetail(settings.describeDetail)).font(.callout).foregroundStyle(.secondary)
                    Picker("Thinking", selection: $settings.reasoning) {
                        Text("As little as the model allows").tag("off")
                        Text("Low").tag("low")
                        Text("Medium").tag("medium")
                        Text("High").tag("high")
                    }
                    Text("How long the model reasons before it checks or describes a picture. More is slower, often many times. Each model gets the nearest level it has; some can’t be switched off.").font(.callout).foregroundStyle(.secondary)
                    Toggle("Let an AI agent read the library", isOn: $settings.agentAccess)
                    if settings.agentAccess {
                        if let port = model.agentPort {
                            LabeledContent("Connect Claude Code") {
                                Button("Copy Command") {
                                    NSPasteboard.general.clearContents()
                                    NSPasteboard.general.setString(settings.agentCommand(port: port, tokenFile: model.agentTokenFile), forType: .string)
                                }
                            }
                            LabeledContent("Connect another app") {
                                Button("Copy Settings") {
                                    // It holds the token: kept off other devices and out of clipboard managers' histories.
                                    let board = NSPasteboard.general
                                    board.prepareForNewContents(with: .currentHostOnly)
                                    board.setString(AppSettings.agentConfig(port: port, token: settings.agentToken), forType: .string)
                                    board.setString("", forType: NSPasteboard.PasteboardType("org.nspasteboard.ConcealedType"))
                                }.help("Contains the token. Paste it only into the app’s own settings.")
                            }
                            LabeledContent("Address", value: "http://127.0.0.1:\(port)/mcp")
                            DisclosureGroup("How to connect") {
                                VStack(alignment: .leading, spacing: 8) {
                                    Text("**Claude Code**\n1. Click Copy Command, paste it into Terminal and press Return.\n2. Start `claude` and type `/mcp`. “afterimage” should say connected.\n3. Ask something like “Use afterimage to tell me what I worked on today.”")
                                    Text("**Claude Desktop, Cursor and other apps**\n1. Click Copy Settings.\n2. Paste it into the app’s MCP settings file. If the file already lists servers, add just the “afterimage” part.\n3. Restart that app.")
                                    Text("**Good to know**\nAfterimage has to be open for an agent to connect. After New Token, connect again: in Terminal run `claude mcp remove afterimage`, then repeat the steps. The agent has five tools: activity, timeline, search, get_shot and get_image.")
                                }
                                .font(.callout).foregroundStyle(.secondary).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                            }
                            Button("New Token") { settings.newAgentToken() }.help("Cuts off any agent using the current one")
                        } else {
                            Text(model.agentProblem ?? "Starting…").foregroundStyle(.secondary)
                        }
                        Text("An agent can search and read everything saved here, pictures included, and can’t change or delete anything. Secrets are already hidden, but if the agent runs in the cloud, what it reads leaves this Mac.").font(.callout).foregroundStyle(.secondary)
                    }
                }
            } footer: {
                Text("Keeps every request to LM Studio and its answer for a week, with secrets removed, along with each decision to take a picture or not. Turning it off deletes the log.").foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .task(id: model.status?.lm == .ready) {
            models = (try? await Task.detached { [e = model.engine] in try e.models() }.value) ?? []
        }
        .onChange(of: settings.model) { Task { await model.probe() } }
        .onChange(of: settings.port) { Task { await model.probe() } }
        .onChange(of: settings.lmHost) { Task { await model.probe() } }
        .onAppear { hostDraft = settings.lmHost }
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
            } footer: { Text("The oldest go first. Pictures you keep are never removed automatically and don’t count toward the limit.").foregroundStyle(.secondary) }
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

/// Connects browsers to the extension that tells Afterimage where secret fields are.
private struct BrowserSection: View {
    @Environment(AppModel.self) private var model
    @State private var link = BrowserLink()
    @State private var heard: Set<String> = []
    @State private var changed = 0
    @State private var problem: String?

    var body: some View {
        let browsers = link.browsers.filter(\.installed)
        Section {
            if browsers.isEmpty { Text("No supported browser found.").foregroundStyle(.secondary) }
            ForEach(browsers) { b in
                let connected = changed >= 0 && link.isConnected(b)
                HStack {
                    AppIcon(bundleId: b.bundleId, size: 20)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(b.name)
                        Text(heard.contains(b.bundleId) ? "Connected" : connected ? "Now add the extension in \(b.name)" : "Not connected").font(.callout).foregroundStyle(.secondary)
                    }
                    Spacer()
                    if connected {
                        Button("Show Extension") { NSWorkspace.shared.activateFileViewerSelecting([BrowserLink.extensionFolder(gecko: link.isGecko(b))]) }
                        Button("Disconnect") { act { try link.disconnect(b) } }
                    } else {
                        Button("Connect") { act { try link.connect(b) } }
                    }
                }
            }
            if let p = problem { Text(p).foregroundStyle(.red) }
        } header: { Text("Browser Extension") } footer: {
            Text("The extension tells Afterimage where password, card and account boxes are, including ones inside a page, so only those are frosted. It sends places, never what’s typed, and only to this Mac. After connecting, open the browser’s extensions page, turn on Developer mode, choose Load unpacked and pick the folder shown.").foregroundStyle(.secondary)
        }
        .task {
            while !Task.isCancelled {
                let running = browsers.flatMap { b in NSRunningApplication.runningApplications(withBundleIdentifier: b.bundleId).map { (b.bundleId, $0.processIdentifier) } }
                heard = await Task.detached { [e = model.engine] in Set(running.filter { e.extensionHeard(pid: $0.1) }.map(\.0)) }.value
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }
    private func act(_ f: () throws -> Void) {
        do { try f(); problem = nil } catch { problem = error.localizedDescription }
        changed += 1
    }
}
