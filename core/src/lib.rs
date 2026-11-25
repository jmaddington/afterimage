//! Afterimage engine. Swift captures and reads the screen; everything after that happens here.
//!
//! AIDEV-NOTE: privacy-critical invariant: raw pixels live only in RAM. `process` is the sole path to disk, and
//! AIDEV-NOTE: `HostServices::encode` is only ever handed a frame whose secrets have been frosted over.

mod activity;
pub mod bridge;
mod detect;
mod devlog;
mod lm;
mod mcp;
pub mod pages;
mod privacy;
mod redact;
mod store;
mod windows;

pub use detect::{Rect, SecretKind, TextLine, Token};
pub use activity::{ActivitySample, ActivitySpan, OpenWindow};
pub use lm::ModelInfo;
pub use pages::{Cover, PageArea, PageHint, PagePlan, plan_page};
pub use privacy::*;
pub use store::*;
pub use windows::{WindowChoice, WindowScene, WindowSeen, WindowSkip, keep_windows, plan_windows};

use detect::Hit;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{BuildHasher, RandomState};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum EngineError {
    #[error("{0}")]
    Failed(String),
}
impl From<rusqlite::Error> for EngineError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Failed(format!("The library couldn't be read: {e}"))
    }
}
impl From<lm::LmError> for EngineError {
    fn from(e: lm::LmError) -> Self {
        Self::Failed(e.to_string())
    }
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum HostError {
    #[error("{reason}")]
    Failed { reason: String },
}
impl From<uniffi::UnexpectedUniFFICallbackError> for HostError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed { reason: e.reason }
    }
}

