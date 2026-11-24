//! SQLite index: metadata, FTS5 text search, embeddings, retention.

use crate::detect::{Rect, SecretKind};
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter, types::Value};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub type Res<T> = rusqlite::Result<T>;

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct AppRef {
    pub name: String,
    pub bundle_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct Region {
    pub kind: String,
    pub rect: Rect,
    /// What hid it, e.g. "card" or "model,label". Empty on pictures from before this was kept.
    #[serde(default)]
    pub why: String,
}

/// One line of the developer log. Text is redacted before it gets here.
#[derive(Debug, Clone, Default, uniffi::Record)]
pub struct DevEntry {
    pub id: i64,
    /// Milliseconds since 1970.
    pub at: i64,
    pub shot_id: Option<i64>,
    /// "check", "describe", "embed", "load", "unload", "capture".
    pub kind: String,
    pub model: String,
    pub ms: u64,
    pub ok: bool,
    pub note: String,
    pub request: String,
    pub response: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct IndexedLine {
    pub text: String,
    pub rect: Rect,
}

/// Everything recorded about the moment of capture.
#[derive(Debug, Clone, Default, Serialize, Deserialize, uniffi::Record)]
pub struct Moment {
    pub taken_at: i64,
    pub trigger: String,
    pub display: String,
    pub app_name: String,
    pub bundle_id: String,
    pub window_title: String,
    pub url: String,
    pub user_name: String,
    pub full_name: String,
    pub host: String,
    pub sessions: Vec<String>,
    pub running_apps: Vec<AppRef>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Shot {
    pub id: i64,
    pub moment: Moment,
    pub path: String,
    pub thumb_path: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub regions: Vec<Region>,
    pub description: String,
    pub tags: Vec<String>,
    pub kept: bool,
    pub indexed: bool,
    /// Whether the language model checked this shot, or only the built-in rules did.
    pub ai_checked: bool,
}

#[derive(Debug, Clone, Default, uniffi::Record)]
pub struct Filter {
    pub text: String,
    pub bundle_id: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub only_kept: bool,
    pub only_hidden: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct AppCount {
    pub name: String,
    pub bundle_id: String,
    pub count: u64,
}

#[derive(Debug, Clone, Default, uniffi::Record)]
pub struct Stats {
    pub count: u64,
    pub bytes: u64,
    pub kept: u64,
    pub hidden: u64,
    pub pending: u64,
    pub oldest: Option<i64>,
}

pub struct NewShot {
    pub moment: Moment,
    pub path: String,
    pub thumb_path: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub regions: Vec<Region>,
    pub lines: Vec<IndexedLine>,
    pub ai_checked: bool,
}

pub struct Store {
    db: Connection,
    pub root: PathBuf,
}

const COLS: &str = "s.id,s.moment,s.path,s.thumb,s.width,s.height,s.bytes,s.regions,s.description,s.tags,s.kept,s.indexed,s.ai_checked";

fn row_to_shot(root: &Path, r: &Row) -> Res<Shot> {
    let js = |i: usize| -> Res<String> { r.get(i) };
    let abs = |p: String| root.join(p).to_string_lossy().into_owned();
    Ok(Shot {
        id: r.get(0)?,
        moment: serde_json::from_str(&js(1)?).unwrap_or_default(),
        path: abs(r.get(2)?),
        thumb_path: abs(r.get(3)?),
        width: r.get(4)?,
        height: r.get(5)?,
        bytes: r.get::<_, i64>(6)? as u64,
        regions: serde_json::from_str(&js(7)?).unwrap_or_default(),
        description: r.get(8)?,
        tags: serde_json::from_str(&js(9)?).unwrap_or_default(),
        kept: r.get(10)?,
        indexed: r.get::<_, i64>(11)? == 1,
        ai_checked: r.get(12)?,
    })
}

/// Quote user text into a safe FTS5 query: every word must match, the last as a prefix.
/// "Quoted phrases" stay phrases; -word excludes.
pub fn fts_query(text: &str) -> Option<String> {
    let mut pos = Vec::new();
    let mut neg = Vec::new();
    for (i, part) in text.split('"').enumerate() {
        if i % 2 == 1 {
            let p = part.trim();
            if p.chars().any(char::is_alphanumeric) {
                pos.push((p.to_owned(), false));
            }
        } else {
            for w in part.split_whitespace() {
                let (n, w) = match w.strip_prefix('-') {
                    Some(x) => (true, x),
                    None => (false, w),
                };
                if w.chars().any(char::is_alphanumeric) {
                    if n { neg.push(w.to_owned()) } else { pos.push((w.to_owned(), true)) }
                }
            }
        }
    }
    if pos.is_empty() {
        return None;
    }
    let mut q = pos.iter().map(|(w, pre)| format!("\"{w}\"{}", if *pre { "*" } else { "" })).collect::<Vec<_>>().join(" AND ");
    for n in neg {
        q = format!("({q}) NOT \"{n}\"");
    }
    Some(q)
}

impl Store {
    pub fn open(root: &Path) -> Res<Self> {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::create_dir_all(root.join("Shots"));
        let _ = std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700));
        let _ = std::fs::write(root.join(".metadata_never_index"), "");
        let db = Connection::open(root.join("index.sqlite"))?;
        // AIDEV-NOTE: secure_delete + FTS secure-delete: a deleted shot's text must not linger in free pages.
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA secure_delete=ON; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS shots(
               id INTEGER PRIMARY KEY AUTOINCREMENT, taken_at INTEGER NOT NULL, bundle_id TEXT NOT NULL, app_name TEXT NOT NULL,
               moment TEXT NOT NULL, path TEXT NOT NULL, thumb TEXT NOT NULL, width INTEGER, height INTEGER, bytes INTEGER,
               regions TEXT NOT NULL, hidden INTEGER NOT NULL, lines TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
               tags TEXT NOT NULL DEFAULT '[]', kept INTEGER NOT NULL DEFAULT 0, indexed INTEGER NOT NULL DEFAULT 0,
               attempts INTEGER NOT NULL DEFAULT 0, ai_checked INTEGER NOT NULL, embedding BLOB);
             CREATE INDEX IF NOT EXISTS shots_time ON shots(taken_at);
             CREATE INDEX IF NOT EXISTS shots_app ON shots(bundle_id, taken_at);
             CREATE INDEX IF NOT EXISTS shots_todo ON shots(id) WHERE indexed=0;
             CREATE VIRTUAL TABLE IF NOT EXISTS fts USING fts5(title, description, body, app, tokenize='unicode61 remove_diacritics 2');
             CREATE TABLE IF NOT EXISTS dev(
               id INTEGER PRIMARY KEY AUTOINCREMENT, at INTEGER NOT NULL, shot_id INTEGER REFERENCES shots(id) ON DELETE CASCADE,
               kind TEXT NOT NULL, model TEXT NOT NULL, ms INTEGER NOT NULL, ok INTEGER NOT NULL, note TEXT NOT NULL,
               request TEXT NOT NULL, response TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS dev_shot ON dev(shot_id);
             INSERT INTO fts(fts, rank) VALUES('secure-delete', 1);
             PRAGMA user_version=1;",
        )?;
        let _ = std::fs::set_permissions(root.join("index.sqlite"), std::fs::Permissions::from_mode(0o600));
        Ok(Self { db, root: root.to_owned() })
    }

    pub fn insert(&mut self, n: &NewShot) -> Res<i64> {
        let j = |v: &dyn erased::Ser| v.json();
        let tx = self.db.transaction()?;
        tx.execute(
            "INSERT INTO shots(taken_at,bundle_id,app_name,moment,path,thumb,width,height,bytes,regions,hidden,lines,ai_checked) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
            params![n.moment.taken_at, n.moment.bundle_id, n.moment.app_name, j(&n.moment), n.path, n.thumb_path, n.width, n.height,
                    n.bytes as i64, j(&n.regions), n.regions.len() as i64, j(&n.lines), n.ai_checked],
        )?;
        let id = tx.last_insert_rowid();
        let body = n.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
        tx.execute("INSERT INTO fts(rowid,title,description,body,app) VALUES(?,?,?,?,?)",
            params![id, format!("{} {}", n.moment.window_title, n.moment.url), "", body, n.moment.app_name])?;
        tx.commit()?;
        Ok(id)
    }

    /// Next shot awaiting a description: (shot, text body).
    pub fn next_pending(&self) -> Res<Option<(Shot, String)>> {
        self.db
            .query_row(&format!("SELECT {COLS}, s.lines FROM shots s WHERE indexed=0 AND attempts<3 ORDER BY id DESC LIMIT 1"), [], |r| {
                let lines: Vec<IndexedLine> = serde_json::from_str(&r.get::<_, String>(13)?).unwrap_or_default();
                Ok((row_to_shot(&self.root, r)?, lines.into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n")))
            })
            .optional()
    }

    pub fn bump_attempts(&self, id: i64) -> Res<()> {
        self.db.execute("UPDATE shots SET attempts=attempts+1 WHERE id=?", [id]).map(|_| ())
    }

    pub fn finish_index(&mut self, id: i64, description: &str, tags: &[String], emb: Option<&[f32]>) -> Res<bool> {
        let blob = emb.map(|e| e.iter().map(|x| (x * 127.0).round().clamp(-127.0, 127.0) as i8 as u8).collect::<Vec<u8>>());
        let tx = self.db.transaction()?;
        let n = tx.execute("UPDATE shots SET description=?,tags=?,embedding=?,indexed=1 WHERE id=?",
            params![description, serde_json::to_string(tags).unwrap_or_default(), blob, id])?;
        if n == 1 {
            tx.execute("UPDATE fts SET description=? WHERE rowid=?", params![format!("{description} {}", tags.join(" ")), id])?;
        }
        tx.commit()?;
        Ok(n == 1)
    }

    fn clauses(f: &Filter) -> (String, Vec<Value>) {
        let mut sql = String::from("1");
        let mut a = Vec::new();
        if let Some(b) = &f.bundle_id {
            sql += " AND s.bundle_id=?";
            a.push(Value::Text(b.clone()));
        }
        if let Some(t) = f.from {
            sql += " AND s.taken_at>=?";
            a.push(Value::Integer(t));
        }
        if let Some(t) = f.to {
            sql += " AND s.taken_at<?";
            a.push(Value::Integer(t));
        }
        if f.only_kept {
            sql += " AND s.kept=1";
        }
        if f.only_hidden {
            sql += " AND s.hidden>0";
        }
        (sql, a)
    }

    pub fn list(&self, f: &Filter, limit: u32, offset: u32) -> Res<Vec<Shot>> {
        let (w, mut a) = Self::clauses(f);
        a.push(Value::Integer(limit as i64));
        a.push(Value::Integer(offset as i64));
        let mut st = self.db.prepare(&format!("SELECT {COLS} FROM shots s WHERE {w} ORDER BY taken_at DESC, id DESC LIMIT ? OFFSET ?"))?;
        st.query_map(params_from_iter(a), |r| row_to_shot(&self.root, r))?.collect()
    }

    /// Keyword hits, best first, optionally fused with nearest neighbours of `emb` (reciprocal rank fusion).
    pub fn search(&self, f: &Filter, emb: Option<&[f32]>, limit: u32) -> Res<Vec<Shot>> {
        let (w, a) = Self::clauses(f);
        let mut score: std::collections::HashMap<i64, f64> = Default::default();
        if let Some(q) = fts_query(&f.text) {
            let mut args = vec![Value::Text(q)];
            args.extend(a.iter().cloned());
            args.push(Value::Integer(limit as i64));
            let mut st = self.db.prepare(&format!("SELECT s.id FROM fts JOIN shots s ON s.id=fts.rowid WHERE fts MATCH ? AND {w} ORDER BY bm25(fts,6.0,4.0,1.0,3.0), s.taken_at DESC LIMIT ?"))?;
            let ids: Vec<i64> = st.query_map(params_from_iter(args), |r| r.get(0))?.collect::<Res<_>>()?;
            for (i, id) in ids.iter().enumerate() {
                *score.entry(*id).or_default() += 1.4 / (30.0 + i as f64);
            }
        }
        if let Some(e) = emb {
            let mut st = self.db.prepare(&format!("SELECT s.id, s.embedding FROM shots s WHERE s.embedding IS NOT NULL AND {w}"))?;
            let mut near: Vec<(f32, i64)> = st
                .query_map(params_from_iter(a.iter().cloned()), |r| {
                    let b = r.get_ref(1)?.as_blob().unwrap_or(&[]);
                    let d: f32 = if b.len() == e.len() { b.iter().zip(e).map(|(x, y)| *x as i8 as f32 * y).sum::<f32>() / 127.0 } else { -1.0 };
                    Ok((d, r.get::<_, i64>(0)?))
                })?
                .collect::<Res<_>>()?;
            near.sort_by(|x, y| y.0.total_cmp(&x.0));
            let best = near.first().map_or(0.0, |n| n.0);
            // Cosine floor: nomic scores unrelated text near 0.5, so only a tight band under the best is meaningful.
            for (i, (d, id)) in near.iter().take(40).enumerate() {
                if *d >= 0.58 && *d >= best - 0.09 {
                    *score.entry(*id).or_default() += 1.0 / (30.0 + i as f64);
                }
            }
        }
        let mut ids: Vec<(i64, f64)> = score.into_iter().collect();
        ids.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.0.cmp(&a.0)));
        ids.truncate(limit as usize);
        let mut st = self.db.prepare(&format!("SELECT {COLS} FROM shots s WHERE s.id=?"))?;
        ids.iter().map(|(id, _)| st.query_row([id], |r| row_to_shot(&self.root, r))).collect()
    }

    pub fn lines(&self, id: i64) -> Res<Vec<IndexedLine>> {
        let j: Option<String> = self.db.query_row("SELECT lines FROM shots WHERE id=?", [id], |r| r.get(0)).optional()?;
        Ok(j.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default())
    }

    pub fn apps(&self) -> Res<Vec<AppCount>> {
        let mut st = self.db.prepare("SELECT MAX(app_name),bundle_id,COUNT(*) FROM shots WHERE bundle_id<>'' GROUP BY bundle_id ORDER BY 3 DESC")?;
        st.query_map([], |r| Ok(AppCount { name: r.get(0)?, bundle_id: r.get(1)?, count: r.get::<_, i64>(2)? as u64 }))?.collect()
    }

    pub fn stats(&self) -> Res<Stats> {
        self.db.query_row(
            "SELECT COUNT(*),COALESCE(SUM(bytes),0),COALESCE(SUM(kept),0),COALESCE(SUM(hidden>0),0),COALESCE(SUM(indexed=0 AND attempts<3),0),MIN(taken_at) FROM shots",
            [],
            |r| {
                let g = |i: usize| -> Res<u64> { Ok(r.get::<_, i64>(i)? as u64) };
                Ok(Stats { count: g(0)?, bytes: g(1)?, kept: g(2)?, hidden: g(3)?, pending: g(4)?, oldest: r.get(5)? })
            },
        )
    }

    pub fn set_kept(&self, ids: &[i64], kept: bool) -> Res<()> {
        for id in ids {
            self.db.execute("UPDATE shots SET kept=? WHERE id=?", params![kept, id])?;
        }
        Ok(())
    }

    pub fn ids_matching(&self, f: &Filter) -> Res<Vec<i64>> {
        let (w, a) = Self::clauses(f);
        let mut st = self.db.prepare(&format!("SELECT s.id FROM shots s WHERE {w}"))?;
        st.query_map(params_from_iter(a), |r| r.get(0))?.collect()
    }

    /// Remove rows and files. Returns how many shots were removed and how many files could not be.
    pub fn delete(&mut self, ids: &[i64]) -> Res<(u64, u64)> {
        let mut files = Vec::new();
        let tx = self.db.transaction()?;
        let mut n = 0;
        for id in ids {
            let f: Option<(String, String)> = tx.query_row("SELECT path,thumb FROM shots WHERE id=?", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
            if let Some((p, t)) = f {
                tx.execute("DELETE FROM fts WHERE rowid=?", [id])?;
                tx.execute("DELETE FROM shots WHERE id=?", [id])?;
                files.push(p);
                files.push(t);
                n += 1;
            }
        }
        tx.commit()?;
        let mut failed = 0;
        for f in &files {
            if let Err(e) = std::fs::remove_file(self.root.join(f)) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    failed += 1;
                    log::error!("could not delete a shot file: {e} [AIMG-DELFILE]");
                }
            }
        }
        if n > 0 {
            let _ = self.db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
        }
        Ok((n, failed))
    }

    pub fn dev_add(&self, e: &DevEntry) -> Res<()> {
        self.db
            .execute("INSERT INTO dev(at,shot_id,kind,model,ms,ok,note,request,response) VALUES(?,?,?,?,?,?,?,?,?)",
                params![e.at, e.shot_id, e.kind, e.model, e.ms as i64, e.ok, e.note, e.request, e.response])
            .map(|_| ())
    }

    /// Newest first; `before` is the last id of the previous page.
    pub fn dev_list(&self, limit: u32, before: Option<i64>) -> Res<Vec<DevEntry>> {
        let mut st = self.db.prepare("SELECT id,at,shot_id,kind,model,ms,ok,note,request,response FROM dev WHERE id<? ORDER BY id DESC LIMIT ?")?;
        st.query_map(params![before.unwrap_or(i64::MAX), limit], |r| {
            Ok(DevEntry { id: r.get(0)?, at: r.get(1)?, shot_id: r.get(2)?, kind: r.get(3)?, model: r.get(4)?, ms: r.get::<_, i64>(5)? as u64,
                          ok: r.get(6)?, note: r.get(7)?, request: r.get(8)?, response: r.get(9)? })
        })?
        .collect()
    }

    fn dev_delete(&self, sql: &str, p: impl rusqlite::Params) -> Res<()> {
        if self.db.execute(sql, p)? > 0 {
            let _ = self.db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
        }
        Ok(())
    }
    pub fn dev_forget(&self, from: i64, to: i64) -> Res<()> {
        self.dev_delete("DELETE FROM dev WHERE at>=? AND at<?", [from, to])
    }
    pub fn dev_clear(&self) -> Res<()> {
        self.dev_delete("DELETE FROM dev", [])
    }
    pub fn dev_trim(&self, now_ms: i64, days: i64, most: i64) -> Res<()> {
        self.dev_delete("DELETE FROM dev WHERE at<? OR id NOT IN (SELECT id FROM dev ORDER BY id DESC LIMIT ?)", [now_ms - days * 86_400_000, most])
    }

    /// Ids that retention says must go: older than `max_age_days`, then oldest-first until under `max_bytes`.
    pub fn expired(&self, now_ms: i64, max_age_days: u32, max_bytes: u64) -> Res<Vec<i64>> {
        let mut out = Vec::new();
        if max_age_days > 0 {
            let cut = now_ms - max_age_days as i64 * 86_400_000;
            let mut st = self.db.prepare("SELECT id FROM shots WHERE kept=0 AND taken_at<?")?;
            out = st.query_map([cut], |r| r.get(0))?.collect::<Res<_>>()?;
        }
        if max_bytes > 0 {
            let gone: std::collections::HashSet<i64> = out.iter().copied().collect();
            let mut st = self.db.prepare("SELECT id,bytes,kept FROM shots ORDER BY taken_at")?;
            let rows: Vec<(i64, i64, bool)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Res<_>>()?;
            // Kept pictures are never removed, so they aren't counted: otherwise enough of them would cost every new one.
            let mut total: i64 = rows.iter().filter(|r| !gone.contains(&r.0) && !r.2).map(|r| r.1).sum();
            for (id, b, kept) in rows {
                if total <= max_bytes as i64 {
                    break;
                }
                if !kept && !gone.contains(&id) {
                    total -= b;
                    out.push(id);
                }
            }
        }
        Ok(out)
    }

    /// Delete image files with no row (crash between encode and insert, or a failed unlink).
    pub fn sweep_orphans(&self) -> Res<u64> {
        let mut known = std::collections::HashSet::new();
        let mut st = self.db.prepare("SELECT path,thumb FROM shots")?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            known.insert(self.root.join(r.get::<_, String>(0)?));
            known.insert(self.root.join(r.get::<_, String>(1)?));
        }
        let mut n = 0;
        let mut stack = vec![self.root.join("Shots")];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                match e.file_type() {
                    Ok(t) if t.is_dir() => stack.push(p),
                    Ok(t) if t.is_file() && !known.contains(&p) && std::fs::remove_file(&p).is_ok() => n += 1,
                    _ => {}
                }
            }
        }
        Ok(n)
    }
}

