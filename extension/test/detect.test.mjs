import test from 'node:test';
import assert from 'node:assert/strict';
import { kindOf, report, cleanAddress, Watcher } from '../detect.js';

// Just enough of an element for the detector.
let seq = 0;
function el(tag, attrs = {}, box = { x: 10, y: 20, w: 100, h: 30 }, extra = {}) {
  return {
    n: seq++, localName: tag, value: 'x', ...extra,
    getAttribute: (k) => attrs[k] ?? null, hasAttribute: (k) => k in attrs, set: (k, v) => { attrs[k] = v; },
    get type() { return (attrs.type ?? 'text').toLowerCase(); },
    labels: (extra.labelText ? [{ textContent: extra.labelText }] : []),
    getBoundingClientRect: () => ({ left: box.x, top: box.y, width: box.w, height: box.h, right: box.x + box.w, bottom: box.y + box.h }),
  };
}
const style = (over = {}) => () => ({ visibility: 'visible', display: 'block', opacity: '1', webkitTextSecurity: 'none', ...over });
const env = (els, over = {}) => ({ elements: () => els, styleOf: style(), width: 1000, height: 800, url: 'https://shop.example/pay?card=4111#x', top: true, focused: null, ...over });

test('kinds by type, autocomplete and naming', () => {
  for (const [a, label, want] of [
    [{ type: 'password' }, '', 'password'], [{ type: 'PASSWORD' }, '', 'password'],
    [{ autocomplete: 'section-a billing cc-number' }, '', 'card'], [{ autocomplete: 'cc-csc' }, '', 'card'],
    [{ autocomplete: 'one-time-code' }, '', 'password'], [{ autocomplete: 'current-password' }, '', 'password'],
    [{ name: 'ssn' }, '', 'ssn'], [{ id: 'socialSecurityNumber' }, '', 'ssn'], [{}, 'Social Security number', 'ssn'],
    [{ name: 'acct_no' }, '', 'account'], [{ placeholder: 'Account number' }, '', 'account'], [{ 'aria-label': 'Routing number' }, '', 'account'], [{ name: 'iban' }, '', 'account'],
    [{ name: 'apiKey' }, '', 'key'], [{}, 'Client secret', 'key'], [{ name: 'recovery-code' }, '', 'key'], [{ name: 'cvv' }, '', 'card'], [{ name: 'cardNumber' }, '', 'card'],
  ]) assert.equal(kindOf(el('input', a, undefined, { labelText: label }), style()()), want, JSON.stringify([a, label]));
});

test('ordinary fields are nothing', () => {
  for (const [a, label] of [[{ type: 'email' }, 'Email'], [{ name: 'q' }, 'Search'], [{ name: 'account_name' }, 'Account name'], [{ name: 'discard' }, ''], [{ name: 'passenger' }, 'Passenger'],
    [{ name: 'tokenize' }, ''], [{ type: 'checkbox', name: 'remember_password' }, 'Remember password'], [{ type: 'submit', name: 'password' }, ''], [{ name: 'lesson' }, 'Lesson'], [{ name: 'keyword' }, '']]) {
    assert.equal(kindOf(el('input', a, undefined, { labelText: label }), style()()), null, JSON.stringify([a, label]));
  }
});

test('masked text and vault overlays count', () => {
  assert.equal(kindOf(el('input', {}), style({ webkitTextSecurity: 'disc' })()), 'password');
  assert.equal(kindOf(el('div', {}), style({ webkitTextSecurity: 'disc' })()), 'password');
  for (const src of ['chrome-extension://nngceckbapebfimnlniiiahkandclblb/menu.html', 'moz-extension://a-b/x', 'safari-web-extension://A/x']) assert.equal(kindOf(el('iframe', { src }), style()()), 'overlay');
  assert.equal(kindOf(el('iframe', { src: 'https://a.com' }), style()()), null);
});

test('a password stays a password after it is revealed', () => {
  const e = el('input', { type: 'password' });
  const w = new Watcher();
  assert.equal(w.kind(e, style()()), 'password');
  e.set('type', 'text');
  assert.equal(w.kind(e, style()()), 'password');
});

