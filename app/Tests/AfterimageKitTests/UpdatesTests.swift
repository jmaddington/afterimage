import Foundation
import Testing
@testable import AfterimageKit

@Suite struct UpdatesTests {
    @Test func aNewerReleaseIsNoticedAndNothingElseIs() {
        #expect(Updates.newer("v0.1.7", than: "0.1.6"))
        #expect(Updates.newer("0.1.10", than: "0.1.9"))
        #expect(Updates.newer("1.0", than: "0.9.9"))
        #expect(!Updates.newer("v0.1.6", than: "0.1.6"))
        #expect(!Updates.newer("0.1.5", than: "0.1.6"))
        #expect(!Updates.newer("0.1", than: "0.1.0"))
        // Not a version: never an update.
        for junk in ["", "latest", "v", "1.x", "0.1.7-beta", "9999999999999999999999.0"] { #expect(!Updates.newer(junk, than: "0.1.6"), "\(junk)") }
    }

    @Test func theAnswerIsReadCarefully() throws {
        let good = Data(#"{"tag_name":"v0.1.7","html_url":"https://github.com/jmaddington/afterimage/releases/tag/v0.1.7","body":"notes"}"#.utf8)
        let r = try #require(Updates.parse(good))
        #expect(r.version == "0.1.7" && r.page.absoluteString == "https://github.com/jmaddington/afterimage/releases/tag/v0.1.7")
        // A link anywhere else isn't opened: the releases page is.
        let odd = Data(#"{"tag_name":"v0.1.7","html_url":"https://example.com/evil"}"#.utf8)
        #expect(Updates.parse(odd)?.page == Updates.releases)
        let near = Data(#"{"tag_name":"v0.1.7","html_url":"https://github.com.evil.com/jmaddington/afterimage/releases/tag/v0.1.7"}"#.utf8)
        #expect(Updates.parse(near)?.page == Updates.releases)
        #expect(Updates.parse(Data("{}".utf8)) == nil && Updates.parse(Data("nonsense".utf8)) == nil)
        #expect(Updates.parse(Data(#"{"tag_name":"<script>"}"#.utf8)) == nil)
    }

    @Test func theQuestionGoesOnlyToTheReleasesListAndSaysNothingAboutThePerson() {
        let q = Updates.request()
        #expect(q.url?.absoluteString == "https://api.github.com/repos/jmaddington/afterimage/releases/latest")
        #expect(q.httpMethod == "GET" && q.httpBody == nil)
        #expect(q.value(forHTTPHeaderField: "User-Agent") == "Afterimage")
        #expect(q.value(forHTTPHeaderField: "Cookie") == nil && q.value(forHTTPHeaderField: "Authorization") == nil)
        #expect(q.httpShouldHandleCookies == false)
    }
}

@MainActor @Suite struct ModelServerTests {
    @Test func theModelIsOnThisMacUnlessAnotherMachineIsNamed() throws {
        let name = "afterimage.tests.\(UUID())"
        let d = try #require(UserDefaults(suiteName: name))
        defer { d.removePersistentDomain(forName: name) }
        let s = AppSettings(d)
        #expect(s.lmHost == "" && s.config.lmUrl == "http://127.0.0.1:1234")
        s.lmHost = "  studio.local "
        #expect(s.config.lmUrl == "http://studio.local:1234" && AppSettings(d).lmHost == "  studio.local ")
        s.port = 4321
        #expect(s.config.lmUrl == "http://studio.local:4321")
    }

    @Test func descriptionDetailIsADeveloperChoice() throws {
        let name = "afterimage.tests.\(UUID())"
        let d = try #require(UserDefaults(suiteName: name))
        defer { d.removePersistentDomain(forName: name) }
        let s = AppSettings(d)
        #expect(s.describeDetail == "brief" && s.config.detail == .brief)
        s.describeDetail = "both"
        // Out of sight, out of force: the choice counts only while developer mode shows it.
        #expect(s.config.detail == .brief)
        s.developer = true
        #expect(s.config.detail == .both && AppSettings(d).describeDetail == "both")
        s.describeDetail = "custom"
        s.customLong = false
        s.customTokens = 1500
        s.customChars = 6000
        s.customEdge = 2000
        let c = s.config
        #expect(c.detail == .custom && !c.customLong && c.customTokens == 1500 && c.customChars == 6000 && c.customEdge == 2000)
        // Labs: the second pass has a model only while it is switched on.
        s.idleModel = "big"
        #expect(s.config.idleModel == "")
        s.idlePass = true
        #expect(s.config.idleModel == "big" && s.config.idleReasoning == "off")
        #expect(!AppSettings.leftAlone(idleSeconds: 179, minutes: 3, enabled: true) && AppSettings.leftAlone(idleSeconds: 180, minutes: 3, enabled: true))
        #expect(!AppSettings.leftAlone(idleSeconds: 9999, minutes: 3, enabled: false) && AppSettings.leftAlone(idleSeconds: 60, minutes: 0, enabled: true))
        #expect(Session.whole(125.9) == 125 && Session.whole(-3) == 0 && Session.whole(.infinity) == 0 && Session.whole(.nan) == 0)
        // Thinking is the same kind of choice.
        #expect(c.reasoning == "off")
        s.reasoning = "medium"
        #expect(s.config.reasoning == "medium")
        s.developer = false
        #expect(s.config.reasoning == "off" && s.config.detail == .brief)
        #expect(AppSettings.detail("detailed", developer: true) == .detailed && AppSettings.detail("nonsense", developer: true) == .brief)
    }

    @Test func whateverIsTypedBecomesAPlainAddress() {
        #expect(AppSettings.lmURL(host: "", port: 1234) == "http://127.0.0.1:1234")
        #expect(AppSettings.lmURL(host: "10.0.0.4", port: 1234) == "http://10.0.0.4:1234")
        #expect(AppSettings.lmURL(host: "http://10.0.0.4:9999/v1", port: 1234) == "http://10.0.0.4:1234")
        #expect(AppSettings.lmURL(host: "fd12:3456::1", port: 1234) == "http://[fd12:3456::1]:1234")
        #expect(AppSettings.lmURL(host: "[fd12:3456::1]", port: 1234) == "http://[fd12:3456::1]:1234")
        // A name and password can't be smuggled in ahead of the host.
        #expect(AppSettings.lmURL(host: "10.0.0.4@8.8.8.8", port: 1234) == "http://8.8.8.8:1234")
    }
}
