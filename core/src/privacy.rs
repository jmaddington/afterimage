//! What must never be in the picture: apps, sites, private windows, password managers.

#[derive(Debug, Clone, Default, uniffi::Record)]
pub struct Exclusions {
    pub apps: Vec<String>,
    pub sites: Vec<String>,
    pub password_managers: bool,
    pub private_windows: bool,
    /// Leave a browser out when its page address can't be read.
    pub unreadable_browsers: bool,
}

/// What accessibility could read from one on-screen browser window.
#[derive(Debug, Clone, Default, uniffi::Record)]
pub struct BrowserWindow {
    pub title: Option<String>,
    /// Addresses of every web area in the window (page, extension popup, side panel).
    pub urls: Vec<String>,
    pub control_labels: Vec<String>,
    pub field_descriptions: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HideReason {
    App,
    PasswordManager,
    Site,
    PrivateWindow,
    Unreadable,
}

/// Desktop password managers. Matched whole, or as a dotted prefix ("com.1password" covers "com.1password.safari").
const MANAGER_APPS: &[&str] = &[
    "com.bitwarden", "com.1password", "com.agilebits", "com.apple.passwords", "com.apple.keychainaccess",
    "com.callpod", "com.keepersecurity", "com.lastpass", "org.keepassxc", "com.hicknhacksoftware.macpass", "com.dashlane",
    "me.proton.pass", "in.sinew", "com.nordpass", "com.nordsec.nordpass", "com.siber.roboform", "com.markmcguill.strongbox",
    "com.outercorner.secrets", "com.enpass", "com.authy", "com.yubico", "com.2fas",
];

/// Web vaults, as site rules. Whole hosts where the vault's path varies by locale.
const MANAGER_SITES: &[&str] = &[
    "vault.bitwarden.com", "vault.bitwarden.eu", "keepersecurity.com", "keepersecurity.eu", "keepersecurity.com.au", "keepersecurity.ca",
    "keepersecurity.jp", "keepersecurity.us", "govcloud.keepersecurity.us", "my.1password.com", "*.1password.com", "*.1password.eu",
    "*.1password.ca", "lastpass.com", "lastpass.eu", "app.dashlane.com", "pass.proton.me", "app.nordpass.com", "nordpass.com/vault",
    "passwords.google.com", "online.roboform.com", "vault.zoho.com", "vault.enpass.io", "app.passbolt.com", "app.logmeonce.com",
];

/// Browser-extension ids: their popups belong to the browser, so hiding the desktop app alone would miss them.
/// AIDEV-NOTE: Chromium store ids; Safari/Firefox ids are per-install and are handled by scheme in `manager_url`.
const MANAGER_EXTENSIONS: &[&str] = &[
    "nngceckbapebfimnlniiiahkandclblb", "jbkfoedolllekgbhcbcoahefnbanhhlh", // Bitwarden (Chrome, Edge)
    "bfogiafebfohielmmehodmfbbebbbpei", "lfochlioelphaglamdcakfjemolpichk", // Keeper
    "aeblfdkhhhdcdjpifhhbdiojplfjncoa", "dppgmdbiimibapkepcbdbmkaabgiofem", // 1Password
    "hdokiejnpimakedhajhdlcegeplioahd", "bbcinlkgjjkejfdpemiealijmmooekmp", // LastPass
    "fdjamakpfbbddfjaooikfcpapjohcfmg", "gehmmocbbkpblljhkekmfhjpfbkclbph", // Dashlane
    "ghmbeldphafepmbegfdlkpapadhbakde",                                     // Proton Pass
    "eiaeiblijfjekdanodkjadfinkhbfgcd",                                     // NordPass
    "oboonakemofpalcgghocfoadofidjkkk", "pdffhmdngciaglkoonimfcmckehcpafo", // KeePassXC
    "kmcfomidfpdkfieipokbalgegidffkal",                                     // Enpass
    "pnlccmojcmeohlpggmfnbbiapkmbliob",                                     // RoboForm
];
const MANAGER_NAMES: &[&str] = &["bitwarden", "1password", "keeper", "lastpass", "dashlane", "protonpass", "nordpass", "keepass", "enpass", "roboform"];

#[derive(Clone, Copy, PartialEq)]
enum Family {
    Chromium,
    Safari,
    Firefox,
    Other,
}

fn family(bundle: &str) -> Option<Family> {
    let b = bundle.to_lowercase();
    let p = |list: &[&str]| list.iter().any(|x| b == *x || b.starts_with(&format!("{x}.")));
    if p(&["com.google.chrome", "org.chromium.chromium", "com.brave.browser", "com.microsoft.edgemac", "com.vivaldi.vivaldi",
           "com.operasoftware.opera", "com.operasoftware.operagx", "ru.yandex.desktop.yandex-browser", "com.naver.whale", "com.coccoc.coccoc"]) {
        Some(Family::Chromium)
    } else if p(&["com.apple.safari", "com.apple.safaritechnologypreview", "com.kagi.kagimacos"]) {
        Some(Family::Safari)
    } else if p(&["org.mozilla.firefox", "org.mozilla.firefoxdeveloperedition", "org.mozilla.nightly", "org.mozilla.librewolf",
                  "io.gitlab.librewolf-community.librewolf", "app.zen-browser.zen", "org.torproject.torbrowser"]) {
        Some(Family::Firefox)
    } else if p(&["company.thebrowser.browser", "company.thebrowser.dia", "com.sigmaos.sigmaos.macos", "ai.perplexity.comet",
                  "com.perplexity.comet", "com.openai.atlas", "com.duckduckgo.macos.browser"]) {
        Some(Family::Other)
    } else {
        None
    }
}

#[uniffi::export]
pub fn is_browser(bundle_id: &str) -> bool {
    family(bundle_id).is_some()
}

#[uniffi::export]
pub fn hide_app(ex: &Exclusions, bundle_id: &str) -> Option<HideReason> {
    let b = bundle_id.to_lowercase();
    if b.is_empty() {
        return None;
    }
    if ex.apps.iter().any(|a| a.to_lowercase() == b) {
        return Some(HideReason::App);
    }
    (ex.password_managers && MANAGER_APPS.iter().any(|m| b == *m || b.starts_with(&format!("{m}.")))).then_some(HideReason::PasswordManager)
}

/// Case-insensitive glob where `*` spans anything.
fn glob(pat: &str, s: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == s;
    }
    let Some(mut rest) = s.strip_prefix(parts[0]) else { return false };
    for p in &parts[1..parts.len() - 1] {
        match rest.find(p) {
            Some(i) => rest = &rest[i + p.len()..],
            None => return false,
        }
    }
    rest.ends_with(parts[parts.len() - 1])
}

