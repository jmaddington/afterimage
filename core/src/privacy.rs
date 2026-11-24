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

/// Web vaults, as site rules.
const MANAGER_SITES: &[&str] = &[
    "vault.bitwarden.com", "vault.bitwarden.eu", "keepersecurity.com/vault", "keepersecurity.eu/vault", "keepersecurity.com.au/vault",
    "keepersecurity.ca/vault", "keepersecurity.jp/vault", "my.1password.com", "*.1password.com", "*.1password.eu", "*.1password.ca",
    "lastpass.com/vault", "app.dashlane.com", "pass.proton.me", "app.nordpass.com", "passwords.google.com",
];

/// Browser-extension ids: their popups belong to the browser, so hiding the desktop app alone would miss them.
/// AIDEV-NOTE: Chromium store ids; Safari/Firefox ids are per-install and are caught by name in `manager_url`.
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

/// A rule is host[:port][/path]. A plain host covers its subdomains; a path covers everything beneath it.
fn site_matches(rule: &str, u: &url::Url) -> bool {
    let (hostport, path) = rule.split_once('/').map_or((rule, ""), |(h, p)| (h, p));
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) if p.parse::<u16>().is_ok() => (h, p.parse().ok()),
        _ => (hostport, None),
    };
    let h = host_of(u);
    if h.is_empty() {
        return false;
    }
    let host_ok = if host.contains('*') { glob(host, &h) } else { h == host || h.ends_with(&format!(".{host}")) };
    let port_ok = port.is_none_or(|p: u16| u.port_or_known_default() == Some(p));
    let up = u.path().to_lowercase();
    let up = up.trim_matches('/');
    let path_ok = path.is_empty() || up == path || up.starts_with(&format!("{path}/"));
    host_ok && port_ok && path_ok
}

fn manager_url(u: &url::Url) -> bool {
    match u.scheme() {
        "http" | "https" => MANAGER_SITES.iter().any(|r| site_matches(r, u)),
        "chrome" | "edge" | "brave" | "vivaldi" => {
            let s = u.as_str().to_lowercase();
            s.contains("://password-manager") || s.contains("://settings/password") || s.contains("://wallet/passwords")
        }
        "about" => u.path().eq_ignore_ascii_case("logins"),
        s if s.ends_with("extension") => {
            let t = u.as_str().to_lowercase();
            MANAGER_EXTENSIONS.contains(&host_of(u).as_str()) || MANAGER_NAMES.iter().any(|n| t.contains(n))
        }
        _ => false,
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
    let (title, label, field): (Vec<&str>, Vec<&str>, &[&str]) = match f {
        Family::Chromium => (INCOGNITO.to_vec(), INCOGNITO.to_vec(), &[]),
        Family::Safari => (PRIVATE_BROWSING.to_vec(), PRIVATE_LABELS.to_vec(), PRIVATE_FIELD),
        Family::Firefox => (PRIVATE_BROWSING.to_vec(), PRIVATE_BROWSING.to_vec(), PRIVATE_BROWSING),
        // Unmeasured browsers: accept any family's marker.
        Family::Other => ([INCOGNITO, PRIVATE_BROWSING].concat(), [INCOGNITO, PRIVATE_BROWSING].concat(), PRIVATE_FIELD),
    };
    let title_hit = w.title.as_deref().is_some_and(|t| {
        t.split(|c: char| "()[]—–-,|·•".contains(c)).map(norm).any(|p| title.iter().any(|m| p == *m || p.ends_with(&format!(" {m}"))))
    });
    title_hit
        || w.control_labels.iter().map(|l| norm(l)).any(|l| label.contains(&l.as_str()))
        || w.field_descriptions.iter().map(|d| norm(d)).any(|d| field.iter().any(|m| d.contains(m)))
}

/// Decide whether a whole browser must be left out. `windows` is its on-screen windows; None means accessibility
/// couldn't look. AIDEV-NOTE: privacy-critical; any one offending window hides the entire browser.
#[uniffi::export]
pub fn hide_browser(ex: &Exclusions, bundle_id: &str, windows: Option<Vec<BrowserWindow>>) -> Option<HideReason> {
    let f = family(bundle_id)?;
    let Some(windows) = windows else { return ex.unreadable_browsers.then_some(HideReason::Unreadable) };
    let mut unreadable = false;
    let mut found = None;
    for w in &windows {
        if ex.private_windows && is_private(f, w) {
            found = found.or(Some(HideReason::PrivateWindow));
        }
        unreadable |= w.urls.is_empty();
        for raw in &w.urls {
            let Ok(u) = url::Url::parse(raw) else {
                unreadable = true;
                continue;
            };
            if ex.password_managers && manager_url(&u) {
                return Some(HideReason::PasswordManager);
            }
            if ex.sites.iter().filter_map(|s| normalize_site(s)).any(|s| site_matches(&s, &u)) {
                found = Some(HideReason::Site);
            }
        }
    }
    found.or((unreadable && ex.unreadable_browsers).then_some(HideReason::Unreadable))
}

/// Tidy what a person typed into a site rule; None if it can't be one.
#[uniffi::export]
pub fn normalize_site(raw: &str) -> Option<String> {
    let s = raw.trim().to_lowercase();
    let s = s.split_once("://").map_or(s.as_str(), |(_, r)| r);
    let s = s.split(['?', '#']).next().unwrap_or("");
    let s = s.strip_prefix("www.").unwrap_or(s);
    let s = s.trim_end_matches('/');
    let host = s.split('/').next().unwrap_or("");
    let ok = host.chars().any(char::is_alphanumeric) && !s.chars().any(char::is_whitespace);
    ok.then(|| s.to_owned())
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
}
