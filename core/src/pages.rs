//! What the browser extension says about open pages.
//!
//! AIDEV-NOTE: privacy-critical. Everything here is untrusted and may only ADD hiding. A missing, stale or odd report
//! AIDEV-NOTE: must never make a page look safer than the address and accessibility checks already found it.

use crate::detect::{Rect, SecretKind};
use crate::privacy::{Exclusions, HideReason};
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PageField {
    pub kind: SecretKind,
    /// Fractions of the visible page, top-left origin.
    pub rect: Rect,
}

/// A tab with something to hide.
#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
pub struct TabSeen {
    /// What the page and the browser each call its address (usually the same).
    pub urls: Vec<String>,
    /// The browser says it isn't the one showing.
    pub background: bool,
    /// Something secret is showing or being typed and the extension can't say where.
    pub sensitive: bool,
    /// Size of the visible page in device pixels.
    pub width: f64,
    pub height: f64,
    pub fields: Vec<PageField>,
}

#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
pub struct PageHint {
    pub private_window: bool,
    /// Something was said that couldn't be understood.
    pub sensitive: bool,
    pub tabs: Vec<TabSeen>,
}

/// A page as accessibility sees it on screen, in points.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PageArea {
    pub url: String,
    pub rect: Rect,
    /// Device pixels per point there.
    pub scale: f64,
}

/// A place to frost whatever is in it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Cover {
    pub kind: SecretKind,
    pub rect: Rect,
}

#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
pub struct PagePlan {
    /// Leave the whole browser out.
    pub hide: Option<HideReason>,
    /// Otherwise frost these, in screen points.
    pub covers: Vec<Cover>,
}

/// The address as `extension/detect.js` writes it: origin and path, or just the scheme of anything that isn't the web.
pub fn clean_address(raw: &str) -> String {
    let Ok(u) = url::Url::parse(raw) else { return String::new() };
    if !matches!(u.scheme(), "http" | "https") {
        return format!("{}:", u.scheme());
    }
    format!("{}{}", u.origin().ascii_serialization(), u.path()).chars().take(MAX_URL).collect()
}

/// What to do about a browser, given what its extension says and what accessibility sees. Only ever adds hiding.
/// AIDEV-NOTE: privacy-critical. A field is frosted in place only when exactly one visible page has its address AND that
/// AIDEV-NOTE: page's on-screen size equals the size the extension measured. Any doubt leaves the browser out.
#[uniffi::export]
pub fn plan_page(ex: &Exclusions, hint: &PageHint, areas: &[PageArea], complete: bool) -> PagePlan {
    let leave_out = |r| PagePlan { hide: Some(r), covers: vec![] };
    if hint.private_window && ex.private_windows {
        return leave_out(HideReason::PrivateWindow);
    }
    if hint.sensitive {
        return leave_out(HideReason::SecretField);
    }
    let mut covers = Vec::new();
    for tab in &hint.tabs {
        let named: Vec<&String> = tab.urls.iter().filter(|u| u.contains("://")).collect();
        let showing: Vec<&PageArea> = areas.iter().filter(|a| named.iter().any(|u| **u == clean_address(&a.url))).collect();
        // Out of sight: the browser says so, its address is known, and everything on screen was read and isn't it.
        if tab.background && complete && !named.is_empty() && named.len() == tab.urls.len() && showing.is_empty() {
            continue;
        }
        let [area] = showing.as_slice() else { return leave_out(HideReason::SecretField) };
        let same = |points: f64, pixels: f64| (points * area.scale - pixels).abs() <= 3.0;
        let r = area.rect;
        // NaN fails every comparison, so it lands here too.
        let trusted = complete && !tab.sensitive && tab.urls.len() == 1 && pixels_ok(tab) && same(r.w, tab.width) && same(r.h, tab.height) && r.x.is_finite() && r.y.is_finite();
        if !trusted {
            return leave_out(HideReason::SecretField);
        }
        covers.extend(tab.fields.iter().map(|f| Cover {
            kind: f.kind,
            rect: Rect { x: r.x + f.rect.x * r.w, y: r.y + f.rect.y * r.h, w: f.rect.w * r.w, h: f.rect.h * r.h },
        }));
    }
    PagePlan { hide: None, covers }
}

fn pixels_ok(tab: &TabSeen) -> bool {
    tab.width >= 1.0 && tab.height >= 1.0
}

