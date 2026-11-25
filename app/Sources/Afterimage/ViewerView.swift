import AfterimageEngine
import AfterimageKit
import SwiftUI

struct ViewerView: View {
    let shot: Shot
    @Environment(AppModel.self) private var model
    @State private var lines: [IndexedLine] = []
    @State private var zoom: CGFloat = 1
    @State private var pinchFrom: CGFloat = 1
    @AppStorage("inspector") private var inspector = true
    @AppStorage("outlineHidden") private var outline = false
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
            ToolbarItemGroup {
                Button("Previous", systemImage: "chevron.left") { model.step(-1) }.help("Previous picture (←)").disabled(!model.canStep(-1))
                Button("Next", systemImage: "chevron.right") { model.step(1) }.help("Next picture (→)").disabled(!model.canStep(1))
            }
            ToolbarItemGroup {
                Button("Zoom Out", systemImage: "minus.magnifyingglass") { setZoom(zoom / 1.5) }.keyboardShortcut("-").disabled(zoom <= 1)
                Button("Zoom In", systemImage: "plus.magnifyingglass") { setZoom(zoom * 1.5) }.keyboardShortcut("=").disabled(zoom >= Self.most)
                Button("Zoom to Fit", systemImage: "arrow.up.left.and.arrow.down.right") { setZoom(1) }.keyboardShortcut("9").disabled(zoom == 1)
            }
            ToolbarItem {
                Toggle("Outline Hidden Areas", systemImage: "eye.slash", isOn: $outline).help(outline ? "Stop outlining what was hidden" : "Outline what was hidden, and why")
            }
            ToolbarItem { Button("Info", systemImage: "info.circle") { inspector.toggle() }.help(inspector ? "Hide details" : "Show details") }
        }
        .focusable().focusEffectDisabled().focused($focused)
        .onAppear { focused = true }
        .onKeyPress(.leftArrow) { model.step(-1); return .handled }
        .onKeyPress(.rightArrow) { model.step(1); return .handled }
        .onKeyPress(.escape) { if zoom > 1 { setZoom(1) } else { model.open = nil }; return .handled }
        .onKeyPress(.space) { model.open = nil; return .handled }
        .onKeyPress(.delete) { model.askDelete([shot.id]); return .handled }
        .onChange(of: shot.id) { zoom = 1; pinchFrom = 1 }
        .task(id: shot.id) {
            lines = []
            let id = shot.id
            lines = (try? await Task.detached { [e = model.engine] in try e.lines(id: id) }.value) ?? []
        }
    }

    private static let most: CGFloat = 8
    private func setZoom(_ z: CGFloat) {
        withAnimation(.smooth(duration: 0.18)) { zoom = min(max(z, 1), Self.most) }
        pinchFrom = zoom
    }

    /// Fits the window at 1x; zoomed in, it scrolls like any document (two fingers, or the scroll bars).
    private var stage: some View {
        GeometryReader { g in
            let aspect = Double(max(shot.width, 1)) / Double(max(shot.height, 1))
            let w = min(g.size.width, g.size.height * aspect)
            ScrollView([.horizontal, .vertical]) {
                picture
                    .frame(width: w * zoom, height: w / aspect * zoom)
                    .frame(minWidth: g.size.width, minHeight: g.size.height)
            }
            .scrollBounceBehavior(.basedOnSize, axes: [.horizontal, .vertical])
            .scrollIndicators(zoom > 1 ? .automatic : .never)
        }
        .gesture(MagnifyGesture()
            .onChanged { zoom = min(max(pinchFrom * $0.magnification, 1), Self.most) }
            .onEnded { _ in pinchFrom = zoom })
    }

    private var picture: some View {
        Picture(path: shot.path, edge: 2880)
            .id(shot.id)
            .overlay { GeometryReader { g in marks(g.size) } }
            .clipShape(.rect(cornerRadius: 12 / zoom))
            .overlay { RoundedRectangle(cornerRadius: 12 / zoom).strokeBorder(.primary.opacity(0.1)) }
            .shadow(color: .black.opacity(0.2), radius: 20, y: 8)
            .onTapGesture(count: 2) { setZoom(zoom > 1 ? 1 : 2.5) }
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
                Group {
                    if outline {
                        RoundedRectangle(cornerRadius: 3).strokeBorder(Color.orange, style: StrokeStyle(lineWidth: 1.5, dash: [5, 3]))
                    } else {
                        Color.clear
                    }
                }
                .contentShape(.rect)
                .frame(width: g.rect.w * size.width, height: g.rect.h * size.height)
                .offset(x: g.rect.x * size.width, y: g.rect.y * size.height)
                .help(["\(Words.kind(g.kind)) hidden", Words.why(g.why)].filter { !$0.isEmpty }.joined(separator: ". "))
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
                    if !shot.describeModel.isEmpty { Text("Described by \(shot.describeModel)").font(.caption).foregroundStyle(.tertiary).textSelection(.enabled) }
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
                    ForEach(reasons, id: \.0) { key, kind, why, n in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(n == 1 ? Words.kind(kind) : "\(Words.kind(kind)) × \(n)")
                            if !why.isEmpty { Text(Words.why(why)).font(.callout).foregroundStyle(.secondary) }
                        }
                        .accessibilityElement(children: .combine)
                    }
                }
                LabeledContent("Checked by", value: shot.aiChecked ? "Local model and rules" : "Rules only")
                if !shot.checkModel.isEmpty { row("Model", shot.checkModel) }
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
    /// One row per kind and reason, most common first.
    private var reasons: [(String, String, String, Int)] {
        Dictionary(grouping: shot.regions) { "\($0.kind)|\($0.why)" }
            .map { ($0.key, $0.value[0].kind, $0.value[0].why, $0.value.count) }.sorted { ($1.3, $0.0) < ($0.3, $1.0) }
    }
    private func row(_ k: String, _ v: String) -> some View {
        LabeledContent(k) { Text(v).textSelection(.enabled).multilineTextAlignment(.trailing).lineLimit(4) }
    }
}