#[derive(uniffi::Record)]
pub struct Encoded {
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

/// Apple-framework services supplied by the app.
#[uniffi::export(with_foreign)]
pub trait HostServices: Send + Sync {
    fn encode(&self, rgba: Vec<u8>, width: u32, height: u32, stride: u32, path: String, thumb_path: String) -> Result<Encoded, HostError>;
    fn model_image(&self, path: String) -> Result<Vec<u8>, HostError>;
    fn library_changed(&self);
}

#[derive(uniffi::Record)]
pub struct Frame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub lines: Vec<TextLine>,
    pub moment: Moment,
    pub display_key: String,
    /// Places known to hold secrets whatever the text says, as fractions of the picture.
    #[uniffi(default = [])]
    pub covers: Vec<pages::Cover>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum DescribeWhen {
    Never,
    PluggedIn,
    Always,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Config {
    pub lm_url: String,
    pub model: String,
    pub embed_model: String,
    /// Fail closed: without the model's check a frame is discarded, never saved.
    pub require_ai: bool,
    pub describe: DescribeWhen,
    pub max_age_days: u32,
    pub max_bytes: u64,
    pub unload_after_secs: u32,
    /// Keep a redacted log of calls to LM Studio and of every capture decision.
    #[uniffi(default = false)]
    pub developer: bool,
}

#[derive(Debug, Clone, uniffi::Enum, PartialEq)]
pub enum LmState {
    Unknown,
    Offline,
    NoModel,
    Ready,
    Error { message: String },
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Status {
    pub lm: LmState,
    pub linked_devices: Vec<String>,
    pub queued: u32,
    pub discarded: u64,
    pub discard_reason: String,
    pub busy: bool,
    pub describing: bool,
}

/// A request to forget a span of time.
/// AIDEV-NOTE: privacy-critical. Frames are refused by arrival number, never by the clock: anything that arrived
/// AIDEV-NOTE: before the request and falls in its span is refused, however its timestamp compares with "now"; so is
/// AIDEV-NOTE: anything taken before the request that arrives after it.
/// The stretch of activity going on.
struct Doing {
    span: ActivitySpan,
    key: u64,
    /// When it was last written down.
    saved: i64,
}

struct Forgot {
    from: i64,
    to: i64,
    /// The last frame to have arrived when it was asked.
    upto: u64,
    /// When it was asked, for log entries, which have no arrival number.
    asked: i64,
}

#[derive(Default)]
struct Queue {
    /// Each with the number it arrived under.
    frames: VecDeque<(u64, Frame)>,
    arrived: u64,
    /// The number of the frame being checked right now, if any.
    in_hand: Option<u64>,
    stop: bool,
}

struct Shared {
    host: Arc<dyn HostServices>,
    root: PathBuf,
    store: Mutex<Store>,
    config: Mutex<Config>,
    queue: Mutex<Queue>,
    wake: Condvar,
    /// Last accepted thumbprint per display or per window; cleared when a frame is lost so the next look is taken afresh.
    prints: Mutex<HashMap<String, Vec<u8>>>,
    /// Line neighbourhoods the model has already called clean. Keyed hash, RAM only.
    clean: Mutex<HashSet<u64>>,
    hasher: RandomState,
    forgotten: Mutex<Vec<Forgot>>,
    loaded_by_us: Mutex<HashSet<String>>,
    last_lm_use: Mutex<Instant>,
    lm_state: Mutex<LmState>,
    links: Mutex<(Option<Instant>, Vec<String>)>,
    discarded: AtomicU64,
    reason: Mutex<String>,
    busy: AtomicBool,
    describing: AtomicBool,
    on_power: AtomicBool,
    calm: AtomicBool,
    pages: Mutex<pages::Pages>,
    outlets: std::sync::OnceLock<Arc<bridge::Outlets>>,
    question: AtomicU64,
    mcp: Mutex<Option<mcp::Server>>,
    /// The stretch of activity being added to, and when it was last written down.
    // AIDEV-NOTE: lock order is now_doing, then store, then cleared. `forgotten` is never held while taking another.
    now_doing: Mutex<Option<Doing>>,
    /// Window key -> its title as a saved picture left it. In memory only.
    cleared: Mutex<HashMap<u64, String>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
pub(crate) fn lock_tape(t: &lm::Tape) -> MutexGuard<'_, Vec<lm::Call>> {
    lock(t)
}
fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// About 30 MB each on a Retina display; deep enough for a flurry of window switches.
const QUEUE_CAP: usize = 12;
/// Displays are few, windows come and go: past this many thumbprints the oldest knowledge is let go.
const PRINTS_CAP: usize = 256;
const CONTEXT: u32 = 8192;
const CHUNK_CHARS: usize = 5000;
const CHUNK_LINES: usize = 90;

#[derive(uniffi::Object)]
pub struct Engine {
    s: Arc<Shared>,
}

#[uniffi::export]
impl Engine {
    #[uniffi::constructor]
    pub fn new(root: String, config: Config, host: Arc<dyn HostServices>) -> Result<Arc<Self>, EngineError> {
        let root = PathBuf::from(root);
        let store = Store::open(&root)?;
        if !config.developer {
            if let Err(e) = store.dev_clear() {
                log::error!("the developer log couldn't be cleared: {e} [AIMG-DEVCLEAR0]");
            }
        }
        if let Err(e) = store.sweep_orphans() {
            log::error!("orphan sweep failed: {e} [AIMG-ORPHAN]");
        }
        let s = Arc::new(Shared {
            host, root, store: Mutex::new(store), config: Mutex::new(config),
            queue: Default::default(), wake: Condvar::new(), prints: Default::default(), clean: Default::default(),
            hasher: RandomState::new(), forgotten: Default::default(), loaded_by_us: Default::default(),
            last_lm_use: Mutex::new(Instant::now()), lm_state: Mutex::new(LmState::Unknown), links: Default::default(),
            discarded: AtomicU64::new(0), reason: Default::default(), busy: AtomicBool::new(false),
            describing: AtomicBool::new(false), on_power: AtomicBool::new(true), calm: AtomicBool::new(true), pages: Default::default(), outlets: Default::default(), question: AtomicU64::new(now_ms() as u64), mcp: Default::default(), now_doing: Default::default(), cleared: Default::default(),
        });
        for (name, f) in [("afterimage-save", save_loop as fn(Arc<Shared>)), ("afterimage-index", index_loop)] {
            let s = s.clone();
            std::thread::Builder::new().name(name.into()).spawn(move || f(s)).map_err(|e| EngineError::Failed(e.to_string()))?;
        }
        Ok(Arc::new(Self { s }))
    }

    pub fn set_config(&self, config: Config) {
        // AIDEV-NOTE: lock order is store, then config. Held together so no entry can slip in between "off" and the clear.
        let store = lock(&self.s.store);
        *lock(&self.s.config) = config.clone();
        if !config.developer {
            if let Err(e) = store.dev_clear() {
                log::error!("the developer log couldn't be cleared: {e} [AIMG-DEVCLEAR]");
            }
        }
        drop(store);
        self.s.wake.notify_all();
    }

    /// Hear from the browser extension, if it's there. Its absence changes nothing.
    pub fn listen_for_pages(&self) -> bool {
        let s = self.s.clone();
        let heard = move |e: bridge::Event| lock(&s.pages).receive(e.pid, e.source, &e.message, Instant::now());
        match bridge::listen(&self.s.root.join("bridge.sock"), heard) {
            Ok(o) => self.s.outlets.set(o).is_ok(),
            Err(e) => {
                log::error!("can't listen for the browser extension: {e} [AIMG-BRIDGE]");
                false
            }
        }
    }

    /// Have this browser's pages measure themselves now. False if its extension is there but didn't answer in time.
    /// Blocking; call off the main thread.
    pub fn refresh_pages(&self, pid: i32) -> bool {
        let Some(o) = self.s.outlets.get() else { return true };
        // Rises across launches too, so a page can't answer with something measured for an earlier run.
        let n = self.s.question.fetch_add(1, SeqCst) + 1;
        lock(&self.s.pages).issue(pid, n);
        let asked = o.ask(pid, n);
        // Someone who spoke lately and can't be asked now can't be vouched for.
        if lock(&self.s.pages).sources(pid, Instant::now()).iter().any(|id| !asked.contains(id)) {
            return false;
        }
        let end = Instant::now() + Duration::from_millis(900);
        loop {
            if asked.iter().all(|id| lock(&self.s.pages).answered(pid, *id, n)) {
                return true;
            }
            if Instant::now() >= end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Has this browser's extension said anything lately?
    pub fn extension_heard(&self, pid: i32) -> bool {
        !lock(&self.s.pages).sources(pid, Instant::now()).is_empty()
    }

    pub fn page_hint(&self, pid: i32) -> pages::PageHint {
        lock(&self.s.pages).hint(pid, Instant::now())
    }

    /// `calm`: thermal state is fine and Low Power Mode is off.
    pub fn set_conditions(&self, on_power: bool, calm: bool) {
        self.s.on_power.store(on_power, SeqCst);
        self.s.calm.store(calm, SeqCst);
    }

    /// Cheap pre-check on a small greyscale thumbprint. False means "same screen as last time". The key names a
    /// display, or a single window ("win:" and its number) when windows are photographed on their own.
    pub fn is_new(&self, display_key: String, luma: Vec<u8>) -> bool {
        let mut p = lock(&self.s.prints);
        let fresh = match p.get(&display_key) {
            Some(old) if old.len() == luma.len() && !luma.is_empty() => {
                let mut total = 0u64;
                let mut peak = 0u8;
                for (a, b) in old.iter().zip(&luma) {
                    let d = a.abs_diff(*b);
                    total += d as u64;
                    peak = peak.max(d);
                }
                peak >= 8 || total as f64 / luma.len() as f64 >= 0.6
            }
            _ => true,
        };
        if fresh {
            // Windows come and go and their numbers aren't reused. Forgetting costs one repeated picture each, no more.
            if p.len() >= PRINTS_CAP && !p.contains_key(&display_key) {
                p.clear();
            }
            p.insert(display_key, luma);
        }
        fresh
    }

    /// A frame that passed `is_new` never arrived; the next look at that display counts as new.
    pub fn lost(&self, display_key: String) {
        lock(&self.s.prints).remove(&display_key);
    }

    /// Queue a frame. The newest wins: when the queue is full the oldest unsaved frame is let go.
    pub fn ingest(&self, frame: Frame) {
        let need = frame.stride as usize * frame.height as usize;
        if frame.rgba.len() < need || (frame.stride as usize) < frame.width as usize * 4 || need == 0 {
            log::error!("rejected a malformed frame [AIMG-BADFRAME]");
            return;
        }
        // A secret whose place makes no sense can't be covered.
        if frame.covers.iter().any(|c| ![c.rect.x, c.rect.y, c.rect.w, c.rect.h].iter().all(|n| n.is_finite()) || c.rect.w < 0.0 || c.rect.h < 0.0) {
            log::error!("rejected a frame with a senseless cover [AIMG-BADCOVER]");
            return;
        }
        let mut q = lock(&self.s.queue);
        if q.frames.len() >= QUEUE_CAP {
            q.frames.pop_front();
            self.s.discard("The model couldn't keep up");
        }
        q.arrived += 1;
        let n = q.arrived;
        q.frames.push_back((n, frame));
        drop(q);
        self.s.wake.notify_all();
    }

    pub fn list(&self, filter: Filter, limit: u32, offset: u32) -> Result<Vec<Shot>, EngineError> {
        Ok(lock(&self.s.store).list(&filter, limit, offset)?)
    }

    /// Keyword search, plus meaning when `semantic` and the embedding model answers in time.
    pub fn search(&self, filter: Filter, semantic: bool, limit: u32) -> Result<Vec<Shot>, EngineError> {
        self.s.search(filter, semantic, limit)
    }

    /// What is in front right now. Called every second or so while recording; cheap unless something changed.
    pub fn note_activity(&self, ex: Exclusions, sample: ActivitySample) -> Result<(), EngineError> {
        // Taken first, so a forget can't fall between reading a title and writing it.
        let mut doing = lock(&self.s.now_doing);
        let (mut span, key) = {
            let cleared = lock(&self.s.cleared);
            activity::describe(&ex, &sample, &|k| cleared.get(&k).cloned())
        };
        // Asked to be forgotten while this was on its way here. Checked under the lock `forget` also takes.
        if lock(&self.s.forgotten).iter().any(|x| (x.from..x.to).contains(&sample.at) && sample.at <= x.asked) {
            return Ok(());
        }
        let near = |d: &Doing| sample.at - d.span.end < 60_000;
        match doing.as_mut() {
            // Out of order: it says nothing new.
            Some(d) if sample.at < d.span.end => {}
            // The same window still, and its title hasn't been withdrawn: stretch it. Written down every quarter
            // minute, so little is lost if the app dies.
            Some(d) if d.key == key && d.span.private_time == span.private_time && near(d)
                && (d.span.title == span.title || span.title.is_empty() == d.span.title.is_empty() || d.span.title.is_empty() && activity::may_retitle(&span.bundle_id)) => {
                d.span.end = sample.at;
                let retitle = d.span.title.is_empty() && !span.title.is_empty();
                if retitle || sample.at - d.saved >= 15_000 {
                    let store = lock(&self.s.store);
                    if retitle {
                        store.activity_retitle(d.span.id, &span.title)?;
                        d.span.title = span.title;
                    }
                    store.activity_extend(d.span.id, d.span.end)?;
                    d.saved = sample.at;
                }
            }
            _ => {
                let store = lock(&self.s.store);
                if let Some(d) = doing.take() {
                    // It lasted until this one began, unless there was a long silence between.
                    store.activity_extend(d.span.id, if near(&d) { sample.at } else { d.span.end })?;
                }
                (span.start, span.end) = (sample.at, sample.at);
                span.id = store.activity_open(&span)?;
                *doing = Some(Doing { span, key, saved: sample.at });
            }
        }
        Ok(())
    }

    /// Nobody is there, or recording stopped: the current stretch ends where it last was.
    pub fn end_activity(&self) {
        if let Some(d) = lock(&self.s.now_doing).take() {
            let _ = lock(&self.s.store).activity_extend(d.span.id, d.span.end);
        }
    }

    pub fn activity(&self, from: i64, to: i64) -> Result<Vec<ActivitySpan>, EngineError> {
        self.s.activity(from, to)
    }

    /// Let an AI agent on this Mac read the library (never change it). Returns the port in use.
    /// `utc_offset_secs`: this Mac's time zone, so times are given as the person's clock shows them.
    pub fn mcp_start(&self, port: u16, token: String, utc_offset_secs: i32) -> Result<u16, EngineError> {
        let server = mcp::start(self.s.clone(), port, token, utc_offset_secs).map_err(|e| EngineError::Failed(format!("The agent connection couldn't start: {e}")))?;
        let port = server.port;
        *lock(&self.s.mcp) = Some(server);
        Ok(port)
    }
    pub fn mcp_stop(&self) {
        *lock(&self.s.mcp) = None;
    }

    pub fn lines(&self, id: i64) -> Result<Vec<IndexedLine>, EngineError> {
        Ok(lock(&self.s.store).lines(id)?)
    }
    pub fn apps(&self) -> Result<Vec<AppCount>, EngineError> {
        Ok(lock(&self.s.store).apps()?)
    }
    pub fn displays(&self) -> Result<Vec<AppCount>, EngineError> {
        Ok(lock(&self.s.store).displays()?)
    }
    pub fn stats(&self) -> Result<Stats, EngineError> {
        let model = lock(&self.s.config).embed_model.clone();
        let store = lock(&self.s.store);
        let mut s = store.stats()?;
        if !model.is_empty() {
            s.reindexing = store.stale_count(&model)?;
        }
        Ok(s)
    }
    pub fn set_kept(&self, ids: Vec<i64>, kept: bool) -> Result<(), EngineError> {
        lock(&self.s.store).set_kept(&ids, kept)?;
        self.s.host.library_changed();
        Ok(())
    }

    pub fn delete(&self, ids: Vec<i64>) -> Result<u64, EngineError> {
        let (n, failed) = {
            let mut store = lock(&self.s.store);
            // A title earned from a picture that's going is earned again or not at all.
            lock(&self.s.cleared).clear();
            store.delete(&ids)?
        };
        self.s.host.library_changed();
        if failed > 0 {
            return Err(EngineError::Failed(format!("{failed} files couldn't be removed. Afterimage will try again next time it opens.")));
        }
        Ok(n)
    }

    /// Delete a span of time, including frames still waiting in memory. Kept shots go too: the person asked.
    pub fn forget(&self, from: i64, to: i64) -> Result<u64, EngineError> {
        // Order matters: mark first, so a frame mid-flight is refused at insert time.
        let mut q = lock(&self.s.queue);
        let mut f = lock(&self.s.forgotten);
        f.push(Forgot { from, to, upto: q.arrived, asked: now_ms() });
        // Kept for as long as the app runs: a picture taken before a request can still be on its way here. Too many,
        // and the two oldest become one that covers both.
        while f.len() > 64 {
            let b = f.remove(1);
            let a = &mut f[0];
            *a = Forgot { from: a.from.min(b.from), to: a.to.max(b.to), upto: a.upto.max(b.upto), asked: a.asked.max(b.asked) };
        }
        drop(f);
        q.frames.retain(|(_, f)| !(from..to).contains(&f.moment.taken_at));
        drop(q);
        // Held together, in this order, so a sample can't write between the delete and the reset.
        let mut doing = lock(&self.s.now_doing);
        let store = lock(&self.s.store);
        store.dev_forget(from, to)?;
        store.activity_forget(from, to)?;
        lock(&self.s.cleared).clear();
        // What is going on right now is inside "everything so far".
        if doing.as_ref().is_some_and(|d| d.span.end >= from && d.span.start < to) {
            *doing = None;
        }
        drop(doing);
        let ids = store.ids_matching(&Filter { from: Some(from), to: Some(to), ..Default::default() })?;
        drop(store);
        self.delete(ids)
    }

    pub fn delete_matching(&self, filter: Filter) -> Result<u64, EngineError> {
        let ids = lock(&self.s.store).ids_matching(&filter)?;
        self.delete(ids)
    }

    /// What applying these limits now would remove.
    pub fn would_expire(&self, max_age_days: u32, max_bytes: u64) -> Result<u64, EngineError> {
        Ok(lock(&self.s.store).expired(now_ms(), max_age_days, max_bytes)?.len() as u64)
    }

    pub fn apply_retention(&self) -> Result<u64, EngineError> {
        Ok(self.s.retain()?)
    }

    /// A capture decision, for the developer log.
    pub fn note_capture(&self, trigger: String, outcome: String) {
        self.s.note(vec![DevEntry { at: now_ms(), kind: "capture".into(), ok: true, request: trigger, note: outcome, ..Default::default() }]);
    }
    pub fn dev_log(&self, limit: u32, before: Option<i64>) -> Result<Vec<DevEntry>, EngineError> {
        Ok(lock(&self.s.store).dev_list(limit, before)?)
    }
    pub fn dev_clear(&self) -> Result<(), EngineError> {
        Ok(lock(&self.s.store).dev_clear()?)
    }

    pub fn models(&self) -> Result<Vec<ModelInfo>, EngineError> {
        let c = lock(&self.s.config).clone();
        Ok(self.s.lm(&c)?.models()?)
    }

    pub fn start_lm_server(&self) -> bool {
        lm::start_server()
    }

    /// Refresh the LM Studio state. Blocking; call off the main thread.
    pub fn probe(&self) -> Status {
        let c = lock(&self.s.config).clone();
        let st = match self.s.lm(&c).and_then(|l| l.models()) {
            Ok(ms) if ms.iter().any(|m| m.key == c.model) => LmState::Ready,
            Ok(_) => LmState::NoModel,
            Err(lm::LmError::Offline) => LmState::Offline,
            Err(e) => LmState::Error { message: e.to_string() },
        };
        *lock(&self.s.lm_state) = st;
        let stale = lock(&self.s.links).0.is_none_or(|t| t.elapsed() > Duration::from_secs(120));
        if stale {
            let d = lm::linked_devices();
            *lock(&self.s.links) = (Some(Instant::now()), d);
        }
        self.status()
    }

    pub fn status(&self) -> Status {
        Status {
            lm: lock(&self.s.lm_state).clone(),
            linked_devices: lock(&self.s.links).1.clone(),
            queued: lock(&self.s.queue).frames.len() as u32,
            discarded: self.s.discarded.load(SeqCst),
            discard_reason: lock(&self.s.reason).clone(),
            busy: self.s.busy.load(SeqCst),
            describing: self.s.describing.load(SeqCst),
        }
    }

    /// Stop threads and hand back any model we loaded. Frames still in memory are dropped.
    pub fn shutdown(&self) {
        let mut q = lock(&self.s.queue);
        q.stop = true;
        q.frames.clear();
        drop(q);
        self.s.wake.notify_all();
        self.mcp_stop();
        self.s.unload_ours();
    }
}

fn take(t: &lm::Tape) -> Vec<lm::Call> {
    std::mem::take(&mut *lock(t))
}

impl Shared {
    /// Stretches overlapping the range, the one still going included up to now.
    fn activity(&self, from: i64, to: i64) -> Result<Vec<ActivitySpan>, EngineError> {
        let doing = lock(&self.now_doing).as_ref().map(|d| d.span.clone());
        let mut spans = lock(&self.store).activity_list(from, to, 5000)?;
        if let Some(cur) = doing {
            if let Some(s) = spans.iter_mut().find(|s| s.id == cur.id) {
                s.end = s.end.max(cur.end);
            }
        }
        Ok(spans)
    }

    /// Keyword search, plus meaning when `semantic` and the embedding model answers in time.
    fn search(&self, filter: Filter, semantic: bool, limit: u32) -> Result<Vec<Shot>, EngineError> {
        let c = lock(&self.config).clone();
        let style = lm::embed_style(&c.embed_model);
        let emb = if semantic && !c.embed_model.is_empty() && filter.text.trim().chars().count() >= 3 {
            let (l, tape) = self.taped(&c);
            let e = l.and_then(|l| {
                self.ensure(&l, &c.embed_model, 2048)?;
                l.embed(&c.embed_model, &format!("{}{}", style.query, filter.text), 10)
            }).ok();
            self.note(devlog::search(take(&tape)));
            e
        } else {
            None
        };
        let near = emb.as_deref().map(|vector| Near { vector, model: &c.embed_model, floor: style.floor, band: style.band });
        Ok(lock(&self.store).search(&filter, near.as_ref(), limit)?)
    }

    /// A client that records its calls when developer mode is on.
    fn taped(&self, c: &Config) -> (Result<lm::Lm, lm::LmError>, lm::Tape) {
        let tape = lm::Tape::default();
        let l = self.lm(c).map(|mut l| {
            l.tape = c.developer.then(|| tape.clone());
            l
        });
        (l, tape)
    }

    fn note(&self, entries: Vec<DevEntry>) {
        if entries.is_empty() {
            return;
        }
        let store = lock(&self.store);
        if !lock(&self.config).developer {
            return;
        }
        let forgotten = lock(&self.forgotten);
        for e in entries {
            // Forgotten while it was happening, or about a picture that's gone.
            if !forgotten.iter().any(|x| (x.from..x.to).contains(&e.at) && e.at <= x.asked) {
                let _ = store.dev_add(&e);
            }
        }
    }

    fn lm(&self, c: &Config) -> Result<lm::Lm, lm::LmError> {
        lm::Lm::new(&c.lm_url)
    }

    fn ensure(&self, l: &lm::Lm, key: &str, ctx: u32) -> Result<(), lm::LmError> {
        *lock(&self.last_lm_use) = Instant::now();
        // Held across the load so two threads can't load the same model twice.
        let mut ours = lock(&self.loaded_by_us);
        if l.ensure_loaded(key, ctx)? {
            ours.insert(key.into());
        }
        Ok(())
    }

    fn unload_ours(&self) {
        let ours: Vec<String> = lock(&self.loaded_by_us).drain().collect();
        let c = lock(&self.config).clone();
        if let Ok(l) = self.lm(&c) {
            for k in ours {
                let _ = l.unload(&k);
            }
        }
    }

    fn discard(&self, why: &str) {
        self.discarded.fetch_add(1, SeqCst);
        *lock(&self.reason) = why.into();
        lock(&self.prints).clear();
    }

    fn retain(&self) -> Result<u64, rusqlite::Error> {
        let c = lock(&self.config).clone();
        let mut st = lock(&self.store);
        // The state, not the moment it changed: with developer mode off there is no log.
        if c.developer { st.dev_trim(now_ms(), 7, 5000)? } else { st.dev_clear()? }
        if c.max_age_days > 0 {
            st.activity_expire(now_ms() - c.max_age_days as i64 * 86_400_000)?;
        }
        let ids = st.expired(now_ms(), c.max_age_days, c.max_bytes)?;
        if ids.is_empty() {
            return Ok(0);
        }
        lock(&self.cleared).clear();
        let (n, _) = st.delete(&ids)?;
        drop(st);
        self.host.library_changed();
        Ok(n)
    }

    fn line_key(&self, lines: &[TextLine], i: usize) -> u64 {
        let t = |j: Option<usize>| j.and_then(|j| lines.get(j)).map_or("", |l| l.text.as_str());
        self.hasher.hash_one((t(i.checked_sub(1)), t(Some(i)), t(Some(i + 1))))
    }

    /// Ask the model about every line it hasn't already cleared. Err means "not checked".
    fn ai_hits(&self, c: &Config, tape: &mut lm::Tape, lines: &[TextLine], known: &[Hit], hits: &mut Vec<Hit>) -> Result<(), lm::LmError> {
        let keys: Vec<u64> = (0..lines.len()).map(|i| self.line_key(lines, i)).collect();
        let todo: Vec<usize> = {
            let clean = lock(&self.clean);
            (0..lines.len())
                .filter(|&i| !lines[i].code && lines[i].text.chars().filter(|c| c.is_alphanumeric()).count() >= 3 && !clean.contains(&keys[i]))
                .collect()
        };
        if todo.is_empty() {
            return Ok(());
        }
        // Neighbours come along for context; what the rules already caught is masked before it leaves the process.
        let mut send: Vec<usize> = todo.iter().flat_map(|&i| i.saturating_sub(2)..=(i + 1).min(lines.len() - 1)).collect();
        send.sort_unstable();
        send.dedup();
        // Codes are judged by rule alone; their payloads are written by whoever made the page.
        send.retain(|&i| !lines[i].code);
        let text: Vec<(usize, String)> = send
            .iter()
            .map(|&i| {
                let r: Vec<_> = known.iter().filter(|h| h.line == i).map(|h| h.range.clone()).collect();
                (i, detect::mask(&lines[i].text, &r))
            })
            .collect();
        let (l, t) = self.taped(c);
        *tape = t;
        let l = l?;
        self.ensure(&l, &c.model, CONTEXT)?;

        let mut unsure = HashSet::new();
        let mut at = 0;
        while at < text.len() {
            let mut end = at;
            let mut chars = 0;
            while end < text.len() && end - at < CHUNK_LINES && (end == at || chars + text[end].1.len() < CHUNK_CHARS) {
                chars += text[end].1.len();
                end += 1;
            }
            self.ask(&l, c, lines, &text[at..end], hits, &mut unsure)?;
            *lock(&self.last_lm_use) = Instant::now();
            if end == text.len() {
                break;
            }
            // Overlap so a label and its value are never split. Always advance.
            at = end.saturating_sub(3).max(at + 1);
        }
        let dirty: HashSet<usize> = hits.iter().chain(known).map(|h| h.line).chain(unsure).collect();
        let mut clean = lock(&self.clean);
        if clean.len() > 30_000 {
            clean.clear();
        }
        clean.extend(todo.iter().filter(|i| !dirty.contains(i)).map(|&i| keys[i]));
        Ok(())
    }

    /// One chunk; an unusable answer is retried on halves before giving up.
    fn ask(&self, l: &lm::Lm, c: &Config, lines: &[TextLine], chunk: &[(usize, String)], hits: &mut Vec<Hit>, unsure: &mut HashSet<usize>) -> Result<(), lm::LmError> {
        let view: Vec<(usize, &str)> = chunk.iter().map(|(i, t)| (*i, t.as_str())).collect();
        match l.find_secrets(&c.model, &view) {
            Ok(f) => {
                // AIDEV-NOTE: findings are written straight into the caller's list, so an error later (here, in the
                // AIDEV-NOTE: other half, in the next chunk) never throws away what was already found.
                let mut wild = false;
                for (i, v, k) in f {
                    // A line that doesn't exist: it isn't talking about this screen. Its value is still looked for.
                    if i >= lines.len() {
                        wild = true;
                        hits.extend(detect::find_secret(lines, &v).into_iter().map(|(line, range)| Hit { line, range, kind: k, why: "model" }));
                        continue;
                    }
                    // It quoted text we had already masked: there's more on that line than we knew. Hide all of it.
                    let v = if v.contains("[hidden]") { String::new() } else { v };
                    let found = lm::locate(lines, i, &v, k);
                    let near = |j: usize| lines.get(j).map_or("", |l| l.text.as_str());
                    // Everywhere it would be left showing, not just where the model pointed.
                    let context: Vec<&str> = found.iter().map(|h| h.line).chain([i]).flat_map(|j| [near(j.wrapping_sub(1)), near(j), near(j + 1)]).collect();
                    if !v.is_empty() && detect::keep_despite_model(&v, k, &context.join("\n")) {
                        // Overruled, not cleared: every line it touched is asked about again next time.
                        unsure.insert(i);
                        unsure.extend(found.iter().map(|h| h.line));
                        continue;
                    }
                    hits.extend(found);
                }
                if wild { Err(lm::LmError::BadAnswer("line")) } else { Ok(()) }
            }
            Err(lm::LmError::BadAnswer(_)) if chunk.len() >= 8 => {
                let m = chunk.len() / 2;
                self.ask(l, c, lines, &chunk[..m + 2], hits, unsure)?;
                self.ask(l, c, lines, &chunk[m.saturating_sub(2)..], hits, unsure)
            }
            Err(e) => Err(e),
        }
    }

    /// Redact and save one frame. Err(reason) means nothing was written.
    fn process(&self, f: &mut Frame, arrived: u64, require_ai: bool) -> Result<(), String> {
        let mut tape = lm::Tape::default();
        let r = self.process_taped(f, arrived, require_ai, &mut tape);
        let calls = take(&tape);
        match &r {
            Ok(Some((id, hits, checked, all))) => self.note(devlog::checks(calls, all, checked.then_some(hits.as_slice()), Some(*id))),
            Ok(None) => {} // Forgotten while it was being checked.
            Err(_) => self.note(devlog::checks(calls, &f.lines, None, None)),
        }
        r.map(|_| ())
    }

    fn process_taped(&self, f: &mut Frame, arrived: u64, require_ai: bool, tape: &mut lm::Tape) -> Result<Option<(i64, Vec<Hit>, bool, Vec<TextLine>)>, String> {
        let c = lock(&self.config).clone();
        // The window's title and address are checked like any other text on screen. They have no place in the
        // picture, so they sit outside it.
        let shown = f.lines.len();
        let nowhere = Rect { x: -10.0, y: -10.0, w: 0.0, h: 0.0 };
        let mut all = f.lines.clone();
        for t in [f.moment.window_title.clone(), detect::scrub_url(&f.moment.url)] {
            all.push(TextLine { text: t, rect: nowhere, tokens: vec![], code: false });
        }
        let all = all;
        let mut hits = detect::scan(&all);
        hits.extend(covered(&f.lines, &f.covers));
        // What it found before any failure counts either way.
        let mut found = Vec::new();
        let checked = self.ai_hits(&c, tape, &all, &hits, &mut found);
        hits.extend(found);
        let ai_checked = match checked {
            Ok(()) => true,
            Err(e) if require_ai => return Err(e.to_string()),
            Err(_) => false,
        };
        detect::extend_wrapped(&all, &mut hits);
        propagate(&all, &mut hits);
        detect::snap(&all, &mut hits);

        let mut regions: Vec<Region> = f.covers.iter().map(|c| region(c.kind, c.rect, "extension")).collect();
        for h in hits.iter().filter(|h| h.line < shown) {
            let r = region(h.kind, detect::hit_rect(&f.lines[h.line], &h.range), h.why);
            match regions.iter_mut().find(|x| x.rect == r.rect) {
                // Several things can agree; say so once each.
                Some(x) if !x.why.split(',').any(|w| w == h.why) => x.why = format!("{},{}", x.why, h.why),
                Some(_) => {}
                None => regions.push(r),
            }
        }
        let rects: Vec<Rect> = regions.iter().map(|r| r.rect).collect();
        redact::frost(&mut f.rgba, f.width as usize, f.height as usize, f.stride as usize, &rects);

        let values: Vec<String> = secret_values(&all, &hits);
        let scrub = |t: &str| values.iter().fold(detect::mask_text(t), |t, v| {
            let l = [TextLine { text: t, rect: Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }, tokens: vec![], code: false }];
            let r: Vec<_> = detect::find_secret(&l, v).into_iter().map(|(_, r)| r).collect();
            detect::mask(&l[0].text, &r)
        });
        let mut m = f.moment.clone();
        let masked = |i: usize| detect::mask(&all[i].text, &hits.iter().filter(|h| h.line == i).map(|h| h.range.clone()).collect::<Vec<_>>());
        m.window_title = scrub(&masked(shown));
        m.url = scrub(&masked(shown + 1));
        let lines: Vec<IndexedLine> = f
            .lines
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let r: Vec<_> = hits.iter().filter(|h| h.line == i).map(|h| h.range.clone()).collect();
                IndexedLine { text: detect::mask(&l.text, &r), rect: l.rect }
            })
            .collect();

        let day = format!("Shots/{}", m.taken_at / 86_400_000);
        std::fs::create_dir_all(self.root.join(&day)).map_err(|e| format!("The disk isn't writable: {e}"))?;
        let stem = format!("{day}/{}-{:08x}", m.taken_at, fastrand::u32(..));
        let (path, thumb_path) = (format!("{stem}.heic"), format!("{stem}.t.heic"));
        let abs = |p: &str| self.root.join(p).to_string_lossy().into_owned();
        let remove = || {
            let _ = std::fs::remove_file(abs(&path));
            let _ = std::fs::remove_file(abs(&thumb_path));
        };
        let e = match self.host.encode(std::mem::take(&mut f.rgba), f.width, f.height, f.stride, abs(&path), abs(&thumb_path)) {
            Ok(e) => e,
            Err(e) => {
                remove();
                return Err(format!("The picture couldn't be written: {e}"));
            }
        };
        let mut store = lock(&self.store);
        let at = m.taken_at;
        if lock(&self.forgotten).iter().any(|x| (x.from..x.to).contains(&at) && (arrived <= x.upto || at <= x.asked)) {
            drop(store);
            remove();
            return Ok(None);
        }
        let title = (activity::key(&m.bundle_id, &f.moment.window_title), m.window_title.clone());
        let new = NewShot { moment: m, path: path.clone(), thumb_path: thumb_path.clone(), width: e.width, height: e.height, bytes: e.bytes, regions, lines, ai_checked, check_model: if ai_checked { c.model.clone() } else { String::new() } };
        let inserted = store.insert(&new);
        if inserted.is_ok() {
            // This window's title has now been through everything the picture went through. Under the store lock,
            // so a forget is wholly before or wholly after.
            let mut cleared = lock(&self.cleared);
            if cleared.len() >= 512 {
                cleared.clear();
            }
            cleared.insert(title.0, title.1);
        }
        drop(store);
        match inserted {
            Ok(id) => Ok(Some((id, hits, ai_checked, all))),
            Err(e) => {
                remove();
                Err(format!("The library couldn't be updated: {e}"))
            }
        }
    }
}

const HINT: &str = "empty-field";

/// Text lying in a covered place. Any word touching it counts; a line whose words can't all be placed counts whole.
fn covered(lines: &[TextLine], covers: &[pages::Cover]) -> Vec<Hit> {
    let overlap = |r: &Rect, c: &Rect| {
        let w = (r.x + r.w).min(c.x + c.w) - r.x.max(c.x);
        let h = (r.y + r.h).min(c.y + c.h) - r.y.max(c.y);
        if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
    };
    // A value half out of its box still counts. A label has to be well inside one: seen live, "Password" brushes
    // the edge of its own field.
    let inside = |r: &Rect, c: &Rect, share: f64| overlap(r, c) > share * (r.w * r.h).min(c.w * c.h);
    let label = |t: &str| {
        let t = t.trim_end_matches([':', '*']).to_lowercase();
        ["password", "passcode", "card", "number", "account", "pin", "code", "key", "secret", "token", "cvv", "routing", "ssn", "name", "email", "username"].contains(&t.as_str())
    };
    let mut hits = Vec::new();
    for c in covers {
        let why = if c.empty { HINT } else { "extension" };
        for (i, l) in lines.iter().enumerate().filter(|(_, l)| !l.code && overlap(&l.rect, &c.rect) > 0.0) {
            let placed: u32 = l.tokens.iter().map(|t| t.len).sum();
            let words = l.text.chars().filter(|c| !c.is_whitespace()).count() as u32;
            let byte = |scalar: u32| l.text.char_indices().nth(scalar as usize).map_or(l.text.len(), |x| x.0);
            let text = |t: &Token| &l.text[byte(t.start)..byte(t.start + t.len)];
            let within: Vec<&Token> = l.tokens.iter().filter(|t| inside(&t.rect, &c.rect, if label(text(t)) { 0.10 } else { 0.03 })).collect();
            if placed < words {
                hits.push(Hit { line: i, range: 0..l.text.len(), kind: c.kind, why });
            } else if within.is_empty() {
                // The line crosses the box yet none of its words is in it: the word boxes are wrong. A mere brush
                // of the edge is not a crossing.
                if inside(&l.rect, &c.rect, 0.03) {
                    hits.push(Hit { line: i, range: 0..l.text.len(), kind: c.kind, why });
                }
            } else {
                hits.extend(within.into_iter().map(|t| Hit { line: i, range: byte(t.start)..byte(t.start + t.len), kind: c.kind, why }));
            }
        }
    }
    hits
}

fn secret_values(lines: &[TextLine], hits: &[Hit]) -> Vec<String> {
    let mut v: Vec<String> = hits.iter().filter(|h| h.why != HINT).map(|h| lines[h.line].text[h.range.clone()].trim().to_owned()).filter(|v| v.chars().count() >= 6).collect();
    v.extend(lines.iter().filter(|l| l.code).flat_map(|l| detect::code_secrets(&l.text)));
    v.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    v.dedup();
    v
}

/// A secret found once is hidden everywhere it appears on screen.
fn propagate(lines: &[TextLine], hits: &mut Vec<Hit>) {
    for v in secret_values(lines, hits) {
        for (line, range) in detect::find_secret(lines, &v) {
            if !hits.iter().any(|x| x.line == line && x.range.start <= range.start && x.range.end >= range.end) {
                hits.push(Hit { line, range, kind: SecretKind::Other, why: "repeated" });
            }
        }
    }
}

fn save_loop(s: Arc<Shared>) {
    loop {
        let mut q = lock(&s.queue);
        while q.frames.is_empty() && !q.stop {
            q = s.wake.wait_timeout(q, Duration::from_secs(15)).map_or_else(|e| e.into_inner().0, |r| r.0);
            if q.frames.is_empty() && !q.stop {
                drop(q);
                let idle = lock(&s.last_lm_use).elapsed().as_secs();
                let limit = lock(&s.config).unload_after_secs as u64;
                if limit > 0 && idle >= limit && !lock(&s.loaded_by_us).is_empty() && !s.describing.load(SeqCst) {
                    s.unload_ours();
                }
                q = lock(&s.queue);
            }
        }
        if q.stop {
            return;
        }
        let (arrived, mut f) = q.frames.pop_front().expect("queue is non-empty");
        q.in_hand = Some(arrived);
        drop(q);
        s.busy.store(true, SeqCst);
        let mut result = Err(String::new());
        // AIDEV-NOTE: rgba is taken by encode; only retry failures that happen before it (model errors).
        for attempt in 0..3 {
            let require = lock(&s.config).require_ai;
            // A frame that can't be handled is dropped; it must not take the saving of every later one with it.
            result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.process(&mut f, arrived, require))).unwrap_or_else(|_| {
                f.rgba.clear();
                log::error!("a frame couldn't be processed and was dropped [AIMG-PANIC]");
                Err("Afterimage couldn't read this picture, so it wasn't saved".into())
            });
            if result.is_ok() || f.rgba.is_empty() || lock(&s.queue).stop {
                break;
            }
            if attempt < 2 {
                std::thread::sleep(Duration::from_secs(3));
            }
        }
        match result {
            Ok(()) => s.host.library_changed(),
            Err(why) => {
                log::warn!("frame not saved: {why} [AIMG-NOSAVE]");
                s.discard(&why);
                s.host.library_changed();
            }
        }
        lock(&s.queue).in_hand = None;
        s.busy.store(false, SeqCst);
    }
}

