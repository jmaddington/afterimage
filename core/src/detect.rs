//! Deterministic secret detection over OCR lines.
//!
//! AIDEV-NOTE: privacy-critical. Runs before (and independently of) the language model, so a model that is
//! wrong, offline or talked out of it by on-screen text can never un-redact what these rules caught.

use regex::Regex;
use std::ops::Range;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum SecretKind {
    Password,
    Ssn,
    Financial,
    Key,
    Other,
}

impl SecretKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::Ssn => "ssn",
            Self::Financial => "financial",
            Self::Key => "key",
            Self::Other => "other",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "password" => Self::Password,
            "ssn" => Self::Ssn,
            "financial" => Self::Financial,
            "key" => Self::Key,
            _ => Self::Other,
        }
    }
}

/// Normalised rect, origin top-left, 0..1.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn union(self, o: Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        let r = (self.x + self.w).max(o.x + o.w);
        let b = (self.y + self.h).max(o.y + o.h);
        Rect { x, y, w: r - x, h: b - y }
    }
    fn cy(&self) -> f64 {
        self.y + self.h / 2.0
    }
}

/// One whitespace-delimited token of a line. Offsets are Unicode scalar counts within the line text.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Token {
    pub start: u32,
    pub len: u32,
    pub rect: Rect,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct TextLine {
    pub text: String,
    pub rect: Rect,
    pub tokens: Vec<Token>,
    /// The payload of a QR or bar code; empty when it couldn't be decoded.
    #[uniffi(default = false)]
    pub code: bool,
}

/// A secret at a byte range of a line.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub line: usize,
    pub range: Range<usize>,
    pub kind: SecretKind,
}

macro_rules! re {
    ($name:ident, $p:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($p).expect("static regex"));
    };
}

