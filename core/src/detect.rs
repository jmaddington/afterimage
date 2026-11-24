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
    /// What found it: a rule's name, "model", "extension"... Never the secret.
    pub why: &'static str,
}

macro_rules! re {
    ($name:ident, $p:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($p).expect("static regex"));
    };
}

re!(SSN, r"\b\d{3}[\s\-\u{2010}-\u{2015}\u{2212}.]{1,3}\d{2}[\s\-\u{2010}-\u{2015}\u{2212}.]{1,3}\d{4}\b");
re!(NINE, r"\b\d{9}\b");
re!(DIGIT_RUN, r"\d(?:[\s\-\u{2010}-\u{2015}\u{2212}.]{0,2}\d){11,}");
re!(CARD_PREFIX, r"^(?:3[47]\d{13}|(?:4\d|5[1-5]|2[2-7]|35|6[025])\d{14}|64[4-9]\d{13})$");
re!(CARD_GROUPS, r"\b\d{4}[\s\-.]{1,2}\d{4}[\s\-.]{1,2}\d{4}[\s\-.]{1,2}\d{1,7}\b|\b3[47]\d{2}[\s\-.]{1,2}\d{6}[\s\-.]{1,2}\d{5}\b");
re!(IBAN, r"\b[A-Z]{2}\d{2}(?:\s?[A-Z0-9]{4}){2,7}(?:\s?[A-Z0-9]{1,4})?\b");
re!(SSN_LABEL, r"(?i)\b(?:ssn|social\s+security|tax(?:payer)?\s*id|itin|tin)\b");
re!(FIN_LABEL, r"(?i)\b(?:routing|aba|account|acct|a/c|iban|sort\s*code|swift|bic|card|debit|credit)(?:\s*(?:number|num|no\.?|nbr|code|#))?(?:\s*\(?(?:required|optional)\)?)?\s*[:#]?");
re!(FIN_VALUE, r"[0-9Xx*•](?:[\s\-.]?[0-9Xx*•]){4,24}");
re!(CVV, r"(?i)\b(?:cvv2?|cvc2?|csc|security\s+code)\b\s*[:#]?\s*(\d{3,4})\b");
re!(CVV_LABEL, r"(?i)^\s*(?:cvv2?|cvc2?|csc|security\s+code)\s*[:#]?\s*$");
// key = value / key: value where the key names a secret.
re!(ASSIGN, r#"([A-Za-z0-9_.\-\[\]'"]+)\s*(:=|=>|=|:)\s*(["']?)([^\s"',;]+)"#);
// Where one pair of a query string ends and the next begins.
re!(NEXT_PAIR, r"&[A-Za-z0-9_.\-\[\]%]+=");
re!(NOT_A_VALUE_WHOLE, r"(?i)^(?:forgot|forgotten|reset|show|hide|sign\s*in|log\s*in|sign\s*up|continue|submit|cancel|save|remember|required|optional|must|characters?|enter|confirm|change|manager|generate|copy|username|email|user\s*name)\W*$");
re!(CODE_SHAPED, r"^[A-Z0-9]{4,}(?:-[A-Z0-9]{2,})+$|^[A-Z0-9]{8,}$");
re!(PATH_TOKEN, r"^[A-Za-z0-9_\-]{20,}$");
re!(SECRET_NEAR, r"(?i)pass|\bpin\b|code|secret|recovery|\bkey|token|ssn|social|combination");
// "Secret key   8f14e4..." laid out in columns, with nothing but space between.
re!(LABELLED, r"(?i)\b(?:api[\s_-]?key|secret(?:[\s_-]?key)?|client[\s_-]?secret|access[\s_-]?(?:key|token)|private[\s_-]?key|auth[\s_-]?token|token|password|passcode)\b\s{1,6}([A-Za-z0-9_\-+/=.~]{16,})");
re!(PLACEHOLDER, r"(?i)^<[^>]*>$|^your(?:[_\-](?:api|secret|token|key|password|here|access|client|id|value|own))+$|^\$\{[^}]*\}$|^\{\{[^}]*\}\}$|^(?-i:\$[A-Z_][A-Z0-9_]*)$|^\$\([^)]*\)?$|^%\([A-Za-z_]+\)s$|^changeme$|^placeholder$|^x{3,}$|^\.{3,}$");
re!(DOTTED, r"^(?:self|this|args|settings|os|config|cfg|env|opts|options|params|props|req|request|ctx|process|row|obj|data|state|user|form)(?:\.[A-Za-z_]\w*)+$");
// A call or a lookup: getpass.getpass(), os.environ["X"].
re!(CODE_START, r"^[A-Za-z_][\w.]*[(\[]");
re!(RECOVERY, r"(?i)\b(?:recovery|backup)\s+(?:codes?|keys?|phrases?)\b[ \t]*([:=])?[ \t]*");
// A row that starts by naming its own field is that field, not the value of the label above it.
re!(FIELD_START, r"(?i)^\s*(?:card|account|routing|name|member|e-?mail|user(?:name)?|phone|address|notes?|date|expir\w*|cvv|cvc|zip|city|state|country|first|last|company)(?:\s+(?:number|no\.?|name|code|date|address))?(?:\s*:\s*|\s+|$)");
re!(WIFI, r#"(?i)\bwi-?fi\b(?:\s+(?:password|passphrase|key|code))?\s*(?:\bis\b|:)\s*["']?([^\s"']+)"#);
re!(COMMAND, r"(?i)\b(mysql(?:dump|admin)?|curl|wget|http)\b");
re!(MYSQL_FLAG, r#"\s-p(?:'([^']+)(?:'|$)|"([^"]+)(?:"|$)|(\S+))"#);
re!(MYSQL_LOOSE, r"\s-p(\S+)");
re!(CURL_LOOSE, r#"\s(?:-u|--user|-U|--proxy-user)(?:=|\s+)["']?[^\s:"']+:(\S+)"#);
re!(CURL_FLAG, r#"\s(?:-u|--user|-U|--proxy-user)(?:=|\s+)(?:'[^':]*:([^']+)(?:'|$)|"[^":]*:([^"]+)(?:"|$)|[^\s:'"]+:([^\s'"]+))"#);
re!(PHONE_334, r"^\d{3}[\s.\-]\d{3}[\s.\-]\d{4}$");
re!(FIN_QUAL, r"(?i)\b(?:number|num|no\.?|nbr|code|iban|routing|aba|swift|bic|sort\s*code)\b|#");
// Long numbers that are plainly something else.
re!(NOT_A_CARD, r"(?i)\b(?:order|tracking|invoice|isbn|zoom|meeting|ticket|sku|serial|imei|parcel|shipment)\b\s*(?:id|no\.?|number)?\s*[:#]?\s*$");
re!(EMAIL, r"^[\w.+\-]+@[\w\-]+(?:\.[\w\-]+)+$");
re!(NAME, r"^[A-Z][a-z'’\-]+(?: [A-Z]\.?)?(?: [A-Z][a-z'’\-]+){1,2}$");
re!(PASS_PHRASE, r#"(?i)\b(?:password|passcode|passphrase|pwd|pin(?:\s+code)?)\s+(?:is|was)\s*:?\s*["'“‘]?([^\s"'”’]+)"#);
re!(PASS_LABEL, r"(?i)^\s*(?:(?:current|new|old|confirm|your|temporary|temp|admin|root|user|wi-?fi|wifi|network|master|login|account)\s+)*(?:password|passcode|pass\s*phrase|passphrase|pin|pin\s+code|secret(?:\s+key)?|api\s+key|access\s+(?:key|token|code)|token|recovery\s+(?:key|code)|client\s+secret|private\s+key|security\s+code)(?:\s+\(?(?:required|optional)\)?)?\s*[:*]?\s*$");
re!(FLAG, r#"(?i)(?:--?(?:password|passwd|pass|token|api-key|secret)|\bsshpass\s+-p|\bmysql\b.*\s-p)(?:[=\s]+)["']?([^\s"']+)"#);
re!(BEARER, r"(?i)\b(bearer|basic|token)\s+([A-Za-z0-9._~+/\-]{12,}=*)");
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

/// A phone number as people write one: grouped, or with a + or brackets.
pub fn phone_like(t: &str) -> bool {
    let t = t.trim();
    let digits = t.chars().filter(char::is_ascii_digit).count();
    let shaped = t.chars().all(|c| c.is_ascii_digit() || " .-()+".contains(c)) && t.chars().any(|c| " .-()+".contains(c));
    shaped && (7..=13).contains(&digits) && (t.starts_with(['+', '(', '0']) || PHONE_334.is_match(t))
}

/// Contact details the model sometimes reports as "other": a person wants to remember who they talked to.
/// AIDEV-NOTE: privacy-critical. Never overrules a finding the model called a password, key, SSN or account number.
pub fn keep_despite_model(value: &str, kind: SecretKind, near: &str) -> bool {
    // "Door code hint: ..." makes anything beside it suspect, in whatever letters it's drawn.
    if SECRET_NEAR.is_match(near) || SECRET_NEAR.is_match(&latinised(near, false).0) {
        return false;
    }
    let v = value.trim();
    let digits = v.chars().filter(char::is_ascii_digit).count();
    let phone = phone_like(v) && digits >= 10 && (v.starts_with(['+', '(']) || PHONE_334.is_match(v));
    kind == SecretKind::Other && (phone || EMAIL.is_match(v) || NAME.is_match(v))
}

/// The words of an identifier: `apiKey`, `APIKey`, `API_KEY` and `api-key` are all "api key".
fn words(key: &str) -> Vec<String> {
    let c: Vec<char> = key.chars().collect();
    let mut out = vec![String::new()];
    for (i, &ch) in c.iter().enumerate() {
        if !ch.is_alphanumeric() {
            out.push(String::new());
            continue;
        }
        let prev = if i > 0 { c[i - 1] } else { ' ' };
        let next_lower = c.get(i + 1).is_some_and(|n| n.is_lowercase());
        if ch.is_uppercase() && (prev.is_lowercase() || prev.is_ascii_digit() || prev.is_uppercase() && next_lower) {
            out.push(String::new());
        }
        out.last_mut().unwrap().extend(ch.to_lowercase());
    }
    out.retain(|w| !w.is_empty());
    out
}

// Words that make a name about a secret, not the secret: token_count, password_hash, api_key_header.
const ABOUT: &[&str] = &[
    "count", "counts", "num", "max", "min", "len", "length", "limit", "size", "type", "types", "id", "ids", "name", "names", "url", "uri", "path", "file", "dir",
    "header", "headers", "prefix", "suffix", "hash", "hashed", "enabled", "disabled", "required", "expiry", "expires", "expiration", "ttl", "timeout", "field", "label",
    "hint", "policy", "strength", "rules", "reset", "stdin", "env", "var", "param", "arg", "placeholder", "regex", "pattern", "format", "manager", "prompt", "index",
    "idx", "version", "algo", "algorithm", "bits", "rotation", "age", "help", "error", "valid", "invalid", "is", "has", "use", "show", "hide", "visible", "input",
];
const CODE_WORDS: &[&str] = &[
    "none", "null", "nil", "true", "false", "undefined", "str", "string", "int", "bool", "boolean", "bytes", "optional", "any", "self", "this", "await", "new",
    "required", "value", "bearer", "void", "char", "float", "object",
];

/// Does `key = value` assign a literal secret? Code that merely handles secrets (types, variables, calls) does not.
/// AIDEV-NOTE: privacy-critical. Every exemption is anchored to the whole value; when in doubt it is a secret.
fn assigned(key: &str, quoted: bool, v: &str, closed: bool) -> Option<SecretKind> {
    let w = words(key);
    // "pgpassword", "authtoken", "jwtsecret": the secret word ends a longer one. Short words must stand alone.
    let ends = |list: &[&str]| w.iter().position(|k| list.iter().any(|x| k.ends_with(x)));
    let is = |list: &[&str]| w.iter().position(|k| list.contains(&k.as_str()));
    let pair = |a: &str, b: &[&str]| w.windows(2).position(|p| p[0] == a && b.contains(&p[1].as_str())).map(|i| i + 1);
    // However the capitals fall ("APIkey"), the name run together still ends the same way.
    let joined = w.concat();
    let run = |list: &[&str]| list.iter().any(|x| joined.ends_with(x)).then(|| w.len() - 1);
    let password = ends(&["password", "passwords", "passwd", "pwd", "passcode", "passphrase"]).or(is(&["pass", "pin", "otp"])).or(run(&["password", "passwd", "passphrase", "passcode"]));
    let key_at = ends(&["secret", "secrets", "token", "apikey", "credential", "credentials"])
        .or(pair("api", &["key"])).or(pair("access", &["key"])).or(pair("private", &["key"])).or(pair("auth", &["key"]))
        .or(pair("recovery", &["key", "code"])).or(pair("backup", &["code"])).or((w == ["key"]).then_some(0))
        .or(run(&["apikey", "secretkey", "privatekey", "accesskey", "authkey", "secret", "token"]));
    let (kind, at) = match (password, key_at) {
        (Some(i), _) => (SecretKind::Password, i),
        (None, Some(i)) => (SecretKind::Key, i),
        _ => return None,
    };
    // token_count, password_hash, api_key_header: the name ends by saying it's about the secret, not the secret.
    if w.len() > at + 1 && ABOUT.contains(&w[w.len() - 1].as_str()) {
        return None;
    }
    if MASKED.is_match(v) || NOT_A_VALUE_WHOLE.is_match(v) || PLACEHOLDER.is_match(v) {
        return None;
    }
    let lower = v.to_lowercase();
    let n = v.chars().count();
    let plain = v.chars().all(|c| c.is_alphabetic() || c == '_');
    if !quoted {
        // A call, a lookup, a path through a well-known object, a keyword, or the variable of the same name.
        // A call or lookup either closes its bracket at the very end, or was cut off where its quoted argument began.
        // "Hunter(2" and "p(ss)w0rd" do neither.
        let call = CODE_START.is_match(v) && (closed || v.ends_with(['(', '[']));
        if call || DOTTED.is_match(v) || CODE_WORDS.contains(&lower.as_str()) || w.contains(&lower) || lower == w.join("_") {
            return None;
        }
        // An unquoted bare word is a variable when the name wants a key, or a type after a colon in code. As a
        // password it is taken at its word: "password = swordfish".
        if plain && kind == SecretKind::Key {
            return None;
        }
    }
    let ok = match kind {
        SecretKind::Password if is(&["pin", "otp"]).is_some() => n >= 4,
        SecretKind::Password => n >= 3,
        _ => n >= 8 && (v.chars().any(|c| c.is_ascii_digit()) || quoted && n >= 12 && !plain),
    };
    ok.then_some(kind)
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

fn push(hits: &mut Vec<Hit>, line: usize, range: Range<usize>, kind: SecretKind, why: &'static str) {
    if range.start < range.end {
        hits.push(Hit { line, range, kind, why });
    }
}

/// Detect within a single line, no layout context. Also used for window titles and URLs.
pub fn scan_text(text: &str, line: usize, hits: &mut Vec<Hit>) {
    if text.is_ascii() {
        return scan_latin(text, line, hits);
    }
    // Every reading: looking through lookalikes may only ever add.
    scan_latin(text, line, hits);
    for ze in [false, true] {
        let (t, at) = latinised(text, ze);
        let mut found = Vec::new();
        scan_latin(&t, line, &mut found);
        hits.extend(found.into_iter().map(|h| Hit { range: at[h.range.start]..at[h.range.end], ..h }));
    }
}

fn scan_latin(text: &str, line: usize, hits: &mut Vec<Hit>) {
    use SecretKind::*;
    for m in SSN.find_iter(text) {
        push(hits, line, m.range(), Ssn, "ssn");
    }
    if SSN_LABEL.is_match(text) {
        for m in NINE.find_iter(text) {
            push(hits, line, m.range(), Ssn, "ssn");
        }
    }
    for m in CARD_GROUPS.find_iter(text) {
        let d: String = m.as_str().chars().filter(char::is_ascii_digit).collect();
        let real = CARD_PREFIX.is_match(&d) && luhn(&d);
        // A tracking number runs on past where a card would stop: another short group follows (not an expiry date)
        // and the whole is too long. A label can also say it's something else, unless the number checks out as a card.
        let rest = &text[m.end()..];
        let after = rest.trim_start_matches([' ', '-', '.']);
        let group = after.chars().take_while(char::is_ascii_digit).count();
        let run_on = rest.len() - after.len() <= 2 && (2..=4).contains(&group) && !after[group..].starts_with('/') && d.len() + group > 19;
        if real || !run_on && !NOT_A_CARD.is_match(&text[..m.start()]) {
            push(hits, line, m.range(), Financial, "card");
        }
    }
    for m in DIGIT_RUN.find_iter(text) {
        let (at, d): (Vec<usize>, String) = m.as_str().char_indices().filter(|x| x.1.is_ascii_digit()).unzip();
        if (13..=19).contains(&d.len()) && luhn(&d) && !NOT_A_CARD.is_match(&text[..m.start()]) {
            push(hits, line, m.range(), Financial, "card");
        }
        // A stray digit beside a card must not hide it; inside a longer run only real card shapes count.
        for n in [15, 16] {
            for i in 0..(d.len() + 1).saturating_sub(n) {
                let w = &d[i..i + n];
                if CARD_PREFIX.is_match(w) && luhn(w) {
                    push(hits, line, m.start() + at[i]..m.start() + at[i + n - 1] + 1, Financial, "card");
                }
            }
        }
    }
    for m in IBAN.find_iter(text) {
        let compact: String = m.as_str().split_whitespace().collect();
        if compact.len() >= 15 && compact.chars().filter(char::is_ascii_digit).count() >= 8 {
            push(hits, line, m.range(), Financial, "iban");
        }
    }
    for label in FIN_LABEL.find_iter(text) {
        // "Order number 88213" has no financial label; "Account number 4417 2280 9931" does.
        if let Some(v) = FIN_VALUE.find_at(text, label.end()) {
            let digits = v.as_str().chars().filter(char::is_ascii_digit).count();
            // A bare "card" or "account" is as likely a contact card or an account manager: the number must follow at
            // once, be long, and not be a phone number.
            let named = FIN_QUAL.is_match(&text[label.start()..v.start()]);
            let gap = v.start() - label.end();
            if named && gap <= 24 && digits >= 4 || !named && gap <= 2 && digits >= 8 && !phone_like(v.as_str()) {
                push(hits, line, v.range(), Financial, "account");
            }
        }
    }
    for c in CVV.captures_iter(text) {
        push(hits, line, c.get(1).unwrap().range(), Financial, "cvv");
    }
    // A name that isn't a secret's gives up only itself: its value is looked at again as a name in its own right,
    // so "user=dana&password=..." and "Name: password: ..." are both found.
    let mut from = 0;
    while let Some(c) = ASSIGN.captures_at(text, from) {
        let (sep, v) = (c.get(2).unwrap(), c.get(4).unwrap());
        let quoted = !c[3].is_empty();
        let judge = |whole: &str| {
            let balanced = |o: char, c: char| whole.matches(o).count() == whole.matches(c).count();
            let closed = !quoted && whole.ends_with([')', ']']) && balanced('(', ')') && balanced('[', ']');
            let trimmed = if quoted { whole } else { whole.trim_end_matches([')', ']', '}', '.']) };
            // A real call gives up its closing bracket and is judged as one; anything else keeps every character.
            match assigned(&c[1], quoted, trimmed, closed) {
                None if closed => None,
                None => None,
                Some(k) => Some(k),
            }
        };
        // In a query string the value stops where the next pair starts; anywhere else "&" is just a character.
        // Both readings are tried, and if either is a secret the whole of it goes.
        let seg = NEXT_PAIR.find(v.as_str()).filter(|_| !quoted).map(|n| &v.as_str()[..n.start()]);
        match seg.and_then(judge).or_else(|| judge(v.as_str())) {
            Some(kind) => {
                let mut end = v.end();
                // "password: correct horse battery staple": a passphrase has spaces. Quoted, it runs to the closing
                // quote; unquoted, to the end of the line (or, if that is very long, not at all: it's prose).
                let rest = text[end..].split('\n').next().unwrap_or("");
                if kind == SecretKind::Password {
                    if quoted {
                        end += rest.find(&c[3]).unwrap_or(rest.trim_end().len());
                    } else if rest.split_whitespace().count() <= 24 {
                        end += rest.trim_end().len();
                    }
                }
                // The word by itself too, so it is recognised wherever else it appears; and the rest of the line is
                // still read for secrets of its own.
                push(hits, line, v.range(), kind, "assignment");
                if end > v.end() {
                    push(hits, line, v.start()..end, kind, "assignment");
                }
                from = v.end();
            }
            None => from = sep.end(),
        }
    }
    for l in RECOVERY.find_iter(text) {
        let tail = text[l.end()..].split('\n').next().unwrap_or("").trim_end();
        let first = tail.split_whitespace().next().unwrap_or("");
        let colon = l.as_str().trim_end().ends_with([':', '=']);
        // With no colon it may be a sentence about recovery keys; a code has digits or is all capitals.
        if !tail.is_empty() && (colon || first.len() >= 6 && first.chars().any(|c| c.is_ascii_digit()) || CODE_SHAPED.is_match(first)) {
            push(hits, line, l.end()..l.end() + tail.len(), Key, "label");
        }
    }
    for cmd in COMMAND.captures_iter(text) {
        let from = cmd.get(0).unwrap().end();
        let tail_end = text[from..].find('\n').map_or(text.len(), |i| from + i);
        // Careful and loose readings both: a stray quote must only ever add to what is hidden.
        let flags: [&Regex; 2] = if cmd[1].to_lowercase().starts_with("mysql") { [&MYSQL_FLAG, &MYSQL_LOOSE] } else { [&CURL_FLAG, &CURL_LOOSE] };
        for f in flags.iter().flat_map(|re| re.captures_iter(&text[from..tail_end])) {
            if let Some(v) = (1..f.len()).find_map(|i| f.get(i)) {
                push(hits, line, from + v.start()..from + v.end(), Password, "flag");
            }
        }
    }
    for c in WIFI.captures_iter(text) {
        let v = c.get(1).unwrap();
        // "the wifi is BlueHeron" names a network; a password has a digit in it.
        if v.as_str().chars().any(|c| c.is_ascii_digit()) && v.as_str().chars().count() >= 6 {
            push(hits, line, v.range(), Password, "phrase");
        }
    }
    for c in LABELLED.captures_iter(text) {
        let v = c.get(1).unwrap();
        if v.as_str().chars().any(|c| c.is_ascii_digit()) && !PLACEHOLDER.is_match(v.as_str()) {
            push(hits, line, v.range(), if c[0].to_lowercase().starts_with("pass") { Password } else { Key }, "label");
        }
    }
    for (re, why) in [(&*PASS_PHRASE, "phrase"), (&*FLAG, "flag")] {
        for c in re.captures_iter(text) {
            let v = c.get(1).unwrap();
            if plausible_value(v.as_str()) {
                push(hits, line, v.range(), Password, why);
            }
        }
    }
    for c in BEARER.captures_iter(text) {
        let v = c.get(2).unwrap();
        // "token authentication" is two words, not a credential; after "Bearer" or "Basic" anything is.
        if !c[1].eq_ignore_ascii_case("token") || v.as_str().chars().any(|c| c.is_ascii_digit()) {
            push(hits, line, v.range(), Key, "bearer");
        }
    }
    for c in URL_CRED.captures_iter(text) {
        push(hits, line, c.get(1).unwrap().range(), Password, "url-login");
    }
    for m in QR_SECRET.find_iter(text) {
        push(hits, line, m.range(), Key, "setup-code");
    }
    for m in KNOWN.find_iter(text) {
        push(hits, line, m.range(), Key, "known-key");
    }
    for m in LONG_TOKEN.find_iter(text) {
        if looks_random(m.as_str()) {
            push(hits, line, m.range(), Key, "random");
        }
    }
}

/// Detect across a whole screen, using layout to pair standalone labels with their values.
pub fn scan(lines: &[TextLine]) -> Vec<Hit> {
    let mut hits = Vec::new();
    let mut in_pem = false;
    let latin: Vec<String> = lines.iter().map(|l| latinised(&l.text, false).0).collect();
    for (i, l) in lines.iter().enumerate() {
        let t = &latin[i];
        if l.code {
            if !code_is_harmless(&l.text) {
                hits.push(Hit { line: i, range: 0..l.text.len(), kind: SecretKind::Key, why: "code" });
            }
            continue;
        }
        if PEM_BEGIN.is_match(t) {
            in_pem = true;
        }
        if in_pem {
            push(&mut hits, i, 0..l.text.len(), SecretKind::Key, "private-key");
        }
        if PEM_END.is_match(t) {
            in_pem = false;
        }
        scan_text(&l.text, i, &mut hits);

        if PASS_LABEL.is_match(t) {
            for j in neighbours(lines, i) {
                let v = &latin[j];
                if (plausible_value(v) || plausible_value(&lines[j].text)) && !PASS_LABEL.is_match(v) {
                    // A row that names its own field keeps its label; what follows goes, unless a rule already has it.
                    // `v` is the Latin reading; offsets into it are not offsets into what was read.
                    let at = latinised(&lines[j].text, false).1;
                    let start = match FIELD_START.find(v) {
                        Some(f) => {
                            let mut own = Vec::new();
                            scan_text(&lines[j].text, j, &mut own);
                            if !own.is_empty() || f.end() >= v.len() {
                                continue;
                            }
                            at[f.end()]
                        }
                        None => 0,
                    };
                    push(&mut hits, j, start..lines[j].text.len(), SecretKind::Password, "label");
                }
            }
        }
        if CVV_LABEL.is_match(t) {
            for j in neighbours(lines, i) {
                let v = latin[j].trim();
                if v.len() <= 4 && v.chars().all(|c| c.is_ascii_digit()) {
                    push(&mut hits, j, 0..lines[j].text.len(), SecretKind::Financial, "label");
                }
            }
        }
        let ssn_label = SSN_LABEL.is_match(t);
        let fin_label = FIN_LABEL.is_match(t) && t.split_whitespace().count() <= 5;
        if ssn_label || fin_label {
            for j in neighbours(lines, i) {
                let v = &latin[j];
                let digits = v.chars().filter(char::is_ascii_digit).count();
                let compact = v.chars().filter(|c| !c.is_whitespace()).count().max(1);
                let mostly = digits * 10 >= compact * 6;
                if ssn_label && digits >= 4 && digits <= 9 && mostly {
                    push(&mut hits, j, 0..lines[j].text.len(), SecretKind::Ssn, "label");
                } else if fin_label && (6..=24).contains(&digits) && mostly && !v.contains(['$', '€', '£', '/']) && (FIN_QUAL.is_match(t) || !phone_like(v)) {
                    push(&mut hits, j, 0..lines[j].text.len(), SecretKind::Financial, "label");
                }
            }
        }
    }
    extend_wrapped(lines, &mut hits);
    snap(lines, &mut hits);
    hits
}

/// Make every hit a whole number of characters on a line that exists. Widens, never narrows.
/// AIDEV-NOTE: a range that splits a character would panic wherever the text is sliced, and a panic stops all saving.
pub fn snap(lines: &[TextLine], hits: &mut Vec<Hit>) {
    hits.retain_mut(|h| {
        let Some(t) = lines.get(h.line).map(|l| l.text.as_str()) else { return false };
        let mut a = h.range.start.min(t.len());
        let mut b = h.range.end.min(t.len());
        while !t.is_char_boundary(a) {
            a -= 1;
        }
        while !t.is_char_boundary(b) {
            b += 1;
        }
        h.range = a..b;
        // A code that couldn't be read has no text; its hit stands for its whole box.
        a < b || t.is_empty()
    });
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
                    && o.text.trim().chars().count() >= 6
                    && o.text.trim().chars().map(latin).all(|c| c.is_ascii_alphanumeric() || "+/=_-.".contains(c))
                    && o.text.chars().map(latin).any(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
            });
            let Some((j, o)) = next else { break };
            let h = Hit { line: j, range: 0..o.text.len(), kind: SecretKind::Key, why: "wrapped" };
            if hits.iter().any(|x| x.line == j && x.range == h.range && x.why == "wrapped") {
                break;
            }
            hits.push(h);
            at = j;
        }
    }
}

/// Replace every hit with a marker; used for indexed text, titles and URLs.
pub fn mask(text: &str, ranges: &[Range<usize>]) -> String {
    // Widened to whole characters and kept inside the text, whatever was asked.
    let fit = |x: &Range<usize>| {
        let (mut a, mut b) = (x.start.min(text.len()), x.end.min(text.len()));
        while !text.is_char_boundary(a) {
            a -= 1;
        }
        while !text.is_char_boundary(b) {
            b += 1;
        }
        a..b
    };
    let mut r: Vec<Range<usize>> = ranges.iter().map(fit).collect();
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
    // A long run of letters and digits in a path is a reset link or a share token, not a page name.
    let path: Vec<String> = u.path().split('/').map(|s| if PATH_TOKEN.is_match(s) && s.chars().any(|c| c.is_ascii_digit()) { "[hidden]".into() } else { s.to_owned() }).collect();
    u.set_path(&path.join("/"));
    mask_text(&u.as_str().replace("%5Bhidden%5D", "[hidden]"))
}

// The raw payload must be just a plain host and perhaps one everyday word. Judged as written, before any URL parser
// can tidy away "..", "@" or a trailing "?".
re!(PLAIN_ADDRESS, r"(?i)^https?://[a-z][a-z-]{0,17}(?:\.[a-z][a-z-]{0,17}){1,3}(?:/(?:menu|about|home|contact|info|app|apps|download|help|support|events|careers|jobs|news|map|hours|shop|store|wifi|feedback|survey|welcome))?/?$");

/// Deny by default: a code stays visible only when it is a bare web address carrying nothing private.
/// AIDEV-NOTE: privacy-critical. Codes hold 2FA seeds, Wi-Fi keys, login tokens; OCR can't see any of them.
fn code_is_harmless(payload: &str) -> bool {
    let mut found = Vec::new();
    scan_text(payload, 0, &mut found);
    PLAIN_ADDRESS.is_match(payload) && found.is_empty()
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

/// The Latin letter a character is drawn like. Vision returns Cyrillic or Greek twins for isolated capitals.
fn latin(c: char) -> char {
    if matches!(c, '\u{ff01}'..='\u{ff5e}') {
        return char::from_u32(c as u32 - 0xfee0).unwrap_or(c);
    }
    const FROM: &str = "АВЕЅІЈКМНОРСТХУЬаеѕіјкморсхуһԁΑΒΕΖΗΙΚΜΝΟΡΤΥΧοκν";
    const TO: &str = "ABESIJKMHOPCTXYbaesijkmopcxyhdABEZHIKMNOPTYXokv";
    if c.is_ascii() { c } else { FROM.chars().position(|f| f == c).map_or(c, |i| TO.as_bytes()[i] as char) }
}

/// Text with lookalikes made Latin, and the original byte offset of every byte boundary in it.
/// `ze` reads the Cyrillic letter as the digit it also looks like.
fn latinised(text: &str, ze: bool) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(text.len());
    let mut at = Vec::with_capacity(text.len() + 1);
    for (i, c) in text.char_indices() {
        let l = if ze && matches!(c, '\u{0417}' | '\u{0437}') { '3' } else { latin(c) };
        out.push(l);
        at.extend(std::iter::repeat_n(i, l.len_utf8()));
    }
    at.push(text.len());
    (out, at)
}

/// Case, spacing and the letters OCR confuses don't make it a different secret.
fn fold(c: char) -> char {
    match c {
        '\u{0437}' => '3', // Cyrillic ze; a letter, so only a twin of the digit when hunting a known value.
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
        .flat_map(|(i, c)| latin(c).to_lowercase().map(move |l| (fold(l), i..i + c.len_utf8())))
}

/// Every place a known secret appears, however it is spaced, grouped or broken across rows.
pub fn find_secret(lines: &[TextLine], value: &str) -> Vec<(usize, Range<usize>)> {
    let needle: Vec<char> = folded(value).map(|x| x.0).collect();
    let mut out: Vec<(usize, Range<usize>)> = Vec::new();
    if needle.len() < 6 {
        // Too short to match loosely: as a word of its own, spacing and all.
        let v = latinised(value.trim(), true).0.to_ascii_lowercase();
        let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
        for (i, l) in lines.iter().enumerate().filter(|_| v.chars().any(char::is_alphanumeric)) {
            let (t, at) = latinised(&l.text, true);
            let t = t.to_ascii_lowercase();
            for (s, m) in t.match_indices(v.as_str()) {
                let e = s + m.len();
                if !word(t[..s].chars().next_back()) && !word(t[e..].chars().next()) {
                    out.push((i, at[s]..at[e]));
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
    if range.start > range.end || !line.text.is_char_boundary(range.start) || !line.text.is_char_boundary(range.end) {
        return line.rect;
    }
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
        for t in ["https://x.com/j/abc123", "https://x.com/i/8f14e45f", "https://x.com/550e8400-e29b-41d4-a716-446655440000", "https://x.com/JBSWYDPEHPKPXP", "https://x.com/aBcDeF",
                  "https://x.com/4417", "https://x.com/%61bc", "https://x.com/jbswydpehpkpxpjbswydpehpkpxp", "https://q7x9k2m4p8w3z6v1b5n0.x.com/", "https://x.com:8443/"] {
            assert!(!scan(&[code(t)]).is_empty(), "{t}");
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

    /// Vision reads isolated capitals as Cyrillic or Greek twins ("ЕНРК ЗРХР" for EHPK 3PXP).
    #[test]
    fn lookalike_letters_do_not_hide_a_secret() {
        for (t, rest) in [
            ("АКIАIОSFОDNN7ЕХАМРLЕ", ""),
            ("é ТОКЕN=zq81хуwerty ok", "é ТОКЕN= ok"),
            ("раsswоrd: hunter2", "раsswоrd: "),
            ("ЅЅN 219099999", "ЅЅN "),
            ("ｐａｓｓｗｏｒｄ： hunter2", "ｐａｓｓｗｏｒｄ： "),
            ("４１１１ １１１１ １１１１ １１１１", ""),
        ] {
            assert_eq!(masked(t).replace("[hidden]", ""), rest, "{t}");
        }
        for c in ('\u{80}'..='\u{ffff}').filter(|c| latin(*c) != *c) {
            assert!(latin(c).is_ascii(), "{c}");
        }
        assert_eq!(masked("Привет, СССР и Москва"), "Привет, СССР и Москва");
        let l = vec![line("Раsswоrd", 0.1, 0.10), line("синяя-heron-7", 0.1, 0.14)];
        assert!(scan(&l).iter().any(|x| x.line == 1 && x.range == (0..l[1].text.len())));
        let found = find_secret(&[line("type JBSW Y3DP ЕНРК ЗРХР", 0.1, 0.1)], "JBSWY3DPEHPK3PXP");
        assert_eq!(found, vec![(0, 5.."type JBSW Y3DP ЕНРК ЗРХР".len())]);
    }

    #[test]
    fn looking_through_lookalikes_only_ever_adds() {
        for (t, secret) in [
            ("password: hunter2（旧）", "hunter2"),
            ("password: abc；def9", "abc"),
            ("З4111111111111111", "4111111111111111"),
            ("4111111111111111З", "4111111111111111"),
            ("password： ｈｕｎｔｅｒ２", "ｈｕｎｔｅｒ２"),
        ] {
            assert!(!masked(t).contains(secret), "{t}: {}", masked(t));
        }
        for v in ["hunter2（旧）", "hunter2；x"] {
            let l = vec![line("Password", 0.1, 0.10), line(v, 0.1, 0.14)];
            assert!(scan(&l).iter().any(|x| x.line == 1), "{v}");
        }
    }

    #[test]
    fn a_card_inside_a_longer_run_is_found() {
        for t in ["9 4111111111111111", "41111111111111117", "ref 378282246310005 2"] {
            assert!(masked(t).contains("[hidden]"), "{t}");
        }
        assert_eq!(masked("at 1790512345678"), "at 1790512345678");
    }

    #[test]
    fn short_secrets_match_through_lookalikes() {
        assert_eq!(find_secret(&[line("pin КЕУ9 ok", 0.1, 0.1)], "key9"), vec![(0, 4..4 + "КЕУ9".len())]);
        assert_eq!(find_secret(&[line("XKEY9", 0.1, 0.1)], "key9"), vec![]);
    }

    #[test]
    fn code_addresses_are_judged_as_written() {
        for t in ["https://x.com/a/../menu", "https://x.com/./menu", "https://x.com\\menu", "https:x.com/menu", "https://@x.com/menu", "https://x.com/?", "https://x.com/#",
                  "https://x.com/correct-horse-battery", "https://x.com/kqzvxwpt", "https://x.com/menu/kqzv", "https://a.b.c.d.e.com/", "https://x7.com/", "https://аррӏе.com/", "https://x.com:443/"] {
            assert!(!scan(&[code(t)]).is_empty(), "{t}");
        }
        for t in ["https://example.com", "https://www.example.co.uk/Menu/", "HTTP://EXAMPLE.COM/ABOUT"] {
            assert_eq!(scan(&[code(t)]), vec![], "{t}");
        }
    }

    #[test]
    fn cyrillic_ze_is_also_read_as_three() {
        assert!(!masked("SSN 21З-09-9999").contains("9999"));
        assert!(!masked("4111 1111 1111 1З11").contains("4111"));
        assert_eq!(find_secret(&[line("pin Зху9 ok", 0.1, 0.1)], "3xy9").len(), 1);
    }

    #[test]
    fn cards_are_found_in_runs_of_any_length() {
        let t = masked("2026 11 24 07 18 4111111111111111 55 1234567890");
        assert!(!t.contains("4111111111111111") && t.contains("2026") && t.contains("1234567890"), "{t}");
        for n in ["3530111333300000", "6200000000000005", "6445644564456445"] {
            assert!(masked(&format!("{n}9")).contains("[hidden]"), "{n}");
        }
    }

    #[test]
    fn every_hit_says_why() {
        let why = |t: &str| { let mut h = Vec::new(); scan_text(t, 0, &mut h); h.into_iter().map(|h| h.why).collect::<Vec<_>>() };
        for (t, w) in [
            ("SSN: 219-09-9999", "ssn"), ("Card 4111 1111 1111 1111", "card"), ("GB82 WEST 1234 5698 7654 32", "iban"), ("Account number: 4417 2280 9931", "account"),
            ("CVV 737", "cvv"), ("DB_PASSWORD=correct-horse-9", "assignment"), ("my password is hunter2", "phrase"), ("curl --token Zx81qwerty", "flag"),
            ("Authorization: Bearer abcdef1234567890abcd", "bearer"), ("postgres://app:s3cr3t!@db.local/x", "url-login"), ("WIFI:S:x;P:marmalade;;", "setup-code"),
            ("ghp_abcdefghijklmnop1234", "known-key"), ("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "random"),
        ] {
            assert!(why(t).contains(&w), "{t}: {:?}", why(t));
        }
        let of = |l: &[TextLine], i: usize| scan(l).into_iter().find(|h| h.line == i).map(|h| h.why);
        assert_eq!(of(&[line("Password", 0.1, 0.10), line("blue-heron-7", 0.1, 0.14)], 1), Some("label"));
        assert_eq!(of(&[line("-----BEGIN RSA PRIVATE KEY-----", 0.1, 0.1), line("MIIabc", 0.1, 0.2)], 1), Some("private-key"));
        assert_eq!(of(&[code("")], 0), Some("code"));
        let l = vec![line("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0", 0.1, 0.10), line("9dGhpcyBpcyBub3Q0IHJlYWw", 0.1, 0.12)];
        assert_eq!(scan(&l).into_iter().filter(|h| h.line == 1).map(|h| h.why).find(|w| *w == "wrapped"), Some("wrapped"));
    }

    // ---- from real use, 2026-11-24: code, names and phone numbers must stay readable; keys must not ----
    #[test]
    fn code_is_left_alone() {
        for t in [
            "mapping = {\"a\": 1}", "spacing=12", "padding: 4px 8px;", "typing: List[int]", "option=--verbose", "desktop: true", "\"spinner\": \"dots\",", "inputs: [x, y]",
            "keyboard = Keyboard()", "monkey=patch", "shipping: standard", "max_tokens=2000,", "num_tokens: 512", "token_count = len(tokens)", "tokens = tokenizer.encode(text)",
            "password = None", "password: str", "password: Optional[str] = None", "self.password = password", "password=args.password,", "token = os.environ[\"API_TOKEN\"]",
            "api_key = os.getenv(\"OPENAI_API_KEY\")", "secret = settings.secret", "api_key=api_key,", "token: string;", "const token = await getToken();", "password = getpass.getpass()",
            "if password == confirm:", "key = row.key", "key: value", "primary_key=True", "foreign_key: users.id", "pin = 4", "keys = list(d.keys())", "def set_password(self, password: str) -> None:",
            "token_type: bearer", "passport: required", "secretary = \"Dana\"", "private_key_path = \"~/.ssh/id_ed25519\"", "API_KEY_HEADER = \"X-Api-Key\"", "password_min_length = 12",
            "return {\"token\": token, \"expires\": 3600}", "client_secret=client_secret", "--password-stdin", "token_url = \"https://example.com/oauth/token\"", "sort_key = lambda x: x.name",
            "from typing import Optional", "pinned: false", "otp_enabled = true", "password_hash = bcrypt.hash(password)", "SECRET_KEY = env(\"SECRET_KEY\")", "export TOKEN=$(cat token.txt)",
            "password = input(\"Password: \")", "\"api_key\": \"${API_KEY}\",", "api_key: <your-api-key>", "token=YOUR_TOKEN_HERE", "password: ********", "kCGWindowListOptionOnScreenOnly",
        ] {
            assert_eq!(masked(t), t, "over-hidden");
        }
    }

    #[test]
    fn people_and_phone_numbers_are_left_alone() {
        for t in [
            "Dana Whitfield  (415) 555-0142", "Call Sam at 415-555-0142", "+1 415 555 0142", "Contact card  415 555 0142", "Account manager: Priya Raman 415.555.0142",
            "Card: thanks for the flowers, 2026", "Credit: Photo by Sam Lee 2024", "dana.whitfield@example.com", "Tel 020 7946 0958", "Fax: +44 20 7946 0958", "Mobile 07700 900123",
            "Account rep  Jordan Blake", "Invoice 2026-000431", "Tracking 9400 1000 0000 0000 0000 00", "Order #1002003004005", "ISBN 978-3-16-148410-0", "Zoom ID 845 2231 9907",
        ] {
            assert_eq!(masked(t), t, "over-hidden");
        }
        // Under a bare "Account" or "Card" heading, a phone number is still a phone number.
        for (label, v) in [("Account", "(415) 555-0142"), ("Card", "415-555-0142"), ("Account", "+1 415 555 0142")] {
            let l = vec![line(label, 0.1, 0.10), line(v, 0.1, 0.14)];
            assert!(!scan(&l).iter().any(|h| h.line == 1), "{label} / {v}");
        }
    }

    #[test]
    fn keys_are_hidden_however_they_are_laid_out() {
        for (t, secret) in [
            ("Secret key   8f14e45fceea167a5a36dedd4bea2543", "8f14e45f"), ("API key: 3b9d7c1e5f2a4d6b8c0e1f3a5b7d9c2e", "3b9d7c"), ("Client secret  q8Zr2LmN0pXv7KdT4hBs", "q8Zr2"),
            ("api_secret = \"k3j4h5g6f7d8s9a0q1w2e3r4\"", "k3j4h5"), ("OPENAI_API_KEY=sk-proj-abcdefghijklmnop1234", "sk-proj"), ("token: 'ghs_abcdefghijklmnop1234'", "ghs_"),
            ("\"apiKey\": \"a1b2c3d4e5f6a7b8c9d0e1f2\",", "a1b2c3"), ("Authorization: Token 9944b09199c62bcf9418ad846dd0e4bbdfc6ee4b", "9944b0"), ("X-Api-Key: 7f3d9a2b5c8e1f4a6d0b3c7e9f2a5d8b", "7f3d9a"),
            ("password = \"swordfish\"", "swordfish"), ("password: Tr0ub4dor&3", "Tr0ub4"), ("STRIPE_SECRET=rk_live_abcdefghij1234", "rk_live"), ("access token  eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sig", "eyJ"),
            ("secret_access_key: wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "wJalr"), ("key=AIzaSyA1234567890abcdefghijklmnopqrstu", "AIza"), ("Webhook secret whsec_abcdefghij1234", "whsec"),
            ("xoxb-123456789012-abcdefghijkl", "xoxb"), ("sk-ant-api03-abcdefghijklmnopqrstuvwx", "sk-ant"), ("sk-or-v1-0123456789abcdef0123456789abcdef", "sk-or"), ("pin: 4821", "4821"),
            ("app.config['SECRET_KEY'] = 'f3a9c1d7e5b24a68'", "f3a9c1"), ("Bearer token   mF_9.B5f-4.1JqM", "mF_9"),
        ] {
            assert!(!masked(t).contains(secret), "missed in {t:?}: {}", masked(t));
        }
    }

    // ---- security review of the rewrite, 2026-11-24 ----
    #[test]
    fn a_secret_is_not_swallowed_by_what_comes_before_it() {
        for (t, secret) in [
            ("user=dana&password=hunter22", "hunter22"), ("https://x.com/cb?state=abc&access_token=Zx81qwerty99", "Zx81qwerty99"), ("Name: password: x1y2z3", "x1y2z3"),
            ("a=b password=swordfish9", "swordfish9"), ("grant_type=client_credentials&client_secret=q8Zr2LmN0pXv", "q8Zr2LmN0pXv"),
        ] {
            assert!(!masked(t).contains(secret), "{t}: {}", masked(t));
        }
    }

    #[test]
    fn secret_names_are_recognised_inside_longer_names() {
        for (t, secret) in [
            ("PGPASSWORD=swordfish", "swordfish"), ("authtoken=Zx81qwerty99", "Zx81"), ("jwtSecret: \"k3j4h5g6f7d8\"", "k3j4"), ("APIKey=a1b2c3d4e5f6", "a1b2"),
            ("DB_PASSWORD=swordfish", "swordfish"), ("password = swordfish", "swordfish"), ("MYSQL_PWD=opensesame", "opensesame"), ("passwords: tuba41 heron7", "tuba41"),
            ("Recovery code: 4821-9937-1120", "4821"), ("backup code 77291034", "77291034"), ("password=first.last", "first.last"), ("password=p(ss)w0rd", "p(ss)w0rd"),
            ("password=your-eyes-only-9", "your-eyes"), ("api_key=abc<def>12345", "abc<def>"),
        ] {
            assert!(!masked(t).contains(secret), "{t}: {}", masked(t));
        }
    }

    #[test]
    fn every_financial_label_on_a_line_is_considered() {
        for (t, secret) in [("Account manager Priya, account number 4417 2280 9931", "9931"), ("Card on file. Routing number: 021000021", "021000021")] {
            assert!(!masked(t).contains(secret), "{t}: {}", masked(t));
        }
    }

    #[test]
    fn the_model_is_overruled_only_about_contact_details_it_called_other() {
        use SecretKind::*;
        for (v, k, keep) in [
            ("(415) 555-0142", Other, true), ("+1 415 555 0142", Other, true), ("415-555-0142", Other, true), ("dana@example.com", Other, true), ("Dana Whitfield", Other, true),
            ("415-555-0142", Password, false), ("Dana Whitfield", Password, false), ("dana@example.com", Key, false), ("219-09-9999", Ssn, false), ("0142 5550 4155", Financial, false),
            ("020 7946", Other, false), ("marmalade-tuba-41", Other, false),
        ] {
            assert_eq!(keep_despite_model(v, k, "Call them back tomorrow"), keep, "{v} {k:?}");
        }
    }

    #[test]
    fn cards_survive_what_is_around_them() {
        for t in ["4111 1111 1111 1111 08/29", "4111 1111 1111 1111 123", "Order 4111111111111111", "Invoice 4111 1111 1111 1111"] {
            assert!(masked(t).contains("[hidden]"), "{t}");
        }
    }

    #[test]
    fn authorization_values_need_no_digit() {
        assert!(!masked("Authorization: Bearer abcdefghijklmnopqrst").contains("abcdefgh"));
        assert!(!masked("Authorization: Basic dXNlcjpwYXNzd29yZA==").contains("dXNl"));
        assert_eq!(masked("token authentication is required"), "token authentication is required");
    }

    #[test]
    fn review_round_two() {
        // An ampersand inside a password is part of it.
        for (t, secret) in [("password: Tr0ub4dor&3", "&3"), ("password=ab&cd1234", "cd1234"), ("pw=x&password=\"a&b=c9\"", "b=c9")] {
            assert!(!masked(t).contains(secret), "{t}: {}", masked(t));
        }
        // Everything after a recovery label goes, short words too.
        assert_eq!(masked("Recovery codes: 4821-9937 ab12 zz"), "Recovery codes: [hidden]");
        assert_eq!(masked("Recovery phrase: witch collapse practice feed"), "Recovery phrase: [hidden]");
        assert_eq!(masked("The recovery key is kept in the safe"), "The recovery key is kept in the safe");
        // Placeholders are whole values of a known shape, nothing looser.
        for (t, secret) in [("password=your-eyes-only", "eyes"), ("password=$ecretPa55", "ecret"), ("password=hunter2...", "hunter2"), ("token=your_9f8e7d6c5b4a", "9f8e"), ("password=%(x", "%(x")] {
            assert!(!masked(t).contains(secret), "{t}: {}", masked(t));
        }
        // A bracket in a password doesn't make it a function call.
        for (t, secret) in [("password=Hunter(2", "Hunter"), ("password=a[1]b", "a[1]b"), ("password={x9", "x9")] {
            assert!(!masked(t).contains(secret), "{t}: {}", masked(t));
        }
        for (t, secret) in [("APIkey=a1b2c3d4e5", "a1b2"), ("ApiKEY: a1b2c3d4e5", "a1b2"), ("MyPrivateKey=a1b2c3d4e5", "a1b2")] {
            assert!(!masked(t).contains(secret), "{t}: {}", masked(t));
        }
        // The model isn't overruled next to a secret's label, nor about a single word.
        assert!(!keep_despite_model("Dana Whitfield", SecretKind::Other, "Door code hint: Dana Whitfield"));
        assert!(!keep_despite_model("Whitfield", SecretKind::Other, "Call them back"));
        assert!(!keep_despite_model("415-555-0142", SecretKind::Other, "alarm PIN 415-555-0142"));
    }

    // ---- P1s from the outside and security reviews ----
    #[test]
    fn p1_detector_gaps() {
        for (t, secret) in [
            ("password=a(b))", "a(b"), ("password=ab&show", "ab&show"), ("password=reset-me-9", "reset-me"), ("password=abc123&next=%2Fhome", "abc123"),
            ("see backup codes 4821-9937, and Recovery phrases: witch collapse", "witch"), ("backup code ABCD-EFGH-JKLM", "ABCD"), ("recovery key QWERTYUIOP", "QWERTY"),
            ("password: correct horse battery staple", "staple"), ("mysql -uroot -pTr0ub4dor9x shop", "Tr0ub4"), ("curl -u admin:Tr0ub4dor9x https://x.com", "Tr0ub4"),
            ("curl --user admin:Tr0ub4dor9x https://x.com", "Tr0ub4"),
        ] {
            assert!(!masked(t).contains(secret), "{t}: {}", masked(t));
        }
        assert_eq!(masked("line one recovery code\nnext line stays"), "line one recovery code\nnext line stays");
        assert_eq!(masked("password = getpass.getpass()"), "password = getpass.getpass()");
        assert_eq!(masked("password: str = None  # set later"), "password: str = None  # set later");
        assert!(!scrub_url("https://example.com/reset/9f8e7d6c5b4a39281706f5e4d3c2b1a0").contains("9f8e"));
        assert_eq!(scrub_url("https://example.com/docs/getting-started"), "https://example.com/docs/getting-started");
        // Beside a secret's label, in any alphabet's lookalikes.
        assert!(!keep_despite_model("Dana Whitfield", SecretKind::Other, "Раsswоrd hint"));
    }

    /// Seen live: an empty-looking password row above "Card number 4111…" took the whole card row for its value.
    #[test]
    fn the_next_field_is_not_this_fields_value() {
        let l = vec![line("Password", 0.1, 0.10), line("Card number 4111 1111 1111 1111", 0.1, 0.14), line("Notes for the plumber", 0.1, 0.18)];
        let h = scan(&l);
        assert!(!h.iter().any(|x| x.line == 1 && x.range.start == 0), "{h:?}");
        assert!(h.iter().any(|x| x.line == 1 && x.why == "card"));
        let l = vec![line("Password", 0.1, 0.10), line("blue-heron-7", 0.1, 0.14)];
        assert!(scan(&l).iter().any(|x| x.line == 1));
    }

    #[test]
    fn release_round() {
        // A row is "another field" only when it plainly starts with a label; then what follows the label still goes
        // unless a rule of its own already covers it.
        let under = |v: &str| { let l = vec![line("Password", 0.1, 0.10), line(v, 0.1, 0.14)]; let h = scan(&l); mask(&l[1].text, &h.iter().filter(|h| h.line == 1).map(|h| h.range.clone()).collect::<Vec<_>>()) };
        assert_eq!(under("name-of-the-rose9"), "[hidden]");
        assert_eq!(under("card.shark!22"), "[hidden]");
        assert_eq!(under("notes4me"), "[hidden]");
        assert_eq!(under("Member name dana"), "Member name [hidden]");
        assert_eq!(under("Card number 4111 1111 1111 1111"), "Card number [hidden]");
        // Wi-Fi passwords said in passing.
        for (t, secret) in [("the gate code is 4471 and wifi is marmalade-tuba-41", "marmalade"), ("Wi-Fi password is heron77blue", "heron77"), ("wifi: tuba41marm", "tuba41")] {
            assert!(!masked(t).contains(secret), "{t}: {}", masked(t));
        }
        assert_eq!(masked("the wifi is BlueHeron today"), "the wifi is BlueHeron today");
        // Flags are case-sensitive, and there can be more than one.
        assert_eq!(masked("mysql -P3306 -uroot -pTr0ub4dor9x shop"), "mysql -P3306 -uroot -p[hidden] shop");
        let two = masked("curl -u dana:first9pass -U proxy:second9pass https://x.com");
        assert!(!two.contains("first9") && !two.contains("second9") && two.contains("dana"), "{two}");
        assert!(!masked("curl -u 'dana:two words9' https://x.com").contains("words9"));
        // A quoted password runs to its closing quote; an unquoted one to the end of the line.
        assert_eq!(masked("password = \"correct horse battery\" # note"), "password = \"[hidden]\" # note");
        assert_eq!(masked("password: correct, horse; battery staple!"), "password: [hidden]");
    }

    #[test]
    fn lookalike_letters_never_split_a_character() {
        // "Nаmе dаna" with Cyrillic а and е: the label ends at a different byte in what was read than in its Latin twin.
        let l = vec![line("Password", 0.1, 0.10), line("Nаmе dаna9", 0.1, 0.14), line("Саrd numbеr 12 34 tеxt", 0.1, 0.18)];
        let h = scan(&l);
        for x in &h {
            assert!(l[x.line].text.is_char_boundary(x.range.start) && l[x.line].text.is_char_boundary(x.range.end), "{x:?}");
        }
        let r: Vec<_> = h.iter().filter(|x| x.line == 1).map(|x| x.range.clone()).collect();
        assert_eq!(mask(&l[1].text, &r), "Nаmе [hidden]");
        // Whatever arrives, masking and placing never panic.
        let odd = line("é😀é", 0.1, 0.1);
        assert_eq!(mask(&odd.text, &[1..3, 2..6, 0..99]), "[hidden]");
        let _ = hit_rect(&odd, &(1..5));
        let mut hits = vec![Hit { line: 0, range: 1..5, kind: SecretKind::Other, why: "model" }, Hit { line: 9, range: 0..1, kind: SecretKind::Other, why: "model" }];
        snap(std::slice::from_ref(&odd), &mut hits);
        assert_eq!(hits, vec![Hit { line: 0, range: 0..6, kind: SecretKind::Other, why: "model" }]);
    }

    #[test]
    fn a_passphrase_and_its_first_word_are_both_remembered() {
        let mut h = Vec::new();
        scan_text("password: correct horse battery token=Zx81qwerty99", 0, &mut h);
        let got: Vec<&str> = h.iter().map(|x| &"password: correct horse battery token=Zx81qwerty99"[x.range.clone()]).collect();
        assert!(got.contains(&"correct") && got.contains(&"correct horse battery token=Zx81qwerty99") && got.contains(&"Zx81qwerty99"), "{got:?}");
        for (t, secret) in [("mysql -p'half open9", "open9"), ("mysql -pab'cd9 x", "cd9"), ("curl -u dana:pa'ss9 x", "ss9"), ("curl -u \"dana:open quote9", "quote9")] {
            assert!(!masked(t).contains(secret), "{t}: {}", masked(t));
        }
    }
}
