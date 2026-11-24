import Foundation

/// Lets a browser start `afterimage-bridge` for the Afterimage extension, by writing the small file browsers look for.
public struct BrowserLink: Sendable {
    public static let host = "com.jmaddington.afterimage"
    /// Fixed by the public key in extension/manifest.chrome.json.
    public static let chromeExtensionID = "fappnpcipafijhkonnifjnjpcikiaije"
    public static let firefoxExtensionID = "afterimage@jmaddington.com"

    public struct Browser: Sendable, Identifiable {
        public let name: String
        public let bundleId: String
        /// Folder under Application Support that exists once the browser has run.
        let folder: String
        let gecko: Bool
        public let installed: Bool
        public var id: String { bundleId }
    }

    let support: URL
    let bridge: String

    public init(support: URL = .applicationSupportDirectory, bridge: String = Bundle.main.bundleURL.appending(path: "Contents/MacOS/afterimage-bridge").path) {
        self.support = support
        self.bridge = bridge
    }

    public var browsers: [Browser] {
        [("Google Chrome", "com.google.Chrome", "Google/Chrome", false), ("Microsoft Edge", "com.microsoft.edgemac", "Microsoft Edge", false),
         ("Brave", "com.brave.Browser", "BraveSoftware/Brave-Browser", false), ("Vivaldi", "com.vivaldi.Vivaldi", "Vivaldi", false),
         ("Arc", "company.thebrowser.Browser", "Arc/User Data", false), ("Chromium", "org.chromium.Chromium", "Chromium", false),
         ("Firefox", "org.mozilla.firefox", "Mozilla", true)].map {
            Browser(name: $0.0, bundleId: $0.1, folder: $0.2, gecko: $0.3, installed: FileManager.default.fileExists(atPath: support.appending(path: $0.2).path))
        }
    }

    private func file(_ b: Browser) -> URL { support.appending(path: b.folder).appending(path: "NativeMessagingHosts/\(Self.host).json") }

    public func isConnected(_ b: Browser) -> Bool {
        guard let d = try? Data(contentsOf: file(b)), let j = try? JSONSerialization.jsonObject(with: d) as? [String: Any] else { return false }
        return j["path"] as? String == bridge
    }

    public func connect(_ b: Browser) throws {
        guard b.installed else { throw CocoaError(.fileNoSuchFile) }
        var j: [String: Any] = ["name": Self.host, "description": "Afterimage", "path": bridge, "type": "stdio"]
        if b.gecko { j["allowed_extensions"] = [Self.firefoxExtensionID] } else { j["allowed_origins"] = ["chrome-extension://\(Self.chromeExtensionID)/"] }
        let f = file(b)
        try FileManager.default.createDirectory(at: f.deletingLastPathComponent(), withIntermediateDirectories: true)
        try JSONSerialization.data(withJSONObject: j, options: [.prettyPrinted, .sortedKeys]).write(to: f, options: .atomic)
    }

    public func disconnect(_ b: Browser) throws {
        if FileManager.default.fileExists(atPath: file(b).path) { try FileManager.default.removeItem(at: file(b)) }
    }

    /// Where the extension itself is, for "Load unpacked".
    public static func extensionFolder(gecko: Bool) -> URL { Bundle.main.bundleURL.appending(path: "Contents/Resources/extension/\(gecko ? "firefox" : "chrome")") }
    public func isGecko(_ b: Browser) -> Bool { b.gecko }
}
