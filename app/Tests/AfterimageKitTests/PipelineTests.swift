import AfterimageEngine
import AppKit
import Foundation
import Testing
@testable import AfterimageKit

let fixtures = URL(fileURLWithPath: #filePath).deletingLastPathComponent().appending(path: "../../../fixtures").standardizedFileURL

func rulesOnly() -> Config {
    Config(lmUrl: "http://127.0.0.1:9", model: "none", embedModel: "", requireAi: false, describe: .never, maxAgeDays: 0, maxBytes: 0, unloadAfterSecs: 0)
}

/// Runs a fixture through the real pipeline and returns the saved shot plus what OCR can still read from the saved file.
func saveAndReread(_ name: String, config: Config = rulesOnly()) async throws -> (shot: Shot, seen: String, indexed: String) {
    let dir = FileManager.default.temporaryDirectory.appending(path: "afterimage-\(UUID())")
    defer { try? FileManager.default.removeItem(at: dir) }
    let engine = try Engine(root: dir.path, config: config, host: ImageHost())
    defer { engine.shutdown() }
    let image = try #require(Pixels.load(fixtures.appending(path: "\(name).png").path))
    let bmp = try #require(Pixels.bitmap(image))
    let lines = try Recognizer.lines(in: image)
    let moment = Moment(takenAt: 1_700_000_000_000, trigger: "manual", display: "Test", appName: "Fixture", bundleId: "test", windowTitle: name, url: "", userName: "u", fullName: "U", host: "h", sessions: [], runningApps: [])
    engine.ingest(frame: Frame(rgba: bmp.data, width: UInt32(bmp.width), height: UInt32(bmp.height), stride: UInt32(bmp.stride), lines: lines, moment: moment, displayKey: "t"))
    for _ in 0..<200 where try engine.stats().count == 0 { try await Task.sleep(for: .milliseconds(50)) }
    let shot = try #require(try engine.list(filter: Filter(text: "", bundleId: nil, from: nil, to: nil, onlyKept: false, onlyHidden: false), limit: 1, offset: 0).first)
    let saved = try #require(Pixels.load(shot.path))
    #expect(Pixels.load(shot.thumbPath) != nil)
    let seen = try Recognizer.lines(in: saved).map(\.text).joined(separator: "\n")
    let indexed = try engine.lines(id: shot.id).map(\.text).joined(separator: "\n")
    return (shot, seen, indexed)
}

@Suite(.serialized) struct PipelineTests {
    @Test func ocrReadsLinesWithTokens() throws {
        let image = try #require(Pixels.load(fixtures.appending(path: "terminal.png").path))
        let lines = try Recognizer.lines(in: image)
        let l = try #require(lines.first { $0.text.contains("DB_PASSWORD") })
        #expect(l.rect.x > 0.05 && l.rect.x < 0.12 && l.rect.y > 0.2 && l.rect.y < 0.5)
        #expect(!l.tokens.isEmpty)
        for t in l.tokens { #expect(Int(t.start + t.len) <= l.text.unicodeScalars.count) }
    }
    @Test func readingOrderIsRowsThenColumns() {
        func l(_ s: String, _ x: Double, _ y: Double) -> TextLine { TextLine(text: s, rect: Rect(x: x, y: y, w: 0.1, h: 0.02), tokens: []) }
        let out = Recognizer.readingOrder([l("d", 0.5, 0.2), l("b", 0.5, 0.101), l("a", 0.1, 0.104), l("c", 0.1, 0.2)]).map(\.text)
        #expect(out == ["a", "b", "c", "d"])
    }
    @Test(arguments: [
        ("onboarding", ["219-09-9999", "021000021", "9931", "2280"], "Dana Whitfield"),
        ("terminal", ["correct-horse", "battery", "AKIA", "IOSFODNN"], "git status"),
        ("checkout", ["4111", "1111", "737"], "Field notebook"),
    ]) func secretsAreGoneFromPixelsAndIndex(name: String, secrets: [String], keep: String) async throws {
        let r = try await saveAndReread(name)
        for s in secrets {
            #expect(!r.seen.contains(s), "\(s) is still readable in the saved picture")
            #expect(!r.indexed.contains(s), "\(s) is still in the index")
        }
        #expect(r.seen.contains(keep), "over-hidden: \(r.seen)")
        #expect(!r.shot.regions.isEmpty)
    }
    @Test func ordinaryScreensAreLeftAlone() async throws {
        let r = try await saveAndReread("clean")
        #expect(r.shot.regions.isEmpty)
        #expect(r.seen.contains("Password reset page redesign"))
    }
    @Test func savedFilesArePrivate() async throws {
        let dir = FileManager.default.temporaryDirectory.appending(path: "afterimage-\(UUID())")
        defer { try? FileManager.default.removeItem(at: dir) }
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let img = try #require(Pixels.load(fixtures.appending(path: "clean.png").path))
        let b = try #require(Pixels.bitmap(img))
        let p = dir.appending(path: "a.heic").path, t = dir.appending(path: "a.t.heic").path
        let e = try ImageHost().encode(rgba: b.data, width: UInt32(b.width), height: UInt32(b.height), stride: UInt32(b.stride), path: p, thumbPath: t)
        #expect(e.bytes > 1000 && e.width == 1600)
        for f in [p, t] { #expect((try FileManager.default.attributesOfItem(atPath: f)[.posixPermissions] as? Int) == 0o600) }
        #expect(throws: (any Error).self) { try ImageHost().encode(rgba: Data(), width: 10, height: 10, stride: 40, path: p, thumbPath: t) }
    }
    @Test func thumbprintNoticesTypingButNotNothing() throws {
        func img(_ s: String) -> CGImage {
            let c = CGContext(data: nil, width: 1600, height: 1000, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
            c.setFillColor(.white); c.fill(CGRect(x: 0, y: 0, width: 1600, height: 1000))
            NSGraphicsContext.current = NSGraphicsContext(cgContext: c, flipped: false)
            (s as NSString).draw(at: NSPoint(x: 300, y: 500), withAttributes: [.font: NSFont.systemFont(ofSize: 22), .foregroundColor: NSColor.black])
            return c.makeImage()!
        }
        let e = try Engine(root: FileManager.default.temporaryDirectory.appending(path: "afterimage-\(UUID())").path, config: rulesOnly(), host: ImageHost())
        defer { e.shutdown() }
        #expect(e.isNew(displayKey: "d", luma: Pixels.thumbprint(img("hello"))))
        #expect(!e.isNew(displayKey: "d", luma: Pixels.thumbprint(img("hello"))))
        #expect(e.isNew(displayKey: "d", luma: Pixels.thumbprint(img("hello w"))))
    }
}
