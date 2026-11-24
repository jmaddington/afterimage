//! Afterimage engine. Swift captures and reads the screen; everything after that happens here.
//!
//! AIDEV-NOTE: privacy-critical invariant: raw pixels live only in RAM. `process` is the sole path to disk, and
//! AIDEV-NOTE: `HostServices::encode` is only ever handed a frame whose secrets have been frosted over.

pub mod bridge;
mod detect;
mod lm;
pub mod pages;
mod privacy;
mod redact;
mod store;

pub use detect::{Rect, SecretKind, TextLine, Token};
pub use lm::ModelInfo;
pub use privacy::*;
pub use store::*;

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

#[derive(Default)]
struct Queue {
    frames: VecDeque<Frame>,
    stop: bool,
}

struct Shared {
    host: Arc<dyn HostServices>,
    root: PathBuf,
    store: Mutex<Store>,
    config: Mutex<Config>,
    queue: Mutex<Queue>,
    wake: Condvar,
    /// Last accepted thumbprint per display; cleared when a frame is lost so the next look is taken afresh.
    prints: Mutex<HashMap<String, Vec<u8>>>,
    /// Line neighbourhoods the model has already called clean. Keyed hash, RAM only.
    clean: Mutex<HashSet<u64>>,
    hasher: RandomState,
    forgotten: Mutex<Vec<(i64, i64)>>,
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
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

const QUEUE_CAP: usize = 6;
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
        if let Err(e) = store.sweep_orphans() {
            log::error!("orphan sweep failed: {e} [AIMG-ORPHAN]");
        }
        let s = Arc::new(Shared {
            host, root, store: Mutex::new(store), config: Mutex::new(config),
            queue: Default::default(), wake: Condvar::new(), prints: Default::default(), clean: Default::default(),
            hasher: RandomState::new(), forgotten: Default::default(), loaded_by_us: Default::default(),
            last_lm_use: Mutex::new(Instant::now()), lm_state: Mutex::new(LmState::Unknown), links: Default::default(),
            discarded: AtomicU64::new(0), reason: Default::default(), busy: AtomicBool::new(false),
            describing: AtomicBool::new(false), on_power: AtomicBool::new(true), calm: AtomicBool::new(true), pages: Default::default(),
        });
        for (name, f) in [("afterimage-save", save_loop as fn(Arc<Shared>)), ("afterimage-index", index_loop)] {
            let s = s.clone();
            std::thread::Builder::new().name(name.into()).spawn(move || f(s)).map_err(|e| EngineError::Failed(e.to_string()))?;
        }
        Ok(Arc::new(Self { s }))
    }

    pub fn set_config(&self, config: Config) {
        *lock(&self.s.config) = config;
        self.s.wake.notify_all();
    }

    /// Hear from the browser extension, if it's there. Its absence changes nothing.
    pub fn listen_for_pages(&self) -> bool {
        let s = self.s.clone();
        let heard = move |e: bridge::Event| lock(&s.pages).receive(e.pid, e.source, &e.message, Instant::now());
        match bridge::listen(&self.s.root.join("bridge.sock"), heard) {
            Ok(()) => true,
            Err(e) => {
                log::error!("can't listen for the browser extension: {e} [AIMG-BRIDGE]");
                false
            }
        }
    }

    pub fn page_hint(&self, pid: i32) -> pages::PageHint {
        lock(&self.s.pages).hint(pid, Instant::now())
    }

    /// `calm`: thermal state is fine and Low Power Mode is off.
    pub fn set_conditions(&self, on_power: bool, calm: bool) {
        self.s.on_power.store(on_power, SeqCst);
        self.s.calm.store(calm, SeqCst);
    }

    /// Cheap pre-check on a small greyscale thumbprint. False means "same screen as last time".
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
        let mut q = lock(&self.s.queue);
        if q.frames.len() >= QUEUE_CAP {
            q.frames.pop_front();
            self.s.discard("The model couldn't keep up");
        }
        q.frames.push_back(frame);
        drop(q);
        self.s.wake.notify_all();
    }

    pub fn list(&self, filter: Filter, limit: u32, offset: u32) -> Result<Vec<Shot>, EngineError> {
        Ok(lock(&self.s.store).list(&filter, limit, offset)?)
    }

    /// Keyword search, plus meaning when `semantic` and the embedding model answers in time.
    pub fn search(&self, filter: Filter, semantic: bool, limit: u32) -> Result<Vec<Shot>, EngineError> {
        let c = lock(&self.s.config).clone();
        let emb = if semantic && !c.embed_model.is_empty() && filter.text.trim().chars().count() >= 3 {
            self.s.lm(&c).and_then(|l| {
                self.s.ensure(&l, &c.embed_model, 2048)?;
                l.embed(&c.embed_model, &format!("search_query: {}", filter.text), 10)
            }).ok()
        } else {
            None
        };
        Ok(lock(&self.s.store).search(&filter, emb.as_deref(), limit)?)
    }

    pub fn lines(&self, id: i64) -> Result<Vec<IndexedLine>, EngineError> {
        Ok(lock(&self.s.store).lines(id)?)
    }
    pub fn apps(&self) -> Result<Vec<AppCount>, EngineError> {
        Ok(lock(&self.s.store).apps()?)
    }
    pub fn stats(&self) -> Result<Stats, EngineError> {
        Ok(lock(&self.s.store).stats()?)
    }
    pub fn set_kept(&self, ids: Vec<i64>, kept: bool) -> Result<(), EngineError> {
        lock(&self.s.store).set_kept(&ids, kept)?;
        self.s.host.library_changed();
        Ok(())
    }

    pub fn delete(&self, ids: Vec<i64>) -> Result<u64, EngineError> {
        let (n, failed) = lock(&self.s.store).delete(&ids)?;
        self.s.host.library_changed();
        if failed > 0 {
            return Err(EngineError::Failed(format!("{failed} files couldn't be removed. Afterimage will try again next time it opens.")));
        }
        Ok(n)
    }

    /// Delete a span of time, including frames still waiting in memory. Kept shots go too: the person asked.
    pub fn forget(&self, from: i64, to: i64) -> Result<u64, EngineError> {
        // Order matters: mark first, so a frame mid-flight is refused at insert time.
        lock(&self.s.forgotten).push((from, to));
        lock(&self.s.queue).frames.retain(|f| !(from..to).contains(&f.moment.taken_at));
        let ids = lock(&self.s.store).ids_matching(&Filter { from: Some(from), to: Some(to), ..Default::default() })?;
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
        self.s.unload_ours();
    }
}

