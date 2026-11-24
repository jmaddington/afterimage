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
/** Every page that's showing measures itself; then the app hears that they have. */
async function measure(n) {
  await tabs();
  const list = await api.tabs.query({});
  const slow = new Promise((r) => setTimeout(r, 400, 'slow'));
  // Pages that can't run the script (settings, stores) have nothing to say and reject at once.
  const all = Promise.all(list.filter((t) => t.id != null && !t.discarded).map((t) =>
    api.tabs.sendMessage(t.id, { type: 'measure', n }).then(() => true, () => true)));
  if ((await Promise.race([all, slow])) === 'slow') return; // No answer is an answer: the app skips the picture.
  await new Promise((r) => setTimeout(r, 40)); // Frames inside pages report in the same breath.
  send({ type: 'asked', n });
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
