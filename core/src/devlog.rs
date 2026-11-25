//! Developer log: what was asked of LM Studio and what it answered, with everything secret taken out first.
//!
//! AIDEV-NOTE: privacy-critical. A `Call` is raw. Nothing here may pass on screen text except through `hidden` ranges that
//! AIDEV-NOTE: are final (rules, model, extension and repeats all applied), and nothing at all for a frame that wasn't saved.

use crate::detect::{self, Hit, TextLine};
use crate::lm::Call;
use crate::store::DevEntry;
use serde_json::{Value, json};

fn entry(c: &Call, kind: &str) -> DevEntry {
    DevEntry {
        at: c.at, kind: kind.into(), ms: c.ms, ok: c.result.is_ok(), note: c.result.as_ref().err().cloned().unwrap_or_default(),
        model: c.request["model"].as_str().or(c.request["instance_id"].as_str()).unwrap_or_default().into(), ..Default::default()
    }
}

fn kind(c: &Call) -> &'static str {
    match c.path {
        "/v1/embeddings" => "embed",
        "/api/v1/models/load" => "load",
        "/api/v1/models/unload" => "unload",
        _ if c.request["messages"].as_array().is_some_and(|m| m.len() > 1) => "check",
        _ => "describe",
    }
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

/// Calls made while a frame was being checked. `hits` is final; None means the frame wasn't saved or the check
/// didn't finish, and then no words are kept.
pub fn checks(calls: Vec<Call>, lines: &[TextLine], hits: Option<&[Hit]>, shot: Option<i64>) -> Vec<DevEntry> {
    calls
        .into_iter()
        .map(|c| {
            let mut e = entry(&c, kind(&c));
            if e.kind != "check" {
                return settings(c, e);
            }
            e.shot_id = shot;
            let Some(hits) = hits else {
                if e.note.is_empty() {
                    e.note = if shot.is_some() { "the check didn\u{2019}t finish" } else { "the picture wasn\u{2019}t saved" }.into();
                }
                e.ok = false;
                return e;
            };
            let masked: Vec<(usize, String)> = c.lines.iter().filter_map(|&i| Some((i, hide(lines.get(i)?, i, hits)))).collect();
            let mut req = c.request;
            req["messages"][1]["content"] = json!(crate::lm::number(&masked.iter().map(|(i, t)| (*i, t.as_str())).collect::<Vec<_>>()));
            e.request = pretty(&req);
            e.response = c.result.map(|v| findings(&v)).unwrap_or_default();
            e
        })
        .collect()
}

fn hide(l: &TextLine, i: usize, hits: &[Hit]) -> String {
    let r: Vec<_> = hits.iter().filter(|h| h.line == i).map(|h| h.range.clone()).collect();
    detect::mask(&l.text, &r)
}

/// Where and what kind, never the value the model quoted.
fn findings(v: &Value) -> String {
    let content = v["choices"][0]["message"]["content"].as_str().unwrap_or_default();
    let parsed = serde_json::from_str::<Value>(content).ok();
    let body = match parsed.as_ref().and_then(|p| p["findings"].as_array()) {
        Some(f) => json!({"findings": f.iter().map(|x| json!({
            "line": x["line"].as_u64(), "kind": detect::SecretKind::parse(x["kind"].as_str().unwrap_or("other")).as_str(), "value": "[hidden]"})).collect::<Vec<_>>()}),
        None => json!({"unreadable_answer_characters": content.chars().count()}),
    };
    pretty(&json!({"finish_reason": v["choices"][0]["finish_reason"].as_str(), "usage": v["usage"].as_object(), "content": body}))
}

/// Loading and unloading carry no screen text.
fn settings(c: Call, mut e: DevEntry) -> DevEntry {
    if matches!(e.kind.as_str(), "load" | "unload") {
        e.request = pretty(&c.request);
        e.response = c.result.map(|v| pretty(&v)).unwrap_or_default();
    }
    e
}

/// Calls about a saved picture: its text is already redacted, the picture itself is only named.
/// `clean`: what makes the model's own words safe to keep; the same as is used on the description that is saved.
pub fn about(calls: Vec<Call>, shot: i64, clean: &dyn Fn(&str) -> String) -> Vec<DevEntry> {
    calls
        .into_iter()
        .map(|c| {
            let mut e = entry(&c, kind(&c));
            let mut req = c.request.clone();
            match e.kind.as_str() {
                "describe" => {
                    // (A model that can't see was sent no picture.)
                    if let Some(picture) = req["messages"][0]["content"].get_mut(1) {
                        *picture = json!({"type": "image_url", "image_url": format!("[picture {shot}]")});
                    }
                    let v = c.result.unwrap_or_default();
                    let text = v["choices"][0]["message"]["content"].as_str().unwrap_or_default();
                    // Decoded first: a rule can't see a key hiding behind a JSON escape.
                    let content = match serde_json::from_str::<Value>(text) {
                        Ok(p) if p["description"].is_string() => json!({
                            "description": clean(p["description"].as_str().unwrap_or_default()),
                            "tags": p["tags"].as_array().into_iter().flatten().filter_map(Value::as_str).map(clean).collect::<Vec<_>>()}),
                        _ => json!({"unreadable_answer_characters": text.chars().count()}),
                    };
                    e.response = pretty(&json!({"finish_reason": v["choices"][0]["finish_reason"].as_str(), "usage": v["usage"].as_object(), "content": content}));
                }
                "embed" => {
                    let n = c.result.ok().and_then(|v| v["data"][0]["embedding"].as_array().map(Vec::len)).unwrap_or(0);
                    e.response = format!("{n} numbers");
                }
                _ => return settings(c, e),
            }
            e.request = pretty(&req);
            e.shot_id = Some(shot);
            e
        })
        .collect()
}

/// A person's own search: that it happened, not what for.
pub fn search(calls: Vec<Call>) -> Vec<DevEntry> {
    calls
        .into_iter()
        .map(|c| {
            let mut e = entry(&c, kind(&c));
            if e.kind == "embed" {
                e.note = if e.ok { "your search".into() } else { e.note };
                return e;
            }
            settings(c, e)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn described(content: &str) -> Call {
        Call { path: "/v1/chat/completions", at: 1, ms: 1, lines: vec![], request: json!({"model": "m", "messages": [{"role": "user", "content": [{"type": "text", "text": "p"}, {"type": "image_url", "image_url": {"url": "data:..."}}]}]}),
               result: Ok(json!({"choices": [{"finish_reason": "stop", "message": {"content": content}}]})) }
    }

    #[test]
    fn captions_are_masked_as_text_not_as_json() {
        let c = json!({"description": "first line\nAKIAIOSFODNN7EXAMPLE and \"password: hunter2zebra\"", "tags": ["ok", "token=Zx81qwertyuiop"]}).to_string();
        let e = about(vec![described(&c)], 5, &detect::mask_text).remove(0);
        for x in ["AKIAIOSFODNN7", "hunter2zebra", "Zx81qwerty", "data:..."] {
            assert!(!format!("{e:?}").contains(x), "{x}: {e:?}");
        }
        assert!(e.response.contains("first line") && e.response.contains("ok") && e.request.contains("[picture 5]"));
    }

    #[test]
    fn a_caption_that_cannot_be_read_is_only_counted() {
        let e = about(vec![described("AKIAIOSFODNN7EXAMPLE {")], 5, &detect::mask_text).remove(0);
        assert!(!e.response.contains("AKIA") && e.response.contains("unreadable_answer_characters"), "{}", e.response);
    }
}
