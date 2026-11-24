import AfterimageEngine
import AfterimageKit
import AppKit
import Observation
import SwiftUI

enum Place: Hashable {
    case all, kept, hidden
    case app(String)
}

enum DateScope: String, CaseIterable, Identifiable {
    case any = "Any Time", today = "Today", yesterday = "Yesterday", week = "Past 7 Days", month = "Past 30 Days"
    var id: Self { self }
    var range: (Int64?, Int64?) {
        let c = Calendar.current, day = c.startOfDay(for: Date())
        func ms(_ d: Date) -> Int64 { Int64(d.timeIntervalSince1970 * 1000) }
        switch self {
        case .any: return (nil, nil)
        case .today: return (ms(day), nil)
        case .yesterday: return (ms(c.date(byAdding: .day, value: -1, to: day)!), ms(day))
        case .week: return (ms(c.date(byAdding: .day, value: -6, to: day)!), nil)
        case .month: return (ms(c.date(byAdding: .day, value: -29, to: day)!), nil)
        }
    }
}

struct Forget: Identifiable {
    let title: String
    let seconds: TimeInterval?
    var id: String { title }
}

struct DayGroup: Identifiable {
    let id: Date
    let shots: [Shot]
}

extension Shot: Identifiable {}
extension Shot {
    var date: Date { Date(timeIntervalSince1970: Double(moment.takenAt) / 1000) }
}

@MainActor @Observable final class AppModel {
    static let shared = AppModel()

    let settings: AppSettings
    let recorder: Recorder
    let engine: Engine
    private let host = ImageHost()

    var shots: [Shot] = []
    var groups: [DayGroup] = []
    var apps: [AppCount] = []
    var stats = Stats(count: 0, bytes: 0, kept: 0, hidden: 0, pending: 0, oldest: nil)
    var status: Status?
    var loaded = false

    var place: Place = .all { didSet { if place != oldValue { open = nil; reload() } } }
    var scope: DateScope = .any { didSet { if scope != oldValue { reload() } } }
    var query = "" { didSet { if query != oldValue { searchSoon() } } }
    var searching = false
    var selection: Set<Int64> = []
    var open: Int64?
    var focusSearch = 0
    var confirmDelete: [Int64]?
    var confirmForget: Forget?
    var problem: String?

    private var searchTask: Task<Void, Never>?
    private var refreshTask: Task<Void, Never>?
    private var generation = 0
    private var limit: UInt32 = 300

    private init() {
        let env = ProcessInfo.processInfo.environment
        let defaults = env["AFTERIMAGE_DEFAULTS"].flatMap { UserDefaults(suiteName: $0) } ?? .standard
        settings = AppSettings(defaults)
        var root = env["AFTERIMAGE_HOME"].map { URL(fileURLWithPath: $0) }
            ?? URL.applicationSupportDirectory.appending(path: "Afterimage", directoryHint: .isDirectory)
        try? FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        // Deleted pictures shouldn't live on in Time Machine.
        var v = URLResourceValues()
        v.isExcludedFromBackup = true
        try? root.setResourceValues(v)
        do {
            engine = try Engine(root: root.path, config: settings.config, host: host)
        } catch {
            let a = NSAlert()
            a.messageText = "Afterimage can’t open its library."
            a.informativeText = error.localizedDescription
            a.runModal()
            exit(1)
        }
        recorder = Recorder(settings: settings, engine: engine)
    }

    func start() {
        host.onChange = { [weak self] in Task { @MainActor in self?.refreshSoon() } }
        settings.changed = { [weak self] in
            guard let self else { return }
            let c = settings.config
            Task.detached { [engine] in engine.setConfig(config: c) }
        }
        reload()
        Task.detached { [engine] in _ = engine.listenForPages() }
        if settings.onboarded { recorder.start() }
        #if DEBUG
        stage(ProcessInfo.processInfo.environment["AFTERIMAGE_STAGE"] ?? "")
        #endif
        Task { [weak self] in
            while let self {
                await self.probe()
                try? await Task.sleep(for: .seconds(8))
            }
        }
    }

    func finishOnboarding() {
        settings.finishOnboarding()
        recorder.start()
    }

    func stop() {
        recorder.stop()
        engine.shutdown()
    }

    func probe() async {
        let s = await Task.detached { [engine] in engine.probe() }.value
        status = s
    }

    #if DEBUG
    /// Puts the window in a given state so screenshots of the UI need no clicking.
    private func stage(_ s: String) {
        Task {
            try? await Task.sleep(for: .seconds(2))
            if s.hasPrefix("search:") { query = String(s.dropFirst(7)) }
            if s.hasPrefix("open:") { query = String(s.dropFirst(5)); try? await Task.sleep(for: .seconds(2)); open = shots.first?.id; selection = Set(shots.prefix(1).map(\.id)) }
        }
    }
    #endif

    // MARK: Library

    private var filter: Filter {
        let (from, to) = scope.range
        var f = Filter(text: query, bundleId: nil, from: from, to: to, onlyKept: false, onlyHidden: false)
        switch place {
        case .all: break
        case .kept: f.onlyKept = true
        case .hidden: f.onlyHidden = true
        case .app(let b): f.bundleId = b
        }
        return f
    }
    var isSearching: Bool { !query.trimmingCharacters(in: .whitespaces).isEmpty }