fn host_of(u: &url::Url) -> String {
    u.host_str().unwrap_or("").to_lowercase().trim_end_matches('.').to_owned()
}

/// host[:port][/path], canonicalised by the same parser that reads page addresses.
struct Rule {
    host: String,
    port: Option<u16>,
    path: String,
}

impl Rule {
    fn parse(raw: &str) -> Option<Rule> {
        let s = raw.trim().to_lowercase();
        if s.is_empty() || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return None;
        }
        let s = s.split_once("://").map_or(s.as_str(), |(_, r)| r).trim_start_matches('/');
        let s = s.split(['?', '#']).next().unwrap_or("");
        let (auth, path) = s.split_once('/').unwrap_or((s, ""));
        let auth = auth.rsplit('@').next().unwrap_or("");
        let after_bracket = auth.rfind(']').map_or(0, |i| i + 1);
        let (host, port) = match auth[after_bracket..].rfind(':').map(|i| i + after_bracket) {
            Some(i) => (&auth[..i], Some(auth[i + 1..].parse::<u16>().ok()?)),
            None => (auth, None),
        };
        let host = host.trim_matches('.');
        let host = host.strip_prefix("www.").unwrap_or(host);
        let host = if host.contains('*') {
            let sane = host.chars().all(|c| c.is_alphanumeric() || ".-*".contains(c)) && host.chars().any(char::is_alphanumeric);
            (sane && !host.contains("..")).then(|| host.to_owned())?
        } else {
            (!host.contains("..")).then(|| url::Host::parse(host).ok().map(|h| h.to_string()))??
        };
        let path = url::Url::parse(&format!("http://x/{path}")).ok()?.path().to_lowercase();
        let path = path.trim_end_matches('*').trim_matches('/').to_owned();
        Some(Rule { host, port, path })
    }

    /// A plain host covers its subdomains; a path covers everything beneath it.
    fn matches(&self, u: &url::Url) -> bool {
        let h = host_of(u);
        if h.is_empty() {
            return false;
        }
        let host_ok = if self.host.contains('*') { glob(&self.host, &h) } else { h == self.host || h.ends_with(&format!(".{}", self.host)) };
        let port_ok = self.port.is_none_or(|p| u.port_or_known_default() == Some(p));
        let up = u.path().to_lowercase();
        let up = up.trim_matches('/');
        let path_ok = self.path.is_empty() || glob(&self.path, up) || glob(&format!("{}/*", self.path), up);
        host_ok && port_ok && path_ok
    }
}

