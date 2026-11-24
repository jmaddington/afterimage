import AfterimageEngine
import AfterimageKit
import SwiftUI

/// What Afterimage asked LM Studio and decided about each capture, with secrets already taken out.
struct DevLogView: View {
    @Environment(AppModel.self) private var model
    @Environment(AppSettings.self) private var settings
    @Environment(\.openWindow) private var openWindow
    @State private var entries: [DevEntry] = []
    @State private var selected: Int64?
    @State private var modelOnly = false

    var body: some View {
        HSplitView {
            List(shown, id: \.id, selection: $selected) { row($0) }.frame(minWidth: 320, idealWidth: 380)
            detail.frame(minWidth: 380, maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(minWidth: 760, minHeight: 420)
        .overlay {
            if entries.isEmpty {
                ContentUnavailableView(settings.developer ? "Nothing yet" : "Developer mode is off", systemImage: "list.bullet.rectangle",
                                       description: Text(settings.developer ? "Requests and decisions appear here as they happen." : "Turn it on in Settings › Intelligence."))
            }
        }
        .toolbar {
            Toggle("Model Requests Only", systemImage: "brain", isOn: $modelOnly).help("Hide capture decisions")
            Button("Clear", systemImage: "trash") {
                Task { _ = try? await Task.detached { [e = model.engine] in try e.devClear() }.value; await load() }
            }.disabled(entries.isEmpty).help("Delete the log")
        }
        .task { while !Task.isCancelled { await load(); try? await Task.sleep(for: .seconds(2)) } }
    }

    private var shown: [DevEntry] { modelOnly ? entries.filter { $0.kind != "capture" } : entries }

    private func load() async {
        entries = (try? await Task.detached { [e = model.engine] in try e.devLog(limit: 500, before: nil) }.value) ?? []
    }

    private func title(_ e: DevEntry) -> String {
        switch e.kind {
        case "check": "Checked for secrets"
        case "describe": "Described a picture"
        case "embed": e.note == "your search" ? "Understood a search" : "Made a picture searchable"
        case "load": "Loaded a model"
        case "unload": "Released a model"
        case "capture": Words.trigger(e.request)
        default: e.kind
        }
    }
    private func symbol(_ e: DevEntry) -> String {
        if !e.ok { return "exclamationmark.triangle" }
        return switch e.kind {
        case "check": "eye.slash"
        case "describe": "text.bubble"
        case "embed": "magnifyingglass"
        case "capture": "camera"
        default: "brain"
        }
    }
    private func when(_ e: DevEntry) -> String { Date(timeIntervalSince1970: Double(e.at) / 1000).formatted(date: .omitted, time: .standard) }

    private func row(_ e: DevEntry) -> some View {
        Label {
            VStack(alignment: .leading, spacing: 2) {
                Text(title(e))
                Text([when(e), e.kind == "capture" ? nil : "\(e.ms.formatted()) ms", e.note.isEmpty ? nil : e.note].compactMap { $0 }.joined(separator: " · "))
                    .font(.callout).foregroundStyle(.secondary).lineLimit(2)
            }
        } icon: { Image(systemName: symbol(e)).foregroundStyle(e.ok ? Color.secondary : Color.orange) }
    }

    @ViewBuilder private var detail: some View {
        if let e = entries.first(where: { $0.id == selected }) {
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(title(e)).font(.title3.weight(.semibold))
                            Text([when(e), e.model, e.kind == "capture" ? "" : "\(e.ms.formatted()) ms"].filter { !$0.isEmpty }.joined(separator: " · ")).foregroundStyle(.secondary)
                        }
                        Spacer()
                        if let id = e.shotId {
                            Button("Show Picture") { model.show(id); openWindow(id: "library") }
                        }
                    }
                    if !e.note.isEmpty { Text(e.note) }
                    if e.kind != "capture" {
                        block("Sent", e.request, empty: "Not kept. The words of a request are only kept once its picture is safely saved.")
                        block("Answer", e.response, empty: "Not kept.")
                    }
                }
                .padding(20).frame(maxWidth: .infinity, alignment: .leading)
            }
        } else {
            Text("Choose an entry").foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func block(_ name: String, _ text: String, empty: String) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(name).font(.headline)
            if text.isEmpty {
                Text(empty).foregroundStyle(.secondary)
            } else {
                Text(text).font(.callout.monospaced()).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                    .padding(10).background(.quaternary.opacity(0.5), in: .rect(cornerRadius: 8))
            }
        }
    }
}