re!(SSN, r"\b\d{3}[\s\-\u{2010}-\u{2015}\u{2212}.]{1,3}\d{2}[\s\-\u{2010}-\u{2015}\u{2212}.]{1,3}\d{4}\b");
re!(NINE, r"\b\d{9}\b");
re!(DIGIT_RUN, r"\d(?:[\s\-\u{2010}-\u{2015}\u{2212}.]{0,2}\d){11,18}");
re!(CARD_GROUPS, r"\b\d{4}[\s\-.]{1,2}\d{4}[\s\-.]{1,2}\d{4}[\s\-.]{1,2}\d{1,7}\b|\b3[47]\d{2}[\s\-.]{1,2}\d{6}[\s\-.]{1,2}\d{5}\b");
re!(IBAN, r"\b[A-Z]{2}\d{2}(?:\s?[A-Z0-9]{4}){2,7}(?:\s?[A-Z0-9]{1,4})?\b");
re!(SSN_LABEL, r"(?i)\b(?:ssn|social\s+security|tax(?:payer)?\s*id|itin|tin)\b");
re!(FIN_LABEL, r"(?i)\b(?:routing|aba|account|acct|a/c|iban|sort\s*code|swift|bic|card|debit|credit)(?:\s*(?:number|num|no\.?|nbr|code|#))?(?:\s*\(?(?:required|optional)\)?)?\s*[:#]?");
re!(FIN_VALUE, r"[0-9Xx*•](?:[\s\-.]?[0-9Xx*•]){4,24}");
re!(CVV, r"(?i)\b(?:cvv2?|cvc2?|csc|security\s+code)\b\s*[:#]?\s*(\d{3,4})\b");
re!(CVV_LABEL, r"(?i)^\s*(?:cvv2?|cvc2?|csc|security\s+code)\s*[:#]?\s*$");
// key = value / key: value where the key names a secret.
re!(ASSIGN, r#"(?i)([A-Za-z0-9_.\-]*(?:pass(?:word|wd|code|phrase|key)?|pwd|secret|token|api[_\-\s]?key|access[_\-\s]?key|private[_\-\s]?key|client[_\-\s]?secret|auth[_\-\s]?key|credential|recovery[_\-\s]?(?:key|code)|backup[_\-\s]?code|pin|otp)[A-Za-z0-9_.\-]*)["']?\s*(?:[:=]|=>|:=)\s*["']?([^\s"',;]+)"#);
re!(PASS_PHRASE, r#"(?i)\b(?:password|passcode|passphrase|pwd|pin(?:\s+code)?)\s+(?:is|was|=)\s*:?\s*["'“‘]?([^\s"'”’]+)"#);
re!(PASS_LABEL, r"(?i)^\s*(?:(?:current|new|old|confirm|your|temporary|temp|admin|root|user|wi-?fi|wifi|network|master|login|account)\s+)*(?:password|passcode|pass\s*phrase|passphrase|pin|pin\s+code|secret(?:\s+key)?|api\s+key|access\s+(?:key|token|code)|token|recovery\s+(?:key|code)|client\s+secret|private\s+key|security\s+code)(?:\s+\(?(?:required|optional)\)?)?\s*[:*]?\s*$");
re!(FLAG, r#"(?i)(?:--?(?:password|passwd|pass|token|api-key|secret)|\bsshpass\s+-p|\bmysql\b.*\s-p)(?:[=\s]+)["']?([^\s"']+)"#);
re!(BEARER, r"(?i)\b(?:bearer|basic)\s+([A-Za-z0-9._~+/\-]{12,}=*)");
re!(URL_CRED, r"(?i)\b[a-z][a-z0-9+.\-]*://[^\s/:@]+:([^\s/@]+)@");
re!(KNOWN, r"(?x)
    \b(?:AKIA|ASIA|AROA|AIDA)[A-Z0-9]{12,20}\b
  | \bgh[pousr]_[A-Za-z0-9]{16,}
  | \bgithub_pat_[A-Za-z0-9_]{16,}
  | \bglpat-[A-Za-z0-9_\-]{12,}
  | \bsk-(?:ant-|proj-|or-|live-|test-)?[A-Za-z0-9_\-]{16,}
  | \b(?:sk|rk|pk)_(?:live|test)_[A-Za-z0-9]{10,}
  | \bwhsec_[A-Za-z0-9]{10,}
  | \bxox[abeoprs]-[A-Za-z0-9\-]{8,}
  | \bAIza[A-Za-z0-9_\-]{20,}
  | \bya29\.[A-Za-z0-9_\-]{10,}
  | \bhf_[A-Za-z0-9]{16,}
  | \bnpm_[A-Za-z0-9]{16,}
  | \bpypi-[A-Za-z0-9_\-]{16,}
  | \bSG\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}
  | \bshp(?:at|ss|ca)_[A-Za-z0-9]{16,}
  | \bsq0(?:atp|csp)-[A-Za-z0-9_\-]{16,}
  | \bfigd_[A-Za-z0-9_\-]{16,}
  | \bfw_[A-Za-z0-9]{15,}
  | \btskey-[a-z]+-[A-Za-z0-9\-]{10,}
  | \bdop_v1_[a-f0-9]{16,}
  | \beyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-.]*
");
// Payloads of setup QR codes, which the recogniser hands over as lines of text.
re!(QR_SECRET, r"(?is)otpauth(?:-migration)?://.*|\bWIFI:.*\bP:.*");
// Query keys whose values are worth hunting for elsewhere on the screen.
re!(SECRET_KEY, r"(?i)secret|token|pass|pwd|key|code|auth|sig|otp|pin|session|credential|^data$");
re!(PEM_BEGIN, r"-{2,}\s*BEGIN [A-Z0-9 ]*(?:PRIVATE KEY|SECRET)");
re!(PEM_END, r"-{2,}\s*END [A-Z0-9 ]*(?:PRIVATE KEY|SECRET)");
re!(LONG_TOKEN, r"[A-Za-z0-9_+/=\-]{24,}");
re!(UUID, r"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$");
re!(NOT_A_VALUE, r"(?i)\b(?:forgot|forgotten|reset|show|hide|sign\s*in|log\s*in|sign\s*up|continue|submit|cancel|save|remember|required|optional|must|characters?|enter|confirm|change|manager|generate|copy|username|email|user\s*name)\b");
// Source code assigning an expression (`let tokens = line.tokens.iter()`), not a literal.
re!(CODE_EXPR, r"^[A-Za-z_]\w*(?:(?:\.|::)[A-Za-z_]\w*)*\(");
re!(MASKED, r"^[\s•●∙·*.\u{2022}xX_\-–—]+$");

pub fn luhn(digits: &str) -> bool {
    let d: Vec<u32> = digits.chars().filter_map(|c| c.to_digit(10)).collect();
    if d.len() < 12 {
        return false;
    }
    let sum: u32 = d
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &n)| if i % 2 == 1 { let x = n * 2; if x > 9 { x - 9 } else { x } } else { n })
        .sum();
    sum % 10 == 0
}

fn entropy(s: &str) -> f64 {
    let mut counts = std::collections::HashMap::new();
    for c in s.chars() {
        *counts.entry(c).or_insert(0usize) += 1;
    }
    let n = s.chars().count() as f64;
    counts.values().map(|&c| { let p = c as f64 / n; -p * p.log2() }).sum()
}

