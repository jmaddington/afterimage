//! What must never be in the picture: apps, sites, private windows, password managers.

#[derive(Debug, Clone, Default, uniffi::Record)]
pub struct Exclusions {
    pub apps: Vec<String>,
    pub sites: Vec<String>,
    pub password_managers: bool,
    pub private_windows: bool,
    /// Leave a browser out when its page address can't be read.
    pub unreadable_browsers: bool,
    /// Leave Tor Browser out altogether; nothing in its windows says they're private.
    #[uniffi(default = true)]
    pub tor: bool,
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
    SystemPrompt,
    SecretField,
    /// The browser's extension was asked and didn't answer in time.
    NoAnswer,
}

/// Desktop password managers. Matched whole, or as a dotted prefix ("com.1password" covers "com.1password.safari").
const MANAGER_APPS: &[&str] = &[
    "com.bitwarden", "com.1password", "com.agilebits", "com.apple.passwords", "com.apple.keychainaccess",
    "com.callpod", "com.keepersecurity", "com.lastpass", "org.keepassxc", "com.hicknhacksoftware.macpass", "com.dashlane",
    "me.proton.pass", "in.sinew", "com.nordpass", "com.nordsec.nordpass", "com.siber.roboform", "com.markmcguill.strongbox",
    "com.outercorner.secrets", "com.enpass", "com.authy", "com.yubico", "com.2fas",
];