impl std::fmt::Display for Rule {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.host)?;
        if let Some(p) = self.port {
            write!(f, ":{p}")?;
        }
        if !self.path.is_empty() {
            write!(f, "/{}", self.path)?;
        }
        Ok(())
    }
}

static MANAGER_RULES: std::sync::LazyLock<Vec<Rule>> = std::sync::LazyLock::new(|| MANAGER_SITES.iter().filter_map(|s| Rule::parse(s)).collect());

/// The page behind view-source:, blob:, reader mode and similar wrappers.
fn unwrap(mut u: url::Url) -> url::Url {
    for _ in 0..4 {
        let inner = match u.scheme() {
            "view-source" | "blob" | "feed" => url::Url::parse(u.as_str().split_once(':').map_or("", |x| x.1)).ok(),
            "http" | "https" | "file" => None,
            _ => u.query_pairs().find(|(k, _)| k == "url").and_then(|(_, v)| url::Url::parse(&v).ok()),
        };
        match inner {
            Some(i) => u = i,
            None => break,
        }
    }
    u
}

/// Could this address be showing a site we can't identify?
fn opaque(u: &url::Url) -> bool {
    match u.scheme() {
        "file" | "chrome" | "edge" | "brave" | "vivaldi" | "opera" | "arc" | "comet" | "favorites" | "topsites" | "bookmarks" | "history" => false,
        "about" => matches!(u.path(), "blank" | "srcdoc" | "reader"),
        _ => host_of(u).is_empty(),
    }
}

fn manager_url(u: &url::Url, title: &str) -> bool {
    match u.scheme() {
        "http" | "https" => MANAGER_RULES.iter().any(|r| r.matches(u)),
        "file" => false,
        "chrome-extension" => {
            let t = format!("{u} {title}").to_lowercase();
            MANAGER_EXTENSIONS.contains(&host_of(u).as_str()) || MANAGER_NAMES.iter().any(|n| t.contains(n))
        }
        // Safari and Firefox give every install a random id, so there is nothing to recognise: assume the worst.
        s if s.ends_with("extension") => true,
        _ => {
            let t = u.as_str().to_lowercase();
            t.contains("password") || t.starts_with("about:logins")
        }
    }
}

// Private-window markers adapted from Dayflow's BrowserWindowClassifier (MIT, (c) 2025 Jerry Liu); see NOTICE.
const INCOGNITO: &[&str] = &[
    "incognito", "in incognito", "incógnito", "modo incógnito", "navigation privée", "inkognito", "anonimowy", "anônima",
    "janela anônima", "navegação anônima", "privé", "incognitomodus", "инкогнито", "режим инкогнито", "シークレット",
    "シークレット モード", "无痕式", "无痕", "無痕式", "無痕", "시크릿", "시크릿 모드", "gizli", "anonymní", "анонімний", "inprivate",
];
const PRIVATE_LABELS: &[&str] = &[
    "private", "privat", "privada", "privado", "privée", "privé", "privata", "частный доступ", "プライベート", "无痕浏览", "無痕瀏覽",
    "개인정보 보호", "yksityinen", "prywatne",
];
const PRIVATE_BROWSING: &[&str] = &[
    "private browsing", "privates surfen", "navegación privada", "navegação privada", "navigation privée", "navigazione privata",
    "privé navigeren", "частный доступ", "プライベートブラウズ", "无痕浏览", "無痕瀏覽", "개인정보 보호 브라우징", "yksityinen selaus",
    "przeglądanie prywatne", "gizli gezinti",
];
const PRIVATE_FIELD: &[&str] = &[
    "private window", "privates fenster", "ventana privada", "janela privada", "fenêtre privée", "finestra privata", "privévenster",
    "プライベートウインドウ", "无痕浏览窗口", "無痕瀏覽視窗", "개인정보 보호 윈도우",
];

