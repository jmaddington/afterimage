//! What the browser extension says about open pages.
//!
//! AIDEV-NOTE: privacy-critical. Everything here is untrusted and may only ADD hiding. A missing, stale or odd report
//! AIDEV-NOTE: must never make a page look safer than the address and accessibility checks already found it.

use crate::detect::{Rect, SecretKind};
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PageField {
    pub kind: SecretKind,
    /// Fractions of the visible page, top-left origin.
    pub rect: Rect,
    pub tab: i64,
    pub window: i64,
}

#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
pub struct PageHint {
    pub private_window: bool,
    /// Something secret is showing or being typed and can't be placed exactly: leave the browser out.
    pub sensitive: bool,
    pub fields: Vec<PageField>,
}

/// Reports repeat every few seconds while they matter; anything older is forgotten.
const FRESH: Duration = Duration::from_secs(12);
const MAX_MESSAGE: usize = 65_536;
const MAX_FIELDS: usize = 200;
const MAX_FRAMES: usize = 400;
const MAX_TABS: usize = 1000;
const MAX_BROWSERS: usize = 8;
const MAX_STORED: usize = 2000;

struct Frame {
    at: Instant,
    fields: Vec<PageField>,
    sensitive: bool,
}

struct Tab {
    active: bool,
    private: bool,
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
    Page { tab: i64, frame: i64, fields: Vec<PageField>, sensitive: bool },
    Tabs(HashMap<i64, Tab>),
}

fn fresh(at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(at) < FRESH
}

fn field(v: &Value, tab: i64, window: i64) -> Option<PageField> {
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
    Some(PageField { kind, rect: Rect { x: l, y: t, w: r - l, h: b - t }, tab, window })
}

