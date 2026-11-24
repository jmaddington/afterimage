// Passes page reports to the Afterimage app, with the facts only the browser knows: which tab, whether it's showing,
// whether it's private. Talks to one local program through the browser's native messaging; never to a network.
const api = globalThis.browser ?? globalThis.chrome;
const HOST = 'com.jmaddington.afterimage';
let port = null;
let wait = 1000;

function connect() {
  if (port) return;
  try {
    port = api.runtime.connectNative(HOST);
  } catch {
    port = null;
  }
  if (!port) return retry();
  port.onMessage.addListener((m) => {
    if (m?.type === 'ask' && Number.isSafeInteger(m.n)) void measure(m.n);
  });
  port.onDisconnect.addListener(() => {
    void api.runtime.lastError;
    port = null;
    retry();
  });
  tabs();
}
function retry() {
  setTimeout(connect, wait);
  wait = Math.min(wait * 2, 60000);
}
function send(m) {
  connect();
  try {
    port?.postMessage({ v: 1, ...m });
    wait = 1000;
  } catch {
    port = null;
  }
}
const RUNS_IN = /^(https?:|about:blank|about:srcdoc)/;

/**
 * Every frame of every tab reports afresh; then the app hears that they have, and which frames still exist.
 * Anything short of that and the app hears nothing, which it takes as "leave this browser out".
 */
async function measure(n) {
  await tabs();
  const list = (await api.tabs.query({})).filter((t) => t.id != null && !t.discarded);
  let frames;
  try {
    frames = (await Promise.all(list.map((t) =>
      api.webNavigation.getAllFrames({ tabId: t.id }).then((f) => {
        if (!f) throw new Error('no frames');
        return f.map((x) => ({ tab: t.id, frame: x.frameId, showing: t.active, must: RUNS_IN.test(x.url ?? '') }));
      })))).flat();
  } catch {
    return;
  }
  const all = Promise.all(frames.map((f) =>
    api.tabs.sendMessage(f.tab, { type: 'measure', n }, { frameId: f.frame }).then((sent) => sent === true, () => false)
      // A frame in a showing tab where the script should be running has to have answered.
      .then((ok) => ok || !(f.showing && f.must))));
  const slow = new Promise((r) => setTimeout(r, 600, 'slow'));
  const done = await Promise.race([all, slow]);
  if (done === 'slow' || done.includes(false)) return;
  // Too many to list: say nothing about which exist, so nothing is forgotten on our word.
  send({ type: 'asked', n, ...(frames.length <= 2000 ? { live: frames.map((f) => [f.tab, f.frame]) } : {}) });
}
/** Pages that were open before the extension arrived have no script yet. */
async function inject() {
  for (const t of await api.tabs.query({})) {
    if (t.id == null || t.discarded) continue;
    api.scripting.executeScript({ target: { tabId: t.id, allFrames: true }, files: ['content.js'] }).catch(() => {});
  }
}
async function tabs() {
  const list = await api.tabs.query({});
  send({ type: 'tabs', tabs: list.filter((t) => t.id != null).slice(0, 1000).map((t) => ({ tab: t.id, window: t.windowId, active: t.active, private: t.incognito, url: cleanAddress(t.url ?? '') })) });
}

api.runtime.onMessage.addListener((m, sender) => {
  if (m?.type !== 'page' || sender.id !== api.runtime.id || sender.tab?.id == null) return;
  send({ ...m, tab: sender.tab.id, window: sender.tab.windowId, frame: sender.frameId ?? 0 });
});
// Where a tab has got to matters as much as which one is showing.
api.tabs.onUpdated.addListener((_, change) => {
  if (change.url) void tabs();
});
for (const e of ['onActivated', 'onRemoved', 'onCreated', 'onAttached', 'onReplaced']) api.tabs[e]?.addListener(() => void tabs());
api.runtime.onStartup?.addListener(connect);
api.runtime.onInstalled?.addListener(() => {
  connect();
  void inject();
});
setInterval(() => void tabs(), 4000);
connect();
