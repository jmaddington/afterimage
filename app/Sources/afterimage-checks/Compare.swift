// Developer tool: the same pictures described by several models, scored side by side.
// AIDEV-NOTE: pictures go through the engine like any capture (secrets hidden first), into a throwaway library per model.
import AfterimageEngine
import AfterimageKit
import Foundation

struct Picture { let name: String, bitmap: Bitmap, lines: [TextLine], screen: String, facts: [String], secrets: [String] }
struct Row: Codable { var model, variant, picture: String; var seconds: Double?; var text: String }
struct Line { var label: String; var done = 0, of = 0; var check: [Double] = []; var seconds: [Double] = [], chars: [Double] = [], recall: [Double] = [], grounded: [Double] = [], words: [Double] = [], meaning: [Double] = []; var leaks = 0; var problem = "" }

func post(_ path: String, _ body: [String: Any], timeout: TimeInterval = 120) -> [String: Any]? {
    var r = URLRequest(url: URL(string: "http://127.0.0.1:1234\(path)")!, timeoutInterval: timeout)
    r.httpMethod = "POST"
    r.setValue("application/json", forHTTPHeaderField: "content-type")
    r.httpBody = try? JSONSerialization.data(withJSONObject: body)
    let done = DispatchSemaphore(value: 0)
    var out: [String: Any]?
    URLSession.shared.dataTask(with: r) { d, _, _ in
        out = d.flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }
        done.signal()
    }.resume()
    done.wait()
    return out
}

func embed(_ text: String, model: String) -> [Double]? {
    let v = post("/v1/embeddings", ["model": model, "input": String(text.prefix(6000))])
    return ((v?["data"] as? [[String: Any]])?.first?["embedding"] as? [NSNumber])?.map(\.doubleValue)
}

func mean(_ v: [Double]) -> String { v.isEmpty ? "-" : String(format: "%.0f%%", 100 * v.reduce(0, +) / Double(v.count)) }