fn looks_random(t: &str) -> bool {
    if UUID.is_match(t) {
        return false;
    }
    let up = t.chars().any(|c| c.is_ascii_uppercase());
    let low = t.chars().any(|c| c.is_ascii_lowercase());
    let dig = t.chars().filter(|c| c.is_ascii_digit()).count();
    let e = entropy(t);
    if !(up && low && (dig >= 2 && e >= 3.8 || dig >= 1 && e >= 4.3)) {
        return false;
    }
    // Paths and kebab/snake identifiers are many short words; random tokens have long unbroken runs.
    let longest = t.split(['/', '-', '_', '=', '+']).map(str::len).max().unwrap_or(0);
    longest >= 16 || t.matches('/').count() == 0 && longest >= 12
}

/// A bare value that plausibly sits in a secret's field.
fn plausible_value(t: &str) -> bool {
    let t = t.trim();
    let words = t.split_whitespace().count();
    !t.is_empty() && words <= 6 && t.chars().count() >= 3 && !MASKED.is_match(t) && !NOT_A_VALUE.is_match(t)
}

/// Lines that hold the value for a standalone label: to the right on the same row, else directly beneath.
/// AIDEV-NOTE: geometry heuristic; tuned for form layouts. Errs towards returning a candidate.
fn neighbours(lines: &[TextLine], i: usize) -> Vec<usize> {
    let l = &lines[i].rect;
    let mut right: Option<(f64, usize)> = None;
    let mut below: Option<(f64, usize)> = None;
    for (j, o) in lines.iter().enumerate() {
        if j == i || o.code {
            continue;
        }
        let r = &o.rect;
        let same_row = (r.cy() - l.cy()).abs() < 0.6 * l.h.max(r.h);
        if same_row && r.x >= l.x + l.w - 0.005 {
            let d = r.x - (l.x + l.w);
            if d < 0.45 && right.is_none_or(|(b, _)| d < b) {
                right = Some((d, j));
            }
        }
        let dy = r.y - (l.y + l.h);
        let overlaps = r.x < l.x + l.w + 0.02 && r.x + r.w > l.x - 0.02;
        if overlaps && dy > -0.3 * l.h && dy < 3.5 * l.h && below.is_none_or(|(b, _)| dy < b) {
            below = Some((dy, j));
        }
    }
    right.into_iter().chain(below).map(|(_, j)| j).collect()
}

fn push(hits: &mut Vec<Hit>, line: usize, range: Range<usize>, kind: SecretKind) {
    if range.start < range.end {
        hits.push(Hit { line, range, kind });
    }
}

/// Detect within a single line, no layout context. Also used for window titles and URLs.
pub fn scan_text(text: &str, line: usize, hits: &mut Vec<Hit>) {
    use SecretKind::*;
    for m in SSN.find_iter(text) {
        push(hits, line, m.range(), Ssn);
    }
    if SSN_LABEL.is_match(text) {
        for m in NINE.find_iter(text) {
            push(hits, line, m.range(), Ssn);
        }
    }
    for m in CARD_GROUPS.find_iter(text) {
        push(hits, line, m.range(), Financial);
    }
    for m in DIGIT_RUN.find_iter(text) {
        let d: String = m.as_str().chars().filter(char::is_ascii_digit).collect();
        if (13..=19).contains(&d.len()) && luhn(&d) {
            push(hits, line, m.range(), Financial);
        }
    }
    for m in IBAN.find_iter(text) {
        let compact: String = m.as_str().split_whitespace().collect();
        if compact.len() >= 15 && compact.chars().filter(char::is_ascii_digit).count() >= 8 {
            push(hits, line, m.range(), Financial);
        }
    }
    if let Some(label) = FIN_LABEL.find(text) {
        // "Order number 88213" has no financial label; "Account number 4417 2280 9931" does.
        if let Some(v) = FIN_VALUE.find_at(text, label.end()) {
            if v.start() - label.end() <= 24 && v.as_str().chars().filter(char::is_ascii_digit).count() >= 4 {
                push(hits, line, v.range(), Financial);
            }
        }
    }
    for c in CVV.captures_iter(text) {
        push(hits, line, c.get(1).unwrap().range(), Financial);
    }
    for c in ASSIGN.captures_iter(text) {
        let v = c.get(2).unwrap();
        if v.as_str().chars().count() >= 3
            && !MASKED.is_match(v.as_str())
            && !NOT_A_VALUE.is_match(v.as_str())
            && !CODE_EXPR.is_match(v.as_str())
        {
            push(hits, line, v.range(), if c[1].to_lowercase().contains("pass") { Password } else { Key });
        }
    }
    for re in [&*PASS_PHRASE, &*FLAG] {
        for c in re.captures_iter(text) {
            let v = c.get(1).unwrap();
            if plausible_value(v.as_str()) {
                push(hits, line, v.range(), Password);
            }
        }
    }
    for c in BEARER.captures_iter(text) {
        push(hits, line, c.get(1).unwrap().range(), Key);
    }
    for c in URL_CRED.captures_iter(text) {
        push(hits, line, c.get(1).unwrap().range(), Password);
    }
    for m in QR_SECRET.find_iter(text) {
        push(hits, line, m.range(), Key);
    }
    for m in KNOWN.find_iter(text) {
        push(hits, line, m.range(), Key);
    }
    for m in LONG_TOKEN.find_iter(text) {
        if looks_random(m.as_str()) {
            push(hits, line, m.range(), Key);
        }
    }
}

