import AfterimageEngine
import CoreGraphics
import Foundation
import Testing
@testable import AfterimageKit

@MainActor @Suite struct WindowSettingTests {
    @Test func windowsAreCapturedSeparatelyOnlyWhenAsked() throws {
        let name = "afterimage.tests.\(UUID())"
        let d = try #require(UserDefaults(suiteName: name))
        defer { d.removePersistentDomain(forName: name) }
        let s = AppSettings(d)
        #expect(!s.windowsSeparately)
        #expect(s.overviewSeconds == 60)
        s.overviewSeconds = 300
        #expect(AppSettings(d).overviewSeconds == 300)
        s.windowsSeparately = true
        #expect(AppSettings(d).windowsSeparately)
        // Nothing about what is off-limits depends on it.
        #expect(s.exclusions == AppSettings(UserDefaults(suiteName: "afterimage.tests.\(UUID())")!).exclusions)
    }
}

@MainActor @Suite struct ExcludingAnAppTests {
    /// Pictures of an app still waiting to be checked must go the moment the app is put off-limits.
    @Test func excludingAnAppAnnouncesOnlyTheNewOnes() {
        let d = UserDefaults(suiteName: "afterimage-test-\(UUID())")!
        let s = AppSettings(d)
        var told: [[String]] = []
        s.onAppsExcluded = { told.append($0) }
        s.excludedApps.append("com.chat")
        s.excludedApps.append("com.notes")
        s.excludedApps.removeAll { $0 == "com.chat" }
        #expect(told == [["com.chat"], ["com.notes"]])
    }
}

/// The parts of per-window capture that need no screen and no permission.
@Suite struct WindowShotsTests {
    let ex = Exclusions(apps: ["com.acme.Ledger"], sites: [], passwordManagers: true, privateWindows: true, unreadableBrowsers: true, tor: true)
    let me: pid_t = 1
    let xcode = "com.apple.dt.Xcode"
    var owners: [pid_t: WindowShots.Owner] {
        [10: .init(bundleId: xcode, regular: true), 11: .init(bundleId: "com.apple.Notes", regular: true), 12: .init(bundleId: "com.acme.Ledger", regular: true),
         14: .init(bundleId: "com.google.Chrome", regular: true), 17: .init(bundleId: "com.example.agent", regular: false)]
    }
    func w(_ id: CGWindowID, _ pid: pid_t, title: String = "Doc", layer: Int = 0, alpha: Double = 1, _ b: CGRect = CGRect(x: 10, y: 20, width: 900, height: 600)) -> OnScreenWindow {
        OnScreenWindow(id: id, pid: pid, title: title, layer: layer, bounds: b, alpha: alpha)
    }
    let laptop = CGRect(x: 0, y: 0, width: 1440, height: 900), monitor = CGRect(x: 1440, y: 0, width: 1920, height: 1080)
    func plan(front: pid_t, _ bundle: String, _ list: [OnScreenWindow], shareable: Set<CGWindowID>? = nil, hidden: Set<pid_t> = [1], desktop: [CGRect]? = nil) -> [WindowChoice] {
        WindowShots.plan(ex, front: (front, bundle), list: list, owners: owners, shareable: shareable ?? Set(list.map(\.id)), hidden: hidden, own: me, desktop: desktop ?? [laptop, monitor])
    }

    /// "Displays: the one in use" holds for windows too: one on another display, or parked off every display, is left alone.
    @Test func onlyWindowsOnTheDisplaysBeingCapturedAreTaken() {
        let list = [w(7, 10), w(8, 11, CGRect(x: 1600, y: 50, width: 900, height: 600)), w(9, 11, CGRect(x: -895, y: 20, width: 900, height: 600))]
        #expect(taken(plan(front: 10, xcode, list)) == [7, 8])
        #expect(taken(plan(front: 10, xcode, list, desktop: [laptop])) == [7])
        #expect(plan(front: 10, xcode, list, desktop: [laptop]).map(\.skip) == [nil, .notShown, .notShown])
    }

