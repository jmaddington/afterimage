// Finds the secret-bearing fields on a page. Reports where they are and what kind, never what they hold.
// AIDEV-NOTE: privacy-critical. Nothing here may read `.value` or text content into a report.

const MAX_FIELDS = 200;

// Matched against the words of a field's name, id, label and placeholder: "acctNo", "acct_no", "Acct no" are the same.
const NAMED = [
  ['ssn', /\b(ssn|itin|social security( (number|no))?|tax(payer)? id)\b/],
  ['card', /\b((card|cc) (number|num|no)|cvv2?|cvc2?|csc|security code|pan)\b/],
  ['account', /\b((account|acct|routing|aba|bank) (number|num|no)|iban|sort code|swift( code)?|bic)\b/],
  ['password', /\b(password|passwd|passcode|passphrase|pwd|pin( (code|number))?|otp|one time (code|password)|verification code)\b/],
  ['key', /\b(api key|secret( key)?|client secret|access (key|token)|private key|auth(entication)? token|bearer token|recovery (code|key|phrase)|backup code|seed phrase)\b/],
];
const COMPLETE = {
  'current-password': 'password', 'new-password': 'password', 'one-time-code': 'password',
  'cc-number': 'card', 'cc-csc': 'card', 'cc-exp': 'card', 'cc-exp-month': 'card', 'cc-exp-year': 'card',
};
// Controls that never show a secret, whatever they are called.
const INERT = new Set(['checkbox', 'radio', 'submit', 'button', 'reset', 'image', 'file', 'range', 'color', 'hidden', 'email', 'search', 'date', 'month', 'week', 'time']);

function words(s) {
  return String(s ?? '').replace(/([a-z0-9])([A-Z])/g, '$1 $2').replace(/[^\p{L}\p{N}]+/gu, ' ').trim().toLowerCase();
}

export function kindOf(el, style) {
  const tag = el.localName;
  if (tag === 'iframe' || tag === 'frame') {
    // Another extension's box drawn inside the page: a password manager's menu, usually.
    return /^(chrome|moz|safari-web)-extension:/i.test(el.getAttribute('src') ?? '') ? 'overlay' : null;
  }
  if (style?.webkitTextSecurity && style.webkitTextSecurity !== 'none') return 'password';
  if (tag !== 'input' && tag !== 'textarea') return null;
  if (tag === 'input' && el.type === 'password') return 'password';
  if (tag === 'input' && INERT.has(el.type)) return null;
  for (const t of (el.getAttribute('autocomplete') ?? '').toLowerCase().split(/\s+/)) if (COMPLETE[t]) return COMPLETE[t];
  const names = [el.getAttribute('name'), el.getAttribute('id'), el.getAttribute('placeholder'), el.getAttribute('aria-label'), el.getAttribute('title'),
    ...Array.from(el.labels ?? [], (l) => l.textContent)].map(words);
  for (const [kind, re] of NAMED) if (names.some((n) => re.test(n))) return kind;
  return null;
}

/** Remembers fields that were ever passwords, so "show password" doesn't turn one into ordinary text. */
export class Watcher {
  #were = new WeakSet();
  kind(el, style) {
    const k = kindOf(el, style);
    if (k === 'password') this.#were.add(el);
    return k ?? (this.#were.has(el) ? 'password' : null);
  }
}

export function cleanAddress(raw) {
  try {
    const u = new URL(raw);
    return /^https?:$/.test(u.protocol) ? `${u.origin}${u.pathname}` : u.protocol;
  } catch {
    return '';
  }
}

const round = (n) => Math.round(n * 1e5) / 1e5;

/** Top-left origin, as fractions of the visible page. Null when none of it shows. */
function place(box, width, height) {
  const l = Math.max(0, box.left), t = Math.max(0, box.top), r = Math.min(width, box.right), b = Math.min(height, box.bottom);
  if (!(r > l && b > t)) return null;
  return { x: round(l / width), y: round(t / height), w: round((r - l) / width), h: round((b - t) / height) };
}

export function report(env, watcher) {
  const out = { v: 1, url: cleanAddress(env.url), top: env.top, fields: [], typing: false, unplaced: false };
  try {
    for (const el of env.elements()) {
      const style = env.styleOf(el);
      const kind = watcher.kind(el, style);
      if (!kind || style.display === 'none' || style.visibility === 'hidden') continue;
      const at = place(el.getBoundingClientRect(), env.width, env.height);
      if (!at) continue;
      if (el === env.focused && kind !== 'overlay') out.typing = true;
      // A frame doesn't know where it sits in the window, so it can only say that it holds something.
      if (!env.top || out.fields.length >= MAX_FIELDS) out.unplaced = true;
      else out.fields.push({ kind, ...at });
    }
  } catch {
    out.unplaced = true;
  }
  return out;
}