/// Detect across a whole screen, using layout to pair standalone labels with their values.
pub fn scan(lines: &[TextLine]) -> Vec<Hit> {
    let mut hits = Vec::new();
    let mut in_pem = false;
    for (i, l) in lines.iter().enumerate() {
        let t = &l.text;
        if l.code {
            if !code_is_harmless(t) {
                hits.push(Hit { line: i, range: 0..t.len(), kind: SecretKind::Key });
            }
            continue;
        }
        if PEM_BEGIN.is_match(t) {
            in_pem = true;
        }
        if in_pem {
            push(&mut hits, i, 0..t.len(), SecretKind::Key);
        }
        if PEM_END.is_match(t) {
            in_pem = false;
        }
        scan_text(t, i, &mut hits);

        if PASS_LABEL.is_match(t) {
            for j in neighbours(lines, i) {
                let v = &lines[j].text;
                if plausible_value(v) && !PASS_LABEL.is_match(v) {
                    push(&mut hits, j, 0..v.len(), SecretKind::Password);
                }
            }
        }
        if CVV_LABEL.is_match(t) {
            for j in neighbours(lines, i) {
                let v = lines[j].text.trim();
                if v.len() <= 4 && v.chars().all(|c| c.is_ascii_digit()) {
                    push(&mut hits, j, 0..lines[j].text.len(), SecretKind::Financial);
                }
            }
        }
        let ssn_label = SSN_LABEL.is_match(t);
        let fin_label = FIN_LABEL.is_match(t) && t.split_whitespace().count() <= 5;
        if ssn_label || fin_label {
            for j in neighbours(lines, i) {
                let v = &lines[j].text;
                let digits = v.chars().filter(char::is_ascii_digit).count();
                let compact = v.chars().filter(|c| !c.is_whitespace()).count().max(1);
                let mostly = digits * 10 >= compact * 6;
                if ssn_label && digits >= 4 && digits <= 9 && mostly {
                    push(&mut hits, j, 0..v.len(), SecretKind::Ssn);
                } else if fin_label && (6..=24).contains(&digits) && mostly && !v.contains(['$', '€', '£', '/']) {
                    push(&mut hits, j, 0..v.len(), SecretKind::Financial);
                }
            }
        }
    }
    extend_wrapped(lines, &mut hits);
    hits
}