    /// A window photographed while it was behind something is photographed again when the person goes to it,
    /// changed or not: otherwise nothing would say they were ever in it.
    /// A blinking cursor changes a few cells and no words. Such a picture is saved only if its words differ from the
    /// last one saved of that window.
    @Test func aSlightChangeIsSavedOnlyIfTheWordsChanged() {
        let busy = Data((0..<64).map { UInt8($0 * 3) })
        #expect(WindowShots.take(front: true, manual: false, print: busy, queued: 0, hadFront: true, howNew: { _ in .slight }) == .ifWordsChanged)
        #expect(WindowShots.take(front: false, manual: false, print: busy, queued: 0, hadFront: false, howNew: { _ in .slight }) == .ifWordsChanged)
        // Asked for by hand, or never yet seen in front: taken whatever the words say.
        #expect(WindowShots.take(front: true, manual: true, print: busy, queued: 0, hadFront: true, howNew: { _ in .slight }) == .take)
        #expect(WindowShots.take(front: true, manual: false, print: busy, queued: 0, hadFront: false, howNew: { _ in .slight }) == .take)
        func l(_ t: String) -> TextLine { TextLine(text: t, rect: Rect(x: 0, y: 0, w: 1, h: 0.1), tokens: []) }
        let a = WindowShots.words([l("Server notes"), l("renew the certificate")])
        // Spacing and a stray cursor mark read by OCR don't make new words; a typed letter does.
        #expect(a == WindowShots.words([l("Server  notes"), l("|renew the certificate ")]))
        #expect(a != WindowShots.words([l("Server notes"), l("renew the certificates")]))
        #expect(WindowShots.words([]) != a)
    }

    /// With windows photographed on their own, a picture of the whole screen is still saved now and then, so there
    /// is something that shows how it all sat together.
    @Test func theWholeScreenIsStillSavedNowAndThen() {
        let now = Date(timeIntervalSince1970: 10_000)
        #expect(WindowShots.overviewDue(last: nil, now: now, every: 60))
        #expect(!WindowShots.overviewDue(last: now.addingTimeInterval(-59), now: now, every: 60))
        #expect(WindowShots.overviewDue(last: now.addingTimeInterval(-60), now: now, every: 60))
        // Never, when asked for never.
        #expect(!WindowShots.overviewDue(last: nil, now: now, every: 0))
        #expect(!WindowShots.overviewDue(last: now.addingTimeInterval(-9999), now: now, every: -5))
        // A clock set back doesn't stop it for good.
        #expect(WindowShots.overviewDue(last: now.addingTimeInterval(500), now: now, every: 60))
    }

    @Test func aWindowComingToTheFrontGetsAPictureAsTheFrontWindow() {
        let busy = Data((0..<64).map { UInt8($0 * 3) })
        #expect(WindowShots.take(front: true, manual: false, print: busy, queued: 0, hadFront: false, howNew: { _ in .same }) == .take)
        #expect(WindowShots.take(front: true, manual: false, print: busy, queued: 0, hadFront: true, howNew: { _ in .same }) == .unchanged)
        #expect(WindowShots.take(front: false, manual: false, print: busy, queued: 0, hadFront: false, howNew: { _ in .same }) == .unchanged)
        // The thumbprint is recorded either way, exactly once.
        var asked = 0
        _ = WindowShots.take(front: true, manual: true, print: busy, queued: 0, hadFront: false) { _ in asked += 1; return .changed }
        #expect(asked == 1)
        // One look never fills the engine's queue of twelve by itself, twice over.
        #expect(WindowShots.maxQueued * 2 <= 12)
    }
    func taken(_ p: [WindowChoice]) -> [UInt32] { p.filter { $0.skip == nil }.map(\.id) }

    @Test func whatTheWindowServerListsIsWhatThePolicyJudges() {
        let seen = WindowShots.seen([w(7, 10), w(8, 99), w(9, 11, layer: 3, alpha: 0.5), w(6, 17)], owners: owners, shareable: [7, 8, 6])
        // A process nobody can name has no app, which the policy refuses.
        #expect(seen.map(\.bundleId) == [xcode, "", "com.apple.Notes", "com.example.agent"])
        #expect(seen.map(\.regular) == [true, false, true, false])
        // A window ScreenCaptureKit doesn't offer as on screen isn't treated as shown.
        #expect(seen.map(\.onScreen) == [true, true, false, true])
        #expect(seen[2].layer == 3 && seen[2].alpha == 0.5 && seen[0].bounds == Rect(x: 10, y: 20, w: 900, h: 600))
        #expect(seen.map(\.id) == [7, 8, 9, 6] && seen.map(\.pid) == [10, 99, 11, 17])
    }

