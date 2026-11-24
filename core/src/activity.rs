//! What was in front, and what else was open, as stretches of time. No pictures: only names and titles.
//!
//! AIDEV-NOTE: privacy-critical. Held to the same rules as capture. An excluded app, a password manager or a system
//! AIDEV-NOTE: prompt is recorded only as private time, with no name. A browser's title is recorded only for a window
//! AIDEV-NOTE: that passed the capture check (the caller says so with `title_ok`); otherwise just "it was the browser".
//! AIDEV-NOTE: A title is kept only as a saved picture of that window left it (rules and model), never as sampled.

use crate::privacy::{Exclusions, hide_app, shows_pages as is_browser};

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
    /// The capture check just passed this very window with this title: a browser's title may be kept.
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

/// Names a window without keeping its title: the app, and the title's letters. Spinners, counters and percentages
/// change without the window changing.
pub fn key(bundle_id: &str, title: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let steady = title.chars().filter(|c| c.is_alphabetic() || *c == ' ').collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (bundle_id.to_lowercase(), steady).hash(&mut h);
    h.finish().max(1)
}

/// What may be written down about a window. None: nothing at all, not even that it was open.
/// AIDEV-NOTE: a title is never taken from the sample. `cleared` gives the text a saved picture of that window ended
/// AIDEV-NOTE: up with, after the rules and the model; until there is one the title is blank.
fn allowed(ex: &Exclusions, w: &OpenWindow, title_ok: bool, cleared: &dyn Fn(u64) -> Option<String>) -> Option<OpenWindow> {
    if w.bundle_id.is_empty() && w.app.is_empty() || hide_app(ex, &w.bundle_id).is_some() {
        return None;
    }
    // A browser's title names a page, and pages are judged by address, which only the capture check reads.
    let shown = if is_browser(&w.bundle_id) && !title_ok { None } else { cleared(key(&w.bundle_id, &w.title)) };
    Some(OpenWindow { app: w.app.chars().take(80).collect(), bundle_id: w.bundle_id.clone(), title: shown.unwrap_or_default().chars().take(MAX_TITLE).collect() })
}

/// The stretch this sample belongs to, with everything private taken out, and the key of its window (0: private time).
/// `id`, `start` and `end` are the caller's.
pub fn describe(ex: &Exclusions, s: &ActivitySample, cleared: &dyn Fn(u64) -> Option<String>) -> (ActivitySpan, u64) {
    let mut others: Vec<OpenWindow> = s.others.iter().filter_map(|w| allowed(ex, w, false, cleared)).collect();
    others.dedup();
    others.truncate(MAX_OTHERS);
    match allowed(ex, &s.front, s.title_ok, cleared) {
        Some(f) => (ActivitySpan { app: f.app, bundle_id: f.bundle_id, title: f.title, others, ..Default::default() }, key(&s.front.bundle_id, &s.front.title)),
        // Something excluded was in front. What else was open is nobody's clue to what it was.
        None => (ActivitySpan { private_time: true, ..Default::default() }, 0),
    }
}

/// A title that turns up late may be written onto the stretch already going. Not for a browser: the same title can
/// be showing in a private window first and an ordinary one after.
pub fn may_retitle(bundle_id: &str) -> bool {
    !is_browser(bundle_id)
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
        ActivitySample { at: 1, front, title_ok: ok, others }
    }
    /// As if every window had been photographed and its title came through unchanged but for one word.
    fn all(_: u64) -> Option<String> {
        Some("checked".into())
    }
    fn none(_: u64) -> Option<String> {
        None
    }

    #[test]
    fn a_title_is_blank_until_a_picture_of_that_window_was_checked() {
        let xcode = || sample(w("Xcode", "com.apple.dt.Xcode", "mysql -uroot -pTr0ub4dor9x shop"), false, vec![w("Notes", "com.apple.Notes", "PIN 4471")]);
        let (s, k) = describe(&ex(), &xcode(), &none);
        assert_eq!((s.app.as_str(), s.title.as_str(), s.private_time, s.others[0].title.as_str()), ("Xcode", "", false, ""));
        assert_eq!(k, key("com.apple.dt.xcode", "mysql -uroot -pTr0ub4dor9x shop"));
        // What is written is what the picture's check left, never the title as sampled.
        let only = |k: u64| (k == key("com.apple.dt.Xcode", "mysql -uroot -pTr0ub4dor9x shop")).then(|| "mysql [hidden] shop".to_owned());
        let (s, _) = describe(&ex(), &xcode(), &only);
        assert_eq!((s.title.as_str(), s.others[0].title.as_str()), ("mysql [hidden] shop", ""));
        let long = |_: u64| Some("a ".repeat(900));
        assert!(describe(&ex(), &xcode(), &long).0.title.chars().count() <= 300);
    }

    #[test]
    fn excluded_apps_are_only_private_time() {
        for b in ["com.acme.ledger", "com.bitwarden.desktop", "com.apple.SecurityAgent", "org.torproject.torbrowser"] {
            let s = describe(&ex(), &sample(w("Secret App", b, "Vault — dana"), true, vec![w("Xcode", "com.apple.dt.Xcode", "x")]), &all);
            assert_eq!(s, (ActivitySpan { private_time: true, ..Default::default() }, 0), "{b}");
        }
        // And they don't appear among the other windows of an ordinary stretch.
        let (s, _) = describe(&ex(), &sample(w("Xcode", "com.apple.dt.Xcode", "x"), false, vec![w("Bitwarden", "com.bitwarden.desktop", "Vault"), w("Notes", "com.apple.Notes", "House")]), &all);
        assert_eq!(s.others, vec![w("Notes", "com.apple.Notes", "checked")]);
    }

    #[test]
    fn a_browsers_title_also_needs_the_capture_check_to_vouch_for_it() {
        let chrome = || w("Google Chrome", "com.google.Chrome", "Chase — Accounts");
        let (no, _) = describe(&ex(), &sample(chrome(), false, vec![]), &all);
        assert_eq!((no.app.as_str(), no.title.as_str(), no.url.as_str(), no.private_time), ("Google Chrome", "", "", false));
        assert_eq!(describe(&ex(), &sample(chrome(), true, vec![]), &all).0.title, "checked");
        assert_eq!(describe(&ex(), &sample(chrome(), true, vec![]), &none).0.title, "");
        // In the background a browser window is never vouched for.
        let (s, _) = describe(&ex(), &sample(w("Xcode", "com.apple.dt.Xcode", "x"), true, vec![chrome()]), &all);
        assert_eq!(s.others, vec![w("Google Chrome", "com.google.Chrome", "")]);
        assert!(!may_retitle("com.google.Chrome") && may_retitle("com.apple.dt.Xcode"));
    }

    #[test]
    fn a_spinner_or_a_counter_is_not_a_different_window() {
        // Seen live: a terminal's title animates.
        assert_eq!(key("t", "◐ Building 41%"), key("T", "◑ Building 42%"));
        assert_eq!(key("t", "(3) Inbox"), key("t", "(4) Inbox"));
        assert!(key("t", "Inbox") != key("t", "Sent") && key("t", "Inbox") != key("u", "Inbox") && key("", "") != 0);
    }

    #[test]
    fn an_app_that_only_registers_for_links_is_not_a_browser() {
        crate::privacy::register_browsers(vec!["com.example.term".into()]);
        assert_eq!(describe(&ex(), &sample(w("Term", "com.example.term", "deploy — zsh"), false, vec![]), &all).0.title, "checked");
    }
}