fn index_loop(s: Arc<Shared>) {
    let mut last_retention: Option<Instant> = None;
    loop {
        if lock(&s.queue).stop {
            return;
        }
        if last_retention.is_none_or(|t| t.elapsed() > Duration::from_secs(600)) {
            last_retention = Some(Instant::now());
            if let Err(e) = s.retain() {
                log::error!("retention failed: {e} [AIMG-RETAIN]");
            }
        }
        // New pictures first; then any whose search data an earlier model made.
        let did = describe_one(&s).and_then(|did| if did { Ok(true) } else { reembed_one(&s) });
        s.describing.store(false, SeqCst);
        let nap = match did {
            Ok(true) => {
                s.host.library_changed();
                50
            }
            Ok(false) => 2000,
            Err(e) => {
                log::warn!("description failed: {e} [AIMG-DESCFAIL]");
                8000
            }
        };
        std::thread::sleep(Duration::from_millis(nap));
    }
}

fn describe_one(s: &Shared) -> Result<bool, String> {
    let c = lock(&s.config).clone();
    let allowed = match c.describe {
        DescribeWhen::Never => false,
        DescribeWhen::PluggedIn => s.on_power.load(SeqCst) && s.calm.load(SeqCst),
        DescribeWhen::Always => s.calm.load(SeqCst),
    };
    // Saving always goes first: a frame waiting to be checked is a secret waiting in RAM.
    if !allowed || s.busy.load(SeqCst) || !lock(&s.queue).frames.is_empty() {
        return Ok(false);
    }
    let Some((shot, body)) = lock(&s.store).next_pending().map_err(|e| e.to_string())? else { return Ok(false) };
    let (l, tape) = s.taped(&c);
    let l = l.map_err(|e| e.to_string())?;
    // However this ends, what was said is logged against the picture.
    struct Log<'a>(&'a Shared, lm::Tape, i64);
    impl Drop for Log<'_> {
        fn drop(&mut self) {
            self.0.note(devlog::about(take(&self.1), self.2));
        }
    }
    let _log = Log(s, tape, shot.id);
    s.describing.store(true, SeqCst);
    s.ensure(&l, &c.model, CONTEXT).map_err(|e| e.to_string())?;
    // Reachable model from here on: failures count against this shot so one bad image can't block the rest.
    let fail = |why: String| {
        let _ = lock(&s.store).bump_attempts(shot.id);
        why
    };
    let img = s.host.model_image(shot.path.clone()).map_err(|e| fail(e.to_string()))?;
    let (d, tags) = match l.describe(&c.model, &img, &shot.moment.app_name, &shot.moment.window_title, &body) {
        Ok(x) => x,
        Err(e @ lm::LmError::Offline) => return Err(e.to_string()),
        Err(e) => return Err(fail(e.to_string())),
    };
    *lock(&s.last_lm_use) = Instant::now();
    let d = detect::mask_text(&d);
    // The model's tags are its words about the screen too.
    let tags: Vec<String> = tags.iter().map(|t| detect::mask_text(t)).collect();
    let emb = if c.embed_model.is_empty() { None } else { embed_shot(s, &l, &c, &shot, &d, &body).ok() };
    lock(&s.store).finish_index(shot.id, &d, &tags, emb.as_deref(), &c.embed_model, &c.model).map_err(|e| e.to_string())
}