    @Test func theFrontWindowAndOrdinaryBackgroundWindowsAreTaken() {
        let p = plan(front: 10, xcode, [w(7, 10), w(8, 11), w(9, 10)])
        #expect(taken(p) == [7, 8, 9] && p.map(\.front) == [true, false, false])
    }

    @Test func excludedAppsBackgroundBrowsersAndStrangersAreNot() {
        let list = [w(7, 10), w(5, 12), w(9, 14), w(3, 99), w(4, 17), w(2, me)]
        let p = plan(front: 10, xcode, list)
        #expect(taken(p) == [7])
        #expect(p.map(\.skip) == [nil, .hidden, .backgroundPages, .unknown, .notAnApp, .ours])
        // The capture check hid an app: none of its windows, whatever the app is.
        #expect(taken(plan(front: 10, xcode, [w(7, 10), w(8, 11)], hidden: [1, 11])) == [7])
        // Minimised or on another desktop: ScreenCaptureKit doesn't offer it as on screen.
        #expect(taken(plan(front: 10, xcode, [w(7, 10), w(8, 11)], shareable: [7])) == [7])
    }

    @Test func somethingPrivateInFrontMeansNothingIsTaken() {
        let list = [w(5, 12), w(7, 10), w(8, 11)]
        #expect(taken(plan(front: 12, "com.acme.Ledger", list)).isEmpty)
        // Afterimage in front is not that: its own window is left out and the rest is taken.
        #expect(taken(plan(front: me, "com.jmaddington.afterimage", [w(2, me), w(7, 10)])) == [7])
        #expect(taken(plan(front: 10, xcode, [w(7, 10), w(8, 11)], hidden: [1, 10])).isEmpty)
    }

    @Test func aBrowserIsTakenOnlyAsTheWindowInFront() {
        let list = [w(9, 14, title: "Page"), w(19, 14, title: "Other page"), w(7, 10)]
        let p = plan(front: 14, "com.google.Chrome", list)
        #expect(taken(p) == [9, 7] && p[0].front && p[1].skip == .backgroundPages)
        // A suggestion list in front of the page isn't the window the person is in.
        let popup = w(30, 14, title: "", CGRect(x: 200, y: 100, width: 300, height: 200))
        let q = plan(front: 14, "com.google.Chrome", [popup] + list)
        #expect(taken(q) == [9, 7] && q[0].skip == .backgroundPages)
    }

    @Test func aPictureIsKeptOnlyIfItsWindowWasAllowedBeforeAndAfter() {
        let list = [w(7, 10), w(8, 11)]
        let before = plan(front: 10, xcode, list)
        #expect(keepWindows(before: before, after: plan(front: 10, xcode, list)) == [7, 8])
        // An app was excluded mid-capture by the check: its picture is dropped, the other stays.
        #expect(keepWindows(before: before, after: plan(front: 10, xcode, list, hidden: [1, 11])) == [7])
        // The window moved while it was being photographed.
        let moved = [w(7, 10, CGRect(x: 60, y: 20, width: 900, height: 600)), w(8, 11)]
        #expect(keepWindows(before: before, after: plan(front: 10, xcode, moved)) == [8])
        // It was minimised or closed.
        #expect(keepWindows(before: before, after: plan(front: 10, xcode, [w(7, 10)])) == [7])
    }

    @Test func aKeyNamesOneWindowForOneRunOfTheApp() {
        #expect(WindowShots.key(session: "ab12cd34", id: 77) == "ab12cd34-77")
        #expect(WindowShots.key(session: "ab12cd34", id: 77) != WindowShots.key(session: "ab12cd34", id: 78))
        // The window server starts its numbers again; a new run must never be mistaken for the old one.
        let a = WindowShots.newSession(), b = WindowShots.newSession()
        #expect(a != b && a.count == 8 && a.allSatisfy(\.isHexDigit))
    }