impl Shared {
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
        let ids = st.expired(now_ms(), c.max_age_days, c.max_bytes)?;
        if ids.is_empty() {
            return Ok(0);
        }
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
    fn ai_hits(&self, c: &Config, lines: &[TextLine], known: &[Hit]) -> Result<Vec<Hit>, lm::LmError> {
        let keys: Vec<u64> = (0..lines.len()).map(|i| self.line_key(lines, i)).collect();
        let todo: Vec<usize> = {
            let clean = lock(&self.clean);
            (0..lines.len())
                .filter(|&i| !lines[i].code && lines[i].text.chars().filter(|c| c.is_alphanumeric()).count() >= 3 && !clean.contains(&keys[i]))
                .collect()
        };
        if todo.is_empty() {
            return Ok(vec![]);
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
        let l = self.lm(c)?;
        self.ensure(&l, &c.model, CONTEXT)?;

        let mut hits = Vec::new();
        let mut at = 0;
        while at < text.len() {
            let mut end = at;
            let mut chars = 0;
            while end < text.len() && end - at < CHUNK_LINES && (end == at || chars + text[end].1.len() < CHUNK_CHARS) {
                chars += text[end].1.len();
                end += 1;
            }
            hits.extend(self.ask(&l, c, lines, &text[at..end])?);
            *lock(&self.last_lm_use) = Instant::now();
            if end == text.len() {
                break;
            }
            // Overlap so a label and its value are never split. Always advance.
            at = end.saturating_sub(3).max(at + 1);
        }
        let dirty: HashSet<usize> = hits.iter().chain(known).map(|h| h.line).collect();
        let mut clean = lock(&self.clean);
        if clean.len() > 30_000 {
            clean.clear();
        }
        clean.extend(todo.iter().filter(|i| !dirty.contains(i)).map(|&i| keys[i]));
        Ok(hits)
    }

    /// One chunk; an unusable answer is retried on halves before giving up.
    fn ask(&self, l: &lm::Lm, c: &Config, lines: &[TextLine], chunk: &[(usize, String)]) -> Result<Vec<Hit>, lm::LmError> {
        let view: Vec<(usize, &str)> = chunk.iter().map(|(i, t)| (*i, t.as_str())).collect();
        match l.find_secrets(&c.model, &view) {
            Ok(f) => Ok(f
                .into_iter()
                .filter(|(_, v, _)| !v.contains("[hidden]"))
                .flat_map(|(i, v, k)| lm::locate(lines, i, &v, k))
                .collect()),
            Err(lm::LmError::BadAnswer(_)) if chunk.len() >= 8 => {
                let m = chunk.len() / 2;
                let mut a = self.ask(l, c, lines, &chunk[..m + 2])?;
                a.extend(self.ask(l, c, lines, &chunk[m.saturating_sub(2)..])?);
                Ok(a)
            }
            Err(e) => Err(e),
        }
    }

    /// Redact and save one frame. Err(reason) means nothing was written.
    fn process(&self, f: &mut Frame, require_ai: bool) -> Result<(), String> {
        let c = lock(&self.config).clone();
        let mut hits = detect::scan(&f.lines);
        let ai_checked = match self.ai_hits(&c, &f.lines, &hits) {
            Ok(h) => {
                hits.extend(h);
                true
            }
            Err(e) if require_ai => return Err(e.to_string()),
            Err(_) => false,
        };
        detect::extend_wrapped(&f.lines, &mut hits);
        propagate(&f.lines, &mut hits);

        let mut regions: Vec<Region> = Vec::new();
        for h in &hits {
            let r = region(h.kind, detect::hit_rect(&f.lines[h.line], &h.range));
            if !regions.iter().any(|x| x.rect == r.rect) {
                regions.push(r);
            }
        }
        let rects: Vec<Rect> = regions.iter().map(|r| r.rect).collect();
        redact::frost(&mut f.rgba, f.width as usize, f.height as usize, f.stride as usize, &rects);

        let values: Vec<String> = secret_values(&f.lines, &hits);
        let scrub = |t: &str| values.iter().fold(detect::mask_text(t), |t, v| {
            let l = [TextLine { text: t, rect: Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }, tokens: vec![], code: false }];
            let r: Vec<_> = detect::find_secret(&l, v).into_iter().map(|(_, r)| r).collect();
            detect::mask(&l[0].text, &r)
        });
        let mut m = f.moment.clone();
        m.window_title = scrub(&m.window_title);
        m.url = scrub(&detect::scrub_url(&m.url));
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
        if lock(&self.forgotten).iter().any(|(a, b)| (*a..*b).contains(&at)) {
            drop(store);
            remove();
            return Ok(());
        }
        let new = NewShot { moment: m, path: path.clone(), thumb_path: thumb_path.clone(), width: e.width, height: e.height, bytes: e.bytes, regions, lines, ai_checked };
        if let Err(e) = store.insert(&new) {
            drop(store);
            remove();
            return Err(format!("The library couldn't be updated: {e}"));
        }
        Ok(())
    }
}