mod erased {
    pub trait Ser {
        fn json(&self) -> String;
    }
    impl<T: serde::Serialize> Ser for T {
        fn json(&self) -> String {
            serde_json::to_string(self).unwrap_or_default()
        }
    }
}

pub fn region(kind: SecretKind, rect: Rect, why: &str) -> Region {
    Region { kind: kind.as_str().into(), rect, why: why.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn shot(root: &Path, at: i64, body: &str, bytes: u64) -> NewShot {
        let p = format!("Shots/{at}.heic");
        let t = format!("Shots/{at}.t.heic");
        std::fs::write(root.join(&p), body).unwrap();
        std::fs::write(root.join(&t), body).unwrap();
        NewShot {
            moment: Moment { taken_at: at, app_name: "Safari".into(), bundle_id: "com.apple.Safari".into(), window_title: "Plan".into(), ..Default::default() },
            path: p, thumb_path: t, width: 10, height: 10, bytes, regions: vec![],
            lines: vec![IndexedLine { text: body.into(), rect: Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 } }], ai_checked: true,
        }
    }

    #[test]
    fn search_delete_leaves_nothing() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(d.path()).unwrap();
        let id = s.insert(&shot(d.path(), 1, "zebraquartz invoice", 5)).unwrap();
        s.insert(&shot(d.path(), 2, "other words", 5)).unwrap();
        let q = |s: &Store, t: &str| s.search(&Filter { text: t.into(), ..Default::default() }, None, 10).unwrap().len();
        assert_eq!((q(&s, "zebraq"), q(&s, "\"invoice zebraquartz\""), q(&s, "plan -zebraquartz")), (1, 0, 1));
        assert!(s.finish_index(id, "A purplewalrus bill", &["bills".into()], Some(&[1.0, 0.0])).unwrap());
        assert_eq!((q(&s, "purplewalrus"), q(&s, "zebraquartz"), q(&s, "safari")), (1, 1, 2));
        assert_eq!(s.delete(&[id]).unwrap(), (1, 0));
        assert_eq!(q(&s, "zebraquartz"), 0);
        assert!(!s.finish_index(id, "ghostcaption", &[], None).unwrap());
        assert!(!d.path().join("Shots/1.heic").exists());
        drop(s);
        for f in ["index.sqlite", "index.sqlite-wal"] {
            let b = std::fs::read(d.path().join(f)).unwrap_or_default();
            for w in ["zebraquartz", "purplewalrus"] {
                assert!(!b.windows(w.len()).any(|x| x == w.as_bytes()), "{w} lingers in {f}");
            }
        }
    }

    #[test]
    fn hostile_queries_are_safe() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(d.path()).unwrap();
        for t in ["\"", "a\" OR b", "NEAR(", "*", "-", "col:x", "' ; drop table shots", "((", "\0"] {
            s.search(&Filter { text: t.into(), ..Default::default() }, None, 5).unwrap_or_else(|e| panic!("{t:?}: {e}"));
        }
    }

    #[test]
    fn retention_respects_age_size_and_kept() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(d.path()).unwrap();
        let day = 86_400_000;
        let ids: Vec<i64> = (1..=4).map(|i| s.insert(&shot(d.path(), i * day, "x", 100)).unwrap()).collect();
        s.set_kept(&ids[..1], true).unwrap();
        let now = 5 * day;
        assert_eq!(s.expired(now, 2, 0).unwrap(), vec![ids[1]]);
        // 300 bytes can go; the kept 100 are outside the budget.
        assert_eq!(s.expired(now, 0, 250).unwrap(), vec![ids[1]]);
        assert_eq!(s.expired(now, 0, 150).unwrap(), vec![ids[1], ids[2]]);
        assert_eq!(s.expired(now, 0, 0).unwrap(), Vec::<i64>::new());
    }

    #[test]
    fn semantic_search_and_orphans() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(d.path()).unwrap();
        let a = s.insert(&shot(d.path(), 1, "x", 1)).unwrap();
        let b = s.insert(&shot(d.path(), 2, "y", 1)).unwrap();
        s.finish_index(a, "", &[], Some(&[1.0, 0.0])).unwrap();
        s.finish_index(b, "", &[], Some(&[0.0, 1.0])).unwrap();
        let r = s.search(&Filter { text: "nomatch".into(), ..Default::default() }, Some(&[1.0, 0.0]), 10).unwrap();
        assert_eq!(r.iter().map(|x| x.id).collect::<Vec<_>>(), [a]);
        std::fs::write(d.path().join("Shots/stray.heic"), "x").unwrap();
        assert_eq!(s.sweep_orphans().unwrap(), 1);
        assert!(d.path().join("Shots/1.heic").exists());
    }

    #[test]
    fn the_developer_log_goes_when_its_picture_does() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(d.path()).unwrap();
        let mk = |s: &mut Store, at: i64| s.insert(&NewShot { moment: Moment { taken_at: at, ..Default::default() }, path: format!("Shots/{at}"), thumb_path: format!("Shots/{at}t"), width: 1, height: 1, bytes: 1, regions: vec![], lines: vec![], ai_checked: true }).unwrap();
        let (a, b) = (mk(&mut s, 100), mk(&mut s, 200));
        let e = |shot, at| DevEntry { shot_id: shot, at, kind: "check".into(), request: format!("zebra{at}"), ..Default::default() };
        for (shot, at) in [(Some(a), 150), (Some(b), 250), (None, 300)] {
            s.dev_add(&e(shot, at)).unwrap();
        }
        // Newest first, in pages.
        let all = s.dev_list(10, None).unwrap();
        assert_eq!(all.iter().map(|x| x.at).collect::<Vec<_>>(), vec![300, 250, 150]);
        assert_eq!(s.dev_list(1, Some(all[0].id)).unwrap()[0].at, 250);
        s.delete(&[a]).unwrap();
        assert_eq!(s.dev_list(10, None).unwrap().len(), 2);
        // A picture that's already gone takes no more notes.
        assert!(s.dev_add(&e(Some(a), 400)).is_err());
        s.dev_forget(240, 260).unwrap();
        assert_eq!(s.dev_list(10, None).unwrap().iter().map(|x| x.at).collect::<Vec<_>>(), vec![300]);
        s.dev_clear().unwrap();
        assert!(s.dev_list(10, None).unwrap().is_empty());
        drop(s);
        let raw: Vec<u8> = ["index.sqlite", "index.sqlite-wal"].iter().flat_map(|f| std::fs::read(d.path().join(f)).unwrap_or_default()).collect();
        assert!(!String::from_utf8_lossy(&raw).contains("zebra"));
    }

    #[test]
    fn the_developer_log_is_bounded() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(d.path()).unwrap();
        let now = 100 * 86_400_000;
        for i in 0..30 {
            s.dev_add(&DevEntry { at: now - i * 86_400_000, ..Default::default() }).unwrap();
        }
        s.dev_trim(now, 7, 1000).unwrap();
        assert_eq!(s.dev_list(100, None).unwrap().len(), 8);
        s.dev_trim(now, 7, 3).unwrap();
        assert_eq!(s.dev_list(100, None).unwrap().len(), 3);
    }

    #[test]
    fn kept_pictures_over_the_limit_do_not_cost_every_new_one() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(d.path()).unwrap();
        let mut mk = |at: i64, bytes: u64| s.insert(&NewShot { moment: Moment { taken_at: at, ..Default::default() }, path: format!("Shots/{at}"), thumb_path: format!("Shots/{at}t"), width: 1, height: 1, bytes, regions: vec![], lines: vec![], ai_checked: true }).unwrap();
        let (a, b, c, e) = (mk(1, 100), mk(2, 100), mk(3, 10), mk(4, 60));
        s.set_kept(&[a, b], true).unwrap();
        // Kept ones are outside the budget: only what can be removed is counted against it.
        assert_eq!(s.expired(10, 0, 150).unwrap(), Vec::<i64>::new());
        assert_eq!(s.expired(10, 0, 65).unwrap(), vec![c]);
        let _ = e;
    }
}
