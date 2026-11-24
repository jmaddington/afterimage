// Runs in every frame. Built together with detect.js by scripts/extension.sh (content scripts can't import).
(() => {
  const api = globalThis.browser ?? globalThis.chrome;
  const watcher = new Watcher();
  const SOON = 200, REPEAT = 4000, MOST = 20000;
  let timer = 0, again = 0, last = '', sentAt = 0, heard = 0, dead = false;

  function shadow(el) {
    try {
      return globalThis.chrome?.dom?.openOrClosedShadowRoot?.(el) ?? el.openOrClosedShadowRoot ?? el.shadowRoot;
    } catch {
      return el.shadowRoot;
    }
  }
  function* all(root, budget = { left: MOST }) {
    for (const el of root.querySelectorAll('*')) {
      // A page too big to walk whole still gets its ordinary fields looked at.
      const field = /^(input|textarea|i?frame)$/.test(el.localName);
      if (--budget.left < 0 && !field) continue;
      if (field || el.isContentEditable) yield el;
      const s = budget.left > 0 ? shadow(el) : null;
      if (s) yield* all(s, budget);
    }
  }
  function focused() {
    let f = document.activeElement;
    for (let s; f && (s = shadow(f))?.activeElement; ) f = s.activeElement;
    return f;
  }

  function filled(el) {
    for (const s of [':autofill', ':-webkit-autofill']) {
      try {
        if (el.matches(s)) return true;
      } catch {} // Not every browser knows both.
    }
    return false;
  }

  /** `asked`: the app wants an answer now, seen or not. Resolves once the report is on its way. */
  function look(asked = false) {
    timer = 0;
    const hidden = document.visibilityState === 'hidden';
    // Out of sight and unasked: say nothing. Being hidden never clears a report; it only lapses.
    if (dead || (hidden && !asked)) return Promise.resolve(false);
    const r = report({
      elements: () => all(document), styleOf: (el) => getComputedStyle(el), width: innerWidth, height: innerHeight,
      url: location.href, top: window === window.top, focused: focused(), ratio: devicePixelRatio,
      pinch: visualViewport?.scale ?? 1, filled,
    }, watcher);
    if (hidden) r.hidden = true;
    // Which question this answers: a place is believed only if measured since the latest.
    if (heard) r.n = heard;
    const s = JSON.stringify(r);
    const matters = r.fields.length > 0 || r.typing || r.unplaced;
    const now = Date.now();
    clearTimeout(again);
    if (matters && !hidden) again = setTimeout(soon, REPEAT);
    if (!asked && s === last && !(matters && now - sentAt >= REPEAT)) return Promise.resolve(false);
    last = s;
    sentAt = now;
    try {
      return Promise.resolve(api.runtime.sendMessage({ type: 'page', ...r })).then(() => true, () => false);
    } catch {
      dead = true; // The extension was updated or removed underneath this page.
      return Promise.resolve(false);
    }
  }
  function soon() {
    if (!timer && !dead) timer = setTimeout(look, SOON);
  }

  new MutationObserver(soon).observe(document, { subtree: true, childList: true, attributes: true, attributeFilter: ['type', 'style', 'class', 'hidden', 'name', 'id', 'autocomplete', 'src'] });
  for (const e of ['scroll', 'resize', 'focusin', 'focusout', 'input', 'change', 'load', 'transitionend', 'animationstart']) addEventListener(e, soon, { capture: true, passive: true });
  visualViewport?.addEventListener('resize', soon);
  // The app asks just before and just after each picture. The answer is "my report is sent", never just "I heard".
  api.runtime.onMessage.addListener((m, sender, reply) => {
    if (m?.type !== 'measure' || sender.id !== api.runtime.id || !Number.isSafeInteger(m.n)) return;
    heard = Math.max(heard, m.n);
    clearTimeout(timer);
    look(true).then(reply);
    return true;
  });
  let was = document.visibilityState;
  document.addEventListener('visibilitychange', (e) => {
    const now = document.visibilityState;
    const back = e.isTrusted && was === 'hidden' && now === 'visible';
    was = now;
    if (!back) return;
    last = ''; // Back in sight: report at once, even if nothing changed.
    clearTimeout(timer);
    void look();
  });
  soon();
})();