fn secret_values(lines: &[TextLine], hits: &[Hit]) -> Vec<String> {
    let mut v: Vec<String> = hits.iter().map(|h| lines[h.line].text[h.range.clone()].trim().to_owned()).filter(|v| v.chars().count() >= 6).collect();
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
                hits.push(Hit { line, range, kind: SecretKind::Other });
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
        let mut f = q.frames.pop_front().expect("queue is non-empty");
        drop(q);
        s.busy.store(true, SeqCst);
        let mut result = Err(String::new());
        // AIDEV-NOTE: rgba is taken by encode; only retry failures that happen before it (model errors).
        for attempt in 0..3 {
            let require = lock(&s.config).require_ai;
            result = s.process(&mut f, require);
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
        let did = describe_one(&s);
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
    let l = s.lm(&c).map_err(|e| e.to_string())?;
    s.describing.store(true, SeqCst);
    s.ensure(&l, &c.model, CONTEXT).map_err(|e| e.to_string())?;
    // Reachable model from here on: failures count against this shot so one bad image can't block the rest.
    let fail = |why: String| {
        let _ = lock(&s.store).bump_attempts(shot.id);
        why
    };
    let img = s.host.model_image(shot.path.clone()).map_err(|e| fail(e.to_string()))?;
    let (d, tags) = match l.describe(&c.model, &img, &shot.moment.app_name, &shot.moment.window_title) {
        Ok(x) => x,
        Err(e @ lm::LmError::Offline) => return Err(e.to_string()),
        Err(e) => return Err(fail(e.to_string())),
    };
    *lock(&s.last_lm_use) = Instant::now();
    let d = detect::mask_text(&d);
    let emb = if c.embed_model.is_empty() {
        None
    } else {
        let head: String = format!("search_document: {} {} {}\n{}", shot.moment.app_name, shot.moment.window_title, d, body).chars().take(3500).collect();
        s.ensure(&l, &c.embed_model, 2048).and_then(|_| l.embed(&c.embed_model, &head, 30)).ok()
    };
    lock(&s.store).finish_index(shot.id, &d, &tags, emb.as_deref()).map_err(|e| e.to_string())
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
        Config { lm_url: "http://127.0.0.1:9".into(), model: "m".into(), embed_model: String::new(), require_ai, describe: DescribeWhen::Never, max_age_days: 0, max_bytes: 0, unload_after_secs: 0 }
    }
    fn frame(at: i64) -> Frame {
        let l = |t: &str, y: f64| TextLine { text: t.into(), rect: Rect { x: 0.1, y, w: 0.5, h: 0.1 }, tokens: vec![], code: false };
        Frame {
            rgba: vec![200; 100 * 100 * 4], width: 100, height: 100, stride: 400, display_key: "d".into(),
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
        let t = std::thread::spawn(move || bridge::forward(&mut s.1, &sock, 77));
        std::io::Write::write_all(&mut s.0, &input).unwrap();
        wait(|| e.page_hint(77).sensitive);
        assert!(!e.page_hint(78).sensitive);
        drop(s.0);
        t.join().unwrap().unwrap();
        e.shutdown();
    }
}