/// Reports repeat every few seconds while they matter; anything older is forgotten.
const FRESH: Duration = Duration::from_secs(12);
const MAX_MESSAGE: usize = 65_536;
const MAX_FIELDS: usize = 200;
const MAX_URL: usize = 2048;
const MAX_FRAMES: usize = 400;
const MAX_TABS: usize = 1000;
const MAX_BROWSERS: usize = 8;
const MAX_STORED: usize = 2000;

struct Frame {
    at: Instant,
    url: String,
    width: f64,
    height: f64,
    fields: Vec<PageField>,
    sensitive: bool,
}

struct Tab {
    active: bool,
    private: bool,
    url: String,
}

#[derive(Default)]
struct Browser {
    frames: HashMap<(i64, i64), Frame>,
    tabs: HashMap<i64, Tab>,
    tabs_at: Option<Instant>,
    /// Something was said that couldn't be understood or remembered.
    muddled: Option<Instant>,
}

/// What each browser (by process id, then connection) has said lately.
#[derive(Default)]
pub struct Pages {
    browsers: HashMap<(i32, u64), Browser>,
    /// More browsers than could be believed: everyone is suspect until it passes.
    crowded: Option<Instant>,
}

enum Message {
    Page { tab: i64, frame: i64, said: Frame },
    Tabs(HashMap<i64, Tab>),
}

fn fresh(at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(at) < FRESH
}

fn field(v: &Value) -> Option<PageField> {
    let n = |k: &str| v.get(k)?.as_f64().filter(|x| x.is_finite());
    let (x, y, w, h) = (n("x")?, n("y")?, n("w")?, n("h")?);
    let (l, t, r, b) = (x.max(0.0), y.max(0.0), (x + w).min(1.0), (y + h).min(1.0));
    if w <= 0.0 || h <= 0.0 || r <= l || b <= t {
        return None;
    }
    let kind = match v.get("kind")?.as_str()? {
        "card" | "account" => SecretKind::Financial,
        k => SecretKind::parse(k),
    };
    Some(PageField { kind, rect: Rect { x: l, y: t, w: r - l, h: b - t } })
}

fn text(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().chars().take(MAX_URL).collect()
}

fn parse(json: &str, now: Instant) -> Option<Message> {
    if json.len() > MAX_MESSAGE {
        return None;
    }
    let v: Value = serde_json::from_str(json).ok()?;
    if v.get("v")?.as_i64()? != 1 {
        return None;
    }
    match v.get("type")?.as_str()? {
        "page" => {
            let (tab, frame) = (v.get("tab")?.as_i64()?, v.get("frame")?.as_i64()?);
            let raw = v.get("fields")?.as_array()?;
            let fields: Vec<PageField> = raw.iter().take(MAX_FIELDS).filter_map(field).collect();
            // A field that can't be placed is still a field; only the top page knows where it is.
            let sensitive = v.get("typing")?.as_bool()? | v.get("unplaced")?.as_bool()? | (fields.len() < raw.len()) | (frame != 0 && !raw.is_empty());
            let size = |k: &str| v.get(k).and_then(Value::as_f64).filter(|n| n.is_finite()).unwrap_or(0.0);
            Some(Message::Page { tab, frame, said: Frame { at: now, url: text(&v, "url"), width: size("width"), height: size("height"), fields, sensitive } })
        }
        "tabs" => {
            let raw = v.get("tabs")?.as_array()?;
            if raw.len() > MAX_TABS {
                return None;
            }
            let tab = |t: &Value| Some((t.get("tab")?.as_i64()?, Tab { active: t.get("active")?.as_bool()?, private: t.get("private")?.as_bool()?, url: text(t, "url") }));
            Some(Message::Tabs(raw.iter().map(tab).collect::<Option<_>>()?))
        }
        _ => None,
    }
}

impl Pages {
    pub fn receive(&mut self, pid: i32, source: u64, json: &str, now: Instant) {
        let browser = (pid, source);
        self.browsers.retain(|_, b| b.frames.values().map(|f| f.at).chain(b.tabs_at).chain(b.muddled).any(|at| fresh(at, now)));
        if !self.browsers.contains_key(&browser) && self.browsers.len() >= MAX_BROWSERS {
            self.crowded = Some(now);
            return;
        }
        let total: usize = self.len();
        let b = self.browsers.entry(browser).or_default();
        b.frames.retain(|_, f| fresh(f.at, now));
        match parse(json, now) {
            Some(Message::Page { tab, frame, said }) => {
                let key = (tab, frame);
                let grown = said.fields.len().max(1);
                if !b.frames.contains_key(&key) && b.frames.len() >= MAX_FRAMES || total + grown > MAX_STORED {
                    b.frames.remove(&key);
                    b.muddled = Some(now);
                } else {
                    b.frames.insert(key, said);
                }
            }
            Some(Message::Tabs(tabs)) => {
                b.tabs = tabs;
                b.tabs_at = Some(now);
            }
            None => b.muddled = Some(now),
        }
    }

