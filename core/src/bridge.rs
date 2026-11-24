//! Carries browser-extension messages from a browser to the app.
//!
//! Browsers start `afterimage-bridge` themselves and speak native messaging to it: a 4-byte length, then JSON.
//! The bridge passes each message on as one line over a Unix socket only this user can reach. The only thing
//! that comes back is a question: "measure yourselves now".


use serde_json::{Value, json};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Chrome caps messages to a host at 64 MB; nothing honest here comes near 64 KB.
const MAX_FRAME: u32 = 1 << 20;
const MAX_LINE: u64 = 1 << 20;
const MAX_CONNECTIONS: usize = 16;
/// Plenty of questions for one connection's lifetime.
const MAX_BACK: u64 = 64 << 20;

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// The browser's process id.
    pub pid: i32,
    /// Which connection; one browser can have several (a bridge per profile).
    pub source: u64,
    pub message: String,
}

/// One native-messaging message as a single line; "null" if it wasn't JSON. None at a clean end.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<String>> {
    let mut len = [0u8; 4];
    match r.read(&mut len[..1])? {
        0 => return Ok(None),
        _ => r.read_exact(&mut len[1..])?,
    }
    let len = u32::from_ne_bytes(len);
    if len > MAX_FRAME {
        let skipped = io::copy(&mut r.take(len as u64), &mut io::sink())?;
        return if skipped == len as u64 { Ok(Some("null".into())) } else { Err(io::ErrorKind::UnexpectedEof.into()) };
    }
    let mut body = vec![0; len as usize];
    r.read_exact(&mut body)?;
    Ok(Some(serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null).to_string()))
}

/// Pass messages on until the browser hangs up, and the app's questions back. While the app isn't running they are
/// dropped; it has nothing to hide from.
pub fn forward(from: &mut impl Read, to: impl Write + Send + 'static, socket: &Path, browser: u32) -> io::Result<()> {
    let to = Arc::new(Mutex::new(to));
    let connect = || -> Option<UnixStream> {
        let mut s = UnixStream::connect(socket).ok()?;
        writeln!(s, "{}", json!({"pid": browser})).ok()?;
        let (back, to) = (s.try_clone().ok()?, to.clone());
        std::thread::spawn(move || {
            for line in BufReader::new(back.take(MAX_BACK)).lines().map_while(Result::ok) {
                // Only a question, rebuilt from its number: nothing else reaches the browser through here.
                let Some(n) = serde_json::from_str::<Value>(&line).ok().and_then(|v| v.get("n")?.as_u64()) else { continue };
                let m = json!({"type": "ask", "n": n}).to_string();
                let mut w = to.lock().unwrap_or_else(|e| e.into_inner());
                if w.write_all(&(m.len() as u32).to_ne_bytes()).and_then(|_| w.write_all(m.as_bytes())).and_then(|_| w.flush()).is_err() {
                    return;
                }
            }
        });
        Some(s)
    };
    let mut out: Option<UnixStream> = None;
    while let Some(line) = read_frame(from)? {
        // One retry: the app may have restarted since the last message.
        for _ in 0..2 {
            out = out.or_else(connect);
            match out.as_mut().map(|s| writeln!(s, "{line}")) {
                Some(Ok(())) => break,
                _ => out = None,
            }
        }
    }
    if let Some(s) = out {
        let _ = s.shutdown(std::net::Shutdown::Both);
    }
    Ok(())
}

/// The way back to each open connection.
#[derive(Default)]
pub struct Outlets(Mutex<HashMap<u64, (i32, UnixStream)>>);

impl Outlets {
    /// Ask every bridge of this browser; returns the connections that were asked.
    pub fn ask(&self, pid: i32, n: u64) -> Vec<u64> {
        let mut open = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let line = format!("{}\n", json!({"type": "ask", "n": n}));
        let mut asked = Vec::new();
        open.retain(|id, (p, s)| {
            if *p != pid {
                return true;
            }
            let ok = s.write_all(line.as_bytes()).is_ok();
            if ok {
                asked.push(*id);
            }
            ok
        });
        asked
    }
}