    @Test func whatToDoWithOnePicture() {
        let busy = Data((0..<64).map { UInt8($0 * 3) }), flat = Data(repeating: 40, count: 64)
        var asked = 0
        func new(_ answer: Bool) -> (Data) -> Newness { { _ in asked += 1; return answer ? .changed : .same } }
        #expect(WindowShots.take(front: true, manual: false, print: busy, queued: 0, hadFront: true, howNew: new(true)) == .take)
        #expect(WindowShots.take(front: false, manual: false, print: busy, queued: 3, hadFront: true, howNew: new(true)) == .take)
        // A background window is its own picture only when it changed, even when asked for by hand.
        #expect(WindowShots.take(front: false, manual: false, print: busy, queued: 0, hadFront: true, howNew: new(false)) == .unchanged)
        #expect(WindowShots.take(front: false, manual: true, print: busy, queued: 0, hadFront: true, howNew: new(false)) == .unchanged)
        #expect(WindowShots.take(front: true, manual: false, print: busy, queued: 0, hadFront: true, howNew: new(false)) == .unchanged)
        #expect(WindowShots.take(front: true, manual: true, print: busy, queued: 0, hadFront: true, howNew: new(false)) == .take)
        #expect(asked == 6)
        // A window that gave nothing isn't a picture, and isn't remembered as one.
        #expect(WindowShots.take(front: true, manual: true, print: flat, queued: 0, hadFront: true, howNew: new(true)) == .blank)
        #expect(WindowShots.take(front: false, manual: false, print: Data(), queued: 0, hadFront: true, howNew: new(true)) == .blank)
        // The queue is never flooded, and what was left out is looked at afresh next time.
        #expect(WindowShots.take(front: false, manual: false, print: busy, queued: WindowShots.maxQueued, hadFront: true, howNew: new(true)) == .full)
        #expect(asked == 6)
        #expect(WindowShots.maxQueued < 12)
    }

    @Test func aBlankPictureIsOneWithNothingInIt() {
        #expect(Pixels.isBlank(Data()) && Pixels.isBlank(Data(repeating: 0, count: 100)) && Pixels.isBlank(Data(repeating: 255, count: 100)))
        var nearly = Data(repeating: 30, count: 100)
        nearly[5] = 32
        #expect(Pixels.isBlank(nearly))
        nearly[5] = 60
        #expect(!Pixels.isBlank(nearly))
    }

    @Test func aWindowsThumbprintCountsItsTopEdge() throws {
        func image(top: CGFloat) throws -> CGImage {
            let c = try #require(CGContext(data: nil, width: 400, height: 250, bitsPerComponent: 8, bytesPerRow: 0, space: Pixels.sRGB, bitmapInfo: Pixels.rgba))
            c.setFillColor(gray: 0.5, alpha: 1)
            c.fill(CGRect(x: 0, y: 0, width: 400, height: 250))
            // CoreGraphics counts from the bottom: this is the top eight rows, where a title bar's text would be.
            c.setFillColor(gray: top, alpha: 1)
            c.fill(CGRect(x: 0, y: 242, width: 400, height: 8))
            return try #require(c.makeImage())
        }
        let a = try image(top: 0.1), b = try image(top: 0.9)
        // A display's menu bar is skipped: its clock always changes.
        #expect(Pixels.thumbprint(a) == Pixels.thumbprint(b))
        // A window has no menu bar; its title bar is part of it.
        #expect(Pixels.thumbprint(a, skipBar: false) != Pixels.thumbprint(b, skipBar: false))
        #expect(Pixels.thumbprint(a, skipBar: false).count == 128 * 80 && !Pixels.isBlank(Pixels.thumbprint(a, skipBar: false)))
    }

