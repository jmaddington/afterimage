import AfterimageEngine
import AfterimageKit
import SwiftUI

struct GridView: View {
    @Environment(AppModel.self) private var model
    private let columns = [GridItem(.adaptive(minimum: 230, maximum: 340), spacing: 18)]

    var body: some View {
        if model.loaded && model.shots.isEmpty {
            empty
        } else {
            ScrollView {
                LazyVGrid(columns: columns, alignment: .leading, spacing: 22, pinnedViews: []) {
                    if model.isSearching {
                        ForEach(model.shots) { cell($0) }
                    } else {
                        ForEach(model.groups) { g in
                            Section {
                                ForEach(g.shots) { cell($0) }
                            } header: {
                                Text(Words.day(g.id)).font(.title2.weight(.semibold)).padding(.top, 10)
                                    .accessibilityAddTraits(.isHeader)
                            }
                        }
                    }
                }
                .padding(.horizontal, 24).padding(.vertical, 16)
            }
            .background { Color.clear.contentShape(.rect).onTapGesture { model.selection = [] } }
        }
    }

    private func cell(_ s: Shot) -> some View {
        ShotCell(shot: s, selected: model.selection.contains(s.id), searching: model.isSearching)
            .onTapGesture(count: 2) { model.selection = [s.id]; model.open = s.id }
            .simultaneousGesture(TapGesture().onEnded { select(s) })
            .contextMenu { ShotMenu(shot: s) }
            .accessibilityAddTraits(model.selection.contains(s.id) ? [.isButton, .isSelected] : .isButton)
            .accessibilityAction { model.open = s.id }
            .onAppear { model.loadMore(after: s) }
    }

    private func select(_ s: Shot) {
        let f = NSEvent.modifierFlags
        if f.contains(.command) {
            model.selection.formSymmetricDifference([s.id])
        } else if f.contains(.shift), let anchor = model.shots.firstIndex(where: { model.selection.contains($0.id) }),
                  let here = model.shots.firstIndex(where: { $0.id == s.id }) {
            model.selection.formUnion(model.shots[min(anchor, here)...max(anchor, here)].map(\.id))
        } else {
            model.selection = [s.id]
        }
    }

    @ViewBuilder private var empty: some View {
        if model.isSearching {
            ContentUnavailableView.search(text: model.query)
        } else {
            switch model.place {
            case .kept: ContentUnavailableView("Nothing Kept", systemImage: "bookmark", description: Text("Keep a picture and it stays until you delete it."))
            case .hidden: ContentUnavailableView("No Secrets Found", systemImage: "eye.slash", description: Text("Pictures with something hidden will appear here."))
            default:
                if model.scope == .any {
                    ContentUnavailableView("Your day will appear here", systemImage: "circle.circle", description: Text("Afterimage takes a picture as you move between apps, windows and tabs."))
                } else {
                    ContentUnavailableView("Nothing from \(model.scope.rawValue.lowercased())", systemImage: "calendar")
                }
            }
        }
    }
}

struct ShotCell: View {
    let shot: Shot
    let selected: Bool
    let searching: Bool
    @State private var hover = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Picture(path: shot.thumbPath, edge: 720)
                .aspectRatio(Double(max(shot.width, 1)) / Double(max(shot.height, 1)), contentMode: .fit)
                .clipShape(.rect(cornerRadius: 10))
                .overlay { RoundedRectangle(cornerRadius: 10).strokeBorder(.primary.opacity(0.1)) }
                .overlay { if selected { RoundedRectangle(cornerRadius: 12).stroke(Color.accentColor, lineWidth: 3).padding(-2) } }
                .shadow(color: .black.opacity(hover ? 0.22 : 0.1), radius: hover ? 12 : 5, y: hover ? 5 : 2)
                .overlay(alignment: .topTrailing) {
                    if shot.kept { Image(systemName: "bookmark.fill").font(.caption).padding(6).glassEffect(.regular, in: .circle).padding(7) }
                }
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                AppIcon(bundleId: shot.moment.bundleId, size: 15).alignmentGuide(.firstTextBaseline) { $0[.bottom] - 2 }
                Text(headline).lineLimit(1).truncationMode(.tail)
                Spacer(minLength: 6)
                Text(when).foregroundStyle(.secondary).monospacedDigit().fixedSize()
            }
            .font(.callout)
        }
        .contentShape(.rect)
        .onHover { hover = $0 }
        .animation(.smooth(duration: 0.18), value: hover)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(shot.moment.appName), \(headline), \(shot.date.formatted(date: .abbreviated, time: .shortened))\(shot.regions.isEmpty ? "" : ", secrets hidden")")
        .help(shot.description.isEmpty ? headline : shot.description)
    }
    private var headline: String { shot.moment.windowTitle.isEmpty ? shot.moment.appName : shot.moment.windowTitle }
    private var when: String {
        searching && !Calendar.current.isDateInToday(shot.date)
            ? shot.date.formatted(.dateTime.month(.abbreviated).day()) : shot.date.formatted(date: .omitted, time: .shortened)
    }
}

struct ShotMenu: View {
    let shot: Shot
    @Environment(AppModel.self) private var model
    @Environment(AppSettings.self) private var settings

    private var targets: Set<Int64> { model.selection.contains(shot.id) ? model.selection : [shot.id] }
    private var excluded: Bool { settings.excludedApps.contains(shot.moment.bundleId) }

    var body: some View {
        Button("Open") { model.selection = [shot.id]; model.open = shot.id }
        Button("Copy Picture") {
            if let i = NSImage(contentsOfFile: shot.path) { NSPasteboard.general.clearContents(); NSPasteboard.general.writeObjects([i]) }
        }
        Button("Show in Finder") { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: shot.path)]) }
        Divider()
        Button(shot.kept ? "Don’t Keep" : "Keep") { model.toggleKept(targets) }
        if !shot.moment.bundleId.isEmpty {
            Button(excluded ? "\(shot.moment.appName) Is Excluded" : "Never Capture \(shot.moment.appName)") { settings.excludedApps.append(shot.moment.bundleId) }.disabled(excluded)
        }
        if let h = URL(string: shot.moment.url)?.host() {
            let site = normalizeSite(raw: h)
            if let site { Button("Never Capture \(site)") { if !settings.excludedSites.contains(site) { settings.excludedSites.append(site) } } }
        }
        Divider()
        Button(targets.count == 1 ? "Delete…" : "Delete \(targets.count) Pictures…", role: .destructive) { model.askDelete(targets) }
    }
}
