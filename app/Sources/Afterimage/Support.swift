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
    static func bytes(_ n: UInt64) -> String { Int64(n).formatted(.byteCount(style: .file)) }
    static func count(_ n: Int, _ one: String, _ many: String) -> String { n == 1 ? "1 \(one)" : "\(n.formatted()) \(many)" }
}