    private func refreshSoon() {
        guard refreshTask == nil else { return }
        refreshTask = Task {
            try? await Task.sleep(for: .milliseconds(400))
            refreshTask = nil
            reload()
        }
    }
    private func searchSoon() {
        searchTask?.cancel()
        searchTask = Task {
            try? await Task.sleep(for: .milliseconds(140))
            if !Task.isCancelled { reload(meaning: true) }
        }
    }
    func loadMore(after s: Shot) {
        guard !isSearching, s.id == shots.last?.id, shots.count >= Int(limit) else { return }
        limit += 300
        reload()
    }

    func reload(meaning: Bool = false) {
        generation += 1
        let g = generation, f = filter, searching = isSearching, n = limit
        self.searching = searching && meaning
        Task {
            @MainActor func show(_ r: [Shot]) {
                guard g == generation else { return }
                shots = r
                groups = Self.group(r)
                loaded = true
                selection.formIntersection(r.map(\.id))
            }
            do {
                let first = try await Task.detached { [engine] in
                    searching ? try engine.search(filter: f, semantic: false, limit: 200) : try engine.list(filter: f, limit: n, offset: 0)
                }.value
                show(first)
                let side = try await Task.detached { [engine] in (try engine.apps(), try engine.stats()) }.value
                if g == generation {
                    (apps, stats) = side
                    // The last picture of an app takes its row with it.
                    if case .app(let b) = place, !apps.contains(where: { $0.bundleId == b }) { place = .all }
                }
                // Words now, meaning a moment later.
                if searching {
                    let full = try await Task.detached { [engine] in try engine.search(filter: f, semantic: true, limit: 200) }.value
                    show(full)
                }
            } catch { problem = error.localizedDescription }
            if g == generation { self.searching = false }
        }
    }

    static func group(_ s: [Shot]) -> [DayGroup] {
        let c = Calendar.current
        return Dictionary(grouping: s) { c.startOfDay(for: $0.date) }
            .map { DayGroup(id: $0.key, shots: $0.value) }.sorted { $0.id > $1.id }
    }

    var openShot: Shot? { open.flatMap { id in shots.first { $0.id == id } } }
    func canStep(_ by: Int) -> Bool {
        guard let id = open, let i = shots.firstIndex(where: { $0.id == id }) else { return false }
        return shots.indices.contains(i + by)
    }
    /// From the developer log: show this picture wherever the person was.
    func show(_ id: Int64) {
        query = ""
        place = .all
        selection = [id]
        open = id
        reload()
    }
    func step(_ by: Int) {
        guard let id = open, let i = shots.firstIndex(where: { $0.id == id }), shots.indices.contains(i + by) else { return }
        open = shots[i + by].id
        selection = [shots[i + by].id]
    }

    // MARK: Actions

    var selectionIsKept: Bool { !selection.isEmpty && shots.filter { selection.contains($0.id) }.allSatisfy(\.kept) }
    func toggleKept(_ ids: Set<Int64>? = nil) {
        let target = ids ?? selection
        let kept = !shots.filter { target.contains($0.id) }.allSatisfy(\.kept)
        run { try $0.setKept(ids: Array(target), kept: kept) }
    }
    func askDelete(_ ids: Set<Int64>? = nil) {
        let t = ids ?? selection
        if !t.isEmpty { confirmDelete = Array(t) }
    }
    func delete(_ ids: [Int64]) {
        // Land on the neighbour rather than dropping the person back at the top.
        if let o = open, ids.contains(o), let i = shots.firstIndex(where: { $0.id == o }) {
            open = (shots[(i + 1)...].first ?? shots[..<i].last)?.id
        }
        selection.subtract(ids)
        run { _ = try $0.delete(ids: ids) }
    }
    func forget(_ f: Forget) {
        open = nil
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let from = f.seconds.map { now - Int64($0 * 1000) } ?? Int64.min
        run { _ = try $0.forget(from: from, to: f.seconds == nil ? .max : now + 1) }
    }
    private func run(_ f: @escaping @Sendable (Engine) throws -> Void) {
        Task {
            do { try await Task.detached { [engine] in try f(engine) }.value } catch { problem = error.localizedDescription }
            reload()
        }
    }

    // MARK: Status

    var menuSymbol: String {
        switch recorder.state {
        case .watching: lmTrouble == nil ? "circle.circle.fill" : "exclamationmark.circle"
        case .off, .paused: "pause.circle"
        case .needsPermission: "exclamationmark.circle"
        case .waiting: "circle.circle"
        }
    }
    /// Non-nil when pictures can't be checked, and so aren't being saved.
    var lmTrouble: String? {
        guard let s = status else { return nil }
        switch s.lm {
        case .offline: return "LM Studio isn’t running"
        case .noModel: return "The chosen model isn’t in LM Studio"
        case .error(let m): return m
        case .ready, .unknown: return nil
        }
    }
    var statusLine: String {
        switch recorder.state {
        case .off: return "Off"
        case .needsPermission: return "Needs permission to see the screen"
        case .paused(let d): return d.map { "Paused until \($0.formatted(date: .omitted, time: .shortened))" } ?? "Paused"
        case .waiting(let why): return why
        case .watching:
            if let t = lmTrouble { return settings.requireAI ? "\(t), so nothing is being saved" : "\(t), using built-in rules" }
            if status?.busy == true { return "Checking for secrets" }
            if !recorder.lastNote.isEmpty { return recorder.lastNote }
            if let d = recorder.lastCapture { return "Last captured \(d.formatted(date: .omitted, time: .shortened))" }
            return "Watching"
        }
    }
}