/// macOS asking for a password, a fingerprint or a permission. Never in the picture, whatever the settings.
const SYSTEM_PROMPTS: &[&str] = &[
    "com.apple.localauthentication.uiagent", "com.apple.localauthenticationremoteservice", "com.apple.securityagent", "com.apple.usernotificationcenter",
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

static OTHER_BROWSERS: std::sync::LazyLock<std::sync::RwLock<std::collections::HashSet<String>>> = std::sync::LazyLock::new(Default::default);

/// Apps this Mac says can open web pages. Any not already known is read like a browser, and left out when it can't be.
#[uniffi::export]
pub fn register_browsers(bundle_ids: Vec<String>) {
    // Only ever added to while the app runs: an app that vanishes from the list for a moment is still a browser.
    OTHER_BROWSERS.write().unwrap_or_else(|e| e.into_inner()).extend(bundle_ids.into_iter().map(|b| b.to_lowercase()));
}

/// Registered apps that have actually shown a web page. Terminals and the like register for links and never do.
static SHOWN_PAGES: std::sync::LazyLock<std::sync::RwLock<std::collections::HashSet<String>>> = std::sync::LazyLock::new(Default::default);

static PAGELESS_LOOKS: std::sync::LazyLock<std::sync::RwLock<std::collections::HashMap<String, (u8, Option<std::time::Instant>)>>> = std::sync::LazyLock::new(Default::default);

#[cfg(not(test))]
const PAGELESS_GAP: std::time::Duration = std::time::Duration::from_millis(1200);
#[cfg(test)]
const PAGELESS_GAP: std::time::Duration = std::time::Duration::from_millis(30);

fn only_registered(bundle: &str) -> bool {
    family_of(bundle, false).is_none() && family_of(bundle, true).is_some()
}

fn family(bundle: &str) -> Option<Family> {
    family_of(bundle, true)
}

/// `registered`: also count apps this Mac says can open web pages.
fn family_of(bundle: &str, registered: bool) -> Option<Family> {
    let b = bundle.to_lowercase();
    let p = |list: &[&str]| list.iter().any(|x| b == *x || b.starts_with(&format!("{x}.")));
    if is_tor(&b) {
        Some(Family::Firefox)
    } else if p(&["com.google.chrome", "org.chromium.chromium", "com.brave.browser", "com.microsoft.edgemac", "com.vivaldi.vivaldi",
           "com.operasoftware.opera", "com.operasoftware.operagx", "ru.yandex.desktop.yandex-browser", "com.naver.whale", "com.coccoc.coccoc"]) {
        Some(Family::Chromium)
    } else if p(&["com.apple.safari", "com.apple.safaritechnologypreview", "com.kagi.kagimacos"]) {
        Some(Family::Safari)
    } else if p(&["org.mozilla.firefox", "org.mozilla.firefoxdeveloperedition", "org.mozilla.nightly", "org.mozilla.librewolf",
                  "io.gitlab.librewolf-community.librewolf", "app.zen-browser.zen"]) {
        Some(Family::Firefox)
    } else if p(&["company.thebrowser.browser", "company.thebrowser.dia", "com.sigmaos.sigmaos.macos", "ai.perplexity.comet",
                  "com.perplexity.comet", "com.openai.atlas", "com.duckduckgo.macos.browser"]) {
        Some(Family::Other)
    } else if registered && SHOWN_PAGES.read().unwrap_or_else(|e| e.into_inner()).contains(&b) {
        Some(Family::Other)
    } else if registered && OTHER_BROWSERS.read().unwrap_or_else(|e| e.into_inner()).contains(&b) {
        Some(Family::Other)
    } else {
        None
    }
}

#[uniffi::export]
pub fn is_browser(bundle_id: &str) -> bool {
    family(bundle_id).is_some()
}

/// Firefox describes its pages to accessibility only once asked to.
#[uniffi::export]
pub fn needs_waking(bundle_id: &str) -> bool {
    // An unfamiliar browser may be built on Firefox; asking does no harm to one that isn't.
    family(bundle_id) == Some(Family::Firefox) || only_registered(bundle_id)
}

fn is_tor(bundle_id: &str) -> bool {
    bundle_id.to_lowercase().starts_with("org.torproject.")
}

/// A browser we know, or an app that has actually shown a web page. A terminal that merely registers for links isn't.
pub fn shows_pages(bundle_id: &str) -> bool {
    family_of(bundle_id, false).is_some() || SHOWN_PAGES.read().unwrap_or_else(|e| e.into_inner()).contains(&bundle_id.to_lowercase())
}

#[uniffi::export]
pub fn hide_app(ex: &Exclusions, bundle_id: &str) -> Option<HideReason> {
    let b = bundle_id.to_lowercase();
    if b.is_empty() {
        return None;
    }
    if SYSTEM_PROMPTS.contains(&b.as_str()) {
        return Some(HideReason::SystemPrompt);
    }
    if ex.apps.iter().any(|a| a.to_lowercase() == b) || ex.tor && is_tor(&b) {
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
            // Page hosts arrive in punycode, so plain labels convert; a label mixing `*` with non-ASCII can't and is refused.
            let labels: Option<Vec<String>> = host
                .split('.')
                .map(|l| match l {
                    "" => None,
                    l if l.is_ascii() => l.chars().all(|c| c.is_ascii_alphanumeric() || "-_*".contains(c)).then(|| l.to_owned()),
                    l if l.contains('*') => None,
                    l => url::Host::parse(l).ok().map(|h| h.to_string()),
                })
                .collect();
            let h = labels?.join(".");
            h.chars().any(char::is_alphanumeric).then_some(h)?
        } else {
            (!host.contains("..")).then(|| url::Host::parse(host).ok().map(|h| h.to_string()))??
        };
        let path = url::Url::parse(&format!("http://x/{path}")).ok()?.path().to_lowercase();
        // Only a whole trailing segment is redundant; "acc*" must keep its star.
        let path = path.trim_matches('/');
        let path = if path == "*" { "" } else { path.strip_suffix("/*").unwrap_or(path) }.trim_matches('/').to_owned();
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

const WRAPPERS: &[&str] = &["view-source", "blob", "feed", "chrome-distiller", "safari-reader", "read"];

/// More addresses, or deeper encoding, than any honest page needs.
const MAX_LAYERS: usize = 32;
const MAX_DECODES: usize = 6;
const MAX_ADDRESS: usize = 16_384;
const MAX_FOUND: usize = 256;

const MAX_PARSE: usize = 2048;

/// Add the address starting here. False when it's too long to read whole.
fn address(s: &str, found: &mut Vec<url::Url>) -> bool {
    let whole = s.len() <= MAX_PARSE;
    let end = if whole { s.len() } else { (0..=MAX_PARSE).rev().find(|i| s.is_char_boundary(*i)).unwrap_or(0) };
    found.extend(url::Url::parse(&s[..end]));
    whole
}

/// This address and every address it carries inside, plain or percent-encoded. A wrapper can add suspicion, never
/// remove it. False when there was too much to look through; what was found is still returned and must be checked.
fn layers(u: &url::Url) -> (Vec<url::Url>, bool) {
    let mut out = vec![u.clone()];
    let mut complete = true;
    let mut i = 0;
    while i < out.len() {
        if i == MAX_LAYERS {
            complete = false;
            break;
        }
        let u = out[i].clone();
        i += 1;
        // Ordinary pages may mention any address; only browser-made schemes display one.
        if matches!(u.scheme(), "http" | "https" | "file") {
            continue;
        }
        let mut rest = u.as_str().split_once(':').map_or("", |x| x.1).to_lowercase();
        if rest.len() > MAX_ADDRESS {
            complete = false;
            continue;
        }
        let mut found = Vec::new();
        for (n, (_, v)) in u.query_pairs().enumerate() {
            complete &= n < MAX_FOUND && address(&v, &mut found);
            if n == MAX_FOUND {
                break;
            }
        }
        for n in 0.. {
            complete &= address(&rest, &mut found);
            for mark in ["https:", "http:"] {
                for (k, (at, _)) in rest.match_indices(mark).enumerate() {
                    complete &= k < MAX_FOUND && address(&rest[at..], &mut found);
                    if k == MAX_FOUND {
                        break;
                    }
                }
            }
            if out.len() + found.len() > MAX_FOUND {
                found.truncate(MAX_FOUND);
                complete = false;
                break;
            }
            let next = percent_encoding::percent_decode_str(&rest).decode_utf8_lossy().to_lowercase();
            if next == rest {
                break;
            }
            if n == MAX_DECODES {
                complete = false;
                break;
            }
            rest = next;
        }
        for f in found {
            if f.has_host() && !out.contains(&f) {
                out.push(f);
            }
        }
    }
    (out, complete)
}

/// Could this address be showing a site we can't identify? Anything unfamiliar counts.
fn opaque(u: &url::Url, carries_page: bool) -> bool {
    match u.scheme() {
        "http" | "https" => host_of(u).is_empty(),
        // Firefox's own window furniture: whatever page is inside it wasn't seen.
        "chrome" if host_of(u) == "browser" => true,
        "file" | "chrome" | "edge" | "brave" | "vivaldi" | "opera" | "arc" | "comet" | "chrome-extension" | "favorites" | "topsites" | "bookmarks" | "history" => false,
        "about" if u.path() == "reader" => !carries_page,
        "about" => matches!(u.path(), "blank" | "srcdoc"),
        "safari-reader" if u.has_host() => false,
        s if WRAPPERS.contains(&s) => !carries_page,
        _ => true,
    }
}

fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
}

fn manager_url(u: &url::Url, title: &str) -> bool {
    if MANAGER_RULES.iter().any(|r| r.matches(u)) {
        return true;
    }
    match u.scheme() {
        "http" | "https" | "file" | "safari-reader" => false,
        "chrome-extension" => {
            let t = squash(&format!("{u} {title}"));
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

/// Decide whether a whole browser must be left out. `windows` is what accessibility could read of its on-screen
/// windows; `complete` is false when something couldn't be read, which never excuses what was. AIDEV-NOTE: privacy-critical; any one offending window hides the entire browser.
#[uniffi::export]
pub fn hide_browser(ex: &Exclusions, bundle_id: &str, windows: &[BrowserWindow], complete: bool) -> Option<HideReason> {
    let f = family(bundle_id)?;
    let rules: Vec<Rule> = ex.sites.iter().filter_map(|s| Rule::parse(s)).collect();
    if ex.tor && is_tor(bundle_id) {
        return Some(HideReason::App);
    }
    if only_registered(bundle_id) {
        let b = bundle_id.to_lowercase();
        let private = ex.private_windows && windows.iter().any(|w| is_private(f, w));
        if private || windows.iter().any(|w| !w.urls.is_empty()) {
            SHOWN_PAGES.write().unwrap_or_else(|e| e.into_inner()).insert(b.clone());
        }
        // Not a browser only on evidence: a full look at its windows that found no page in any of them.
        // And looked at properly: each window's own controls were read, three times running, with never a page.
        let read = |w: &BrowserWindow| w.urls.is_empty() && w.control_labels.iter().chain(&w.field_descriptions).any(|l| !l.trim().is_empty());
        let pageless = complete && !windows.is_empty() && windows.iter().all(read);
        let mut looks = PAGELESS_LOOKS.write().unwrap_or_else(|e| e.into_inner());
        let (n, last) = looks.entry(b.clone()).or_insert((0, None));
        // One look counts per second or so: three in the same breath are one look.
        let spaced = last.is_none_or(|t| t.elapsed() >= PAGELESS_GAP);
        if !pageless {
            (*n, *last) = (0, None);
        } else if spaced {
            (*n, *last) = (n.saturating_add(1), Some(std::time::Instant::now()));
        }
        if *n >= 3 && !SHOWN_PAGES.read().unwrap_or_else(|e| e.into_inner()).contains(&b) {
            return None;
        }
    }
    let mut unreadable = !complete;
    let mut evasive = false;
    let mut found = None;
    for w in windows {
        if ex.private_windows {
            if is_private(f, w) {
                found = found.or(Some(HideReason::PrivateWindow));
            }
            // Markers live in the window's controls; a title alone doesn't prove they were read.
            unreadable |= w.control_labels.iter().chain(&w.field_descriptions).all(|l| l.trim().is_empty());
        }
        unreadable |= w.urls.is_empty();
        for raw in &w.urls {
            let Ok(u) = url::Url::parse(raw) else {
                unreadable = true;
                continue;
            };
            let (all, complete) = layers(&u);
            // Built to evade: hidden whenever any rule might have applied, whatever the unreadable setting says.
            unreadable |= !complete;
            evasive |= !complete && (ex.password_managers || !rules.is_empty());
            if ex.password_managers && all.iter().any(|u| manager_url(u, w.title.as_deref().unwrap_or(""))) {
                return Some(HideReason::PasswordManager);
            }
            if all.iter().any(|u| rules.iter().any(|r| r.matches(u))) {
                found = Some(HideReason::Site);
            }
            unreadable |= opaque(&u, all.len() > 1);
        }
    }
    found.or((evasive || unreadable && ex.unreadable_browsers).then_some(HideReason::Unreadable))
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
        Exclusions { apps: vec!["com.acme.Ledger".into()], sites: vec![], password_managers: true, private_windows: true, unreadable_browsers: true, tor: true }
    }
    fn sites(s: &[&str]) -> Exclusions {
        Exclusions { sites: s.iter().map(|x| x.to_string()).collect(), ..ex() }
    }
    fn win(url: &str) -> BrowserWindow {
        BrowserWindow { title: Some("Page".into()), urls: vec![url.into()], control_labels: vec!["Reload".into()], ..Default::default() }
    }
    const CHROME: &str = "com.google.Chrome";
    const SAFARI: &str = "com.apple.Safari";
    fn hid(e: &Exclusions, url: &str) -> Option<HideReason> {
        hide_browser(e, CHROME, &[win(url)], true)
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
        assert_eq!(hide_browser(&ex(), CHROME, &[win("https://a.com"), two], true), Some(HideReason::PasswordManager));
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
        assert_eq!(hide_browser(&ex(), CHROME, &[], false), Some(HideReason::Unreadable));
        let blank = BrowserWindow { title: Some("x".into()), ..Default::default() };
        assert_eq!(hide_browser(&ex(), CHROME, &[blank.clone()], true), Some(HideReason::Unreadable));
        assert_eq!(hide_browser(&ex(), CHROME, &[win("https://a.com"), blank.clone()], true), Some(HideReason::Unreadable));
        let lax = Exclusions { unreadable_browsers: false, ..ex() };
        assert_eq!(hide_browser(&lax, CHROME, &[], false), None);
        assert_eq!(hide_browser(&lax, CHROME, &[blank], true), None);
        // No windows on screen: nothing to hide.
        assert_eq!(hide_browser(&ex(), CHROME, &[], true), None);
        assert_eq!(hide_browser(&ex(), "com.apple.TextEdit", &[], false), None);
    }

    #[test]
    fn private_windows() {
        let t = |b: &str, title: &str| hide_browser(&ex(), b, &[BrowserWindow { title: Some(title.into()), urls: vec!["https://a.com".into()], ..Default::default() }], true);
        assert_eq!(t(CHROME, "News - Google Chrome (Incognito)"), Some(HideReason::PrivateWindow));
        assert_eq!(t(CHROME, "Seite - Google Chrome (Inkognito)"), Some(HideReason::PrivateWindow));
        assert_eq!(t(SAFARI, "News, Private Browsing"), Some(HideReason::PrivateWindow));
        assert_eq!(t("org.mozilla.firefox", "News — Mozilla Firefox Private Browsing"), Some(HideReason::PrivateWindow));
        assert_eq!(t(CHROME, "How incognito mode works - Google Chrome"), Some(HideReason::Unreadable));
        assert_eq!(hide_browser(&ex(), CHROME, &[BrowserWindow { title: Some("How incognito mode works - Google Chrome".into()), ..win("https://a.com") }], true), None);
        let f = BrowserWindow { title: Some("x".into()), urls: vec!["https://a.com".into()], field_descriptions: vec!["smart search field in a private window".into()], ..Default::default() };
        assert_eq!(hide_browser(&ex(), SAFARI, &[f], true), Some(HideReason::PrivateWindow));
        let c = BrowserWindow { title: Some("x".into()), urls: vec!["https://a.com".into()], control_labels: vec!["Incognito".into()], ..Default::default() };
        assert_eq!(hide_browser(&ex(), CHROME, &[c.clone()], true), Some(HideReason::PrivateWindow));
        let off = Exclusions { private_windows: false, ..ex() };
        assert_eq!(hide_browser(&off, CHROME, &[c], true), None);
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
        assert_eq!(hide_browser(&ex(), CHROME, &[named], true), Some(HideReason::PasswordManager));
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
            hide_browser(&ex(), b, &[BrowserWindow { title: Some(title.into()), urls: vec!["https://a.com".into()], control_labels: vec![label.into()], ..Default::default() }], true)
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
        assert_eq!(hide_browser(&ex(), CHROME, &[w.clone()], true), Some(HideReason::Unreadable));
        let e = Exclusions { private_windows: false, ..ex() };
        assert_eq!(hide_browser(&e, CHROME, &[w], true), None);
    }

    #[test]
    fn more_vaults() {
        for u in ["https://online.roboform.com/", "https://keepersecurity.us/vault/", "https://keepersecurity.com/en_GB/vault", "https://vault.keepersecurity.eu/",
                  "https://lastpass.com/?ac=1", "https://nordpass.com/vault", "https://vault.zoho.com/", "https://vault.enpass.io/",
                  "opera://settings/passwords", "arc://password-manager", "comet://password-manager/", "chrome://settings/passwords", "vivaldi:settings/passwords", "about:preferences#privacy-passwords"] {
            assert_eq!(hid(&ex(), u), Some(HideReason::PasswordManager), "{u}");
        }
    }

    // ---- security review 2026-11-23, second round ----
    #[test]
    fn a_wrapper_cannot_disguise_what_it_wraps() {
        let e = sites(&["chase.com"]);
        // The outer page is the excluded one; ?url= must not swap it for an innocent one.
        assert_eq!(hid(&ex(), "chrome://password-manager/x?url=https%3A%2F%2Fa.com"), Some(HideReason::PasswordManager));
        assert_eq!(hid(&ex(), "chrome-extension://nngceckbapebfimnlniiiahkandclblb/p?url=https%3A%2F%2Fa.com"), Some(HideReason::PasswordManager));
        assert_eq!(hid(&ex(), "moz-extension://u-u/p?url=https%3A%2F%2Fa.com"), Some(HideReason::PasswordManager));
        assert_eq!(hid(&e, "safari-reader://chase.com/?url=https%3A%2F%2Fa.com"), Some(HideReason::Site));
        assert_eq!(hid(&ex(), "about:blank?url=https%3A%2F%2Fa.com"), Some(HideReason::Unreadable));
        assert_eq!(hid(&e, "view-source:view-source:https://chase.com/"), Some(HideReason::Site));
    }

    #[test]
    fn pages_inside_extensions_are_checked() {
        let e = sites(&["chase.com"]);
        for u in ["chrome-extension://cjpalhdlnbpafiamejdnhcphjbkeiagm/view.html#https://chase.com/x", "chrome-extension://cjpalhdlnbpafiamejdnhcphjbkeiagm/https://www.chase.com/x",
                  "chrome-extension://cjpalhdlnbpafiamejdnhcphjbkeiagm/v?page=https%3A%2F%2Fchase.com%2F"] {
            assert_eq!(hid(&e, u), Some(HideReason::Site), "{u}");
        }
    }

    #[test]
    fn unfamiliar_schemes_are_unreadable() {
        for u in ["chrome-distiller://abc/", "devtools://devtools/bundled/x.html", "isolated-app://abc/", "chrome-untrusted://x/", "safari-resource:/x"] {
            assert_eq!(hid(&ex(), u), Some(HideReason::Unreadable), "{u}");
        }
    }

    #[test]
    fn vaults_in_reader_schemes() {
        assert_eq!(hid(&ex(), "safari-reader://vault.bitwarden.com/x"), Some(HideReason::PasswordManager));
    }

    #[test]
    fn rules_are_never_narrowed() {
        assert_eq!(hid(&sites(&["example.com/acc*"]), "https://example.com/accounts/1"), Some(HideReason::Site));
        assert_eq!(hid(&sites(&["*.bücher.de"]), "https://x.xn--bcher-kva.de/"), Some(HideReason::Site));
        assert_eq!(normalize_site("*bü*.de"), None);
    }

    #[test]
    fn extension_names_ignore_spacing() {
        for t in ["Proton Pass", "NordPass", "Nord Pass", "Robo-Form", "Kee Pass XC", "1 Password"] {
            let w = BrowserWindow { title: Some(t.into()), ..win("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/p") };
            assert_eq!(hide_browser(&ex(), CHROME, &[w], true), Some(HideReason::PasswordManager), "{t}");
        }
    }

    #[test]
    fn a_title_alone_does_not_prove_a_window_is_ordinary() {
        let w = BrowserWindow { control_labels: vec![], ..win("https://a.com") };
        assert_eq!(hide_browser(&ex(), CHROME, &[w], true), Some(HideReason::Unreadable));
        let blank = BrowserWindow { title: Some("  ".into()), control_labels: vec![" ".into()], ..win("https://a.com") };
        assert_eq!(hide_browser(&ex(), CHROME, &[blank], true), Some(HideReason::Unreadable));
    }

    #[test]
    fn encoded_addresses_inside_extensions_are_checked() {
        let e = sites(&["chase.com"]);
        let base = "chrome-extension://cjpalhdlnbpafiamejdnhcphjbkeiagm";
        for tail in ["view.html#https%3A%2F%2Fchase.com%2Fx", "https%3A%2F%2Fwww.chase.com/x", "v#page=https%3a%2f%2fchase.com", "v?p=https%253A%252F%252Fchase.com%252F",
                     "v#https%25253A%25252F%25252Fchase.com", "v#HTTPS://CHASE.COM/x", "v#HtTpS%3A%2F%2FCHASE.com", "v?%68ttps://chase.com", "v#x=view-source%3Ahttps%3A%2F%2Fchase.com"] {
            assert_eq!(hid(&e, &format!("{base}/{tail}")), Some(HideReason::Site), "{tail}");
        }
        assert_eq!(hid(&ex(), &format!("{base}/v#https%3A%2F%2Fvault.bitwarden.com")), Some(HideReason::PasswordManager));
        assert_eq!(hid(&e, &format!("{base}/v#https%3A%2F%2Fexample.com")), None);
        assert_eq!(hid(&e, &format!("{base}/100%")), None);
    }

    #[test]
    fn too_many_nested_addresses_to_check_is_unreadable() {
        let many: String = (0..200).map(|i| format!("https://a{i}.com,")).collect();
        let base = "chrome-extension://cjpalhdlnbpafiamejdnhcphjbkeiagm/v#";
        assert!(hid(&sites(&["chase.com"]), &format!("{base}{many}")).is_some());
        assert!(hid(&sites(&["chase.com"]), &format!("{base}{many}https://chase.com/")).is_some());
        // Encoded deeper than anyone honest would.
        let mut deep = "https://chase.com".to_owned();
        for _ in 0..12 {
            deep = deep.replace('%', "%25").replace(':', "%3A").replace('/', "%2F");
        }
        assert!(hid(&sites(&["chase.com"]), &format!("{base}{deep}")).is_some());
    }

    #[test]
    fn too_much_to_check_still_hides_what_was_found() {
        // Even with "leave out unreadable browsers" off.
        let lax = |s: &[&str]| Exclusions { unreadable_browsers: false, ..sites(s) };
        let base = "chrome-extension://cjpalhdlnbpafiamejdnhcphjbkeiagm/v#";
        let many: String = (0..200).map(|i| format!("https://a{i}.com,")).collect();
        let mut deep = "%41".to_owned();
        for _ in 0..12 {
            deep = deep.replace('%', "%25");
        }
        for u in [format!("{base}https://chase.com/,{deep}"), format!("{base}{many}https://chase.com/"), format!("{base}https://chase.com/,{many}")] {
            assert_eq!(hid(&lax(&["chase.com"]), &u), Some(HideReason::Site), "{u}");
        }
        // Megabytes of address must not take the Mac with them.
        let t = std::time::Instant::now();
        assert!(hid(&lax(&["chase.com"]), &format!("{base}{}", "https://a.com,".repeat(200_000))).is_some());
        assert!(t.elapsed().as_secs() < 2);
        // Nothing found, but not everything was seen: hidden while any rule could apply.
        let u = format!("{base}{deep}");
        assert_eq!(hid(&lax(&["chase.com"]), &u), Some(HideReason::Unreadable));
        assert_eq!(hid(&Exclusions { password_managers: true, ..lax(&[]) }, &u), Some(HideReason::Unreadable));
        assert_eq!(hid(&Exclusions { password_managers: false, ..lax(&[]) }, &u), None);
    }

    #[test]
    fn too_much_to_check_counts_as_unreadable() {
        let mut deep = "%41".to_owned();
        for _ in 0..12 {
            deep = deep.replace('%', "%25");
        }
        let u = format!("chrome-extension://cjpalhdlnbpafiamejdnhcphjbkeiagm/v#{deep}");
        assert_eq!(hid(&Exclusions { password_managers: false, ..sites(&[]) }, &u), Some(HideReason::Unreadable));
    }

    #[test]
    fn nested_addresses_are_bounded() {
        let t = std::time::Instant::now();
        let many: String = (0..1200).map(|i| format!("http://{i}.a/")).collect();
        let u = format!("chrome-extension://cjpalhdlnbpafiamejdnhcphjbkeiagm/v#{many}");
        assert!(hid(&sites(&["chase.com"]), &u).is_some());
        assert!(t.elapsed().as_secs() < 3, "{:?}", t.elapsed());
    }

    #[test]
    fn junk_or_length_before_an_address_cannot_hide_it() {
        let lax = Exclusions { unreadable_browsers: false, ..sites(&["chase.com"]) };
        let base = "chrome-extension://cjpalhdlnbpafiamejdnhcphjbkeiagm/v#";
        for tail in [format!("{}https://chase.com/", "https://[,".repeat(600)), format!("https://{}@chase.com/", "u".repeat(3000)), format!("https:{}chase.com/", "/".repeat(3000))] {
            assert!(hid(&lax, &format!("{base}{tail}")).is_some(), "{}", &tail[..20]);
        }
        let deep = Exclusions { sites: vec!["chase.com/*/secret".into()], ..lax };
        assert!(hid(&deep, &format!("{base}https://chase.com/{}/secret", "a".repeat(3000))).is_some());
    }

    #[test]
    fn system_prompts_are_always_hidden() {
        let off = Exclusions { password_managers: false, private_windows: false, unreadable_browsers: false, tor: false, apps: vec![], sites: vec![] };
        for b in ["com.apple.LocalAuthentication.UIAgent", "com.apple.SecurityAgent", "com.apple.UserNotificationCenter", "com.apple.LocalAuthenticationRemoteService"] {
            assert_eq!(hide_app(&off, b), Some(HideReason::SystemPrompt), "{b}");
        }
        assert_eq!(hide_app(&off, "com.apple.finder"), None);
    }

    #[test]
    fn tor_is_a_setting_of_its_own() {
        let tor = |tor, private_windows| hide_browser(&Exclusions { tor, private_windows, ..ex() }, "org.torproject.torbrowser", &[win("https://a.com")], true);
        assert_eq!(tor(true, true), Some(HideReason::App));
        assert_eq!(tor(true, false), Some(HideReason::App));
        assert_eq!(tor(false, true), None);
        assert_eq!(tor(false, false), None);
        // Excluded whole: not read, not woken, not listed among running apps, like any other excluded app.
        assert_eq!(hide_app(&ex(), "org.torproject.TorBrowser"), Some(HideReason::App));
        assert_eq!(hide_app(&Exclusions { tor: false, ..ex() }, "org.torproject.torbrowser"), None);
        // Allowed, but every other rule still applies.
        let e = Exclusions { tor: false, ..sites(&["chase.com"]) };
        assert_eq!(hide_browser(&e, "org.torproject.torbrowser", &[win("https://chase.com")], true), Some(HideReason::Site));
        assert_eq!(hide_browser(&e, "org.torproject.torbrowser", &[], false), Some(HideReason::Unreadable));
        assert_eq!(hide_browser(&ex(), "org.mozilla.firefox", &[win("https://a.com")], true), None);
    }

    #[test]
    fn only_firefox_needs_waking() {
        assert!(needs_waking("org.mozilla.Firefox") && needs_waking("app.zen-browser.zen") && needs_waking("org.torproject.torbrowser"));
        for b in [CHROME, SAFARI, "company.thebrowser.Browser", "com.apple.finder"] {
            assert!(!needs_waking(b), "{b}");
        }
    }

    #[test]
    fn every_tor_build_is_a_browser() {
        for b in ["org.torproject.torbrowser", "org.torproject.TorBrowser.alpha", "org.torproject.something"] {
            assert!(is_browser(b) && needs_waking(b), "{b}");
            assert_eq!(hide_browser(&ex(), b, &[win("https://a.com")], true), Some(HideReason::App), "{b}");
        }
    }

    #[test]
    fn firefoxs_own_furniture_is_not_a_page() {
        for u in ["chrome://browser/content/browser.xhtml", "chrome://browser/content/spotlight.html"] {
            assert_eq!(hide_browser(&Exclusions { private_windows: false, ..ex() }, "org.mozilla.firefox", &[win(u)], true), Some(HideReason::Unreadable), "{u}");
        }
        assert_eq!(hid(&ex(), "chrome://newtab/"), None);
    }

    #[test]
    fn an_app_that_opens_web_pages_is_a_browser_once_it_shows_one() {
        assert!(!is_browser("net.mullvad.mullvadbrowser"));
        register_browsers(vec!["net.Mullvad.MullvadBrowser".into(), "one.ablaze.floorp".into(), "com.example.terminal".into()]);
        assert!(is_browser("net.mullvad.mullvadbrowser") && is_browser("one.ablaze.floorp") && !is_browser("com.apple.finder") && needs_waking("one.ablaze.floorp"));
        let e = sites(&["chase.com"]);
        // A terminal also says it opens links. With no page in it, it's an ordinary app, readable or not.
        let plain = BrowserWindow { urls: vec![], ..win("x") };
        // Not on the first look: its tree may not have filled in yet.
        let nap = || std::thread::sleep(std::time::Duration::from_millis(40));
        assert_eq!(hide_browser(&e, "com.example.terminal", &[plain.clone()], true), Some(HideReason::Unreadable));
        // Looks in the same breath count once.
        for _ in 0..5 {
            assert_eq!(hide_browser(&e, "com.example.terminal", &[plain.clone()], true), Some(HideReason::Unreadable));
        }
        nap();
        assert_eq!(hide_browser(&e, "com.example.terminal", &[plain.clone()], true), Some(HideReason::Unreadable));
        nap();
        assert_eq!(hide_browser(&e, "com.example.terminal", &[plain.clone()], true), None);
        let bare = BrowserWindow { urls: vec![], control_labels: vec![], ..win("x") };
        assert_eq!(hide_browser(&e, "com.example.terminal", &[bare], true), Some(HideReason::Unreadable));
        for _ in 0..3 {
            hide_browser(&e, "com.example.terminal", &[plain.clone()], true);
            nap();
        }
        assert_eq!(hide_browser(&e, "com.example.terminal", &[plain.clone()], true), None);
        // Only a full look that finds no page says so; a failed look says nothing.
        assert_eq!(hide_browser(&e, "com.example.terminal", &[plain.clone()], false), Some(HideReason::Unreadable));
        assert_eq!(hide_browser(&e, "com.example.terminal", &[], false), Some(HideReason::Unreadable));
        // A private-window marker makes it a browser on the spot.
        // Registering again adds; nothing registered before is forgotten.
        register_browsers(vec!["com.example.quiet".into()]);
        assert!(is_browser("one.ablaze.floorp"));
        let marked = BrowserWindow { title: Some("x — Private Browsing".into()), urls: vec![], ..win("x") };
        assert_eq!(hide_browser(&ex(), "com.example.quiet", &[marked], true), Some(HideReason::PrivateWindow));
        // One page seen, and from then on it is held to a browser's rules.
        assert_eq!(hide_browser(&e, "net.mullvad.mullvadbrowser", &[win("https://chase.com/")], true), Some(HideReason::Site));
        assert_eq!(hide_browser(&e, "net.mullvad.mullvadbrowser", &[], false), Some(HideReason::Unreadable));
        assert_eq!(hide_browser(&e, "net.mullvad.mullvadbrowser", &[plain], true), Some(HideReason::Unreadable));
        let private = BrowserWindow { title: Some("Chase — Private Browsing".into()), ..win("https://a.com") };
        assert_eq!(hide_browser(&ex(), "one.ablaze.floorp", &[private], true), Some(HideReason::PrivateWindow));
    }
}
