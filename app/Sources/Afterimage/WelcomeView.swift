import AfterimageEngine
import AfterimageKit
import SwiftUI

struct WelcomeView: View {
    @Environment(AppModel.self) private var model
    @Environment(AppSettings.self) private var settings
    @State private var screen = Permissions.screen
    @State private var ax = Permissions.accessibility
    @State private var asked = false
    @State private var models: [ModelInfo] = []
    @State private var starting = false

    var body: some View {
        VStack(spacing: 0) {
            Spacer()
            Image(nsImage: NSApp.applicationIconImage).resizable().frame(width: 112, height: 112)
            Text("Afterimage").font(.largeTitle.weight(.semibold)).padding(.top, 14)
            Text("Remembers what was on your screen, and forgets your secrets.\nEverything stays on machines you own.")
                .multilineTextAlignment(.center).foregroundStyle(.secondary).padding(.top, 4)

            Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 20) {
                step("eyeglasses", "Page addresses", "So it can leave out private windows and the sites you choose.", done: ax) {
                    Button("Allow…") { Permissions.requestAccessibility() }
                }
                step("brain", "A local model", modelLine, done: modelReady) {
                    if case .offline = model.status?.lm {
                        Button(starting ? "Starting…" : "Start LM Studio") {
                            starting = true
                            Task {
                                _ = await Task.detached { [e = model.engine] in e.startLmServer() }.value
                                await refresh()
                                starting = false
                            }
                        }.disabled(starting)
                    } else {
                        ModelPicker(models: models, vision: true, selection: Bindable(settings).model).labelsHidden().frame(width: 190)
                    }
                }
                step("rectangle.dashed.badge.record", "Your screen", asked && !screen ? "Turn it on in System Settings, then reopen." : "So it can take pictures.", done: screen) {
                    if asked { Button("Reopen Afterimage") { Permissions.relaunch() } } else {
                        Button("Allow…") { asked = true; if !Permissions.requestScreen() { Permissions.openScreenSettings() } }
                    }
                }
            }
            .frame(maxWidth: 560).padding(.top, 40)

            Button("Start") { model.finishOnboarding() }
                .buttonStyle(.glassProminent).controlSize(.extraLarge).keyboardShortcut(.defaultAction)
                .disabled(!screen || !modelReady).padding(.top, 40)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity).padding(40)
        .task {
            while !Task.isCancelled {
                screen = Permissions.screen
                ax = Permissions.accessibility
                await refresh()
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }

    private var modelReady: Bool { model.status?.lm == .ready }
    private var modelLine: String {
        switch model.status?.lm {
        case .ready: "\(models.first { $0.key == settings.model }?.name ?? settings.model) will look for secrets."
        case .noModel: "Choose one that can see pictures."
        case .error(let m): m
        default: "LM Studio finds secrets and describes pictures."
        }
    }
    private func refresh() async {
        await model.probe()
        models = (try? await Task.detached { [e = model.engine] in try e.models() }.value) ?? []
    }

    private func step<B: View>(_ symbol: String, _ title: String, _ text: String, done: Bool, @ViewBuilder _ button: () -> B) -> some View {
        GridRow {
            Image(systemName: symbol).font(.title2).foregroundStyle(.secondary).frame(width: 34)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.headline)
                Text(text).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Group {
                if done { Image(systemName: "checkmark.circle.fill").font(.title2).foregroundStyle(.green).accessibilityLabel("Done") } else { button() }
            }
            .gridColumnAlignment(.trailing)
        }
    }
}

struct ModelPicker: View {
    let models: [ModelInfo]
    let vision: Bool
    @Binding var selection: String
    var body: some View {
        let fit = models.filter { vision ? ($0.vision && !$0.embedding) : $0.embedding }
        Picker("Model", selection: $selection) {
            if !fit.contains(where: { $0.key == selection }) { Text(selection.isEmpty ? "None" : selection).tag(selection) }
            if !vision { Text("None").tag("") }
            ForEach(fit, id: \.key) { Text("\($0.name) (\(Words.bytes($0.sizeBytes)))").tag($0.key) }
        }
    }
}