fn norm(s: &str) -> String {
    s.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_private(f: Family, w: &BrowserWindow) -> bool {
    // Labels are whole-control text, so every vendor's word is safe to accept. Titles contain page text, so be stricter.
    let label = [INCOGNITO, PRIVATE_LABELS, PRIVATE_BROWSING].concat();
    let title: &[&str] = match f {
        Family::Chromium => INCOGNITO,
        Family::Safari | Family::Firefox => PRIVATE_BROWSING,
        Family::Other => &[INCOGNITO, PRIVATE_BROWSING].concat(),
    };
    let field = [PRIVATE_FIELD, PRIVATE_BROWSING].concat();
    let t = norm(w.title.as_deref().unwrap_or(""));
    let piece = t.split(|c: char| "()[]—–-,|·•".contains(c)).map(norm).any(|p| title.iter().any(|m| p == *m || p.ends_with(&format!(" {m}"))));
    // "News - Brave (Private)": a bracketed suffix is the browser's, not the page's.
    let suffix = t.strip_suffix(')').and_then(|x| x.rsplit_once('(')).is_some_and(|(_, x)| label.contains(&x.trim()));
    piece
        || suffix
        || w.control_labels.iter().map(|l| norm(l)).any(|l| {
            let bare = l.split(" (").next().unwrap_or("").trim();
            label.contains(&bare) || l.starts_with("private with tor") || l.starts_with("private window")
        })
        || w.field_descriptions.iter().map(|d| norm(d)).any(|d| field.iter().any(|m| d.contains(m)))
}

/// Decide whether a whole browser must be left out. `windows` is its on-screen windows; None means accessibility
/// couldn't look. AIDEV-NOTE: privacy-critical; any one offending window hides the entire browser.
#[uniffi::export]
pub fn hide_browser(ex: &Exclusions, bundle_id: &str, windows: Option<Vec<BrowserWindow>>) -> Option<HideReason> {
    let f = family(bundle_id)?;
    let Some(windows) = windows else { return ex.unreadable_browsers.then_some(HideReason::Unreadable) };
    let rules: Vec<Rule> = ex.sites.iter().filter_map(|s| Rule::parse(s)).collect();
    let mut unreadable = false;
    let mut found = None;
    for w in &windows {
        if ex.private_windows {
            if is_private(f, w) {
                found = found.or(Some(HideReason::PrivateWindow));
            }
            unreadable |= w.title.is_none() && w.control_labels.is_empty() && w.field_descriptions.is_empty();
        }
        unreadable |= w.urls.is_empty();
        for raw in &w.urls {
            let Ok(u) = url::Url::parse(raw) else {
                unreadable = true;
                continue;
            };
            let u = unwrap(u);
            if ex.password_managers && manager_url(&u, w.title.as_deref().unwrap_or("")) {
                return Some(HideReason::PasswordManager);
            }
            if rules.iter().any(|r| r.matches(&u)) {
                found = Some(HideReason::Site);
            }
            unreadable |= opaque(&u);
        }
    }
    found.or((unreadable && ex.unreadable_browsers).then_some(HideReason::Unreadable))
}

/// Tidy what a person typed into a site rule; None if it could never match anything.
#[uniffi::export]
pub fn normalize_site(raw: &str) -> Option<String> {
    Rule::parse(raw).map(|r| r.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ex() -> Exclusions {
        Exclusions { apps: vec!["com.acme.Ledger".into()], sites: vec![], password_managers: true, private_windows: true, unreadable_browsers: true }
    }
    fn sites(s: &[&str]) -> Exclusions {
        Exclusions { sites: s.iter().map(|x| x.to_string()).collect(), ..ex() }
    }
    fn win(url: &str) -> BrowserWindow {
        BrowserWindow { title: Some("Page".into()), urls: vec![url.into()], ..Default::default() }
    }
    const CHROME: &str = "com.google.Chrome";
    const SAFARI: &str = "com.apple.Safari";
    fn hid(e: &Exclusions, url: &str) -> Option<HideReason> {
        hide_browser(e, CHROME, Some(vec![win(url)]))
    }

    #[test]
    fn password_manager_apps() {
        for b in ["com.bitwarden.desktop", "com.1password.1password", "com.agilebits.onepassword7", "com.apple.Passwords", "com.apple.keychainaccess",
                  "com.callpod.keepermac.lite", "com.keepersecurity.passwordmanager", "com.lastpass.LastPass", "org.keepassxc.keepassxc",
                  "com.dashlane.Dashlane", "me.proton.pass.electron", "in.sinew.Enpass-Desktop", "COM.BITWARDEN.DESKTOP"] {
            assert_eq!(hide_app(&ex(), b), Some(HideReason::PasswordManager), "{b}");
        }
        let off = Exclusions { password_managers: false, ..ex() };
        assert_eq!(hide_app(&off, "com.bitwarden.desktop"), None);
        assert_eq!(hide_app(&ex(), "com.apple.TextEdit"), None);
        assert_eq!(hide_app(&ex(), ""), None);
    }

    #[test]
    fn chosen_apps() {
        assert_eq!(hide_app(&ex(), "com.acme.ledger"), Some(HideReason::App));
        assert_eq!(hide_app(&ex(), "com.acme.ledger2"), None);
    }

    #[test]
    fn password_manager_pages_and_extension_popups() {
        for u in ["https://vault.bitwarden.com/#/vault", "https://vault.bitwarden.eu/", "https://keepersecurity.com/vault/", "https://keepersecurity.eu/vault",
                  "https://my.1password.com/app", "https://team.1password.com/", "https://lastpass.com/vault", "https://app.dashlane.com/",
                  "https://pass.proton.me/u/0", "https://passwords.google.com/", "chrome://password-manager/passwords", "about:logins",
                  "chrome-extension://nngceckbapebfimnlniiiahkandclblb/popup/index.html", "chrome-extension://bfogiafebfohielmmehodmfbbebbbpei/x",
                  "chrome-extension://aeblfdkhhhdcdjpifhhbdiojplfjncoa/popup/index.html"] {
            assert_eq!(hid(&ex(), u), Some(HideReason::PasswordManager), "{u}");
        }
        assert_eq!(hid(&ex(), "https://bitwarden.com/blog/"), None);
        let off = Exclusions { password_managers: false, ..ex() };
        assert_eq!(hid(&off, "https://vault.bitwarden.com/"), None);
    }

    #[test]
    fn a_password_manager_in_any_window_or_frame_hides_the_browser() {
        let mut two = win("https://example.com/");
        two.urls.push("chrome-extension://nngceckbapebfimnlniiiahkandclblb/popup".into());
        assert_eq!(hide_browser(&ex(), CHROME, Some(vec![win("https://a.com"), two])), Some(HideReason::PasswordManager));
    }

    #[test]
    fn site_rules() {
        let e = sites(&["chase.com", "github.com/settings", "*.internal.*", "localhost:8443"]);
        for u in ["https://chase.com/", "https://secure.chase.com/x?y", "HTTPS://WWW.CHASE.COM", "https://chase.com./",
                  "https://github.com/settings", "https://github.com/settings/tokens", "https://wiki.internal.acme.io/", "http://localhost:8443/a"] {
            assert_eq!(hid(&e, u), Some(HideReason::Site), "{u}");
        }
        for u in ["https://notchase.com/", "https://chase.com.evil.io/", "https://example.com/chase.com", "https://example.com/?u=chase.com",
                  "https://github.com/settingsfoo", "https://github.com/", "http://localhost:3000/"] {
            assert_eq!(hid(&e, u), None, "{u}");
        }
    }

    #[test]
    fn site_rules_are_tidied() {
        for (a, b) in [(" https://www.Chase.com/ ", "chase.com"), ("github.com/settings/", "github.com/settings"), ("*.corp.example", "*.corp.example"),
                       ("localhost:8443", "localhost:8443"), ("chase.com/?a=1#b", "chase.com")] {
            assert_eq!(normalize_site(a).as_deref(), Some(b), "{a}");
        }
        for bad in ["", "   ", "*", "*.*", "/", "two words", "http://", "."] {
            assert_eq!(normalize_site(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn unreadable_browsers_fail_closed() {
        assert_eq!(hide_browser(&ex(), CHROME, None), Some(HideReason::Unreadable));
        let blank = BrowserWindow { title: Some("x".into()), ..Default::default() };
        assert_eq!(hide_browser(&ex(), CHROME, Some(vec![blank.clone()])), Some(HideReason::Unreadable));
        assert_eq!(hide_browser(&ex(), CHROME, Some(vec![win("https://a.com"), blank.clone()])), Some(HideReason::Unreadable));
        let lax = Exclusions { unreadable_browsers: false, ..ex() };
        assert_eq!(hide_browser(&lax, CHROME, None), None);
        assert_eq!(hide_browser(&lax, CHROME, Some(vec![blank])), None);
        // No windows on screen: nothing to hide.
        assert_eq!(hide_browser(&ex(), CHROME, Some(vec![])), None);
        assert_eq!(hide_browser(&ex(), "com.apple.TextEdit", None), None);
    }

    #[test]
    fn private_windows() {
        let t = |b: &str, title: &str| hide_browser(&ex(), b, Some(vec![BrowserWindow { title: Some(title.into()), urls: vec!["https://a.com".into()], ..Default::default() }]));
        assert_eq!(t(CHROME, "News - Google Chrome (Incognito)"), Some(HideReason::PrivateWindow));
        assert_eq!(t(CHROME, "Seite - Google Chrome (Inkognito)"), Some(HideReason::PrivateWindow));
        assert_eq!(t(SAFARI, "News, Private Browsing"), Some(HideReason::PrivateWindow));
        assert_eq!(t("org.mozilla.firefox", "News — Mozilla Firefox Private Browsing"), Some(HideReason::PrivateWindow));
        assert_eq!(t(CHROME, "How incognito mode works - Google Chrome"), None);
        let f = BrowserWindow { title: Some("x".into()), urls: vec!["https://a.com".into()], field_descriptions: vec!["smart search field in a private window".into()], ..Default::default() };
        assert_eq!(hide_browser(&ex(), SAFARI, Some(vec![f])), Some(HideReason::PrivateWindow));
        let c = BrowserWindow { title: Some("x".into()), urls: vec!["https://a.com".into()], control_labels: vec!["Incognito".into()], ..Default::default() };
        assert_eq!(hide_browser(&ex(), CHROME, Some(vec![c.clone()])), Some(HideReason::PrivateWindow));
        let off = Exclusions { private_windows: false, ..ex() };
        assert_eq!(hide_browser(&off, CHROME, Some(vec![c])), None);
    }

    #[test]
    fn browsers_are_known() {
        for b in [CHROME, SAFARI, "org.mozilla.firefox", "com.brave.Browser", "com.microsoft.edgemac", "company.thebrowser.Browser", "com.kagi.kagimacOS"] {
            assert!(is_browser(b), "{b}");
        }
        assert!(!is_browser("com.apple.finder"));
    }

    // ---- security review 2026-11-23 ----
    #[test]
    fn extensions_with_random_ids_fail_closed() {
        for u in ["moz-extension://3c1f0a52-6f0e-4f1e-9d2b-0a1b2c3d4e5f/popup.html", "safari-web-extension://8E1C2B7A-1111-4222-8333-444455556666/popup/index.html",
                  "safari-extension://com.x-ABCD/123/p.html"] {
            assert_eq!(hid(&ex(), u), Some(HideReason::PasswordManager), "{u}");
        }
        // A known, unrelated Chromium extension is fine.
        assert_eq!(hid(&ex(), "chrome-extension://cjpalhdlnbpafiamejdnhcphjbkeiagm/dashboard.html"), None);
        let named = BrowserWindow { title: Some("Bitwarden".into()), urls: vec!["chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/p".into()], ..Default::default() };
        assert_eq!(hide_browser(&ex(), CHROME, Some(vec![named])), Some(HideReason::PasswordManager));
    }

    #[test]
    fn site_rules_are_canonical() {
        for r in ["chase.com/*", "*://chase.com/*", ".chase.com", "chase.com.", "https://user:pw@chase.com/", "//chase.com", "CHASE.com:443"] {
            let e = sites(&[r]);
            assert_eq!(hid(&e, "https://secure.chase.com/accounts"), Some(HideReason::Site), "{r}");
            assert_eq!(hid(&e, "https://example.com/"), None, "{r}");
        }
        assert_eq!(hid(&sites(&["bücher.de"]), "https://xn--bcher-kva.de/"), Some(HideReason::Site));
        assert_eq!(hid(&sites(&["example.com/a/*/private"]), "https://example.com/a/b/private/x"), Some(HideReason::Site));
        assert_eq!(hid(&sites(&["example.com/my%20docs"]), "https://example.com/my%20docs/1"), Some(HideReason::Site));
        assert_eq!(hid(&sites(&["example.com/é"]), "https://example.com/%C3%A9"), Some(HideReason::Site));
        assert_eq!(hid(&sites(&["192.168.1.1"]), "http://192.168.1.1/admin"), Some(HideReason::Site));
        assert_eq!(hid(&sites(&["[::1]:8080"]), "http://[::1]:8080/"), Some(HideReason::Site));
        for bad in ["exa mple.com", "chase.com:99999", "http://[::1", "a..b"] {
            assert_eq!(normalize_site(bad), None, "{bad}");
        }
    }

    #[test]
    fn every_accepted_rule_matches_its_own_address() {
        for r in ["chase.com", "www.chase.com", "a.b.c.example/x/y", "localhost:8443", "bücher.de/é", "chase.com/*", "10.0.0.1:81/z"] {
            let n = normalize_site(r).unwrap_or_else(|| panic!("{r}"));
            let u = format!("http://{}", n.replace('*', "q"));
            assert_eq!(hid(&sites(&[r]), &u), Some(HideReason::Site), "{r} -> {u}");
        }
    }

    #[test]
    fn other_vendors_private_windows() {
        let t = |b: &str, title: &str, label: &str| {
            hide_browser(&ex(), b, Some(vec![BrowserWindow { title: Some(title.into()), urls: vec!["https://a.com".into()], control_labels: vec![label.into()], ..Default::default() }]))
        };
        for (b, title, label) in [
            ("com.brave.Browser", "News - Brave (Private)", ""), ("com.brave.Browser", "News - Brave", "Private with Tor"),
            ("com.vivaldi.Vivaldi", "News - Vivaldi (Private)", ""), ("com.operasoftware.Opera", "News - Opera (Private)", ""),
            ("com.microsoft.edgemac", "News - Microsoft Edge (InPrivate)", ""), (CHROME, "News", "Incognito (2)"),
            ("company.thebrowser.Browser", "News", "Private"), ("com.duckduckgo.macos.browser", "News", "Private Browsing"),
        ] {
            assert_eq!(t(b, title, label), Some(HideReason::PrivateWindow), "{b} {title} {label}");
        }
        assert_eq!(t("com.brave.Browser", "Private equity news - Brave", "Reload"), None);
    }

    #[test]
    fn wrapped_addresses_are_unwrapped() {
        let e = sites(&["chase.com"]);
        for u in ["view-source:https://chase.com/", "blob:https://chase.com/123-456", "about:reader?url=https%3A%2F%2Fchase.com%2Fx",
                  "chrome-distiller://abc/?url=https%3A%2F%2Fchase.com%2F", "safari-reader://chase.com/x"] {
            assert_eq!(hid(&e, u), Some(HideReason::Site), "{u}");
        }
        for u in ["data:text/html,hi", "about:blank", "about:srcdoc", "weird:thing"] {
            assert_eq!(hid(&e, u), Some(HideReason::Unreadable), "{u}");
        }
        for u in ["chrome://newtab/", "about:home", "file:///tmp/a.html", "favorites://", "about:newtab", "edge://settings"] {
            assert_eq!(hid(&e, u), None, "{u}");
        }
    }

    #[test]
    fn a_window_with_no_readable_labels_is_unreadable() {
        let w = BrowserWindow { urls: vec!["https://a.com".into()], ..Default::default() };
        assert_eq!(hide_browser(&ex(), CHROME, Some(vec![w.clone()])), Some(HideReason::Unreadable));
        let e = Exclusions { private_windows: false, ..ex() };
        assert_eq!(hide_browser(&e, CHROME, Some(vec![w])), None);
    }

    #[test]
    fn more_vaults() {
        for u in ["https://online.roboform.com/", "https://keepersecurity.us/vault/", "https://keepersecurity.com/en_GB/vault", "https://vault.keepersecurity.eu/",
                  "https://lastpass.com/?ac=1", "https://nordpass.com/vault", "https://vault.zoho.com/", "https://vault.enpass.io/",
                  "opera://settings/passwords", "arc://password-manager", "comet://password-manager/", "chrome://settings/passwords", "vivaldi:settings/passwords", "about:preferences#privacy-passwords"] {
            assert_eq!(hid(&ex(), u), Some(HideReason::PasswordManager), "{u}");
        }
    }
}