fn parse(json: &str) -> Option<Message> {
    if json.len() > MAX_MESSAGE {
        return None;
    }
    let v: Value = serde_json::from_str(json).ok()?;
    if v.get("v")?.as_i64()? != 1 {
        return None;
    }
    match v.get("type")?.as_str()? {
        "page" => {
            let (tab, window) = (v.get("tab")?.as_i64()?, v.get("window")?.as_i64()?);
            let raw = v.get("fields")?.as_array()?;
            let fields: Vec<PageField> = raw.iter().take(MAX_FIELDS).filter_map(|f| field(f, tab, window)).collect();
            // A field that can't be placed is still a field.
            let sensitive = v.get("typing")?.as_bool()? | v.get("unplaced")?.as_bool()? | (fields.len() < raw.len());
            Some(Message::Page { tab, frame: v.get("frame")?.as_i64()?, fields, sensitive })
        }
        "tabs" => {
            let raw = v.get("tabs")?.as_array()?;
            if raw.len() > MAX_TABS {
                return None;
            }
            let tab = |t: &Value| Some((t.get("tab")?.as_i64()?, Tab { active: t.get("active")?.as_bool()?, private: t.get("private")?.as_bool()? }));
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
        match parse(json) {
            Some(Message::Page { tab, frame, fields, sensitive }) => {
                let key = (tab, frame);
                let grown = fields.len().max(1);
                if !b.frames.contains_key(&key) && b.frames.len() >= MAX_FRAMES || total + grown > MAX_STORED {
                    b.frames.remove(&key);
                    b.muddled = Some(now);
                } else {
                    b.frames.insert(key, Frame { at: now, fields, sensitive });
                }
            }
            Some(Message::Tabs(tabs)) => {
                b.tabs = tabs;
                b.tabs_at = Some(now);
            }
            None => b.muddled = Some(now),
        }
    }

    /// That connection hung up.
    pub fn gone(&mut self, pid: i32, source: u64) {
        self.browsers.remove(&(pid, source));
    }

    pub fn hint(&self, pid: i32, now: Instant) -> PageHint {
        let mut h = PageHint { sensitive: self.crowded.is_some_and(|at| fresh(at, now)), ..Default::default() };
        for b in self.browsers.iter().filter(|((p, _), _)| *p == pid).map(|x| x.1) {
            let tabs = b.tabs_at.filter(|at| fresh(*at, now)).map(|_| &b.tabs);
            // Only a tab known to be in the background is ignored.
            let showing = |tab: i64| tabs.and_then(|t| t.get(&tab)).is_none_or(|t| t.active);
            h.private_window |= tabs.is_some_and(|t| t.values().any(|t| t.active && t.private));
            h.sensitive |= b.muddled.is_some_and(|at| fresh(at, now));
            for f in b.frames.iter().filter(|((tab, _), f)| fresh(f.at, now) && showing(*tab)).map(|x| x.1) {
                h.sensitive |= f.sensitive;
                h.fields.extend(f.fields.iter().cloned());
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
        format!(r#"{{"v":1,"type":"page","tab":{tab},"window":1,"frame":0,"url":"https://a.com/x",{body}}}"#)
    }
    const FIELD: &str = r#""fields":[{"kind":"password","x":0.1,"y":0.2,"w":0.3,"h":0.05}],"typing":false,"unplaced":false"#;
    const CLEAN: &str = r#""fields":[],"typing":false,"unplaced":false"#;
    fn at(secs: u64) -> Duration {
        Duration::from_secs(secs)
    }

    #[test]
    fn fields_are_reported_for_their_browser_only() {
        let (mut p, t) = (Pages::default(), Instant::now());
        p.receive(7, 1, &page(3, FIELD), t);
        let h = p.hint(7, t);
        let f = &h.fields[0];
        assert_eq!((h.fields.len(), f.kind, f.tab, f.window), (1, SecretKind::Password, 3, 1));
        for (a, b) in [(f.rect.x, 0.1), (f.rect.y, 0.2), (f.rect.w, 0.3), (f.rect.h, 0.05)] {
            assert!((a - b).abs() < 1e-9);
        }
        assert!(!h.sensitive && !h.private_window);
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
        assert_eq!(p.hint(7, t).fields.len(), 1);
    }

    #[test]
    fn typing_and_unplaced_are_sensitive() {
        for b in [CLEAN.replace("\"typing\":false", "\"typing\":true"), CLEAN.replace("\"unplaced\":false", "\"unplaced\":true")] {
            let (mut p, t) = (Pages::default(), Instant::now());
            p.receive(7, 1, &page(3, &b), t);
            assert!(p.hint(7, t).sensitive, "{b}");
        }
    }

    #[test]
    fn reports_go_stale() {
        let (mut p, t) = (Pages::default(), Instant::now());
        p.receive(7, 1, &page(3, FIELD), t);
        assert_eq!(p.hint(7, t + at(10)).fields.len(), 1);
        assert_eq!(p.hint(7, t + at(13)), PageHint::default());
    }

    #[test]
    fn only_showing_tabs_count_but_unknown_ones_do_too() {
        let tabs = |active| format!(r#"{{"v":1,"type":"tabs","tabs":[{{"tab":3,"window":1,"active":{active},"private":false}}]}}"#);
        let (mut p, t) = (Pages::default(), Instant::now());
        p.receive(7, 1, &page(3, FIELD), t);
        p.receive(7, 1, &tabs(false), t);
        assert_eq!(p.hint(7, t).fields.len(), 0);
        p.receive(7, 1, &tabs(true), t);
        assert_eq!(p.hint(7, t).fields.len(), 1);
        // A tab the snapshot doesn't mention, and a snapshot too old to trust.
        p.receive(7, 1, &page(4, FIELD), t);
        p.receive(7, 1, &tabs(false), t);
        assert_eq!(p.hint(7, t).fields.iter().map(|f| f.tab).collect::<Vec<_>>(), vec![4]);
        p.receive(7, 1, &page(3, FIELD), t + at(13));
        assert_eq!(p.hint(7, t + at(13)).fields.len(), 1);
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
        let hint = |x: &str| { let (mut p, t) = (Pages::default(), Instant::now()); p.receive(7, 1, &f(x), t); p.hint(7, t) };
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
        p.gone(7, 1);
        assert_eq!(p.hint(7, t + at(30)), PageHint::default());
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
        assert_eq!(p.hint(7, t).fields.len(), 1);
        p.gone(7, 2);
        assert!(p.hint(7, t).private_window);
        p.gone(7, 1);
        assert_eq!(p.hint(7, t), PageHint::default());
    }
}