/// A key that runs to the end of its line usually wraps. Hide following rows that look like more of it.
pub fn extend_wrapped(lines: &[TextLine], hits: &mut Vec<Hit>) {
    let starts: Vec<usize> = hits
        .iter()
        .filter(|h| matches!(h.kind, SecretKind::Key | SecretKind::Other))
        .filter(|h| !lines[h.line].code && lines[h.line].text[h.range.end..].trim().is_empty() && h.range.len() >= 16)
        .map(|h| h.line)
        .collect();
    for mut at in starts {
        for _ in 0..40 {
            let cur = &lines[at].rect;
            let next = lines.iter().enumerate().find(|(j, o)| {
                let dy = o.rect.y - (cur.y + cur.h);
                *j != at && dy > -0.3 * cur.h && dy < 1.2 * cur.h && o.rect.x < cur.x + cur.w && o.rect.x + o.rect.w > cur.x
                    && o.text.split_whitespace().count() == 1
                    && o.text.trim().len() >= 6
                    && o.text.trim().chars().all(|c| c.is_ascii_alphanumeric() || "+/=_-.".contains(c))
                    && o.text.chars().any(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
            });
            let Some((j, o)) = next else { break };
            let h = Hit { line: j, range: 0..o.text.len(), kind: SecretKind::Key };
            if hits.contains(&h) {
                break;
            }
            hits.push(h);
            at = j;
        }
    }
}

/// Replace every hit with a marker; used for indexed text, titles and URLs.
pub fn mask(text: &str, ranges: &[Range<usize>]) -> String {
    let mut r: Vec<Range<usize>> = ranges.to_vec();
    r.sort_by_key(|x| x.start);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for x in r {
        if x.end <= at {
            continue;
        }
        let s = x.start.max(at);
        if x.start >= at {
            out.push_str(&text[at..s]);
            out.push_str("[hidden]");
        }
        at = x.end;
    }
    out.push_str(&text[at..]);
    out
}

pub fn mask_text(text: &str) -> String {
    let mut hits = Vec::new();
    scan_text(text, 0, &mut hits);
    let r: Vec<_> = hits.into_iter().map(|h| h.range).collect();
    mask(text, &r)
}

/// Keep scheme, host and path only: queries and fragments routinely carry tokens.
pub fn scrub_url(raw: &str) -> String {
    let Ok(mut u) = url::Url::parse(raw) else { return String::new() };
    if !matches!(u.scheme(), "http" | "https" | "file") {
        return format!("{}:", u.scheme());
    }
    u.set_query(None);
    u.set_fragment(None);
    let _ = u.set_username("");
    let _ = u.set_password(None);
    mask_text(u.as_str())
}

/// Longest code payload that can still be an ordinary web address.
const CODE_MAX: usize = 300;

/// Deny by default: a code stays visible only when it is a bare web address carrying nothing private.
/// AIDEV-NOTE: privacy-critical. Codes hold 2FA seeds, Wi-Fi keys, login tokens; OCR can't see any of them.
fn code_is_harmless(payload: &str) -> bool {
    if payload.len() > CODE_MAX || payload.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return false;
    }
    let Ok(u) = url::Url::parse(payload) else { return false };
    let mut found = Vec::new();
    scan_text(payload, 0, &mut found);
    matches!(u.scheme(), "http" | "https")
        && u.username().is_empty()
        && u.password().is_none()
        && u.query().is_none()
        && u.fragment().is_none()
        && found.is_empty()
}

/// The secrets inside a code's payload, as they might also be shown as text ("or type this key instead").
pub fn code_secrets(payload: &str) -> Vec<String> {
    let mut out = Vec::new();
    if payload.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("wifi:")) {
        let mut fields = vec![String::new()];
        let mut rest = payload[5..].chars();
        while let Some(c) = rest.next() {
            match c {
                '\\' => fields.last_mut().unwrap().extend(rest.next()),
                ';' => fields.push(String::new()),
                c => fields.last_mut().unwrap().push(c),
            }
        }
        for f in fields {
            if let Some(("P" | "p", v)) = f.split_once(':') {
                out.push(v.to_owned());
                out.push(v.trim_matches('"').to_owned());
            }
        }
    } else if let Ok(u) = url::Url::parse(payload) {
        out.extend(u.query_pairs().filter(|(k, _)| SECRET_KEY.is_match(k)).map(|(_, v)| v.into_owned()));
        out.extend(u.password().map(|p| percent_encoding::percent_decode_str(p).decode_utf8_lossy().into_owned()));
    }
    out.retain(|v| v.trim().chars().count() >= 4);
    out.dedup();
    out
}

/// Case, spacing and the letters OCR confuses don't make it a different secret.
fn fold(c: char) -> char {
    match c {
        'o' => '0',
        'l' | 'i' | '|' => '1',
        's' => '5',
        'b' => '8',
        'z' => '2',
        c => c,
    }
}
fn folded(text: &str) -> impl Iterator<Item = (char, Range<usize>)> + '_ {
    text.char_indices()
        .filter(|(_, c)| !c.is_whitespace() && !matches!(c, '-' | '\u{2010}'..='\u{2015}' | '\u{2212}'))
        .flat_map(|(i, c)| c.to_lowercase().map(move |l| (fold(l), i..i + c.len_utf8())))
}

