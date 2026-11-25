import AfterimageEngine
import AppKit
import Foundation
import Testing
@testable import AfterimageKit

/// Scores models on the eval screens: what must be hidden is, what must stay does, and how long it took.
///   ./scripts/eval-render.sh
///   AFTERIMAGE_EVAL=1 AFTERIMAGE_EVAL_MODELS="model-a,model-b" swift test --filter Eval     (in app/)
/// Results: build/eval/results.json and build/eval/results.md. "rules" is the app with no model at all.
/// AIDEV-NOTE: a fragment only counts if text recognition can read it in the untouched picture; otherwise its
/// AIDEV-NOTE: absence afterwards would prove nothing.
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["AFTERIMAGE_EVAL"] != nil)) struct EvalTests {
    struct Case: Decodable { let hide: [String]; let keep: [String]; let note: String }
    struct Row: Encodable {
        var model: String, name: String
        var hideTotal = 0, hidden = 0, keepTotal = 0, kept = 0, regions = 0
        var seconds = 0.0, aiChecked = false, missed: [String] = [], lost: [String] = [], problem = ""
    }

    static func squash(_ s: String) -> String { s.lowercased().filter { !$0.isWhitespace } }

    @Test func scoreModels() async throws {
        let root = fixtures.deletingLastPathComponent()
        let cases = try JSONDecoder().decode([String: Case].self, from: Data(contentsOf: root.appending(path: "eval/cases.json")))
        let env = ProcessInfo.processInfo.environment
        let models = ["rules"] + (env["AFTERIMAGE_EVAL_MODELS"] ?? "").split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
        // Read every picture once.
        var loaded: [(name: String, c: Case, bmp: Bitmap, lines: [TextLine], raw: String)] = []
        for (name, c) in cases.sorted(by: { $0.key < $1.key }) {
            guard let image = Pixels.load(root.appending(path: "eval/png/\(name).png").path), let bmp = Pixels.bitmap(image) else { print("EVAL no picture for \(name); run scripts/eval-render.sh"); continue }
            let lines = try Recognizer.lines(in: image)
            loaded.append((name, c, bmp, lines, Self.squash(lines.map(\.text).joined(separator: "\n"))))
        }
        #expect(!loaded.isEmpty)
        var rows: [Row] = []
        for model in models {
            let dir = FileManager.default.temporaryDirectory.appending(path: "afterimage-eval-\(UUID())")
            defer { try? FileManager.default.removeItem(at: dir) }
            let rulesOnly = model == "rules"
            let config = Config(lmUrl: rulesOnly ? "http://127.0.0.1:9" : (env["AFTERIMAGE_LM_URL"] ?? "http://127.0.0.1:1234"), model: rulesOnly ? "none" : model, embedModel: "", requireAi: !rulesOnly, describe: .never, maxAgeDays: 0, maxBytes: 0, unloadAfterSecs: 0, developer: false)
            let engine = try Engine(root: dir.path, config: config, host: ImageHost())
            for (i, x) in loaded.enumerated() {
                var row = Row(model: model, name: x.name)
                let before = (try engine.stats().count, engine.status().discarded)
                let started = Date()
                let moment = Moment(takenAt: 1_800_000_000_000 + Int64(i), trigger: "manual", display: "Eval", appName: "Eval", bundleId: "eval", windowTitle: x.name, url: "", userName: "", fullName: "", host: "", sessions: [], runningApps: [])
                engine.ingest(frame: Frame(rgba: x.bmp.data, width: UInt32(x.bmp.width), height: UInt32(x.bmp.height), stride: UInt32(x.bmp.stride), lines: x.lines, moment: moment, displayKey: "eval-\(i)"))
                for _ in 0..<6000 where try engine.stats().count == before.0 && engine.status().discarded == before.1 { try await Task.sleep(for: .milliseconds(50)) }
                row.seconds = (Date().timeIntervalSince(started) * 10).rounded() / 10
                let shot = try engine.list(filter: Filter(text: "", bundleId: nil, from: nil, to: nil, onlyKept: false, onlyHidden: false), limit: 200, offset: 0).first { $0.moment.windowTitle == x.name }
                if let shot, let saved = Pixels.load(shot.path) {
                    let seen = Self.squash(try Recognizer.lines(in: saved).map(\.text).joined(separator: "\n"))
                    let indexed = Self.squash(try engine.lines(id: shot.id).map(\.text).joined(separator: "\n"))
                    (row.regions, row.aiChecked) = (shot.regions.count, shot.aiChecked)
                    for h in x.c.hide.map(Self.squash) where x.raw.contains(h) {
                        row.hideTotal += 1
                        if seen.contains(h) || indexed.contains(h) { row.missed.append(h) } else { row.hidden += 1 }
                    }
                    for k in x.c.keep.map(Self.squash) where x.raw.contains(k) {
                        row.keepTotal += 1
                        if seen.contains(k) { row.kept += 1 } else { row.lost.append(k) }
                    }
                } else {
                    row.problem = engine.status().discardReason.isEmpty ? "not saved" : engine.status().discardReason
                }
                print("EVAL \(model) \(x.name): hidden \(row.hidden)/\(row.hideTotal) kept \(row.kept)/\(row.keepTotal) areas \(row.regions) \(row.seconds)s \(row.problem)")
                rows.append(row)
            }
            engine.shutdown()
            // AIDEV-NOTE: free the model before the next one; left loaded, later models fail for lack of memory.
            if !rulesOnly, let u = URL(string: config.lmUrl + "/api/v1/models/unload") {
                var r = URLRequest(url: u); r.httpMethod = "POST"; r.timeoutInterval = 60
                r.setValue("application/json", forHTTPHeaderField: "content-type")
                r.httpBody = try JSONSerialization.data(withJSONObject: ["instance_id": model])
                _ = try? await URLSession.shared.data(for: r)
            }
        }
        let out = root.appending(path: "build/eval")
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        let enc = JSONEncoder(); enc.outputFormatting = [.prettyPrinted, .sortedKeys]
        try enc.encode(rows).write(to: out.appending(path: "results.json"))
        var md = "| Model | Secrets hidden | Ordinary text kept | Areas hidden on no-secret screens | Seconds per screen | Screens failed |\n|---|---|---|---|---|---|\n"
        for m in models {
            let r = rows.filter { $0.model == m }, ok = r.filter { $0.problem.isEmpty }
            let clean = ok.filter { cases[$0.name]?.hide.isEmpty == true }.map(\.regions).reduce(0, +)
            let secs = ok.isEmpty ? 0 : (ok.map(\.seconds).reduce(0, +) / Double(ok.count) * 10).rounded() / 10
            md += "| \(m) | \(ok.map(\.hidden).reduce(0, +))/\(ok.map(\.hideTotal).reduce(0, +)) | \(ok.map(\.kept).reduce(0, +))/\(ok.map(\.keepTotal).reduce(0, +)) | \(clean) | \(secs) | \(r.count - ok.count) |\n"
        }
        try md.write(to: out.appending(path: "results.md"), atomically: true, encoding: .utf8)
        print(md)
    }
}
