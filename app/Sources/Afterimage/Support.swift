import AfterimageEngine
import AfterimageKit
import AppKit
import SwiftUI

/// Decoded pictures, kept only as long as they're useful.
@MainActor final class ImageStore {
    static let shared = ImageStore()
    private let cache = NSCache<NSString, NSImage>()
    private init() { cache.totalCostLimit = 300 << 20 }

    func cached(_ path: String, _ edge: Int) -> NSImage? { cache.object(forKey: "\(edge)|\(path)" as NSString) }
    func load(_ path: String, _ edge: Int) async -> NSImage? {
        if let i = cached(path, edge) { return i }
        let cg = await Task.detached(priority: .userInitiated) { Pixels.load(path, maxEdge: edge) }.value
        guard let cg, !Task.isCancelled else { return nil }
        let i = NSImage(cgImage: cg, size: NSSize(width: cg.width, height: cg.height))
        cache.setObject(i, forKey: "\(edge)|\(path)" as NSString, cost: cg.width * cg.height * 4)
        return i
    }
}

struct Picture: View {
    let path: String
    let edge: Int
    @State private var image: NSImage?
    var body: some View {
        ZStack {
            if let i = image ?? ImageStore.shared.cached(path, edge) {
                Image(nsImage: i).resizable().interpolation(.high).aspectRatio(contentMode: .fit)
            } else {
                Rectangle().fill(.quaternary)
            }
        }
        .task(id: path) {
            image = ImageStore.shared.cached(path, edge)
            if image == nil { image = await ImageStore.shared.load(path, edge) }
        }
    }
}

struct AppIcon: View {
    let bundleId: String
    var size: CGFloat = 18
    var body: some View {
        if let u = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleId) {
            Image(nsImage: NSWorkspace.shared.icon(forFile: u.path)).resizable().frame(width: size, height: size)
        } else {
            Image(systemName: "app.dashed").resizable().scaledToFit().foregroundStyle(.secondary).frame(width: size, height: size)
        }
    }
}

enum Words {
    static func day(_ d: Date) -> String {
        let c = Calendar.current
        if c.isDateInToday(d) { return "Today" }
        if c.isDateInYesterday(d) { return "Yesterday" }
        let sameYear = c.isDate(d, equalTo: Date(), toGranularity: .year)
        return d.formatted(sameYear ? .dateTime.weekday(.wide).month(.wide).day() : .dateTime.weekday(.wide).month(.wide).day().year())
    }
    static func trigger(_ t: String) -> String {
        switch t {
        case "manual": "Captured by you"
        case "app": "Switched app"
        case "launch": "Opened an app"
        case "window": "Switched window"
        case "tab": "Page or tab changed"
        case "space": "Switched space"
        default: "On schedule"
        }
    }
    static func kind(_ k: String) -> String {
        switch k {
        case "password": "Password"
        case "ssn": "Social Security number"
        case "financial": "Account or card number"
        case "key": "Key or token"
        default: "Secret"
        }
    }
    /// What found a hidden spot, from the engine's short names.
    static func why(_ w: String) -> String {
        let names = ["model": "the local model", "extension": "the browser extension", "empty-field": "an empty secret field", "code": "a QR or bar code",
                     "label": "the label beside it", "assignment": "a name = value rule", "card": "the card number rule", "ssn": "the Social Security rule",
                     "iban": "the IBAN rule", "account": "the account number rule", "cvv": "the security code rule", "phrase": "“password is…”",
                     "flag": "a command-line flag", "bearer": "an authorization header", "url-login": "a login inside an address", "setup-code": "a setup code",
                     "known-key": "a known key format", "random": "its random look", "private-key": "a private key block", "key-block": "a block of key or certificate data", "wrapped": "the key it continues",
                     "repeated": "the same secret elsewhere"]
        let parts = w.split(separator: ",").map { names[String($0)] ?? String($0) }
        return parts.isEmpty ? "" : "Found by " + parts.formatted(.list(type: .and))
    }
    /// "Afterimage 0.1.2 (41)": the version and the build that made it.
    static var version: String {
        let i = Bundle.main.infoDictionary
        return "Afterimage \(i?["CFBundleShortVersionString"] as? String ?? "?") (\(i?["CFBundleVersion"] as? String ?? "?"))"
    }
    /// Opened in the browser. The app looks for updates only when asked to (Updates.check).
    static let releases = Updates.releases
    static var shortVersion: String { Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0" }
    /// Ask GitHub once and say what it answered. Only ever called from a button.
    @MainActor static func checkForUpdates() {
        Task { @MainActor in
            let answer = await Updates.check(current: shortVersion)
            let a = NSAlert()
            switch answer {
            case .current:
                a.messageText = "Afterimage is up to date"
                a.informativeText = "\(shortVersion) is the latest version."
            case .newer(let r):
                a.messageText = "Afterimage \(r.version) is available"
                a.informativeText = "You have \(shortVersion). The download opens in your browser."
                a.addButton(withTitle: "Open Download Page")
                a.addButton(withTitle: "Not Now")
                NSApp.activate()
                if a.runModal() == .alertFirstButtonReturn { NSWorkspace.shared.open(r.page) }
                return
            case .failed(let why):
                a.messageText = "Couldn’t check for updates"
                a.informativeText = why
            }
            NSApp.activate()
            a.runModal()
        }
    }
    static func bytes(_ n: UInt64) -> String { Int64(n).formatted(.byteCount(style: .file)) }
    static func count(_ n: Int, _ one: String, _ many: String) -> String { n == 1 ? "1 \(one)" : "\(n.formatted()) \(many)" }
}