/// Every place a known secret appears, however it is spaced, grouped or broken across rows.
pub fn find_secret(lines: &[TextLine], value: &str) -> Vec<(usize, Range<usize>)> {
    let needle: Vec<char> = folded(value).map(|x| x.0).collect();
    let mut out: Vec<(usize, Range<usize>)> = Vec::new();
    if needle.len() < 6 {
        // Too short to match loosely: verbatim, as a word of its own.
        let v = value.trim();
        let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
        for (i, l) in lines.iter().enumerate().filter(|_| v.chars().any(char::is_alphanumeric)) {
            for (s, m) in l.text.match_indices(v) {
                if !word(l.text[..s].chars().next_back()) && !word(l.text[s + m.len()..].chars().next()) {
                    out.push((i, s..s + m.len()));
                }
            }
        }
        return out;
    }
    let all: Vec<(char, usize, Range<usize>)> = lines.iter().enumerate().flat_map(|(i, l)| folded(&l.text).map(move |(c, r)| (c, i, r))).collect();
    let mut at = 0;
    while at + needle.len() <= all.len() {
        let span = &all[at..at + needle.len()];
        if span.iter().map(|x| x.0).ne(needle.iter().copied()) {
            at += 1;
            continue;
        }
        for row in span.chunk_by(|a, b| a.1 == b.1) {
            out.push((row[0].1, row[0].2.start..row[row.len() - 1].2.end));
        }
        at += needle.len();
    }
    out
}

