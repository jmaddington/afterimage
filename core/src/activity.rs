//! What was in front, and what else was open, as stretches of time. No pictures: only names and titles.
//!
//! AIDEV-NOTE: privacy-critical. Held to the same rules as capture. An excluded app, a password manager or a system
//! AIDEV-NOTE: prompt is recorded only as private time, with no name. A browser's title is recorded only for a window
//! AIDEV-NOTE: that passed the capture check (the caller says so with `title_ok`); otherwise just "it was the browser".
//! AIDEV-NOTE: Every title goes through the secret rules before it is kept.

use crate::detect;
use crate::privacy::{Exclusions, hide_app, is_browser};

/// A window that was open, as the screen reported it.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct OpenWindow {
    pub app: String,
    pub bundle_id: String,
    pub title: String,
}

/// What was seen at one moment.
#[derive(Debug, Clone, Default, uniffi::Record)]
pub struct ActivitySample {
    /// Milliseconds since 1970.
    pub at: i64,
    pub front: OpenWindow,
    /// Its page address, if it is a browser and one was read.
    pub url: String,
    /// The front window passed the capture check with this very title: a browser's title may be kept.
    pub title_ok: bool,
    /// Every other window on screen.
    pub others: Vec<OpenWindow>,
}

/// One stretch during which the same window was in front.
#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
pub struct ActivitySpan {
    pub id: i64,
    pub start: i64,
    pub end: i64,
    /// Empty for private time.
    pub app: String,
    pub bundle_id: String,
    pub title: String,
    pub url: String,
    pub private_time: bool,
    pub others: Vec<OpenWindow>,
}

const MAX_TITLE: usize = 300;
const MAX_OTHERS: usize = 40;

fn title(t: &str) -> String {
    detect::mask_text(&t.chars().take(MAX_TITLE).collect::<String>())
}

/// What may be written down about a window. None: nothing at all, not even that it was open.
fn allowed(ex: &Exclusions, w: &OpenWindow, title_ok: bool) -> Option<OpenWindow> {
    if w.bundle_id.is_empty() && w.app.is_empty() || hide_app(ex, &w.bundle_id).is_some() {
        return None;
    }
    // A browser's title names a page, and pages are judged by address, which only the capture check reads.
    let shown = if is_browser(&w.bundle_id) && !title_ok { String::new() } else { title(&w.title) };
    Some(OpenWindow { app: w.app.chars().take(80).collect(), bundle_id: w.bundle_id.clone(), title: shown })
}

/// The stretch this sample belongs to, with everything private taken out. `id`, `start` and `end` are the caller's.
pub fn describe(ex: &Exclusions, s: &ActivitySample) -> ActivitySpan {
    let mut others: Vec<OpenWindow> = s.others.iter().filter_map(|w| allowed(ex, w, false)).collect();
    others.dedup();
    others.truncate(MAX_OTHERS);
    match allowed(ex, &s.front, s.title_ok) {
        Some(f) => {
            let url = if s.title_ok && is_browser(&s.front.bundle_id) { detect::scrub_url(&s.url) } else { String::new() };
            ActivitySpan { app: f.app, bundle_id: f.bundle_id, title: f.title, url, others, ..Default::default() }
        }
        // Something excluded was in front. What else was open is nobody's clue to what it was.
        None => ActivitySpan { private_time: true, ..Default::default() },
    }
}

/// Two samples of the same stretch? The other windows may come and go without starting a new one.
pub fn same(a: &ActivitySpan, b: &ActivitySpan) -> bool {
    a.private_time == b.private_time && a.bundle_id == b.bundle_id && a.title == b.title && a.url == b.url
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ex() -> Exclusions {
        Exclusions { apps: vec!["com.acme.Ledger".into()], password_managers: true, private_windows: true, unreadable_browsers: true, tor: true, sites: vec![] }
    }
    fn w(app: &str, bundle: &str, title: &str) -> OpenWindow {
        OpenWindow { app: app.into(), bundle_id: bundle.into(), title: title.into() }
    }
    fn sample(front: OpenWindow, ok: bool, others: Vec<OpenWindow>) -> ActivitySample {
        ActivitySample { at: 1, front, url: "https://x.com/a?token=abc".into(), title_ok: ok, others }
    }

    #[test]
    fn ordinary_apps_are_recorded_with_masked_titles() {
        let s = describe(&ex(), &sample(w("Xcode", "com.apple.dt.Xcode", "afterimage — Capturer.swift"), false, vec![]));
        assert_eq!((s.app.as_str(), s.title.as_str(), s.private_time, s.url.as_str()), ("Xcode", "afterimage — Capturer.swift", false, ""));
        let s = describe(&ex(), &sample(w("Terminal", "com.apple.Terminal", "mysql -uroot -pTr0ub4dor9x shop"), false, vec![]));
        assert!(!s.title.contains("Tr0ub4") && s.title.contains("mysql"), "{}", s.title);
        let long = describe(&ex(), &sample(w("Notes", "com.apple.Notes", &"a ".repeat(900)), false, vec![]));
        assert!(long.title.chars().count() <= 300);
    }

    #[test]
    fn excluded_apps_are_only_private_time() {
        for b in ["com.acme.ledger", "com.bitwarden.desktop", "com.apple.SecurityAgent", "org.torproject.torbrowser"] {
            let s = describe(&ex(), &sample(w("Secret App", b, "Vault — dana"), true, vec![w("Xcode", "com.apple.dt.Xcode", "x")]));
            assert_eq!(s, ActivitySpan { private_time: true, ..Default::default() }, "{b}");
        }
        // And they don't appear among the other windows of an ordinary stretch.
        let s = describe(&ex(), &sample(w("Xcode", "com.apple.dt.Xcode", "x"), false, vec![w("Bitwarden", "com.bitwarden.desktop", "Vault"), w("Notes", "com.apple.Notes", "House")]));
        assert_eq!(s.others, vec![w("Notes", "com.apple.Notes", "House")]);
    }

    #[test]
    fn a_browsers_title_is_kept_only_when_the_capture_check_passed_it() {
        let chrome = || w("Google Chrome", "com.google.Chrome", "Chase — Accounts");
        let no = describe(&ex(), &sample(chrome(), false, vec![]));
        assert_eq!((no.app.as_str(), no.title.as_str(), no.url.as_str(), no.private_time), ("Google Chrome", "", "", false));
        let yes = describe(&ex(), &sample(chrome(), true, vec![]));
        assert_eq!((yes.title.as_str(), yes.url.as_str()), ("Chase — Accounts", "https://x.com/a"));
        // In the background a browser window is never vouched for.
        let s = describe(&ex(), &sample(w("Xcode", "com.apple.dt.Xcode", "x"), true, vec![chrome()]));
        assert_eq!(s.others, vec![w("Google Chrome", "com.google.Chrome", "")]);
    }

    #[test]
    fn the_same_window_is_one_stretch_whatever_else_opens() {
        let a = describe(&ex(), &sample(w("Xcode", "b", "x"), false, vec![]));
        let b = describe(&ex(), &sample(w("Xcode", "b", "x"), false, vec![w("Notes", "n", "y")]));
        let c = describe(&ex(), &sample(w("Xcode", "b", "z"), false, vec![]));
        let p = describe(&ex(), &sample(w("x", "com.bitwarden.desktop", "x"), false, vec![]));
        assert!(same(&a, &b) && !same(&a, &c) && !same(&a, &p) && same(&p, &p));
    }
}
