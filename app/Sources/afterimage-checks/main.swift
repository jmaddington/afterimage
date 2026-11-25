// Developer tool.  seed <dir> [fixtures]: fill a demo library from synthetic pictures.  ocr <png>: show what OCR reads.
// compare <dir> [pictures]: describe the same pictures with each model in AFTERIMAGE_MODELS and score them (Compare.swift).
import AfterimageEngine
import AfterimageKit
import Foundation

let args = CommandLine.arguments
guard args.count >= 3 else { print("usage: afterimage-checks seed <dir> [fixtures] | ocr <png> | compare <dir> [pictures]"); exit(2) }
if args[1] == "ocr" {
    guard let i = Pixels.load(args[2]) else { print("can't read \(args[2])"); exit(1) }
    for l in try Recognizer.lines(in: i) { print(String(format: "%.3f %.3f  ", l.rect.x, l.rect.y) + l.text) }
    exit(0)
}
if args[1] == "compare" {
    try compare(out: args[2], pictures: args.count > 3 ? args[3] : "fixtures", env: ProcessInfo.processInfo.environment)
    exit(0)
}
let fixtures = args.count > 3 ? args[3] : "fixtures"
let env = ProcessInfo.processInfo.environment
let live = env["AFTERIMAGE_LIVE"] != nil
// AFTERIMAGE_MODEL, AFTERIMAGE_DETAIL (brief, detailed, both) and AFTERIMAGE_ROUNDS shape a live run.
let detail = AppSettings.detail(env["AFTERIMAGE_DETAIL"] ?? "brief", developer: true)
let config = Config(lmUrl: live ? "http://127.0.0.1:1234" : "http://127.0.0.1:9", model: env["AFTERIMAGE_MODEL"] ?? "google/gemma-4-26b-a4b-qat", embedModel: live ? "text-embedding-nomic-embed-text-v1.5" : "",
                    requireAi: live, describe: live ? .always : .never, maxAgeDays: 0, maxBytes: 0, unloadAfterSecs: 0, detail: detail, customLong: false, customTokens: 900, customChars: 4000, customEdge: 1280)
let engine = try Engine(root: args[2], config: config, host: ImageHost())
let cast: [(String, String, String, String)] = [
    ("onboarding", "Numbers", "com.apple.Numbers", "Employee record"), ("terminal", "Terminal", "com.apple.Terminal", "deploy — zsh"),
    ("notes", "Notes", "com.apple.Notes", "House stuff"), ("checkout", "Safari", "com.apple.Safari", "Checkout"), ("clean", "Pages", "com.apple.Pages", "Quarterly plan"),
]
let running = cast.map { AppRef(name: $0.1, bundleId: $0.2) }
let triggers = ["app", "window", "tab", "interval", "launch"]
let start = Date()
var n: UInt64 = 0
for round in 0..<(Int(env["AFTERIMAGE_ROUNDS"] ?? "") ?? 4) {
    for (i, c) in cast.enumerated() {
        guard let img = Pixels.load("\(fixtures)/\(c.0).png"), let b = Pixels.bitmap(img) else { continue }
        let at = start.addingTimeInterval(-Double(round * 5 + i) * 3700 - Double(round) * 30000)
        let m = Moment(takenAt: Int64(at.timeIntervalSince1970 * 1000), trigger: triggers[(i + round) % 5], display: "Built-in Display", appName: c.1, bundleId: c.2, windowTitle: c.3,
                       url: c.0 == "checkout" ? "https://shop.example/checkout?session=abc" : "", userName: "dana", fullName: "Dana Whitfield", host: "Demo Mac", sessions: ["dana (console)"], runningApps: running)
        engine.ingest(frame: Frame(rgba: b.data, width: UInt32(b.width), height: UInt32(b.height), stride: UInt32(b.stride), lines: try Recognizer.lines(in: img), moment: m, displayKey: "\(round)-\(i)"))
        n += 1
        while try engine.stats().count < n && engine.status().discarded == 0 { Thread.sleep(forTimeInterval: 0.1) }
        if engine.status().discarded > 0 { print("discarded: \(engine.status().discardReason)"); exit(1) }
    }
}
if live { while try engine.stats().pending > 0 { Thread.sleep(forTimeInterval: 0.5) } }
if live {
    for shot in try engine.list(filter: Filter(text: "", bundleId: nil, from: nil, to: nil, onlyKept: false, onlyHidden: false), limit: 50, offset: 0) {
        print("--- \(shot.moment.windowTitle) [\(shot.describeModel)]\n\(shot.description)\(shot.detail.isEmpty ? "" : "\n  IN FULL (\(shot.detail.count) characters):\n\(shot.detail)")")
    }
}
engine.shutdown()
print("seeded \(n) pictures")
