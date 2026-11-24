import AfterimageEngine
import AfterimageKit
import SwiftUI

struct ViewerView: View {
    let shot: Shot
    @Environment(AppModel.self) private var model
    @State private var lines: [IndexedLine] = []
    @AppStorage("inspector") private var inspector = true
    @FocusState private var focused: Bool

    var body: some View {
        VStack(spacing: 0) {
            stage.padding(.horizontal, 28).padding(.top, 20).padding(.bottom, 14)
            Filmstrip(current: shot.id)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .windowBackgroundColor))
        .inspector(isPresented: $inspector) {
            Details(shot: shot, lines: lines).inspectorColumnWidth(min: 240, ideal: 290, max: 380)
        }
        .toolbar {
            ToolbarItem { Button("Info", systemImage: "info.circle") { inspector.toggle() }.help(inspector ? "Hide details" : "Show details") }
        }
        .focusable().focusEffectDisabled().focused($focused)
        .onAppear { focused = true }
        .onKeyPress(.leftArrow) { model.step(-1); return .handled }
        .onKeyPress(.rightArrow) { model.step(1); return .handled }
        .onKeyPress(.escape) { model.open = nil; return .handled }
        .onKeyPress(.space) { model.open = nil; return .handled }
        .onKeyPress(.delete) { model.askDelete([shot.id]); return .handled }
        .task(id: shot.id) {
            lines = []
            let id = shot.id
            lines = (try? await Task.detached { [e = model.engine] in try e.lines(id: id) }.value) ?? []
        }
    }

    private var stage: some View {
        Picture(path: shot.path, edge: 2880)
            .id(shot.id)
            .aspectRatio(Double(max(shot.width, 1)) / Double(max(shot.height, 1)), contentMode: .fit)
            .overlay { GeometryReader { g in marks(g.size) } }
            .clipShape(.rect(cornerRadius: 12))
            .overlay { RoundedRectangle(cornerRadius: 12).strokeBorder(.primary.opacity(0.1)) }
            .shadow(color: .black.opacity(0.2), radius: 20, y: 8)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .contextMenu { ShotMenu(shot: shot) }
            .accessibilityLabel(shot.description.isEmpty ? "Screenshot of \(shot.moment.appName)" : shot.description)
    }

    private func marks(_ size: CGSize) -> some View {
        ZStack(alignment: .topLeading) {
            ForEach(Array(matches.enumerated()), id: \.offset) { _, r in
                RoundedRectangle(cornerRadius: 4).fill(Color.yellow.opacity(0.35)).blendMode(.multiply)
                    .overlay { RoundedRectangle(cornerRadius: 4).strokeBorder(Color.yellow, lineWidth: 1.5) }
                    .frame(width: r.w * size.width + 8, height: r.h * size.height + 6)
                    .offset(x: r.x * size.width - 4, y: r.y * size.height - 3)
            }
            ForEach(Array(shot.regions.enumerated()), id: \.offset) { _, g in
                Color.clear.contentShape(.rect)
                    .frame(width: g.rect.w * size.width, height: g.rect.h * size.height)
                    .offset(x: g.rect.x * size.width, y: g.rect.y * size.height)
                    .help("\(Words.kind(g.kind)) hidden")
            }
        }
        .allowsHitTesting(true)
    }

    private var matches: [AfterimageEngine.Rect] {
        let words = model.query.lowercased().split { !$0.isLetter && !$0.isNumber }.map(String.init).filter { $0.count > 1 }
        guard !words.isEmpty else { return [] }
        return lines.filter { l in let t = l.text.lowercased(); return words.contains { t.contains($0) } }.map(\.rect)
    }
}