    @Test func aSecretFieldIsPlacedOnTheWindowOrTheWindowIsLeftOut() throws {
        let window = CGRect(x: 100, y: 200, width: 800, height: 400)
        let field = Cover(kind: .password, rect: Rect(x: 300, y: 300, w: 80, h: 40))
        let elsewhere = Cover(kind: .password, rect: Rect(x: 2000, y: 300, w: 80, h: 40))
        // Desktop points become fractions of the window, not of the display.
        let on = try #require(WindowShots.covers([field, elsewhere], window: window, content: CGSize(width: 800, height: 400)))
        #expect(on.count == 1)
        #expect(abs(on[0].rect.x - 0.25) < 1e-9 && abs(on[0].rect.y - 0.25) < 1e-9 && abs(on[0].rect.w - 0.1) < 1e-9 && abs(on[0].rect.h - 0.1) < 1e-9)
        #expect(try #require(WindowShots.covers([field], window: window, content: CGSize(width: 800.5, height: 399.5))).count == 1)
        // The picture isn't the size of the window (a shadow, a different scale): the field can't be placed, so no picture.
        #expect(WindowShots.covers([field], window: window, content: CGSize(width: 840, height: 440)) == nil)
        #expect(WindowShots.covers([field], window: window, content: nil) == nil)
        // Nothing to place: nothing to doubt.
        #expect(WindowShots.covers([], window: window, content: nil) == [])
        #expect(WindowShots.covers([elsewhere], window: window, content: nil) == [])
    }

    @Test func thePictureIsAsBigAsTheWindow() {
        #expect(WindowShots.pixels(CGSize(width: 800, height: 400.4), scale: 2).map { [$0.width, $0.height] } == [1600, 800])
        for bad in [CGSize.zero, CGSize(width: -5, height: 10), CGSize(width: CGFloat.nan, height: 10), CGSize(width: CGFloat.infinity, height: 10), CGSize(width: 40000, height: 10)] {
            #expect(WindowShots.pixels(bad, scale: 2) == nil, "\(bad)")
        }
        #expect(WindowShots.pixels(CGSize(width: 800, height: 400), scale: 0) == nil && WindowShots.pixels(CGSize(width: 800, height: 400), scale: .nan) == nil)
    }

    @Test func theLogSaysWhatWasTaken() {
        #expect(Recorder.taken(1, windows: false) == "Taken" && Recorder.taken(2, windows: false) == "Taken (2 displays)")
        #expect(Recorder.taken(1, windows: true) == "Taken (1 window)" && Recorder.taken(3, windows: true) == "Taken (3 windows)")
    }

    /// A window's picture goes through the same check as any other, and comes back knowing which window it was.
    @Test func aWindowPictureIsCheckedAndRemembersItsWindow() async throws {
        let dir = FileManager.default.temporaryDirectory.appending(path: "afterimage-\(UUID())")
        defer { try? FileManager.default.removeItem(at: dir) }
        let engine = try Engine(root: dir.path, config: rulesOnly(), host: ImageHost())
        defer { engine.shutdown() }
        let image = try #require(Pixels.load(fixtures.appending(path: "terminal.png").path))
        let bmp = try #require(Pixels.bitmap(image))
        let key = WindowShots.key(session: "ab12cd34", id: 77)
        let moment = Moment(takenAt: 1_700_000_000_000, trigger: "app", display: "Test", appName: "Terminal", bundleId: "com.apple.Terminal", windowTitle: "deploy", url: "", userName: "u", fullName: "U",
                            host: "h", sessions: [], runningApps: [], windowKey: key, windowFront: false)
        engine.ingest(frame: Frame(rgba: bmp.data, width: UInt32(bmp.width), height: UInt32(bmp.height), stride: UInt32(bmp.stride), lines: try Recognizer.lines(in: image), moment: moment, displayKey: "win:77"))
        for _ in 0..<600 where try engine.stats().count == 0 && engine.status().discarded == 0 { try await Task.sleep(for: .milliseconds(50)) }
        let shot = try #require(try engine.list(filter: Filter(text: "", bundleId: nil, from: nil, to: nil, onlyKept: false, onlyHidden: false), limit: 1, offset: 0).first)
        #expect(shot.moment.windowKey == "ab12cd34-77" && !shot.moment.windowFront && !shot.regions.isEmpty)
        let seen = try Recognizer.lines(in: try #require(Pixels.load(shot.path))).map(\.text).joined(separator: "\n")
        #expect(!seen.contains("correct-horse") && !seen.contains("AKIA"))
    }
}