test('report has places and kinds, never values', () => {
  const r = report(env([el('input', { type: 'password' }, { x: 100, y: 80, w: 200, h: 40 }, { value: 'hunter2-zebra' }), el('input', { type: 'email' }, undefined, { value: 'dana@example.com' })]), new Watcher());
  assert.deepEqual(r.fields, [{ kind: 'password', x: 0.1, y: 0.1, w: 0.2, h: 0.05 }]);
  const s = JSON.stringify(r);
  for (const leak of ['hunter2', 'zebra', 'dana', '4111', 'card=']) assert.ok(!s.includes(leak), leak);
  assert.equal(r.url, 'https://shop.example/pay');
  assert.equal(r.unplaced, false);
});

test('what cannot be seen is not reported, what is partly off screen is clipped', () => {
  const p = (box) => report(env([el('input', { type: 'password' }, box)]), new Watcher()).fields;
  assert.deepEqual(p({ x: -500, y: 0, w: 100, h: 20 }), []);
  assert.deepEqual(p({ x: 0, y: 900, w: 100, h: 20 }), []);
  assert.deepEqual(p({ x: 0, y: 0, w: 0, h: 20 }), []);
  assert.deepEqual(p({ x: -50, y: 0, w: 100, h: 80 }), [{ kind: 'password', x: 0, y: 0, w: 0.05, h: 0.1 }]);
  const hidden = (o) => report(env([el('input', { type: 'password' })], { styleOf: style(o) }), new Watcher()).fields.length;
  assert.equal(hidden({ visibility: 'hidden' }), 0);
  assert.equal(hidden({ display: 'none' }), 0);
  assert.equal(hidden({ opacity: '0.01' }), 1); // nearly invisible is still visible
});

test('fields in frames cannot be placed, and say so', () => {
  const r = report(env([el('input', { type: 'password' })], { top: false }), new Watcher());
  assert.equal(r.unplaced, true);
  assert.deepEqual(r.fields, []);
  assert.equal(report(env([], { top: false }), new Watcher()).unplaced, false);
});

test('too many fields, or a page that will not be read, is unplaced', () => {
  const many = Array.from({ length: 300 }, () => el('input', { type: 'password' }));
  const r = report(env(many), new Watcher());
  assert.equal(r.unplaced, true);
  assert.ok(r.fields.length <= 200);
  assert.equal(report(env([], { elements: () => { throw new Error('no'); } }), new Watcher()).unplaced, true);
  const bad = el('input', { type: 'password' });
  bad.getBoundingClientRect = () => { throw new Error('no'); };
  assert.equal(report(env([bad]), new Watcher()).unplaced, true);
});

test('the size of the page is reported in device pixels', () => {
  const r = report(env([el('input', { type: 'password' })], { ratio: 2 }), new Watcher());
  assert.deepEqual([r.width, r.height], [2000, 1600]);
});

test('a pinch-zoomed page cannot be measured', () => {
  assert.equal(report(env([el('input', { type: 'password' })], { pinch: 1.5 }), new Watcher()).unplaced, true);
  assert.equal(report(env([], { pinch: 1.5 }), new Watcher()).unplaced, false);
  assert.equal(report(env([el('input', { type: 'password' })], { pinch: 1 }), new Watcher()).unplaced, false);
});

test('an empty field is still covered, and marked', () => {
  const e = el('input', { type: 'password' }, undefined, { value: '' });
  const one = (over = {}) => report(env([e], over), new Watcher()).fields;
  assert.equal(one().length, 1);
  assert.equal(one()[0].empty, true);
  assert.equal(report(env([e], { focused: e }), new Watcher()).typing, true);
  // The browser fills it in without telling the page what it wrote.
  assert.equal('empty' in one({ filled: (x) => x === e })[0], false);
  // Emptiness that can't be known counts as full.
  assert.equal('empty' in report(env([el('div', {}, undefined, { value: undefined })], { styleOf: style({ webkitTextSecurity: 'disc' }) }), new Watcher()).fields[0], false);
  assert.equal('empty' in report(env([el('iframe', { src: 'chrome-extension://a/x' }, undefined, { value: '' })]), new Watcher()).fields[0], false);
});

test('typing in a secret field is reported', () => {
  const e = el('input', { type: 'password' });
  assert.equal(report(env([e], { focused: e }), new Watcher()).typing, true);
  assert.equal(report(env([e, el('input')], { focused: null }), new Watcher()).typing, false);
});

test('addresses lose queries, fragments and logins', () => {
  assert.equal(cleanAddress('https://u:p@a.com:8443/x/y?token=1#f'), 'https://a.com:8443/x/y');
  assert.equal(cleanAddress('data:text/html,secret'), 'data:');
  assert.equal(cleanAddress('nonsense'), '');
});
