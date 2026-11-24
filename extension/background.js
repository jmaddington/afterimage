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
/** Every frame of every tab reports afresh; then the app hears that they have, and which frames still exist. */
async function measure(n) {
  await tabs();
  const list = (await api.tabs.query({})).filter((t) => t.id != null && !t.discarded);
  const frames = (await Promise.all(list.map((t) =>
    api.webNavigation.getAllFrames({ tabId: t.id }).then((f) => (f ?? []).map((x) => [t.id, x.frameId]), () => [])))).flat();
  // A frame the script can't run in (settings, stores) rejects at once and has never said anything to go stale.
  const all = Promise.all(frames.map(([tab, frame]) =>
    api.tabs.sendMessage(tab, { type: 'measure', n }, { frameId: frame }).then((sent) => sent === true, () => false)));
  const slow = new Promise((r) => setTimeout(r, 600, 'slow'));
  // No answer is an answer: the app leaves this browser out of the picture.
  if ((await Promise.race([all, slow])) === 'slow') return;
  // The app holds any frame that reported before and stayed silent now against the whole browser.
  send({ type: 'asked', n, live: frames.slice(0, 2000) });
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
api.runtime.onInstalled?.addListener(connect);
setInterval(() => void tabs(), 4000);
connect();