/// Pixel-space cover for a hit: every token the hit touches, or the whole line when tokens are missing.
pub fn hit_rect(line: &TextLine, range: &Range<usize>) -> Rect {
    let start = line.text[..range.start].chars().count() as u32;
    let end = start + line.text[range.clone()].chars().count() as u32;
    let mut acc: Option<Rect> = None;
    let mut covered = 0u32;
    for t in &line.tokens {
        let (a, b) = (t.start.max(start), (t.start + t.len).min(end));
        if a < b {
            covered += b - a;
            acc = Some(acc.map_or(t.rect, |r| r.union(t.rect)));
        }
    }
    let needed = line.text[range.clone()].chars().filter(|c| !c.is_whitespace()).count() as u32;
    match acc {
        Some(r) if covered >= needed => r,
        _ => line.rect,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(t: &str, x: f64, y: f64) -> TextLine {
        TextLine { text: t.into(), rect: Rect { x, y, w: t.len() as f64 * 0.008, h: 0.02 }, tokens: vec![], code: false }
    }
    fn code(t: &str) -> TextLine {
        TextLine { code: true, ..line(t, 0.5, 0.5) }
    }
    fn masked(t: &str) -> String {
        mask_text(t)
    }

    #[test]
    fn hides_secrets() {
        for (t, secret) in [
            ("SSN: 219-09-9999", "219-09-9999"),
            ("219 09 9999", "9999"),
            ("219–09–9999", "9999"),
            ("Social Security number 219099999", "219099999"),
            ("Card 4111 1111 1111 1111", "1111"),
            ("Card 4111111111111111", "4111111111111111"),
            ("Amex 3782 822463 10005", "822463"),
            ("4111 1111 1111 1112", "1112"),
            ("CVV 737", "737"),
            ("Routing number  021000021", "021000021"),
            ("Account number: 4417 2280 9931", "9931"),
            ("IBAN GB82 WEST 1234 5698 7654 32", "7654"),
            ("DB_PASSWORD=correct-horse-battery-9", "correct"),
            ("password: hunter2", "hunter2"),
            ("my password is hunter2", "hunter2"),
            ("AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE", "AKIA"),
            ("export GITHUB_TOKEN=ghp_abcdefghijklmnop1234", "ghp_"),
            ("Authorization: Bearer abcdef1234567890abcd", "abcdef"),
            ("postgres://app:s3cr3t!@db.local/x", "s3cr3t"),
            ("curl --token Zx81qwerty", "Zx81"),
            ("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "bPxRfi"),
            ("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.abc", "eyJ"),
            ("card:$4111111111111111", "41111"),
            ("password=$123abc", "123abc"),
            ("{\"password\": \"hunter2!\"}", "hunter2"),
            ("POSTGRES://a:topsecret@h/db", "topsecret"),
            ("shpat_abcdefghijklmnop12", "shpat"),
            ("otpauth://totp/Acme:dana?secret=JBSWY3DPEHPK3PXP&issuer=Acme", "JBSWY"),
            ("otpauth-migration://offline?data=CjEKCkhl", "CjEK"),
            ("WIFI:T:WPA;S:BlueHeron;P:marmalade tuba;;", "marmalade"),
        ] {
            assert!(!masked(t).contains(secret), "leaked from {t:?}: {}", masked(t));
        }
    }

    #[test]
    fn leaves_ordinary_text() {
        for t in [
            "Revenue is up 12% over last quarter",
            "Order number 88213 shipped on 2026-03-14",
            "Call Sam at 555-0142 about the launch",
            "Version 4.2.1 is live",
            "Password reset page redesign is done",
            "Forgot password?",
            "Meeting moved to 3:30 PM",
            "/Users/dana/dev/afterimage/core/src/detect.rs",
            "commit 9fceb02d0ae598e95dc970b74767f19372d61af8",
            "550e8400-e29b-41d4-a716-446655440000",
            "Password: ••••••••",
            "let tokens = line.tokens.iter()",
            "password rotation failed on two hosts",
            "use crate::detect::Rect",
            "219-099-999",
        ] {
            assert_eq!(masked(t), t);
        }
    }

    #[test]
    fn pairs_label_with_field() {
        let l = vec![line("Password", 0.1, 0.10), line("blue-heron-7", 0.1, 0.14), line("Forgot password?", 0.1, 0.20)];
        let h = scan(&l);
        assert!(h.iter().any(|x| x.line == 1 && x.range == (0..12)));
        assert!(!h.iter().any(|x| x.line == 2));
        let l = vec![line("PIN", 0.1, 0.1), line("7725", 0.3, 0.1)];
        assert!(scan(&l).iter().any(|x| x.line == 1));
    }

    #[test]
    fn hides_pem_body() {
        let l = vec![line("-----BEGIN RSA PRIVATE KEY-----", 0.1, 0.1), line("MIIabc", 0.1, 0.2), line("-----END RSA PRIVATE KEY-----", 0.1, 0.3), line("ok", 0.1, 0.4)];
        let h: Vec<_> = scan(&l).iter().map(|x| x.line).collect();
        assert!(h.contains(&1) && !h.contains(&3));
    }

    #[test]
    fn hides_wrapped_tokens() {
        let l = vec![
            line("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0", 0.1, 0.10),
            line("9dGhpcyBpcyBub3Q0IHJlYWw", 0.1, 0.12),
            line("hcmQgd29yazE5", 0.1, 0.14),
            line("plain words follow here", 0.1, 0.16),
        ];
        let h: Vec<_> = scan(&l).iter().map(|x| x.line).collect();
        assert!(h.contains(&1) && h.contains(&2) && !h.contains(&3), "{h:?}");
    }

    #[test]
    fn urls_lose_queries() {
        assert_eq!(scrub_url("https://u:p@x.com/a?token=1#f"), "https://x.com/a");
        assert_eq!(scrub_url("data:text/plain,secret"), "data:");
    }

    #[test]
    fn missing_tokens_cover_whole_line() {
        let mut l = line("pw hunter2 x", 0.0, 0.0);
        assert_eq!(hit_rect(&l, &(3..10)), l.rect);
        l.tokens = vec![Token { start: 3, len: 7, rect: Rect { x: 0.1, y: 0.0, w: 0.1, h: 0.02 } }];
        assert_eq!(hit_rect(&l, &(3..10)).x, 0.1);
    }

    #[test]
    fn mask_handles_overlap_and_unicode() {
        assert_eq!(mask("é abc def", &[3..6, 4..10]), "é [hidden]");
    }

    // ---- security review 2026-11-23: codes ----
    #[test]
    fn codes_are_hidden_unless_plainly_harmless() {
        for t in ["", "otpauth://totp/A?secret=JBSWY3DPEHPK3PXP", "WIFI:T:WPA;S:x;P:marmalade;;", "WIFI:S:a\nb;P:marmalade;;", "https://x.com/login?token=abc",
                  "https://x.com/#abc", "https://u:p@x.com/", "https://u@x.com/", "HTTPS://X.COM/?A", "2@abc,def,ghi", "sgnl://linkdevice?uuid=1", "bitcoin:1A2b", "LPA:1$smdp.io$ABC",
                  "MATMSG:TO:a@b.c;", "tel:5550142", "5901234123457", "https://x.com/ra/Zk3Lq9TtWv2Xy7Pb4Nc8Rd5M", "https://x.com/a\nb", "just text"] {
            let h = scan(&[code(t)]);
            assert!(h.iter().any(|h| h.line == 0 && h.range == (0..t.len())), "{t:?} {h:?}");
        }
        assert!(!scan(&[code(&format!("https://x.com/{}", "a/".repeat(400)))]).is_empty());
        for t in ["https://example.com/menu", "http://example.com", "HTTPS://EXAMPLE.COM/MENU"] {
            assert_eq!(scan(&[code(t)]), vec![], "{t}");
        }
        // The same characters as ordinary text aren't a code.
        assert_eq!(scan(&[line("just text", 0.1, 0.1)]), vec![]);
    }

    #[test]
    fn an_undecodable_code_covers_its_whole_box() {
        let c = code("");
        let h = scan(std::slice::from_ref(&c)).remove(0);
        assert_eq!(hit_rect(&c, &h.range), c.rect);
    }

    #[test]
    fn a_code_neither_displaces_nor_spreads() {
        // Nearer than the real value, but not a form field.
        let l = vec![line("Password", 0.1, 0.10), TextLine { rect: Rect { x: 0.1, y: 0.125, w: 0.1, h: 0.01 }, ..code("https://example.com/menu") }, line("blue-heron-7", 0.1, 0.15)];
        assert!(scan(&l).iter().any(|h| h.line == 2));
        let l = vec![TextLine { rect: Rect { x: 0.1, y: 0.1, w: 0.3, h: 0.3 }, ..code("otpauth://totp/A?secret=JBSWY3DPEHPK3PXP") }, line("Continue1", 0.1, 0.45)];
        assert!(!scan(&l).iter().any(|h| h.line == 1));
        let l = vec![code("-----BEGIN RSA PRIVATE KEY-----"), line("hello", 0.1, 0.1)];
        assert!(!scan(&l).iter().any(|h| h.line == 1));
    }

    #[test]
    fn codes_give_up_their_inner_secrets() {
        for (t, want) in [
            ("otpauth://totp/Acme:dana?secret=JBSWY3DPEHPK3PXP&issuer=Acme", "JBSWY3DPEHPK3PXP"),
            ("OTPAUTH://totp/a?issuer=Acme&SECRET=JBSW%20Y3DP", "JBSW Y3DP"),
            ("otpauth-migration://offline?data=CjEKCkhl", "CjEKCkhl"),
            (r"WIFI:T:WPA;S:Blue;P:mar\;ma\\la\:de;;", r"mar;ma\la:de"),
            ("wifi:P:first-field;S:Blue;;", "first-field"),
            ("WIFI:S:a\nb;P:marmalade;;", "marmalade"),
            (r#"WIFI:S:P:x;P:"quoted1";;"#, "quoted1"),
            ("https://x.com/l?session_token=abc123&lang=en", "abc123"),
            ("https://dana:hunter2@x.com/", "hunter2"),
        ] {
            let v = code_secrets(t);
            assert!(v.iter().any(|x| x == want), "{t}: {v:?}");
        }
        assert!(!code_secrets("otpauth://totp/Acme:dana?secret=JBSWY3DPEHPK3PXP&issuer=Acme").iter().any(|v| v == "Acme"));
    }

    #[test]
    fn a_known_secret_is_found_however_it_is_spaced() {
        let find = |l: &[TextLine], v: &str| find_secret(l, v).into_iter().map(|(i, r)| (i, l[i].text[r].to_owned())).collect::<Vec<_>>();
        assert_eq!(find(&[line("Key: jbsw y3dp ehpk 3pxp ok", 0.1, 0.1)], "JBSWY3DPEHPK3PXP"), vec![(0, "jbsw y3dp ehpk 3pxp".to_owned())]);
        assert_eq!(find(&[line("JBSWY3DP-EHPK3PXP", 0.1, 0.1)], "JBSW Y3DP EHPK 3PXP"), vec![(0, "JBSWY3DP-EHPK3PXP".to_owned())]);
        // Split across rows, and with the letters OCR confuses.
        assert_eq!(find(&[line("key JBSWY3DP", 0.1, 0.1), line("EHPK3PXP", 0.1, 0.2), line("x", 0.1, 0.3)], "JBSWY3DPEHPK3PXP"),
                   vec![(0, "JBSWY3DP".to_owned()), (1, "EHPK3PXP".to_owned())]);
        assert_eq!(find(&[line("J8SWY3DPEHPK3PXP", 0.1, 0.1)], "JBSWY3DPEHPK3PXP").len(), 1);
        assert_eq!(find(&[line("é hunter22 é hunter22", 0.1, 0.1)], "hunter22").len(), 2);
        assert_eq!(find(&[line("hunter 2 miles", 0.1, 0.1)], "hunter22").len(), 0);
        assert_eq!(find(&[line("anything", 0.1, 0.1)], " - "), vec![]);
        // Too short to trust loosely.
        assert_eq!(find(&[line("see 77 25 rows", 0.1, 0.1)], "7725").len(), 0);
        assert_eq!(find(&[line("code 7725", 0.1, 0.1)], "7725").len(), 1);
    }
}