    pub fn hint(&self, pid: i32, now: Instant) -> PageHint {
        let mut h = PageHint { sensitive: self.crowded.is_some_and(|at| fresh(at, now)), ..Default::default() };
        for b in self.browsers.iter().filter(|((p, _), _)| *p == pid).map(|x| x.1) {
            let tabs = b.tabs_at.filter(|at| fresh(*at, now)).map(|_| &b.tabs);
            h.private_window |= tabs.is_some_and(|t| t.values().any(|t| t.active && t.private));
            h.sensitive |= b.muddled.is_some_and(|at| fresh(at, now));
            let mut seen: HashMap<i64, TabSeen> = HashMap::new();
            for ((tab, frame), f) in b.frames.iter().filter(|(_, f)| fresh(f.at, now)) {
                if !f.sensitive && f.fields.is_empty() {
                    continue;
                }
                let s = seen.entry(*tab).or_default();
                s.sensitive |= f.sensitive;
                if *frame == 0 {
                    (s.width, s.height) = (f.width, f.height);
                    s.fields = f.fields.clone();
                }
            }
            for (id, mut s) in seen {
                let fact = tabs.and_then(|t| t.get(&id));
                s.background = fact.is_some_and(|t| !t.active);
                // Every name this tab goes by, so one out-of-date name can't make it look gone.
                let page = b.frames.get(&(id, 0)).filter(|f| fresh(f.at, now)).map(|f| f.url.clone());
                s.urls = page.into_iter().chain(fact.map(|t| t.url.clone())).collect();
                s.urls.dedup();
                h.tabs.push(s);
            }
        }
        h
    }

