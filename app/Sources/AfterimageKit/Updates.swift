import Foundation

/// "Check for Updates": asks GitHub which release is the latest, once, when the person asks.
/// AIDEV-NOTE: privacy-critical. The only place the app talks to the internet. Never called on a timer or at launch;
/// AIDEV-NOTE: the request carries nothing about the person, the Mac or the library, and no cookies are kept.
public enum Updates {
    public static let releases = URL(string: "https://github.com/jmaddington/afterimage/releases")!

    public struct Release: Sendable, Equatable {
        /// Without the leading "v".
        public let version: String
        /// Where to get it. Always on this project's GitHub pages.
        public let page: URL
    }

    public enum Answer: Sendable, Equatable {
        case current
        case newer(Release)
        case failed(String)
    }

    /// The numbers of a plain version like "0.1.7" or "v0.1.7". Nil for anything else.
    static func numbers(_ v: String) -> [Int]? {
        let s = v.hasPrefix("v") ? String(v.dropFirst()) : v
        let parts = s.split(separator: ".", omittingEmptySubsequences: false)
        guard (1...4).contains(parts.count) else { return nil }
        let n = parts.map { p in p.count <= 6 && p.allSatisfy(\.isASCII) && p.allSatisfy(\.isNumber) ? Int(p) : nil }
        return n.contains(nil) ? nil : n.compactMap { $0 }
    }

    static func newer(_ candidate: String, than current: String) -> Bool {
        guard let a = numbers(candidate), let b = numbers(current) else { return false }
        for i in 0..<max(a.count, b.count) {
            let (x, y) = (i < a.count ? a[i] : 0, i < b.count ? b[i] : 0)
            if x != y { return x > y }
        }
        return false
    }

    static func request() -> URLRequest {
        var r = URLRequest(url: URL(string: "https://api.github.com/repos/jmaddington/afterimage/releases/latest")!)
        r.httpMethod = "GET"
        r.timeoutInterval = 15
        r.httpShouldHandleCookies = false
        r.cachePolicy = .reloadIgnoringLocalAndRemoteCacheData
        r.setValue("Afterimage", forHTTPHeaderField: "User-Agent")
        r.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
        return r
    }

    static func parse(_ data: Data) -> Release? {
        guard let o = try? JSONSerialization.jsonObject(with: data) as? [String: Any], let tag = o["tag_name"] as? String, let n = numbers(tag) else { return nil }
        // Only this project's own pages are ever opened, whatever the answer says.
        var page = releases
        if let s = o["html_url"] as? String, let u = URL(string: s), u.scheme == "https", u.host == "github.com", u.path.hasPrefix("/jmaddington/afterimage/releases/"), u.user == nil { page = u }
        return Release(version: n.map(String.init).joined(separator: "."), page: page)
    }

    /// `current`: this app's version. Asks once and forgets: no cookies, no cache, nothing stored.
    public static func check(current: String) async -> Answer {
        let cfg = URLSessionConfiguration.ephemeral
        cfg.httpCookieStorage = nil
        cfg.urlCache = nil
        cfg.httpShouldSetCookies = false
        let session = URLSession(configuration: cfg)
        defer { session.invalidateAndCancel() }
        do {
            let (data, response) = try await session.data(for: request())
            guard (response as? HTTPURLResponse)?.statusCode == 200, data.count < 2_000_000, let r = parse(data) else { return .failed("GitHub didn’t give a usable answer.") }
            return newer(r.version, than: current) ? .newer(r) : .current
        } catch {
            return .failed("GitHub couldn’t be reached.")
        }
    }
}