/// The moments around this one.
struct Filmstrip: View {
    let current: Int64
    @Environment(AppModel.self) private var model
    var body: some View {
        ScrollViewReader { p in
            ScrollView(.horizontal, showsIndicators: false) {
                LazyHStack(spacing: 8) {
                    ForEach(model.shots) { s in
                        Button { model.open = s.id; model.selection = [s.id] } label: {
                            Picture(path: s.thumbPath, edge: 240)
                                .aspectRatio(Double(max(s.width, 1)) / Double(max(s.height, 1)), contentMode: .fit)
                                .clipShape(.rect(cornerRadius: 5))
                                .overlay { if s.id == current { RoundedRectangle(cornerRadius: 6).stroke(Color.accentColor, lineWidth: 2.5).padding(-1.5) } }
                                .opacity(s.id == current ? 1 : 0.72)
                        }
                        .buttonStyle(.plain).id(s.id)
                        .accessibilityLabel("\(s.moment.appName), \(s.date.formatted(date: .omitted, time: .shortened))")
                    }
                }
                .padding(.horizontal, 28).padding(.vertical, 4)
            }
            .frame(height: 62)
            .padding(.bottom, 14)
            .onAppear { p.scrollTo(current, anchor: .center) }
            .onChange(of: current) { withAnimation(.smooth(duration: 0.2)) { p.scrollTo(current, anchor: .center) } }
        }
    }
}

struct Details: View {
    let shot: Shot
    let lines: [IndexedLine]
    @State private var showText = false

    var body: some View {
        Form {
            Section {
                VStack(alignment: .leading, spacing: 10) {
                    if shot.description.isEmpty {
                        Text(shot.indexed ? "No description" : "Not described yet").foregroundStyle(.secondary)
                    } else {
                        Text(shot.description).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                    }
                    if !shot.tags.isEmpty { Text(shot.tags.joined(separator: " · ")).font(.callout).foregroundStyle(.secondary) }
                }
            }
            Section {
                row("When", shot.date.formatted(date: .abbreviated, time: .standard))
                row("Why", Words.trigger(shot.moment.trigger))
                row("App", shot.moment.appName)
                if !shot.moment.windowTitle.isEmpty { row("Window", shot.moment.windowTitle) }
                if !shot.moment.url.isEmpty { row("Page", shot.moment.url) }
                row("Display", shot.moment.display)
            }
            Section("Privacy") {
                if shot.regions.isEmpty {
                    Text("No secrets found").foregroundStyle(.secondary)
                } else {
                    ForEach(kinds, id: \.0) { k, n in LabeledContent(Words.kind(k), value: n == 1 ? "Hidden" : "\(n) hidden") }
                }
                LabeledContent("Checked by", value: shot.aiChecked ? "Local model and rules" : "Rules only")
            }
            Section("Signed In") {
                row("User", shot.moment.fullName.isEmpty ? shot.moment.userName : "\(shot.moment.fullName) (\(shot.moment.userName))")
                row("Mac", shot.moment.host)
                if !shot.moment.sessions.isEmpty { row("Sessions", shot.moment.sessions.joined(separator: "\n")) }
            }
            if !shot.moment.runningApps.isEmpty {
                Section("Running") {
                    ForEach(shot.moment.runningApps, id: \.bundleId) { a in
                        Label { Text(a.name) } icon: { AppIcon(bundleId: a.bundleId, size: 16) }
                    }
                }
            }
            if !lines.isEmpty {
                Section {
                    DisclosureGroup("Text on Screen", isExpanded: $showText) {
                        Text(lines.map(\.text).joined(separator: "\n")).font(.callout).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
            }
        }
        .formStyle(.grouped).scrollContentBackground(.hidden)
    }
    private var kinds: [(String, Int)] { Dictionary(grouping: shot.regions, by: \.kind).map { ($0.key, $0.value.count) }.sorted { $0.0 < $1.0 } }
    private func row(_ k: String, _ v: String) -> some View {
        LabeledContent(k) { Text(v).textSelection(.enabled).multilineTextAlignment(.trailing).lineLimit(4) }
    }
}