func compare(out: String, pictures dir: String, env: [String: String]) throws {
    let listed = (env["AFTERIMAGE_MODELS"] ?? "").split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
    guard !listed.isEmpty else { print("set AFTERIMAGE_MODELS to a comma-separated list of LM Studio model keys"); exit(2) }
    // AFTERIMAGE_REASONING: thinking levels to try, e.g. "off,low,high". Each is a pass of its own for every model.
    let efforts = (env["AFTERIMAGE_REASONING"] ?? "off").split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
    let detail = AppSettings.detail(env["AFTERIMAGE_DETAIL"] ?? "brief", developer: true)
    let embedModel = env["AFTERIMAGE_EMBED"] ?? "text-embedding-nomic-embed-text-v1.5"
    let perPicture = Double(env["AFTERIMAGE_PATIENCE"] ?? "") ?? 300
    let fm = FileManager.default
    var shown: [Picture] = []
    for file in try fm.contentsOfDirectory(atPath: dir).filter({ $0.hasSuffix(".png") }).sorted() {
        let name = String(file.dropLast(4))
        guard let img = Pixels.load("\(dir)/\(file)"), let b = Pixels.bitmap(img) else { continue }
        let lines = try Recognizer.lines(in: img)
        // name.facts: one thing per line a good description mentions; a line starting with ! is a secret it must not.
        let notes = ((try? String(contentsOfFile: "\(dir)/\(name).facts", encoding: .utf8)) ?? "").split(separator: "\n").map(String.init)
        shown.append(Picture(name: name, bitmap: b, lines: lines, screen: lines.map(\.text).joined(separator: "\n"),
                             facts: notes.filter { !$0.hasPrefix("!") }, secrets: notes.filter { $0.hasPrefix("!") }.map { String($0.dropFirst()) }))
    }
    guard !shown.isEmpty else { print("no .png pictures in \(dir)"); exit(1) }
    setvbuf(stdout, nil, _IOLBF, 0)
    print("\(shown.count) pictures, \(listed.count) models, thinking: \(efforts.joined(separator: ", ")), detail: \(detail)")
    var rows: [Row] = [], problems: [String: String] = [:], checkTimes: [String: [Double]] = [:]
    // Each model gets the nearest level it allows, so two levels asked for can be one pass: off and low on a model
    // with only off and on.
    let known = ((try? Engine(root: "\(out)/.models", config: Config(lmUrl: "http://127.0.0.1:1234", model: "", embedModel: "", requireAi: false, describe: .never, maxAgeDays: 0, maxBytes: 0,
        unloadAfterSecs: 0, detail: .brief, customLong: false, customTokens: 900, customChars: 4000, customEdge: 1280), host: ImageHost()).models()) ?? [])
    var passes: [(name: String, key: String, effort: String)] = []
    for m in listed {
        let allowed = known.first { $0.key == m }?.reasoning ?? []
        var done: Set<String> = []
        for e in efforts {
            let real = thinkingLevel(wanted: e, allowed: allowed) ?? "none"
            guard done.insert(real).inserted else { continue }
            passes.append((efforts.count > 1 ? "\(m) @\(real)" : m, m, e))
        }
    }
    let models = passes.map(\.name)
    for pass in passes {
        let model = pass.name
        let root = "\(out)/\(model.replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: " @", with: "_"))"
        try? fm.removeItem(atPath: root)
        let config = Config(lmUrl: "http://127.0.0.1:1234", model: pass.key, embedModel: "", requireAi: true, describe: .always, maxAgeDays: 0, maxBytes: 0,
                            unloadAfterSecs: 0, detail: detail, customLong: false, customTokens: 900, customChars: 4000, customEdge: 1280, reasoning: pass.effort)
        let engine = try Engine(root: root, config: config, host: ImageHost())
        defer { engine.shutdown() }
        let began = Date()
        var saved: UInt64 = 0
        var checks: [Double] = []
        for (i, p) in shown.enumerated() {
            let m = Moment(takenAt: Int64(Date().timeIntervalSince1970 * 1000) + Int64(i), trigger: "manual", display: "Compare", appName: "", bundleId: "compare.test", windowTitle: p.name,
                           url: "", userName: "", fullName: "", host: "", sessions: [], runningApps: [])
            engine.ingest(frame: Frame(rgba: p.bitmap.data, width: UInt32(p.bitmap.width), height: UInt32(p.bitmap.height), stride: UInt32(p.bitmap.stride), lines: p.lines, moment: m, displayKey: "c\(i)"))
            saved += 1
            let sent = Date()
            let give = Date().addingTimeInterval(perPicture)
            while try engine.stats().count < saved && engine.status().discarded == 0 && Date() < give { Thread.sleep(forTimeInterval: 0.1) }
            if try engine.stats().count < saved {
                problems[model] = engine.status().discarded > 0 ? "couldn't check a picture: \(engine.status().discardReason)" : "the secret check took over \(Int(perPicture)) s"
                break
            }
            checks.append(Date().timeIntervalSince(sent))
        }
        checkTimes[model] = checks
        if problems[model] != nil { print("\(model): \(problems[model]!)"); continue }
        // Every picture is saved; from here the model only describes, one picture after another.
        var seen: Set<Int64> = [], last = Date()
        let all = Filter(text: "", bundleId: nil, from: nil, to: nil, onlyKept: false, onlyHidden: false)
        let give = Date().addingTimeInterval(perPicture * Double(shown.count) * (detail == .both ? 2 : 1))
        // A picture the model failed on three times is given up; pending counts only those still to be tried.
        while seen.count < shown.count && Date() < give {
            Thread.sleep(forTimeInterval: 0.1)
            let left = try engine.stats().pending
            for shot in try engine.list(filter: all, limit: 500, offset: 0) where shot.indexed && !seen.contains(shot.id) {
                seen.insert(shot.id)
                let took = Date().timeIntervalSince(last)
                last = Date()
                guard !shot.description.isEmpty else { continue }
                if detail == .both {
                    // The two are written back to back; the time is for the pair.
                    rows.append(Row(model: model, variant: "brief", picture: shot.moment.windowTitle, seconds: nil, text: shot.description))
                    rows.append(Row(model: model, variant: "full", picture: shot.moment.windowTitle, seconds: took, text: shot.detail))
                } else {
                    rows.append(Row(model: model, variant: detail == .detailed ? "full" : "brief", picture: shot.moment.windowTitle, seconds: took, text: shot.description))
                }
            }
            let now = try engine.stats().pending
            if left == 0 && now == 0 { break }
        }
        print("\(model): \(rows.filter { $0.model == model && $0.seconds != nil }.count)/\(shown.count) described in \(Int(Date().timeIntervalSince(began))) s")
        // Kept as it goes: a long run that is stopped still leaves what it had.
        try? JSONEncoder().encode(rows).write(to: URL(fileURLWithPath: "\(out)/descriptions.json"))
    }

    // Scores. Agreement is with every other model's description of the same picture, same variant.
    var vectors: [Int: [Double]] = [:]
    for (i, r) in rows.enumerated() where !r.text.isEmpty { vectors[i] = embed(r.text, model: embedModel) }
    _ = post("/api/v1/models/unload", ["instance_id": embedModel], timeout: 20)
    let variants = detail == .both ? ["brief", "full"] : [detail == .detailed ? "full" : "brief"]
    var table: [Line] = []
    for model in models {
        for variant in variants {
            var l = Line(label: variants.count > 1 ? "\(model) (\(variant))" : model, of: shown.count, check: checkTimes[model] ?? [], problem: problems[model] ?? "")
            for (i, r) in rows.enumerated() where r.model == model && r.variant == variant && !r.text.isEmpty {
                guard let p = shown.first(where: { $0.name == r.picture }) else { continue }
                l.done += 1
                if let s = r.seconds { l.seconds.append(s) }
                l.chars.append(Double(r.text.count))
                if let x = DescriptionScores.recall(of: p.facts, in: r.text) { l.recall.append(x) }
                if let x = DescriptionScores.grounded(r.text, screen: p.screen + "\n" + p.name) { l.grounded.append(x) }
                l.leaks += p.secrets.filter { DescriptionScores.recall(of: [$0], in: r.text) == 1 }.count
                for (j, o) in rows.enumerated() where o.model != model && o.variant == variant && o.picture == r.picture && !o.text.isEmpty {
                    l.words.append(DescriptionScores.overlap(r.text, o.text))
                    if let a = vectors[i], let b = vectors[j] { l.meaning.append(DescriptionScores.cosine(a, b)) }
                }
            }
            table.append(l)
        }
    }
    var report = "# Descriptions compared\n\n\(shown.count) pictures, detail: \(detail). Facts: share of listed facts mentioned. Grounded: share of the names and numbers a description gives that are on the screen. Agreement: with the other models on the same picture (shared words / shared meaning). Leaks: listed secrets repeated. Check s: median seconds to check a picture for secrets and save it (the first includes loading the model). Describe s: median seconds to describe one.\n\n"
    report += "| Model | Described | Check s | Describe s | Characters | Facts | Grounded | Agree (words) | Agree (meaning) | Leaks |\n|---|---|---|---|---|---|---|---|---|---|\n"
    for l in table {
        let check = DescriptionScores.median(l.check).map { String(format: "%.1f", $0) } ?? "-"
        let secs = DescriptionScores.median(l.seconds).map { String(format: "%.1f", $0) } ?? "-"
        let chars = DescriptionScores.median(l.chars).map { String(Int($0)) } ?? "-"
        report += "| \(l.label) | \(l.done)/\(l.of)\(l.problem.isEmpty ? "" : " (\(l.problem))") | \(check) | \(secs) | \(chars) | \(mean(l.recall)) | \(mean(l.grounded)) | \(mean(l.words)) | \(mean(l.meaning)) | \(l.leaks) |\n"
    }
    let enc = JSONEncoder()
    enc.outputFormatting = [.prettyPrinted, .sortedKeys]
    try enc.encode(rows).write(to: URL(fileURLWithPath: "\(out)/descriptions.json"))
    try report.write(toFile: "\(out)/report.md", atomically: true, encoding: .utf8)
    print("\n" + report + "\nEvery description: \(out)/descriptions.json")
}
