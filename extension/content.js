// Runs in every frame. Built together with detect.js by scripts/extension.sh (content scripts can't import).
(() => {
  const api = globalThis.browser ?? globalThis.chrome;
  const watcher = new Watcher();
  const SOON = 200, REPEAT = 4000, MOST = 20000;
  let timer = 0, last = '', sentAt = 0, dead = false;

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

  function look() {
    timer = 0;
    if (dead) return;
    // Out of sight: say so once, so nothing stale is held against whatever shows next.
    const r = document.visibilityState === 'hidden' ? { v: 1, url: '', top: window === window.top, fields: [], typing: false, unplaced: false } : report({
      elements: () => all(document), styleOf: (el) => getComputedStyle(el), width: innerWidth, height: innerHeight,
      url: location.href, top: window === window.top, focused: focused(),
    }, watcher);
    const s = JSON.stringify(r);
    const matters = r.fields.length > 0 || r.typing || r.unplaced;
    const now = Date.now();
    if (s !== last || (matters && now - sentAt >= REPEAT)) {
      last = s;
      sentAt = now;
      try {
        api.runtime.sendMessage({ type: 'page', ...r })?.catch?.(() => {});
      } catch {
        dead = true; // The extension was updated or removed underneath this page.
      }
    }
    if (matters) setTimeout(soon, REPEAT);
  }
  function soon() {
    if (!timer && !dead) timer = setTimeout(look, SOON);
  }

  new MutationObserver(soon).observe(document, { subtree: true, childList: true, attributes: true, attributeFilter: ['type', 'style', 'class', 'hidden', 'name', 'id', 'autocomplete', 'src'] });
  for (const e of ['scroll', 'resize', 'focusin', 'focusout', 'input', 'load', 'transitionend']) addEventListener(e, soon, { capture: true, passive: true });
  document.addEventListener('visibilitychange', soon);
  soon();
})();