    pub fn len(&self) -> usize {
        self.browsers.values().flat_map(|b| b.frames.values()).map(|f| f.fields.len().max(1)).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(tab: i64, body: &str) -> String {
        format!(r#"{{"v":1,"type":"page","tab":{tab},"window":1,"frame":0,"url":"https://a.com/x","width":2000,"height":1000,{body}}}"#)
    }
    const FIELD: &str = r#""fields":[{"kind":"password","x":0.1,"y":0.2,"w":0.3,"h":0.05}],"typing":false,"unplaced":false"#;
    const CLEAN: &str = r#""fields":[],"typing":false,"unplaced":false"#;
    fn fields(p: &Pages, pid: i32, t: Instant) -> usize {
        p.hint(pid, t).tabs.iter().map(|x| x.fields.len()).sum()
    }
    fn at(secs: u64) -> Duration {
        Duration::from_secs(secs)
    }

    #[test]
    fn fields_are_reported_for_their_browser_only() {
        let (mut p, t) = (Pages::default(), Instant::now());
        p.receive(7, 1, &page(3, FIELD), t);
        let h = p.hint(7, t);
        let f = &h.tabs[0].fields[0];
        assert_eq!((h.tabs.len(), h.tabs[0].fields.len(), f.kind), (1, 1, SecretKind::Password));
        assert_eq!((h.tabs[0].urls.as_slice(), h.tabs[0].background), (["https://a.com/x".to_owned()].as_slice(), false));
        assert_eq!((h.tabs[0].width, h.tabs[0].height), (2000.0, 1000.0));
        for (a, b) in [(f.rect.x, 0.1), (f.rect.y, 0.2), (f.rect.w, 0.3), (f.rect.h, 0.05)] {
            assert!((a - b).abs() < 1e-9);
        }
        assert!(!h.sensitive && !h.private_window && !h.tabs[0].sensitive);
        assert_eq!(p.hint(8, t), PageHint::default());
    }

    #[test]
    fn the_latest_report_of_a_frame_replaces_the_last() {
        let (mut p, t) = (Pages::default(), Instant::now());
        p.receive(7, 1, &page(3, FIELD), t);
        p.receive(7, 1, &page(3, CLEAN), t);
        assert_eq!(p.hint(7, t), PageHint::default());
        p.receive(7, 1, &page(3, FIELD), t);
        p.receive(7, 1, &page(3, CLEAN).replace("\"frame\":0", "\"frame\":9"), t);
        assert_eq!(fields(&p, 7, t), 1);
    }

    #[test]
    fn typing_and_unplaced_are_sensitive() {
        for b in [CLEAN.replace("\"typing\":false", "\"typing\":true"), CLEAN.replace("\"unplaced\":false", "\"unplaced\":true")] {
            let (mut p, t) = (Pages::default(), Instant::now());
            p.receive(7, 1, &page(3, &b), t);
            assert!(p.hint(7, t).tabs[0].sensitive, "{b}");
        }
    }

    #[test]
    fn reports_go_stale() {
        let (mut p, t) = (Pages::default(), Instant::now());
        p.receive(7, 1, &page(3, FIELD), t);
        assert_eq!(fields(&p, 7, t + at(10)), 1);
        assert_eq!(p.hint(7, t + at(13)), PageHint::default());
    }

    /// In a split view a tab can be on screen without being "active". The page itself says when it's hidden.
    #[test]
    fn a_tab_called_inactive_still_counts() {
        let (mut p, t) = (Pages::default(), Instant::now());
        p.receive(7, 1, &page(3, FIELD), t);
        p.receive(7, 1, r#"{"v":1,"type":"tabs","tabs":[{"tab":3,"window":1,"active":false,"private":false}]}"#, t);
        assert_eq!(fields(&p, 7, t), 1);
        p.receive(7, 1, &page(3, CLEAN), t);
        assert_eq!(p.hint(7, t), PageHint::default());
    }

    #[test]
    fn private_windows_are_flagged_while_showing() {
        let tabs = |active, private| format!(r#"{{"v":1,"type":"tabs","tabs":[{{"tab":3,"window":1,"active":{active},"private":{private}}}]}}"#);
        let (mut p, t) = (Pages::default(), Instant::now());
        p.receive(7, 1, &tabs(true, true), t);
        assert!(p.hint(7, t).private_window);
        p.receive(7, 1, &tabs(false, true), t);
        assert!(!p.hint(7, t).private_window);
        p.receive(7, 1, &tabs(true, false), t);
        assert!(!p.hint(7, t).private_window);
    }

    #[test]
    fn nonsense_is_sensitive_for_a_while_not_ignored() {
        let long = page(3, &format!(r#""fields":[],"typing":false,"unplaced":false,"x":"{}""#, "a".repeat(70_000)));
        for bad in ["", "{", "[]", "null", r#"{"v":2,"type":"page"}"#, r#"{"v":1,"type":"what"}"#, r#"{"v":1,"type":"page"}"#, &page(3, r#""fields":7,"typing":false,"unplaced":false"#),
                    &page(3, r#""fields":[],"typing":"no","unplaced":false"#), r#"{"v":1,"type":"tabs","tabs":[{"tab":3}]}"#, &long] {
            let (mut p, t) = (Pages::default(), Instant::now());
            p.receive(7, 1, bad, t);
            assert!(p.hint(7, t).sensitive, "{}", &bad[..bad.len().min(60)]);
            assert!(!p.hint(7, t + at(13)).sensitive);
            assert!(!p.hint(8, t).sensitive);
        }
    }

    #[test]
    fn odd_fields_are_clipped_or_counted_as_unplaced() {
        let f = |x: &str| page(3, &format!(r#""fields":[{x}],"typing":false,"unplaced":false"#));
        let hint = |x: &str| { let (mut p, t) = (Pages::default(), Instant::now()); p.receive(7, 1, &f(x), t); p.hint(7, t).tabs.remove(0) };
        let h = hint(r#"{"kind":"card","x":-0.5,"y":0.9,"w":1.0,"h":0.5}"#);
        assert_eq!(h.fields[0].rect, Rect { x: 0.0, y: 0.9, w: 0.5, h: 0.09999999999999998 });
        assert_eq!(h.fields[0].kind, SecretKind::Financial);
        assert_eq!(hint(r#"{"kind":"overlay","x":0,"y":0,"w":1,"h":1}"#).fields[0].kind, SecretKind::Other);
        for bad in [r#"{"kind":"password","x":"a","y":0,"w":1,"h":1}"#, r#"{"kind":"password"}"#, "5", r#"{"kind":"password","x":0,"y":0,"w":-1,"h":1}"#, r#"{"kind":"password","x":2,"y":0,"w":1,"h":1}"#] {
            assert!(hint(bad).sensitive, "{bad}");
        }
        let many = vec![r#"{"kind":"password","x":0,"y":0,"w":0.1,"h":0.1}"#; 300].join(",");
        let h = hint(&many);
        assert!(h.sensitive && h.fields.len() <= 200);
    }

    #[test]
    fn memory_is_bounded() {
        let (mut p, t) = (Pages::default(), Instant::now());
        for i in 0..5000 {
            p.receive(7, 1, &page(i, FIELD), t);
            p.receive(i as i32 + 100, 1, &page(1, FIELD), t);
        }
        assert!(p.len() <= 2000, "{}", p.len());
        // Too much to remember is not the same as nothing to hide.
        assert!(p.hint(7, t).sensitive);
        // And it recovers.
        p.receive(7, 1, &page(1, FIELD), t + at(30));
        assert!(!p.hint(7, t + at(30)).sensitive);
    }

    /// Two Chrome profiles share one process but speak through two bridges.
    #[test]
    fn profiles_of_one_browser_do_not_overwrite_each_other() {
        let tabs = |tab, private| format!(r#"{{"v":1,"type":"tabs","tabs":[{{"tab":{tab},"window":1,"active":true,"private":{private}}}]}}"#);
        let (mut p, t) = (Pages::default(), Instant::now());
        p.receive(7, 1, &tabs(3, true), t);
        p.receive(7, 1, &page(3, FIELD), t);
        p.receive(7, 2, &tabs(4, false), t);
        assert!(p.hint(7, t).private_window);
        assert_eq!(fields(&p, 7, t), 1);
        // Hanging up proves nothing about what's on screen; only time does.
        assert!(p.hint(7, t + at(11)).private_window);
        assert_eq!(p.hint(7, t + at(13)), PageHint::default());
    }

    // ---- deciding what to do about it
    use crate::privacy::{Exclusions, HideReason};
    fn ex() -> Exclusions {
        Exclusions { private_windows: true, ..Default::default() }
    }
    fn area(url: &str) -> PageArea {
        // 1000x500 points at 2x: the 2000x1000 pixels the page reports.
        PageArea { url: url.into(), rect: Rect { x: 100.0, y: 80.0, w: 1000.0, h: 500.0 }, scale: 2.0 }
    }
    fn hint_of(msgs: &[String]) -> PageHint {
        let (mut p, t) = (Pages::default(), Instant::now());
        for m in msgs {
            p.receive(7, 1, m, t);
        }
        p.hint(7, t)
    }
    fn tabs(active: bool, url: &str) -> String {
        format!(r#"{{"v":1,"type":"tabs","tabs":[{{"tab":3,"window":1,"active":{active},"private":false,"url":"{url}"}}]}}"#)
    }
    const URL: &str = "https://a.com/x";

    #[test]
    fn a_field_is_placed_on_the_screen() {
        let p = plan_page(&ex(), &hint_of(&[page(3, FIELD)]), &[area("https://a.com/x?q=1#f"), area("https://b.com/")], true);
        assert_eq!(p.hide, None);
        let r = p.covers[0].rect;
        assert_eq!((p.covers.len(), p.covers[0].kind), (1, SecretKind::Password));
        for (a, b) in [(r.x, 200.0), (r.y, 180.0), (r.w, 300.0), (r.h, 25.0)] {
            assert!((a - b).abs() < 1e-6, "{r:?}");
        }
    }

    #[test]
    fn nothing_reported_is_nothing_added() {
        assert_eq!(plan_page(&ex(), &PageHint::default(), &[area(URL)], true), PagePlan::default());
        assert_eq!(plan_page(&ex(), &hint_of(&[page(3, CLEAN)]), &[area(URL)], true), PagePlan::default());
    }

    #[test]
    fn a_field_that_cannot_be_placed_exactly_hides_the_browser() {
        let h = hint_of(&[page(3, FIELD)]);
        let resized = |f: fn(&mut PageArea)| { let mut a = area(URL); f(&mut a); vec![a] };
        for (areas, complete) in [
            (vec![], true), (vec![area("https://b.com/")], true), (vec![area(URL), area(URL)], true), (vec![area(URL)], false),
            (resized(|a| a.rect.w += 8.0), true), (resized(|a| a.rect.h -= 8.0), true), (resized(|a| a.scale = 1.0), true), (resized(|a| a.scale = 0.0), true),
            (resized(|a| a.rect.x = f64::NAN), true),
        ] {
            let p = plan_page(&ex(), &h, &areas, complete);
            assert_eq!((p.hide, p.covers.len()), (Some(HideReason::SecretField), 0), "{areas:?} {complete}");
        }
        // A report with no size can't be checked.
        let sizeless = page(3, FIELD).replace("\"width\":2000,\"height\":1000,", "");
        assert_eq!(plan_page(&ex(), &hint_of(&[sizeless]), &[area(URL)], true).hide, Some(HideReason::SecretField));
    }

    #[test]
    fn sensitive_pages_hide_the_browser() {
        let typing = page(3, &FIELD.replace("\"typing\":false", "\"typing\":true"));
        assert_eq!(plan_page(&ex(), &hint_of(&[typing]), &[area(URL)], true).hide, Some(HideReason::SecretField));
        assert_eq!(plan_page(&ex(), &hint_of(&["nonsense".into()]), &[area(URL)], true).hide, Some(HideReason::SecretField));
        // Something in a frame inside the page.
        let frame = page(3, &CLEAN.replace("\"unplaced\":false", "\"unplaced\":true")).replace("\"frame\":0", "\"frame\":4");
        assert_eq!(plan_page(&ex(), &hint_of(&[page(3, CLEAN), frame]), &[area(URL)], true).hide, Some(HideReason::SecretField));
        // A frame that claims places anyway isn't believed about where.
        let odd = page(3, FIELD).replace("\"frame\":0", "\"frame\":4");
        assert_eq!(plan_page(&ex(), &hint_of(&[odd]), &[area(URL)], true).hide, Some(HideReason::SecretField));
    }

    #[test]
    fn private_windows_follow_the_setting() {
        let h = hint_of(&[tabs(true, URL).replace("\"private\":false", "\"private\":true")]);
        assert_eq!(plan_page(&ex(), &h, &[area(URL)], true).hide, Some(HideReason::PrivateWindow));
        assert_eq!(plan_page(&Exclusions::default(), &h, &[area(URL)], true).hide, None);
    }

    /// Both the browser and the screen must agree a tab is out of sight before its last report is set aside.
    #[test]
    fn a_background_tab_is_set_aside_only_when_everything_agrees() {
        let go = |tab: String, areas: &[PageArea], complete| plan_page(&ex(), &hint_of(&[page(3, FIELD), tab]), areas, complete).hide;
        let other = [area("https://b.com/")];
        assert_eq!(go(tabs(false, URL), &other, true), None);
        assert_eq!(go(tabs(true, URL), &other, true), Some(HideReason::SecretField));            // browser says it's showing
        assert_eq!(go(tabs(false, URL), &other, false), Some(HideReason::SecretField));          // couldn't see everything
        assert_eq!(go(tabs(false, "https://b.com/"), &other, true), Some(HideReason::SecretField)); // moved to a page that is showing
        // Still on screen (split view): covered, not set aside.
        let p = plan_page(&ex(), &hint_of(&[page(3, FIELD), tabs(false, URL)]), &[area(URL)], true);
        assert_eq!((p.hide, p.covers.len()), (None, 1));
        // A tab nothing is known about.
        assert_eq!(plan_page(&ex(), &hint_of(&[page(9, FIELD), tabs(false, URL)]), &other, true).hide, Some(HideReason::SecretField));
    }

    #[test]
    fn addresses_are_compared_as_the_extension_writes_them() {
        for (raw, want) in [("https://a.com/x?q#f", "https://a.com/x"), ("https://u:p@a.com:8443/x", "https://a.com:8443/x"), ("https://A.com", "https://a.com/"),
                            ("file:///Users/x/secret.html", "file:"), ("about:blank", "about:"), ("nonsense", "")] {
            assert_eq!(clean_address(raw), want);
        }
        assert_eq!(clean_address(&format!("https://a.com/{}", "a".repeat(5000))).len(), 2048);
        // Addresses that say nothing can't be matched to a page.
        let blank = page(3, FIELD).replace("https://a.com/x", "");
        assert_eq!(plan_page(&ex(), &hint_of(&[blank]), &[area("nonsense")], true).hide, Some(HideReason::SecretField));
        let file = page(3, FIELD).replace("https://a.com/x", "file:");
        assert_eq!(plan_page(&ex(), &hint_of(&[file]), &[area("file:///a"), area("file:///b")], true).hide, Some(HideReason::SecretField));
    }
}