/// Accept bridges on a socket only this user can open.
pub fn listen(socket: &Path, heard: impl Fn(Event) + Send + Sync + 'static) -> io::Result<Arc<Outlets>> {
    match std::fs::symlink_metadata(socket) {
        Ok(m) if m.file_type().is_socket() => std::fs::remove_file(socket)?,
        Ok(_) => return Err(io::ErrorKind::AlreadyExists.into()),
        Err(_) => {}
    }
    let l = UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    let heard = Arc::new(heard);
    let outlets = Arc::new(Outlets::default());
    let back = outlets.clone();
    std::thread::Builder::new().name("afterimage-bridge".into()).spawn(move || {
        for (n, s) in l.incoming().flatten().enumerate() {
            // Each connection holds a clone; the count is how many are open.
            if Arc::strong_count(&heard) > MAX_CONNECTIONS {
                continue;
            }
            let (heard, back) = (heard.clone(), back.clone());
            let _ = std::thread::Builder::new().name("afterimage-page".into()).spawn(move || serve(s, n as u64, &*heard, &back));
        }
    })?;
    Ok(outlets)
}

fn serve(s: UnixStream, id: u64, heard: &dyn Fn(Event), outlets: &Outlets) {
    let Ok(back) = s.try_clone() else { return };
    // A bridge that stops reading must not hold up a capture.
    let _ = back.set_write_timeout(Some(Duration::from_millis(200)));
    let mut r = BufReader::new(s);
    // None: the end. Some(None): too long to be honest.
    let mut line = move || -> Option<Option<String>> {
        let mut buf = Vec::new();
        let n = (&mut r).take(MAX_LINE + 1).read_until(b'\n', &mut buf).ok()?;
        match n as u64 {
            0 => None,
            n if n > MAX_LINE => Some(None),
            _ => Some(Some(String::from_utf8_lossy(&buf).trim_end().to_owned())),
        }
    };
    let Some(Some(hello)) = line() else { return };
    let pid = serde_json::from_str::<Value>(&hello).ok().and_then(|v| v.get("pid")?.as_i64()).and_then(|p| i32::try_from(p).ok()).filter(|p| *p > 0);
    let Some(pid) = pid else { return };
    outlets.0.lock().unwrap_or_else(|e| e.into_inner()).insert(id, (pid, back));
    // AIDEV-NOTE: hanging up clears nothing. What was last said stands until it goes stale.
    while let Some(l) = line() {
        let end = l.is_none();
        heard(Event { pid, source: id, message: l.unwrap_or_else(|| "null".into()) });
        if end {
            break;
        }
    }
    if let Some((_, s)) = outlets.0.lock().unwrap_or_else(|e| e.into_inner()).remove(&id) {
        let _ = s.shutdown(std::net::Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::mpsc;

    fn frame(s: &str) -> Vec<u8> {
        let mut v = (s.len() as u32).to_ne_bytes().to_vec();
        v.extend_from_slice(s.as_bytes());
        v
    }

    #[test]
    fn frames_are_read_whole_and_flattened() {
        let mut c = Cursor::new([frame("{\"a\":\n 1}"), frame("{}")].concat());
        assert_eq!(read_frame(&mut c).unwrap().as_deref(), Some("{\"a\":1}"));
        assert_eq!(read_frame(&mut c).unwrap().as_deref(), Some("{}"));
        assert_eq!(read_frame(&mut c).unwrap(), None);
    }

    #[test]
    fn bad_frames_are_passed_on_as_nonsense_or_end_the_stream() {
        // Not JSON: the app should hear that something was wrong.
        assert_eq!(read_frame(&mut Cursor::new(frame("not json\nat all"))).unwrap().as_deref(), Some("null"));
        assert_eq!(read_frame(&mut Cursor::new([4u32.to_ne_bytes().as_slice(), &[0xff, 0xfe, 0xff, 0xff]].concat())).unwrap().as_deref(), Some("null"));
        // Too long to be honest: heard as nonsense, and the next one still arrives.
        let long = format!("\"{}\"", "a".repeat(2 << 20));
        let mut c = Cursor::new([frame(&long), frame("{}")].concat());
        assert_eq!(read_frame(&mut c).unwrap().as_deref(), Some("null"));
        assert_eq!(read_frame(&mut c).unwrap().as_deref(), Some("{}"));
        // Cut short: there's no telling where the next one starts.
        assert!(read_frame(&mut Cursor::new(u32::MAX.to_ne_bytes().to_vec())).is_err());
        assert!(read_frame(&mut Cursor::new(frame("{}")[..5].to_vec())).is_err());
        assert!(read_frame(&mut Cursor::new(vec![1, 0])).is_err());
    }

    #[test]
    fn messages_reach_the_listener_with_the_browsers_pid() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("b.sock");
        let (tx, rx) = mpsc::channel();
        listen(&path, move |e| drop(tx.send(e))).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let input = [frame(r#"{"v":1}"#), frame("oops")].concat();
        forward(&mut Cursor::new(input), std::io::sink(), &path, 4242).unwrap();
        let mut got = vec![];
        while let Ok(e) = rx.recv_timeout(Duration::from_secs(2)) {
            got.push(e);
            if got.len() == 2 { break; }
        }
        assert_eq!(got, vec![Event { pid: 4242, source: 0, message: r#"{"v":1}"#.into() }, Event { pid: 4242, source: 0, message: "null".into() }]);
    }

    #[test]
    fn a_stranger_without_a_header_is_dropped() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("b.sock");
        let (tx, rx) = mpsc::channel();
        listen(&path, move |e| drop(tx.send(e))).unwrap();
        for hello in ["{}\n", "nope\n", "{\"pid\":-4}\n", "{\"pid\":0}\n"] {
            let mut s = std::os::unix::net::UnixStream::connect(&path).unwrap();
            s.write_all(hello.as_bytes()).unwrap();
            s.write_all(b"{\"v\":1}\n").ok();
            let mut b = [0];
            assert!(matches!(s.read(&mut b), Ok(0) | Err(_)));
        }
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    }

    #[test]
    fn an_endless_line_ends_the_connection() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("b.sock");
        let (tx, rx) = mpsc::channel();
        listen(&path, move |e| drop(tx.send(e))).unwrap();
        let mut s = std::os::unix::net::UnixStream::connect(&path).unwrap();
        s.write_all(b"{\"pid\":9}\n").unwrap();
        let junk = vec![b'a'; 1 << 20];
        for _ in 0..4 {
            if s.write_all(&junk).is_err() { break; }
        }
        // Heard as nonsense.
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), Event { pid: 9, source: 0, message: "null".into() });
    }

    #[test]
    fn the_bridge_waits_quietly_when_the_app_is_not_running() {
        let d = tempfile::tempdir().unwrap();
        let input = [frame("{}"), frame("{}")].concat();
        forward(&mut Cursor::new(input), std::io::sink(), &d.path().join("none.sock"), 5).unwrap();
    }

    #[test]
    fn a_stale_socket_is_replaced_but_nothing_else_is() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("b.sock");
        listen(&path, |_| ()).unwrap();
        listen(&path, |_| ()).unwrap();
        let file = d.path().join("file");
        std::fs::write(&file, "mine").unwrap();
        assert!(listen(&file, |_| ()).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "mine");
    }

    #[derive(Clone, Default)]
    struct Sink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl Write for Sink {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn questions_travel_back_to_the_browser() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("b.sock");
        let (tx, rx) = mpsc::channel();
        let out = listen(&path, move |e| drop(tx.send(e))).unwrap();
        assert!(out.ask(4242, 1).is_empty(), "nobody is listening yet");
        let (mut browser, mut stdin) = UnixStream::pair().unwrap();
        let (sink, p2) = (Sink::default(), path.clone());
        let to = sink.clone();
        let t = std::thread::spawn(move || forward(&mut stdin, to, &p2, 4242));
        browser.write_all(&frame(r#"{"v":1}"#)).unwrap();
        let first = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(out.ask(4242, 9), vec![first.source]);
        assert!(out.ask(4243, 9).is_empty());
        for _ in 0..100 {
            if !sink.0.lock().unwrap().is_empty() { break; }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(*sink.0.lock().unwrap(), frame(r#"{"n":9,"type":"ask"}"#));
        drop(browser);
        t.join().unwrap().unwrap();
        for _ in 0..100 {
            if out.ask(4242, 10).is_empty() { return; }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("a closed connection is still being asked");
    }
}