/// What a picture is, as one piece of text for the embedding model.
fn embed_shot(s: &Shared, l: &lm::Lm, c: &Config, shot: &Shot, description: &str, body: &str) -> Result<Vec<f32>, lm::LmError> {
    let style = lm::embed_style(&c.embed_model);
    let head: String = format!("{}{} {} {}\n{}", style.document, shot.moment.app_name, shot.moment.window_title, description, body).chars().take(3500).collect();
    s.ensure(l, &c.embed_model, 2048)?;
    l.embed(&c.embed_model, &head, 30)
}

/// After a change of embedding model, remake one picture's vector from what is already saved about it.
fn reembed_one(s: &Shared) -> Result<bool, String> {
    let c = lock(&s.config).clone();
    let allowed = match c.describe {
        DescribeWhen::Never => false,
        DescribeWhen::PluggedIn => s.on_power.load(SeqCst) && s.calm.load(SeqCst),
        DescribeWhen::Always => s.calm.load(SeqCst),
    };
    if !allowed || c.embed_model.is_empty() || s.busy.load(SeqCst) || !lock(&s.queue).frames.is_empty() {
        return Ok(false);
    }
    let Some((shot, body)) = lock(&s.store).next_stale(&c.embed_model).map_err(|e| e.to_string())? else { return Ok(false) };
    let (l, tape) = s.taped(&c);
    let l = l.map_err(|e| e.to_string())?;
    let r = embed_shot(s, &l, &c, &shot, &shot.description, &body);
    s.note(devlog::about(take(&tape), shot.id));
    *lock(&s.last_lm_use) = Instant::now();
    match r {
        Ok(e) => lock(&s.store).set_embedding(shot.id, Some(&e), &c.embed_model).map(|_| true).map_err(|e| e.to_string()),
        // Not there to ask: try again later, and don't mark anything.
        Err(e @ (lm::LmError::Offline | lm::LmError::Model(_))) => Err(e.to_string()),
        // It answered and had nothing usable for this one: move on.
        Err(e) => lock(&s.store).set_embedding(shot.id, None, &c.embed_model).map(|_| true).map_err(|x| format!("{e}; {x}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeHost(Mutex<Vec<Vec<u8>>>);
    impl HostServices for FakeHost {
        fn encode(&self, rgba: Vec<u8>, width: u32, height: u32, _: u32, path: String, thumb: String) -> Result<Encoded, HostError> {
            std::fs::write(path, &rgba).unwrap();
            std::fs::write(thumb, b"t").unwrap();
            lock(&self.0).push(rgba);
            Ok(Encoded { width, height, bytes: 100 })
        }
        fn model_image(&self, _: String) -> Result<Vec<u8>, HostError> {
            Ok(vec![])
        }
        fn library_changed(&self) {}
    }

    // Port 9 refuses connections: stands in for LM Studio being off.
    fn config(require_ai: bool) -> Config {
        Config { lm_url: "http://127.0.0.1:9".into(), model: "m".into(), embed_model: String::new(), require_ai, describe: DescribeWhen::Never, max_age_days: 0, max_bytes: 0, unload_after_secs: 0, developer: false }
    }
    fn frame(at: i64) -> Frame {
        let l = |t: &str, y: f64| TextLine { text: t.into(), rect: Rect { x: 0.1, y, w: 0.5, h: 0.1 }, tokens: vec![], code: false };
        Frame {
            covers: vec![], rgba: vec![200; 100 * 100 * 4], width: 100, height: 100, stride: 400, display_key: "d".into(),
            lines: vec![l("hello world", 0.1), l("password: hunter22", 0.5), l("again hunter22 here", 0.8)],
            moment: Moment { taken_at: at, window_title: "login hunter22".into(), url: "https://x.com/a?t=hunter22".into(), ..Default::default() },
        }
    }
    fn wait(f: impl Fn() -> bool) {
        for _ in 0..400 {
            if f() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("timed out");
    }

    #[test]
    fn offline_model_means_nothing_is_saved() {
        let d = tempfile::tempdir().unwrap();
        let h = Arc::new(FakeHost(Default::default()));
        let e = Engine::new(d.path().to_string_lossy().into(), config(true), h.clone()).unwrap();
        e.ingest(frame(1000));
        wait(|| e.status().discarded == 1);
        assert!(lock(&h.0).is_empty());
        assert_eq!(e.stats().unwrap().count, 0);
        e.shutdown();
    }

    #[test]
    fn rules_only_mode_hides_pixels_text_title_and_url() {
        let d = tempfile::tempdir().unwrap();
        let h = Arc::new(FakeHost(Default::default()));
        let e = Engine::new(d.path().to_string_lossy().into(), config(false), h.clone()).unwrap();
        e.ingest(frame(1000));
        wait(|| e.stats().unwrap().count == 1);
        let s = e.list(Filter::default(), 10, 0).unwrap().remove(0);
        assert!(!s.ai_checked && s.regions.len() >= 2);
        assert_eq!(s.check_model, "");
        let all = format!("{:?}{:?}", s, e.lines(s.id).unwrap());
        assert!(!all.contains("hunter22"), "{all}");
        let px = &lock(&h.0)[0];
        assert_ne!(&px[55 * 400 + 30 * 4..][..8], &[200; 8]);
        assert_eq!(&px[..8], &[200; 8]);
        assert_eq!(e.search(Filter { text: "hunter22".into(), ..Default::default() }, false, 5).unwrap().len(), 0);
        e.shutdown();
    }

    #[test]
    fn forget_reaches_frames_in_memory() {
        let d = tempfile::tempdir().unwrap();
        let h = Arc::new(FakeHost(Default::default()));
        let e = Engine::new(d.path().to_string_lossy().into(), config(true), h.clone()).unwrap();
        e.ingest(frame(5000));
        e.forget(0, 10_000).unwrap();
        e.set_config(config(false));
        std::thread::sleep(Duration::from_secs(5));
        assert_eq!(e.stats().unwrap().count, 0);
        assert!(std::fs::read_dir(d.path().join("Shots")).unwrap().flatten().all(|x| std::fs::read_dir(x.path()).unwrap().count() == 0));
        e.shutdown();
    }

    #[test]
    fn duplicates_are_skipped_until_a_frame_is_lost() {
        let d = tempfile::tempdir().unwrap();
        let e = Engine::new(d.path().to_string_lossy().into(), config(true), Arc::new(FakeHost(Default::default()))).unwrap();
        assert!(e.is_new("a".into(), vec![10; 64]));
        assert!(!e.is_new("a".into(), vec![10; 64]));
        assert!(e.is_new("b".into(), vec![10; 64]));
        e.s.discard("x");
        assert!(e.is_new("a".into(), vec![10; 64]));
        e.lost("a".into());
        assert!(e.is_new("a".into(), vec![10; 64]));
        e.shutdown();
    }

    #[test]
    fn thumbprints_of_windows_long_gone_do_not_pile_up() {
        let d = tempfile::tempdir().unwrap();
        let e = Engine::new(d.path().to_string_lossy().into(), config(true), Arc::new(FakeHost(Default::default()))).unwrap();
        assert!(e.is_new("42".into(), vec![10; 64]));
        for i in 0..2000 {
            assert!(e.is_new(format!("win:{i}"), vec![10; 64]));
            assert!(!e.is_new(format!("win:{i}"), vec![10; 64]));
        }
        assert!(lock(&e.s.prints).len() <= PRINTS_CAP);
        // Each window has its own: one changing says nothing about another.
        assert!(e.is_new("win:a".into(), vec![10; 64]) && e.is_new("win:b".into(), vec![10; 64]));
        assert!(e.is_new("win:a".into(), vec![90; 64]) && !e.is_new("win:b".into(), vec![10; 64]));
        e.shutdown();
    }

    #[test]
    fn a_window_picture_keeps_which_window_it_was() {
        let d = tempfile::tempdir().unwrap();
        let e = Engine::new(d.path().to_string_lossy().into(), config(false), Arc::new(FakeHost(Default::default()))).unwrap();
        let mut f = frame(1000);
        (f.display_key, f.moment.window_key, f.moment.window_front) = ("win:77".into(), "ab12-77".into(), false);
        e.ingest(f);
        wait(|| e.stats().unwrap().count == 1);
        let m = e.list(Filter::default(), 10, 0).unwrap().remove(0).moment;
        assert_eq!((m.window_key.as_str(), m.window_front), ("ab12-77", false));
        // Its title went through the same check as any other.
        assert!(!m.window_title.contains("hunter22"));
        e.shutdown();
    }

    #[test]
    fn a_codes_secret_is_hidden_wherever_it_is_typed_out() {
        let d = tempfile::tempdir().unwrap();
        let h = Arc::new(FakeHost(Default::default()));
        let e = Engine::new(d.path().to_string_lossy().into(), config(false), h.clone()).unwrap();
        let l = |t: &str, y: f64, code: bool| TextLine { text: t.into(), rect: Rect { x: 0.1, y, w: 0.5, h: 0.1 }, tokens: vec![], code };
        let mut f = frame(1000);
        f.lines = vec![l("Or type jbsw y3dp ehpk 3pxp", 0.1, false), l("network BlueHeron", 0.25, false), l("marmalade", 0.4, false),
                       l("otpauth://totp/A:d?secret=JBSWY3DPEHPK3PXP", 0.6, true), l("WIFI:T:WPA;S:BlueHeron;P:marmalade;;", 0.7, true), l("", 0.85, true)];
        f.moment.window_title = "Setup JBSW-Y3DP-EHPK-3PXP".into();
        e.ingest(f);
        wait(|| e.stats().unwrap().count == 1);
        let s = e.list(Filter::default(), 10, 0).unwrap().remove(0);
        let all = format!("{:?}{:?}", s, e.lines(s.id).unwrap()).to_lowercase();
        for x in ["jbsw", "3pxp", "marmalade", "otpauth"] {
            assert!(!all.contains(x), "{x}: {all}");
        }
        assert!(all.contains("network blueheron"), "{all}");
        // The undecodable code is gone from the picture too.
        assert_ne!(&lock(&h.0)[0][90 * 400 + 30 * 4..][..8], &[200; 8]);
        e.shutdown();
    }

    #[test]
    fn codes_are_never_sent_to_the_model() {
        // A harmless code and nothing else: no question to ask, so an offline model is not an error.
        let d = tempfile::tempdir().unwrap();
        let e = Engine::new(d.path().to_string_lossy().into(), config(true), Arc::new(FakeHost(Default::default()))).unwrap();
        let mut f = frame(1000);
        f.lines = vec![TextLine { text: "https://example.com/menu".into(), rect: Rect { x: 0.1, y: 0.1, w: 0.2, h: 0.2 }, tokens: vec![], code: true }];
        // No title or address either: they would be questions for the model.
        f.moment.window_title.clear();
        f.moment.url.clear();
        e.ingest(f);
        wait(|| e.stats().unwrap().count == 1 || e.status().discarded == 1);
        assert_eq!(e.status().discarded, 0);
        e.shutdown();
    }

    #[test]
    fn the_engine_hears_the_extension() {
        let d = tempfile::tempdir().unwrap();
        let e = Engine::new(d.path().to_string_lossy().into(), config(true), Arc::new(FakeHost(Default::default()))).unwrap();
        assert!(e.listen_for_pages());
        let m = r#"{"v":1,"type":"page","tab":1,"window":1,"frame":0,"fields":[],"typing":true,"unplaced":false}"#;
        let mut input = (m.len() as u32).to_ne_bytes().to_vec();
        input.extend_from_slice(m.as_bytes());
        let (mut s, sock) = (std::os::unix::net::UnixStream::pair().unwrap(), d.path().join("bridge.sock"));
        let t = std::thread::spawn(move || bridge::forward(&mut s.1, std::io::sink(), &sock, 77));
        std::io::Write::write_all(&mut s.0, &input).unwrap();
        wait(|| e.page_hint(77).tabs.iter().any(|t| t.sensitive));
        assert!(e.page_hint(78).tabs.is_empty());
        drop(s.0);
        t.join().unwrap().unwrap();
        e.shutdown();
    }

    #[test]
    fn covered_places_are_frosted_and_their_text_hidden_everywhere() {
        let d = tempfile::tempdir().unwrap();
        let h = Arc::new(FakeHost(Default::default()));
        let e = Engine::new(d.path().to_string_lossy().into(), config(false), h.clone()).unwrap();
        let tok = |start, len, x: f64, w: f64, y: f64| Token { start, len, rect: Rect { x, y, w, h: 0.05 } };
        let mut f = frame(1000);
        f.lines = vec![
            TextLine { text: "Member marmot-88421 ok".into(), rect: Rect { x: 0.1, y: 0.3, w: 0.8, h: 0.05 }, tokens: vec![tok(0, 6, 0.1, 0.2, 0.3), tok(7, 12, 0.35, 0.3, 0.3), tok(20, 2, 0.8, 0.1, 0.3)], code: false },
            TextLine { text: "no tokens here 5512".into(), rect: Rect { x: 0.3, y: 0.5, w: 0.4, h: 0.05 }, tokens: vec![], code: false },
            TextLine { text: "repeat marmot-88421 below".into(), rect: Rect { x: 0.1, y: 0.8, w: 0.5, h: 0.05 }, tokens: vec![], code: false },
        ];
        f.moment.window_title = "marmot-88421".into();
        f.covers = vec![Cover { kind: SecretKind::Financial, rect: Rect { x: 0.33, y: 0.29, w: 0.35, h: 0.07 }, empty: false }, Cover { kind: SecretKind::Other, rect: Rect { x: 0.3, y: 0.5, w: 0.4, h: 0.05 }, empty: false }];
        e.ingest(f);
        wait(|| e.stats().unwrap().count == 1);
        let s = e.list(Filter::default(), 10, 0).unwrap().remove(0);
        let all = format!("{:?}{:?}", s, e.lines(s.id).unwrap());
        assert!(!all.contains("marmot") && !all.contains("5512"), "{all}");
        assert!(all.contains("Member [hidden] ok") && all.contains("repeat [hidden] below"), "{all}");
        // Frosted even where nothing was read.
        let px = &lock(&h.0)[0];
        assert_ne!(&px[33 * 400 + 45 * 4..][..8], &[200; 8]);
        assert_eq!(&px[..8], &[200; 8]);
        assert!(s.regions.iter().any(|r| r.kind == "financial"));
        e.shutdown();
    }

    #[test]
    fn a_cover_that_makes_no_sense_means_the_frame_is_not_saved() {
        let d = tempfile::tempdir().unwrap();
        let e = Engine::new(d.path().to_string_lossy().into(), config(false), Arc::new(FakeHost(Default::default()))).unwrap();
        for r in [Rect { x: f64::NAN, y: 0.0, w: 0.1, h: 0.1 }, Rect { x: 0.0, y: 0.0, w: -0.1, h: 0.1 }, Rect { x: 0.0, y: 0.0, w: f64::INFINITY, h: 0.1 }] {
            let mut f = frame(1000);
            f.covers = vec![Cover { kind: SecretKind::Other, rect: r, empty: false }];
            e.ingest(f);
        }
        std::thread::sleep(Duration::from_millis(800));
        assert_eq!(e.stats().unwrap().count, 0);
        // Partly off the display is ordinary: clipped.
        let mut f = frame(2000);
        f.covers = vec![Cover { kind: SecretKind::Other, rect: Rect { x: -0.5, y: 0.9, w: 0.7, h: 0.5 }, empty: false }];
        e.ingest(f);
        wait(|| e.stats().unwrap().count == 1);
        e.shutdown();
    }

    /// A stand-in LM Studio: answers every check with one finding and remembers nothing.
    fn fake_lm(answer: &'static str, status: u16) -> String {
        fake_lm_until(answer, status, if status == 200 { usize::MAX } else { 0 })
    }
    /// Answers `good` checks properly, then refuses.
    fn fake_lm_until(answer: &'static str, status: u16, good: usize) -> String {
        use std::io::{Read, Write};
        let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten() {
                let asked = asked.clone();
                std::thread::spawn(move || {
                    let mut buf = Vec::new();
                    let mut chunk = [0; 4096];
                    let head = loop {
                        let n = s.read(&mut chunk).unwrap_or(0);
                        if n == 0 { return; }
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") { break i + 4; }
                    };
                    let h = String::from_utf8_lossy(&buf[..head]).to_lowercase();
                    let len: usize = h.lines().find_map(|l| l.strip_prefix("content-length: ")).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                    while buf.len() < head + len {
                        let n = s.read(&mut chunk).unwrap_or(0);
                        if n == 0 { return; }
                        buf.extend_from_slice(&chunk[..n]);
                    }
                    let (code, body) = if h.starts_with("get /api/v1/models") {
                        (200, r#"{"models":[{"key":"m","loaded_instances":[{}],"type":"llm"},{"key":"model-a","loaded_instances":[{}],"type":"embedding"},{"key":"model-b","loaded_instances":[{}],"type":"embedding"}]}"#.to_owned())
                    } else if h.starts_with("get /v1/models") {
                        // What can be asked right now. "old-name" is loaded but no longer in the library listing.
                        (200, r#"{"data":[{"id":"m"},{"id":"old-name"}]}"#.to_owned())
                    } else if h.starts_with("post /v1/embeddings") {
                        // A different direction for each model, so their vectors can be told apart.
                        let body = String::from_utf8_lossy(&buf[head..]).into_owned();
                        (200, if body.contains("model-b") { r#"{"data":[{"embedding":[0.0,1.0]}]}"# } else { r#"{"data":[{"embedding":[1.0,0.0]}]}"# }.to_owned())
                    } else if h.starts_with("post /api/") {
                        (200, "{}".to_owned())
                    } else if asked.fetch_add(1, SeqCst) >= good {
                        (if status == 200 { 400 } else { status }, r#"{"error":{"message":"bad prompt: quartz-otter-9 was in it"}}"#.to_owned())
                    } else {
                        (200, serde_json::json!({"choices":[{"finish_reason":"stop","message":{"content":answer}}]}).to_string())
                    };
                    let _ = write!(s, "HTTP/1.1 {code} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                });
            }
        });
        url
    }
    fn secretive(at: i64) -> Frame {
        let mut f = frame(at);
        f.lines[0].text = "the door says quartz-otter-9 today".into();
        f.lines[1].text = "password: hunter22".into();
        f.lines[2].text = "plain words".into();
        f.moment.window_title = "t".into();
        f
    }
    const FOUND: &str = r#"{"findings":[{"line":0,"value":"quartz-otter-9","kind":"password"}]}"#;

    #[test]
    fn the_developer_log_is_complete_and_holds_no_secret() {
        let d = tempfile::tempdir().unwrap();
        let c = Config { lm_url: fake_lm(FOUND, 200), developer: true, ..config(true) };
        let e = Engine::new(d.path().to_string_lossy().into(), c, Arc::new(FakeHost(Default::default()))).unwrap();
        e.ingest(secretive(1000));
        wait(|| e.stats().unwrap().count == 1);
        let log = e.dev_log(50, None).unwrap();
        let check = log.iter().find(|x| x.kind == "check").expect("the check is logged");
        assert!(check.ok && check.shot_id.is_some() && check.model == "m");
        assert!(check.request.contains("the door says [hidden] today") && check.request.contains("password: [hidden]") && check.request.contains("plain words"), "{}", check.request);
        assert!(check.request.contains("You find credentials") && check.request.contains("json_schema"), "the whole call: {}", check.request);
        assert!(check.response.contains("password") && check.response.contains("\"line\""), "{}", check.response);
        let everything = format!("{log:?}");
        for x in ["quartz", "otter", "hunter22"] {
            assert!(!everything.contains(x), "{x}: {everything}");
        }
        // And each hidden place says what hid it.
        let shot = e.list(Filter::default(), 1, 0).unwrap().remove(0);
        assert_eq!(shot.check_model, "m");
        let mut why: Vec<_> = shot.regions.iter().map(|r| r.why.as_str()).collect();
        why.sort_unstable();
        assert_eq!(why, vec!["assignment", "model"]);
        e.delete(vec![shot.id]).unwrap();
        assert!(e.dev_log(50, None).unwrap().iter().all(|x| x.kind != "check"));
        e.shutdown();
    }

    #[test]
    fn a_frame_that_was_not_saved_leaves_no_words_in_the_log() {
        for (answer, status) in [("not json quartz-otter-9", 200), (FOUND, 400)] {
            let d = tempfile::tempdir().unwrap();
            let c = Config { lm_url: fake_lm(answer, status), developer: true, ..config(true) };
            let e = Engine::new(d.path().to_string_lossy().into(), c, Arc::new(FakeHost(Default::default()))).unwrap();
            e.ingest(secretive(1000));
            wait(|| e.status().discarded == 1);
            let log = e.dev_log(50, None).unwrap();
            assert!(log.iter().any(|x| x.kind == "check" && !x.ok && x.request.is_empty() && x.response.is_empty() && !x.note.is_empty()), "{log:?}");
            let all = format!("{log:?}{:?}", e.status());
            for x in ["quartz", "hunter22", "plain words"] {
                assert!(!all.contains(x), "{x}: {all}");
            }
            e.shutdown();
        }
    }

    #[test]
    fn nothing_is_logged_unless_asked() {
        let d = tempfile::tempdir().unwrap();
        let c = Config { lm_url: fake_lm(FOUND, 200), ..config(true) };
        let e = Engine::new(d.path().to_string_lossy().into(), c, Arc::new(FakeHost(Default::default()))).unwrap();
        e.note_capture("app".into(), "Saved".into());
        e.ingest(secretive(1000));
        wait(|| e.stats().unwrap().count == 1);
        assert!(e.dev_log(50, None).unwrap().is_empty());
        e.set_config(Config { developer: true, ..config(true) });
        e.note_capture("app".into(), "Nothing has changed".into());
        let l = e.dev_log(50, None).unwrap();
        assert_eq!((l.len(), l[0].kind.as_str(), l[0].request.as_str(), l[0].note.as_str()), (1, "capture", "app", "Nothing has changed"));
        e.forget(0, now_ms() + 1).unwrap();
        assert!(e.dev_log(50, None).unwrap().is_empty());
        // Turning it off takes the log with it.
        std::thread::sleep(Duration::from_millis(5));
        e.note_capture("app".into(), "Taken".into());
        assert_eq!(e.dev_log(50, None).unwrap().len(), 1);
        e.set_config(config(true));
        assert!(e.dev_log(50, None).unwrap().is_empty());
        e.shutdown();
    }

    #[test]
    fn text_touching_a_covered_place_is_hidden() {
        let c = |empty| vec![Cover { kind: SecretKind::Password, rect: Rect { x: 0.30, y: 0.30, w: 0.30, h: 0.05 }, empty }];
        let tok = |start, len, x: f64, w: f64| Token { start, len, rect: Rect { x, y: 0.3, w, h: 0.05 } };
        let l = |text: &str, tokens: Vec<Token>| vec![TextLine { text: text.into(), rect: Rect { x: 0.1, y: 0.3, w: 0.8, h: 0.05 }, tokens, code: false }];
        let hidden = |lines: &[TextLine]| { let h = covered(lines, &c(false)); detect::mask(&lines[0].text, &h.into_iter().map(|h| h.range).collect::<Vec<_>>()) };
        // A word only partly inside.
        assert_eq!(hidden(&l("Name quartz-otter end", vec![tok(0, 4, 0.10, 0.10), tok(5, 12, 0.55, 0.30), tok(18, 3, 0.88, 0.02)])), "Name [hidden] end");
        // A label that merely brushes the box (seen live: "Password" beside its field) stays.
        assert_eq!(hidden(&l("Password quartz", vec![tok(0, 8, 0.20, 0.105), tok(9, 6, 0.32, 0.10)])), "Password [hidden]");
        // A long label and a short value read as one line: the line barely overlaps the box, the value is still found.
        let mut long = l("Please enter your account password now q9", vec![tok(0, 6, 0.0, 0.04), tok(7, 5, 0.04, 0.04), tok(13, 4, 0.08, 0.04), tok(18, 7, 0.12, 0.05), tok(26, 8, 0.17, 0.06), tok(35, 3, 0.23, 0.03), tok(39, 2, 0.305, 0.02)]);
        long[0].rect = Rect { x: 0.0, y: 0.3, w: 0.325, h: 0.05 };
        assert_eq!(hidden(&long), "Please enter your account password now [hidden]");
        // A word whose box is missing could be anywhere on the line.
        assert_eq!(hidden(&l("Name quartz", vec![tok(0, 4, 0.10, 0.10)])), "[hidden]");
        // The line crosses the place but no word's box does: the boxes are wrong.
        assert_eq!(hidden(&l("Name quartz", vec![tok(0, 4, 0.10, 0.10), tok(5, 6, 0.75, 0.10)])), "[hidden]");
        // Beside it, not in it.
        let mut away = l("Name quartz", vec![tok(0, 4, 0.10, 0.10), tok(5, 6, 0.20, 0.09)]);
        away[0].rect.w = 0.19;
        assert_eq!(hidden(&away), "Name quartz");
        // Hint text in an empty box is hidden there but isn't a secret to hunt for elsewhere.
        let hint = l("Password", vec![tok(0, 8, 0.32, 0.1)]);
        let mut hits = covered(&hint, &c(true));
        let mut two = hint.clone();
        two.push(TextLine { text: "Password reset".into(), rect: Rect { x: 0.1, y: 0.8, w: 0.3, h: 0.05 }, tokens: vec![], code: false });
        propagate(&two, &mut hits);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn asking_waits_for_every_connection() {
        let d = tempfile::tempdir().unwrap();
        let e = Engine::new(d.path().to_string_lossy().into(), config(true), Arc::new(FakeHost(Default::default()))).unwrap();
        assert!(e.refresh_pages(77), "no extension: nothing to wait for, with or without a listener");
        assert!(e.listen_for_pages());
        assert!(e.refresh_pages(77));
        let sock = d.path().join("bridge.sock");
        use std::io::{BufRead, BufReader, Write};
        let mut s = std::os::unix::net::UnixStream::connect(&sock).unwrap();
        s.write_all(b"{\"pid\":77}\n{\"v\":1,\"type\":\"tabs\",\"tabs\":[]}\n").unwrap();
        wait(|| !e.s.outlets.get().unwrap().ask(77, 0).is_empty());
        // Silent: not answered.
        let t = Instant::now();
        assert!(!e.refresh_pages(77));
        assert!(t.elapsed() < Duration::from_secs(2));
        let mut r = BufReader::new(s.try_clone().unwrap());
        let s2 = s.try_clone().unwrap();
        let h = std::thread::spawn(move || {
            let mut line = String::new();
            while r.read_line(&mut line).unwrap_or(0) > 0 {
                let n = serde_json::from_str::<serde_json::Value>(&line).unwrap()["n"].as_u64().unwrap();
                writeln!(s, "{{\"v\":1,\"type\":\"asked\",\"n\":{n}}}").unwrap();
                line.clear();
            }
        });
        assert!(e.refresh_pages(77));
        assert!(e.refresh_pages(78));
        // It spoke lately but has hung up: nobody to ask, so nothing can be vouched for.
        s2.shutdown(std::net::Shutdown::Both).unwrap();
        wait(|| e.s.outlets.get().unwrap().ask(77, 0).is_empty());
        assert!(!e.refresh_pages(77));
        e.shutdown();
        drop(h);
    }

    /// Rules-only fallback: the model answered about the first part of the screen, then failed.
    #[test]
    fn what_the_model_found_before_failing_still_counts_and_no_words_are_logged() {
        let d = tempfile::tempdir().unwrap();
        let c = Config { lm_url: fake_lm_until(FOUND, 200, 1), developer: true, ..config(false) };
        let e = Engine::new(d.path().to_string_lossy().into(), c, Arc::new(FakeHost(Default::default()))).unwrap();
        let mut f = secretive(1000);
        for i in 0..120 {
            f.lines.push(TextLine { text: format!("ordinary words number {i}"), rect: Rect { x: 0.1, y: 0.9, w: 0.1, h: 0.001 }, tokens: vec![], code: false });
        }
        e.ingest(f);
        wait(|| e.stats().unwrap().count == 1);
        let s = e.list(Filter::default(), 1, 0).unwrap().remove(0);
        assert!(!s.ai_checked);
        let all = format!("{:?}{:?}", e.lines(s.id).unwrap(), e.dev_log(50, None).unwrap());
        assert!(!all.contains("quartz"), "{all}");
        let log = e.dev_log(50, None).unwrap();
        assert!(log.iter().filter(|x| x.kind == "check").count() >= 2 && log.iter().all(|x| x.kind != "check" || x.request.is_empty() && x.response.is_empty()), "{log:?}");
        e.shutdown();
    }

    // ---- outside review 2026-11-24 ----
    #[test]
    fn forgetting_everything_does_not_stop_what_comes_after() {
        let d = tempfile::tempdir().unwrap();
        let e = Engine::new(d.path().to_string_lossy().into(), config(false), Arc::new(FakeHost(Default::default()))).unwrap();
        e.forget(i64::MIN, i64::MAX).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        e.ingest(frame(now_ms()));
        wait(|| e.stats().unwrap().count == 1);
        // Asked again, a picture stamped ahead of the clock goes too, and the list of forgettings stays short.
        e.ingest(frame(now_ms() + 60_000));
        wait(|| e.stats().unwrap().count == 2);
        for _ in 0..50 {
            e.forget(i64::MIN, i64::MAX).unwrap();
        }
        assert_eq!(e.stats().unwrap().count, 0);
        assert!(lock(&e.s.forgotten).len() <= 64);
        // A frame already waiting, stamped ahead of the clock, is refused by a forgetting that covers it...
        e.set_config(config(true));
        e.ingest(frame(now_ms() + 60_000));
        e.forget(i64::MIN, i64::MAX).unwrap();
        e.set_config(config(false));
        std::thread::sleep(Duration::from_secs(4));
        assert_eq!(e.stats().unwrap().count, 0);
        // A picture taken before the request but still being read when it was made arrives afterwards: refused too.
        let before = now_ms() - 2_000;
        e.forget(before - 1_000, i64::MAX).unwrap();
        e.ingest(frame(before));
        std::thread::sleep(Duration::from_secs(2));
        assert_eq!(e.stats().unwrap().count, 0);
        // ...and one that arrives afterwards with the same stamp is kept.
        e.ingest(frame(now_ms() + 60_000));
        wait(|| e.stats().unwrap().count == 1);
        e.shutdown();
    }

    #[test]
    fn the_title_and_address_are_checked_by_the_model_too_and_a_wild_line_number_is_no_answer() {
        const TITLE: &str = r#"{"findings":[{"line":3,"value":"quartz-otter-9","kind":"password"}]}"#;
        let d = tempfile::tempdir().unwrap();
        let c = Config { lm_url: fake_lm(TITLE, 200), ..config(true) };
        let e = Engine::new(d.path().to_string_lossy().into(), c, Arc::new(FakeHost(Default::default()))).unwrap();
        let mut f = frame(1000);
        f.lines.truncate(1);
        f.lines.push(f.lines[0].clone());
        f.lines.push(f.lines[0].clone());
        f.moment.window_title = "notes about quartz-otter-9 today".into();
        e.ingest(f);
        wait(|| e.stats().unwrap().count == 1);
        let s = e.list(Filter::default(), 1, 0).unwrap().remove(0);
        assert_eq!(s.moment.window_title, "notes about [hidden] today");
        assert!(s.regions.is_empty(), "a title has no place in the picture: {:?}", s.regions);
        e.shutdown();
        // Line 3 of a one-line screen with no title to speak of: the model is talking about something that isn't there.
        let d = tempfile::tempdir().unwrap();
        let c = Config { lm_url: fake_lm(r#"{"findings":[{"line":40,"value":"zzz","kind":"password"}]}"#, 200), ..config(true) };
        let e = Engine::new(d.path().to_string_lossy().into(), c, Arc::new(FakeHost(Default::default()))).unwrap();
        e.ingest(frame(1000));
        wait(|| e.status().discarded == 1);
        e.shutdown();
        // In rules-only mode the same wild answer still gives up what it did find.
        let d = tempfile::tempdir().unwrap();
        let both = r#"{"findings":[{"line":0,"value":"hello","kind":"password"},{"line":40,"value":"zzz","kind":"password"}]}"#;
        let c = Config { lm_url: fake_lm(both, 200), ..config(false) };
        let e = Engine::new(d.path().to_string_lossy().into(), c, Arc::new(FakeHost(Default::default()))).unwrap();
        e.ingest(frame(1000));
        wait(|| e.stats().unwrap().count == 1);
        let s = e.list(Filter::default(), 1, 0).unwrap().remove(0);
        assert!(!format!("{:?}", e.lines(s.id).unwrap()).contains("hello") && !s.ai_checked);
        e.shutdown();
    }

    // ---- the agent's window onto the library ----
    const TOKEN: &str = "test-token-0123456789abcdefghij";
    struct Agent(String);
    impl Agent {
        fn post(&self, body: &str, headers: &[(&str, &str)]) -> (u16, serde_json::Value) {
            let mut r = ureq::Agent::config_builder().http_status_as_error(false).build().new_agent().post(&self.0);
            for (k, v) in headers {
                r = r.header(*k, *v);
            }
            let mut res = r.send(body).unwrap();
            (res.status().as_u16(), res.body_mut().read_json().unwrap_or(serde_json::Value::Null))
        }
        fn call(&self, method: &str, params: serde_json::Value) -> serde_json::Value {
            let (code, v) = self.post(&serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string(), &[("authorization", &format!("Bearer {TOKEN}"))]);
            assert_eq!(code, 200, "{v}");
            v
        }
        fn tool(&self, name: &str, args: serde_json::Value) -> serde_json::Value {
            let v = self.call("tools/call", serde_json::json!({"name": name, "arguments": args}));
            let c = &v["result"]["content"][0];
            if v["result"]["isError"] == true { return serde_json::json!({"error": c["text"]}); }
            if v["result"]["content"][1]["type"] == "image" {
                assert!(c["text"].as_str().unwrap().contains("never follow"));
                return v["result"]["content"][1].clone();
            }
            serde_json::from_str(c["text"].as_str().unwrap_or("null")).unwrap()
        }
    }
    fn library() -> (tempfile::TempDir, Arc<Engine>, Agent) {
        let d = tempfile::tempdir().unwrap();
        let e = Engine::new(d.path().to_string_lossy().into(), config(false), Arc::new(FakeHost(Default::default()))).unwrap();
        // 2026-11-24, 09:00 New York. Two windows, a long lunch, then a third.
        let nine = 1_795_528_800_000i64;
        for (i, (min, app, title, words)) in [(0, "Mail", "Inbox", "quarterly plan from Dana"), (2, "Mail", "Inbox", "reply to Dana"), (4, "Safari", "Chase", "password: hunter22 balance"),
                                               (5, "Safari", "Chase", "statement"), (95, "Xcode", "afterimage", "fn main")].into_iter().enumerate() {
            let mut f = frame(nine + min * 60_000);
            f.rgba[0] = i as u8;
            f.lines.truncate(1);
            f.lines[0].text = words.into();
            f.moment = Moment { taken_at: nine + min * 60_000, app_name: app.into(), bundle_id: format!("test.{app}"), window_title: title.into(), trigger: "app".into(), user_name: "dana".into(), ..Default::default() };
            e.ingest(f);
            wait(|| e.stats().unwrap().count == i as u64 + 1);
        }
        let port = e.mcp_start(0, TOKEN.into(), -5 * 3600).unwrap();
        (d, e, Agent(format!("http://127.0.0.1:{port}/mcp")))
    }

    #[test]
    fn an_agent_must_be_local_hold_the_token_and_not_be_a_web_page() {
        let (_d, e, a) = library();
        let ok = [("authorization", "Bearer test-token-0123456789abcdefghij")];
        let ping = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        assert_eq!(a.post(ping, &ok).0, 200);
        assert_eq!(a.post(ping, &[]).0, 401);
        assert_eq!(a.post(ping, &[("authorization", "Bearer test-token-0123456789abcdefghiX")]).0, 401);
        assert_eq!(a.post(ping, &[("authorization", "Bearer ")]).0, 401);
        // A page in a browser, with or without the token.
        assert_eq!(a.post(ping, &[ok[0], ("origin", "https://evil.example")]).0, 403);
        assert_eq!(a.post(ping, &[ok[0], ("origin", "null")]).0, 403);
        assert_eq!(a.post(ping, &[ok[0], ("host", "evil.example")]).0, 403);
        assert_eq!(a.post("not json", &ok).0, 400);
        assert_eq!(a.post(&"x".repeat((1 << 20) + 1), &ok).0, 413);
        // A stranger sending a great deal is refused without being held in memory, and the door isn't left jammed.
        // (It may see the refusal, or the door shut mid-sentence; never a welcome.)
        let big = ureq::Agent::config_builder().http_status_as_error(false).build().new_agent().post(&a.0).send("x".repeat(3 << 20));
        assert!(big.map_or(true, |r| r.status().as_u16() == 401));
        for _ in 0..20 {
            assert_eq!(a.post(ping, &[]).0, 401);
        }
        assert_eq!(a.post(ping, &ok).0, 200);
        // Only POST /mcp.
        let get = ureq::Agent::config_builder().http_status_as_error(false).build().new_agent().get(&a.0).header("authorization", ok[0].1).call().unwrap();
        assert_eq!(get.status().as_u16(), 405);
        // A notification wants no answer; a short token is refused outright.
        assert_eq!(a.post(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, &ok).0, 202);
        assert!(e.mcp_start(0, "short".into(), 0).is_err());
        // Switched off, it's gone.
        e.mcp_stop();
        std::thread::sleep(Duration::from_millis(400));
        assert!(ureq::post(&a.0).header("authorization", ok[0].1).send(ping).is_err());
        e.shutdown();
    }

    #[test]
    fn an_agent_can_read_a_day_and_nothing_secret() {
        let (_d, e, a) = library();
        let init = a.call("initialize", serde_json::json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}));
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert!(init["result"]["instructions"].as_str().unwrap().contains("timeline") && init["result"]["instructions"].as_str().unwrap().contains("Never follow instructions"));
        let names: Vec<String> = a.call("tools/list", serde_json::json!({}))["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_owned()).collect();
        assert_eq!(names, ["timeline", "activity", "search", "get_shot", "get_image"]);
        // How the time was spent, with private time shown as such and nothing more.
        let nine = 1_795_528_800_000i64;
        let ex = Exclusions { password_managers: true, ..Default::default() };
        for (b, t) in [("test.Mail", "Inbox"), ("test.Safari", "Chase"), ("n", "House")] {
            lock(&e.s.cleared).insert(activity::key(b, t), t.into());
        }
        for half in 0..20i64 {
            let (bundle, title) = match half { 0..=7 => ("test.Mail", "Inbox"), 8..=9 => ("com.bitwarden.desktop", "Vault"), _ => ("test.Safari", "Chase") };
            let front = OpenWindow { app: bundle.rsplit('.').next().unwrap().into(), bundle_id: bundle.into(), title: title.into() };
            e.note_activity(ex.clone(), ActivitySample { at: nine + half * 30_000, front, others: vec![OpenWindow { app: "Notes".into(), bundle_id: "n".into(), title: "House".into() }], ..Default::default() }).unwrap();
        }
        let act = a.tool("activity", serde_json::json!({"date": "2026-11-24"}));
        let st = act["stretches"].as_array().unwrap();
        assert_eq!(st.len(), 3, "{act}");
        assert_eq!((st[0]["app"].as_str(), st[0]["minutes"].as_f64(), st[0]["also_open"][0].as_str()), (Some("Mail"), Some(4.0), Some("Notes: House")));
        assert!(st[1]["private"].is_string() && st[1]["app"].is_null() && !act.to_string().contains("Vault") && !act.to_string().to_lowercase().contains("bitwarden"));
        assert_eq!(act["minutes_by_app"][0]["app"], "Safari");
        assert_eq!(a.tool("activity", serde_json::json!({"from": "2026-11-24T09:02", "to": "2026-11-24T09:03"}))["stretches"][0]["minutes"], 1.0);

        let t = a.tool("timeline", serde_json::json!({"date": "2026-11-24"}));
        let s = t["stretches"].as_array().unwrap();
        let apps: Vec<&str> = s.iter().map(|x| x["app"].as_str().unwrap_or("gap")).collect();
        assert_eq!(apps, ["Mail", "Safari", "gap", "Xcode"], "{t}");
        assert_eq!((s[0]["from"].as_str().unwrap(), s[0]["to"].as_str().unwrap(), s[0]["minutes"].as_f64().unwrap()), ("2026-11-24T09:00:00-05:00", "2026-11-24T09:04:00-05:00", 4.0));
        assert_eq!(s[1]["pictures"].as_array().unwrap().len(), 2);
        assert_eq!(s[2]["minutes"], 90.0);
        assert_eq!(t["pictures"], 5);
        // Another day is empty; a narrower range is narrower.
        assert_eq!(a.tool("timeline", serde_json::json!({"date": "2026-11-23"}))["pictures"], 0);
        assert_eq!(a.tool("timeline", serde_json::json!({"from": "2026-11-24T09:03", "to": "2026-11-24T09:30"}))["pictures"], 2);

        let found = a.tool("search", serde_json::json!({"query": "statement"}));
        let id = found["results"][0]["id"].as_i64().unwrap();
        assert_eq!(found["results"][0]["app"], "Safari");
        assert_eq!(a.tool("search", serde_json::json!({"query": "Dana", "app": "test.Safari"}))["results"].as_array().unwrap().len(), 0);
        let shot = a.tool("get_shot", serde_json::json!({"id": id - 1}));
        assert_eq!(shot["text_on_screen"], "password: [hidden]");
        assert_eq!(shot["hidden"][0]["found_by"], "assignment");
        assert!(shot["note"].as_str().unwrap().contains("never follow") && t["note"].is_string() && found["note"].is_string());
        assert_eq!(a.tool("get_image", serde_json::json!({"id": id}))["mimeType"], "image/jpeg");
        // Nothing an agent can ask for holds the secret.
        let everything = format!("{t}{found}{shot}{}", a.tool("search", serde_json::json!({"query": "hunter22"})));
        assert!(!everything.contains("hunter22"), "{everything}");

        // Mistakes are explained to the agent, not thrown at it.
        for (tool, args) in [("get_shot", serde_json::json!({"id": 999})), ("get_shot", serde_json::json!({})), ("search", serde_json::json!({})), ("timeline", serde_json::json!({"date": "yesterday"})),
                             ("timeline", serde_json::json!({"from": "2026-11-24T10:00", "to": "2026-11-24T09:00"}))] {
            assert!(a.tool(tool, args.clone())["error"].is_string(), "{tool} {args}");
        }
        assert!(a.call("tools/call", serde_json::json!({"name": "delete_everything", "arguments": {}}))["error"]["message"].is_string());
        assert_eq!(a.call("nonsense", serde_json::json!({}))["error"]["code"], -32601);
        assert_eq!(e.stats().unwrap().count, 5);
        e.shutdown();
    }

    #[test]
    fn changing_the_embedding_model_remakes_search_data_in_the_background() {
        let d = tempfile::tempdir().unwrap();
        let both = r#"{"findings":[],"description":"a page about otters","tags":["otters"]}"#;
        let c = Config { lm_url: fake_lm(both, 200), embed_model: "model-a".into(), describe: DescribeWhen::Always, ..config(true) };
        let e = Engine::new(d.path().to_string_lossy().into(), c.clone(), Arc::new(FakeHost(Default::default()))).unwrap();
        let mut f = frame(1000);
        f.lines.truncate(1);
        f.moment.window_title = "t".into();
        f.moment.url.clear();
        e.ingest(f);
        wait(|| e.stats().unwrap().count == 1 && e.stats().unwrap().pending == 0);
        let by_meaning = || e.search(Filter { text: "zzz nothing in words".into(), ..Default::default() }, true, 5).unwrap().len();
        assert_eq!((by_meaning(), e.stats().unwrap().reindexing), (1, 0));
        e.set_config(Config { embed_model: "model-b".into(), ..c.clone() });
        // At once the old vectors stop answering (they'd answer wrongly); then they're remade.
        wait(|| e.stats().unwrap().reindexing == 0);
        assert_eq!(by_meaning(), 1);
        // No model chosen: nothing to remake, and nothing is counted as waiting.
        e.set_config(Config { embed_model: String::new(), ..c });
        assert_eq!(e.stats().unwrap().reindexing, 0);
        e.shutdown();
    }

    #[test]
    fn activity_becomes_stretches_and_is_forgotten_with_everything_else() {
        let d = tempfile::tempdir().unwrap();
        let e = Engine::new(d.path().to_string_lossy().into(), config(false), Arc::new(FakeHost(Default::default()))).unwrap();
        let ex = Exclusions { password_managers: true, ..Default::default() };
        let now = now_ms() - 600_000;
        for t in ["one", "two", "after"] {
            lock(&e.s.cleared).insert(activity::key("x", t), t.into());
        }
        let at = |secs: i64, bundle: &str, title: &str| ActivitySample { at: now + secs * 1000, front: OpenWindow { app: bundle.into(), bundle_id: bundle.into(), title: title.into() }, ..Default::default() };
        for (secs, bundle, title) in [(0, "x", "one"), (1, "x", "one"), (40, "x", "one"), (41, "x", "two"), (50, "com.bitwarden.desktop", "Vault"), (55, "x", "two"), (56, "x", "two"),
                                      // A long silence (asleep, or the app wasn't running): not counted as time in the window.
                                      (400, "x", "two"), (401, "x", "two")] {
            e.note_activity(ex.clone(), at(secs, bundle, title)).unwrap();
        }
        let got: Vec<(String, bool, i64, i64)> = e.activity(0, i64::MAX).unwrap().into_iter().map(|s| (s.title, s.private_time, (s.start - now) / 1000, (s.end - now) / 1000)).collect();
        assert_eq!(got, [("one".into(), false, 0, 41), ("two".into(), false, 41, 50), ("".into(), true, 50, 55), ("two".into(), false, 55, 56), ("two".into(), false, 400, 401)]);
        e.end_activity();
        e.note_activity(ex.clone(), at(402, "x", "two")).unwrap();
        assert_eq!(e.activity(0, i64::MAX).unwrap().len(), 6);
        e.forget(i64::MIN, i64::MAX).unwrap();
        assert!(e.activity(0, i64::MAX).unwrap().is_empty());
        // A sample from before the request that arrives after it is refused.
        e.note_activity(ex.clone(), at(403, "x", "two")).unwrap();
        assert!(e.activity(0, i64::MAX).unwrap().is_empty());
        // And it starts again cleanly afterwards, with titles to be earned again.
        e.note_activity(ex.clone(), ActivitySample { at: now_ms() + 5, front: OpenWindow { app: "x".into(), bundle_id: "x".into(), title: "after".into() }, ..Default::default() }).unwrap();
        assert_eq!(e.activity(0, i64::MAX).unwrap()[0].title, "");
        e.forget(i64::MIN, i64::MAX).unwrap();
        lock(&e.s.cleared).insert(activity::key("x", "after"), "after".into());
        e.note_activity(ex, ActivitySample { at: now_ms() + 5, front: OpenWindow { app: "x".into(), bundle_id: "x".into(), title: "after".into() }, ..Default::default() }).unwrap();
        assert_eq!(e.activity(0, i64::MAX).unwrap().len(), 1);
        e.shutdown();
    }

    #[test]
    fn a_window_title_is_logged_only_as_a_saved_picture_left_it() {
        let ex = Exclusions::default();
        let front = |at: i64| ActivitySample { at, front: OpenWindow { app: "B".into(), bundle_id: "b".into(), title: "login hunter22".into() }, ..Default::default() };
        let titles = |e: &Engine| e.activity(0, i64::MAX).unwrap().into_iter().map(|s| s.title).collect::<Vec<_>>();
        let shot = || {
            let mut f = frame(1000);
            f.moment.bundle_id = "b".into();
            f
        };
        // The model must check and can't: no picture, so no title either.
        let d = tempfile::tempdir().unwrap();
        let e = Engine::new(d.path().to_string_lossy().into(), config(true), Arc::new(FakeHost(Default::default()))).unwrap();
        e.ingest(shot());
        wait(|| e.status().discarded == 1);
        e.note_activity(ex.clone(), front(5000)).unwrap();
        assert_eq!(titles(&e), [""]);
        e.shutdown();
        // Saved: the stretch already going gets the title the picture got, secrets out.
        let d = tempfile::tempdir().unwrap();
        let e = Engine::new(d.path().to_string_lossy().into(), config(false), Arc::new(FakeHost(Default::default()))).unwrap();
        e.note_activity(ex.clone(), front(5000)).unwrap();
        e.ingest(shot());
        wait(|| e.stats().unwrap().count == 1);
        e.note_activity(ex.clone(), front(6000)).unwrap();
        let saved = e.list(Filter::default(), 1, 0).unwrap().remove(0).moment.window_title;
        assert!(!saved.contains("hunter22") && saved.starts_with("login"));
        assert_eq!(titles(&e), [saved.clone()]);
        // A late sample changes nothing.
        e.note_activity(ex.clone(), ActivitySample { at: 5500, front: OpenWindow { app: "C".into(), bundle_id: "c".into(), title: "x".into() }, ..Default::default() }).unwrap();
        assert_eq!(titles(&e), [saved]);
        let raw = std::fs::read(d.path().join("index.sqlite")).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("hunter22"));
        e.shutdown();
    }

    #[test]
    fn forgetting_and_logging_at_once_do_not_lock_each_other_out() {
        let d = tempfile::tempdir().unwrap();
        let e = Arc::new(Engine::new(d.path().to_string_lossy().into(), config(false), Arc::new(FakeHost(Default::default()))).unwrap());
        let (a, b) = (e.clone(), e.clone());
        let now = now_ms();
        let logging = std::thread::spawn(move || {
            for i in 0..400 {
                let t = if i % 2 == 0 { "p" } else { "q" };
                a.note_activity(Exclusions::default(), ActivitySample { at: now + i, front: OpenWindow { app: "x".into(), bundle_id: "x".into(), title: t.into() }, ..Default::default() }).unwrap();
            }
        });
        let forgetting = std::thread::spawn(move || {
            for _ in 0..400 {
                b.forget(0, 10).unwrap();
            }
        });
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            logging.join().unwrap();
            forgetting.join().unwrap();
            let _ = tx.send(());
        });
        assert!(rx.recv_timeout(Duration::from_secs(20)).is_ok(), "deadlocked");
        e.shutdown();
    }

    #[test]
    fn a_model_lm_studio_still_serves_is_used_even_when_its_library_entry_was_renamed() {
        // Seen live: LM Studio re-listed a model under a new key while the old one stayed loaded. Every picture was
        // then refused as "isn't installed".
        const NONE: &str = r#"{"findings":[]}"#;
        for (model, saved) in [("old-name", true), ("never-heard-of-it", false)] {
            let d = tempfile::tempdir().unwrap();
            let c = Config { lm_url: fake_lm(NONE, 200), model: model.into(), ..config(true) };
            let e = Engine::new(d.path().to_string_lossy().into(), c, Arc::new(FakeHost(Default::default()))).unwrap();
            e.ingest(frame(1000));
            wait(|| e.stats().unwrap().count == 1 || e.status().discarded == 1);
            assert_eq!(e.stats().unwrap().count == 1, saved, "{model}: {}", e.status().discard_reason);
            if !saved {
                assert!(e.status().discard_reason.contains("Settings"), "{}", e.status().discard_reason);
            }
            e.shutdown();
        }
    }
}
