//! Remote mode (SPEC 29): the hub runs on a server as `sb serve`, and the window on
//! another computer talks to it through `ssh host sb attach`.
//!
//! The protocol is one JSON object per line, over the window socket:
//! - The window starts with `{"hello":{"proto":1,"version":"0.1.0"}}`. The hub answers
//!   `{"hello":{"ok":true,...}}`, or `{"hello":{"ok":false,"why":...}}` and closes.
//! - A command: `{"id":7,"cmd":"pane_open","args":{...}}`. Its answer:
//!   `{"id":7,"ok":<value>}` or `{"id":7,"err":"..."}`. Answers can come out of order.
//! - News from the hub: `{"event":"panes","payload":...}`, and pane bytes as
//!   `{"event":"pane_output","payload":{"pane":"p3","data":"<base64>"}}`.
//!
//! One window at a time. A new window takes over: the old one gets the
//! `taken_over` event, and its socket closes.
//!
//! Nothing that a hub thread calls does I/O under a lock. The sink only puts lines
//! and bytes in queues, and a writer thread for each window writes them. The pane
//! bytes go through the proved queue of `outq`: past its budget a pane drops its
//! bytes, and the window gets a fresh screen of it. Lines (answers and news) never
//! drop; when too many wait, the window is too slow and the hub closes it.

use crate::dispatch;
use crate::hub::{Hub, Sink};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::Duration;

/// The protocol version. A window and a hub with different versions refuse each other.
pub const PROTO: u64 = 1;
/// The bytes that one pane can have waiting before they drop.
pub const BUDGET: usize = 2 << 20;
/// The lines that can wait for a window before the hub closes it.
pub const MAX_LINES: usize = 20_000;

/// The commands that change what the window shows. They run under the owner lock,
/// so a command of a window that lost a takeover never changes the screen of the new one.
const SCREEN: &[&str] = &[
    "pane_expand",
    "pane_attach",
    "pane_detach",
    "pane_collapse",
    "pane_resize",
    "part_resize",
];

/// The first line of a window.
pub fn hello() -> Value {
    json!({ "hello": { "proto": PROTO, "version": env!("CARGO_PKG_VERSION") } })
}

/// Why a hello does not fit, or `None`.
pub fn hello_problem(v: &Value) -> Option<String> {
    let h = v.get("hello")?;
    let theirs = h["version"].as_str().unwrap_or("?");
    if h["proto"].as_u64() != Some(PROTO) || theirs != env!("CARGO_PKG_VERSION") {
        return Some(format!(
            "the window runs Switchboard {theirs}, and the server runs {}. Install the same version on both.",
            env!("CARGO_PKG_VERSION")
        ));
    }
    None
}

/// One window on the socket.
struct Conn {
    epoch: u64,
    stream: UnixStream,
    lines: Mutex<VecDeque<String>>,
    out: Mutex<outq::Queue>,
    wake: Condvar,
    /// Set with `lines`: the writer waits on it.
    pending: Mutex<bool>,
    /// The socket is shut.
    closed: AtomicBool,
    /// Another window took over: the writer sends what waits, then shuts the socket.
    ending: AtomicBool,
}

impl Conn {
    fn ring(&self) {
        *self.pending.lock().unwrap() = true;
        self.wake.notify_one();
    }

    fn line(&self, v: &Value) {
        let n = {
            let mut l = self.lines.lock().unwrap();
            l.push_back(v.to_string());
            l.len()
        };
        if n > MAX_LINES {
            tracing::warn!(target: "sb::remote", n, "the window does not read: close it");
            self.close();
        }
        self.ring();
    }

    fn close(&self) {
        if !self.closed.swap(true, Ordering::SeqCst) {
            let _ = self.stream.shutdown(std::net::Shutdown::Both);
        }
        self.ring();
    }
}

/// The sink of a hub with remote windows.
#[derive(Default)]
pub struct RemoteSink {
    /// The window that gets the news. A leaf lock: nothing else is locked under it.
    current: Mutex<Option<Arc<Conn>>>,
    /// The epoch of the window that owns the screen. Takeovers and screen commands
    /// hold it. Lock order: `owner`, then the hub's locks.
    owner: Mutex<u64>,
    next_epoch: AtomicU64,
    /// The `outq` key of each pane id.
    keys: Mutex<HashMap<String, u32>>,
}

impl RemoteSink {
    fn conn(&self) -> Option<Arc<Conn>> {
        self.current.lock().unwrap().clone()
    }

    fn key(&self, pane: &str) -> u32 {
        let mut k = self.keys.lock().unwrap();
        let n = k.len() as u32;
        *k.entry(pane.to_owned()).or_insert(n)
    }

    fn pane_of(&self, key: u32) -> Option<String> {
        self.keys.lock().unwrap().iter().find(|(_, k)| **k == key).map(|(p, _)| p.clone())
    }
}

impl Sink for RemoteSink {
    fn emit(&self, event: &str, payload: Value) {
        if let Some(c) = self.conn() {
            c.line(&json!({ "event": event, "payload": payload }));
        }
    }

    fn output(&self, pane: &str, data: &[u8]) {
        let key = self.key(pane);
        if let Some(c) = self.conn() {
            outq::push(&mut c.out.lock().unwrap(), key, data);
            c.ring();
        }
    }

    fn reset(&self, pane: &str) {
        let key = self.key(pane);
        if let Some(c) = self.conn() {
            outq::forget(&mut c.out.lock().unwrap(), key);
        }
    }
}

/// Serves remote windows on `listener` until the hub goes. One thread for each
/// window, one writer for each window, and one thread for each command.
pub fn serve(listener: UnixListener, hub: Weak<Hub>, sink: Arc<RemoteSink>) {
    for stream in listener.incoming().flatten() {
        let Some(h) = hub.upgrade() else { break };
        let sink = Arc::clone(&sink);
        std::thread::spawn(move || window(h, sink, stream));
    }
}

fn window(hub: Arc<Hub>, sink: Arc<RemoteSink>, stream: UnixStream) {
    let Ok(read) = stream.try_clone() else { return };
    let mut reader = BufReader::new(read);
    let mut first = String::new();
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    if reader.read_line(&mut first).is_err() {
        return;
    }
    let _ = stream.set_read_timeout(None);
    let hello: Value = serde_json::from_str(&first).unwrap_or(Value::Null);
    let problem = if hello.get("hello").is_none() {
        Some("the first line is not a hello".to_owned())
    } else {
        hello_problem(&hello)
    };
    let mut w = &stream;
    if let Some(why) = problem {
        tracing::warn!(target: "sb::remote", %why, "a window does not fit");
        let _ = writeln!(w, "{}", json!({ "hello": { "ok": false, "why": why } }));
        return;
    }
    let conn = Arc::new(Conn {
        epoch: sink.next_epoch.fetch_add(1, Ordering::SeqCst) + 1,
        stream,
        lines: Mutex::default(),
        out: Mutex::new(outq::new(BUDGET)),
        wake: Condvar::new(),
        pending: Mutex::new(false),
        closed: AtomicBool::new(false),
        ending: AtomicBool::new(false),
    });
    // The answer goes first, before any news can reach this window.
    conn.line(&json!({ "hello": { "ok": true, "proto": PROTO, "version": env!("CARGO_PKG_VERSION") } }));
    // The takeover: under the owner lock, so no screen command of the old window
    // runs after it. The old window's screen goes.
    let old = {
        let mut owner = sink.owner.lock().unwrap();
        *owner = conn.epoch;
        let old = sink.current.lock().unwrap().replace(Arc::clone(&conn));
        hub.collapse();
        old
    };
    if let Some(old) = old {
        tracing::info!(target: "sb::remote", old = old.epoch, new = conn.epoch, "a window takes over");
        old.line(&json!({ "event": "taken_over", "payload": null }));
        // The writer sends the event, then shuts the socket.
        old.ending.store(true, Ordering::SeqCst);
        old.ring();
    }
    tracing::info!(target: "sb::remote", epoch = conn.epoch, "a window attached");

    let (screens, screen_rx) = mpsc::channel::<u32>();
    {
        let (conn, sink) = (Arc::clone(&conn), Arc::clone(&sink));
        std::thread::spawn(move || writer(conn, sink, screens));
    }
    {
        // Fresh screens, one at a time, away from the writer: a capture asks tmux.
        let (conn, sink, hub) = (Arc::clone(&conn), Arc::clone(&sink), Arc::downgrade(&hub));
        std::thread::spawn(move || {
            for key in screen_rx {
                let Some(hub) = hub.upgrade() else { break };
                let pane = sink.pane_of(key);
                let screen = pane.as_deref().and_then(|p| hub.screen_for_resync(p).ok());
                tracing::info!(target: "sb::remote", pane = ?pane, ok = screen.is_some(), "fresh screen");
                // Always fill: a pane that left the screen is live again, not stuck.
                outq::fill(&mut conn.out.lock().unwrap(), key, screen.unwrap_or_default());
                conn.ring();
                if let Some(p) = &pane {
                    hub.resume_stream(p);
                }
            }
        });
    }

    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if conn.closed.load(Ordering::SeqCst) {
            break;
        }
        let Ok(req) = serde_json::from_str::<Value>(&line) else {
            tracing::warn!(target: "sb::remote", "a bad line from the window");
            continue;
        };
        let (hub, sink, conn) = (Arc::clone(&hub), Arc::clone(&sink), Arc::clone(&conn));
        std::thread::spawn(move || {
            let id = req["id"].clone();
            let cmd = req["cmd"].as_str().unwrap_or("").to_owned();
            let args = req.get("args").cloned().unwrap_or(json!({}));
            let out = run(&hub, &sink, conn.epoch, &cmd, &args);
            if let Err(err) = &out {
                tracing::warn!(target: "sb::remote", cmd, %err, "command failed");
            }
            conn.line(&match out {
                Ok(v) => json!({ "id": id, "ok": v }),
                Err(e) => json!({ "id": id, "err": e }),
            });
        });
    }
    tracing::info!(target: "sb::remote", epoch = conn.epoch, "a window left");
    conn.close();
    // The hub keeps running. Its screen goes with the window that owned it.
    let owner = sink.owner.lock().unwrap();
    if *owner == conn.epoch {
        let mut cur = sink.current.lock().unwrap();
        if cur.as_ref().map(|c| c.epoch) == Some(conn.epoch) {
            *cur = None;
        }
        drop(cur);
        hub.collapse();
    }
}

/// One command. A screen command runs only while its window owns the screen.
fn run(hub: &Hub, sink: &RemoteSink, epoch: u64, cmd: &str, args: &Value) -> Result<Value, String> {
    if SCREEN.contains(&cmd) {
        let owner = sink.owner.lock().unwrap();
        if *owner != epoch {
            return Err("another window took over".into());
        }
        let out = dispatch::dispatch(hub, cmd, args);
        drop(owner);
        return out;
    }
    dispatch::dispatch(hub, cmd, args)
}

/// Writes the lines and the pane bytes of one window. All I/O happens here, with
/// no lock held.
fn writer(conn: Arc<Conn>, sink: Arc<RemoteSink>, screens: mpsc::Sender<u32>) {
    let mut w = &conn.stream;
    loop {
        {
            let mut p = conn.pending.lock().unwrap();
            while !*p {
                p = conn.wake.wait_timeout(p, Duration::from_millis(500)).unwrap().0;
                if conn.closed.load(Ordering::SeqCst) || conn.ending.load(Ordering::SeqCst) {
                    break;
                }
            }
            *p = false;
        }
        let lines = std::mem::take(&mut *conn.lines.lock().unwrap());
        let mut bytes: Vec<(u32, Vec<u8>)> = vec![];
        let mut want: Vec<u32> = vec![];
        {
            let mut q = conn.out.lock().unwrap();
            let mut i = 0;
            while i < q.panes.len() {
                let key = q.panes[i].key;
                let b = outq::take(&mut q, i);
                if !b.is_empty() {
                    bytes.push((key, b));
                }
                if outq::want_screen(&mut q, i) {
                    want.push(key);
                }
                i += 1;
            }
        }
        let mut ok = true;
        for l in &lines {
            ok = ok && writeln!(w, "{l}").is_ok();
        }
        for (key, b) in bytes {
            if let Some(pane) = sink.pane_of(key) {
                let line = json!({ "event": "pane_output", "payload": { "pane": pane, "data": B64.encode(&b) } });
                ok = ok && writeln!(w, "{line}").is_ok();
            }
        }
        for key in want {
            let _ = screens.send(key);
        }
        let _ = w.flush();
        let ending = conn.ending.load(Ordering::SeqCst) && conn.lines.lock().unwrap().is_empty();
        if !ok || ending || conn.closed.load(Ordering::SeqCst) {
            conn.close();
            return;
        }
    }
}

// ---------- the window side ----------

/// What a client tells its owner.
pub enum News {
    /// An event of the hub, as Tauri would send it.
    Event(String, Value),
    /// The link went down, with the reason. `taken` when another window took over.
    Down { why: String, taken: bool },
}

type Waiter = mpsc::Sender<Result<Value, String>>;

/// Why the remote command ended: its last stderr lines, when it wrote any.
fn why_ended(errs: &Mutex<VecDeque<String>>) -> String {
    std::thread::sleep(Duration::from_millis(100));
    let e = errs.lock().unwrap();
    if e.is_empty() {
        "the remote command ended".to_owned()
    } else {
        e.iter().cloned().collect::<Vec<_>>().join(" ")
    }
}

/// A window's link to a remote hub, through a program such as `ssh host sb attach`.
pub struct Client {
    input: Mutex<Box<dyn Write + Send>>,
    waiting: Arc<Mutex<HashMap<u64, Waiter>>>,
    next: AtomicU64,
    up: Arc<AtomicBool>,
    child: Mutex<Option<std::process::Child>>,
}

impl Client {
    /// Runs `argv` (for example `ssh -T host sb attach`), says hello, and waits for
    /// the answer. `news` gets the events and the end of the link.
    pub fn connect(argv: &[String], news: impl Fn(News) + Send + 'static) -> Result<Arc<Client>, String> {
        let (prog, rest) = argv.split_first().ok_or("no remote command")?;
        tracing::info!(target: "sb::remote", ?argv, "connect");
        let mut child = std::process::Command::new(prog)
            .args(rest)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("{prog}: {e}"))?;
        let mut stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = child.stderr.take().ok_or("no stderr")?;
        writeln!(stdin, "{}", hello()).map_err(|e| e.to_string())?;
        let _ = stdin.flush();
        // The last stderr lines tell why ssh failed.
        let errs: Arc<Mutex<VecDeque<String>>> = Arc::default();
        {
            let errs = Arc::clone(&errs);
            std::thread::spawn(move || {
                for l in BufReader::new(stderr).lines().map_while(Result::ok) {
                    tracing::info!(target: "sb::remote", "remote stderr: {l}");
                    let mut e = errs.lock().unwrap();
                    e.push_back(l);
                    if e.len() > 5 {
                        e.pop_front();
                    }
                }
            });
        }
        let mut lines = BufReader::new(stdout);
        let mut first = String::new();
        let n = lines.read_line(&mut first).map_err(|e| e.to_string())?;
        if n == 0 {
            let _ = child.kill();
            return Err(why_ended(&errs));
        }
        let answer: Value = serde_json::from_str(&first).map_err(|_| format!("not a Switchboard answer: {}", first.trim()))?;
        if answer["hello"]["ok"] != true {
            let _ = child.kill();
            return Err(answer["hello"]["why"].as_str().unwrap_or("the hub refused the window").to_owned());
        }
        let client = Arc::new(Client {
            input: Mutex::new(Box::new(stdin)),
            waiting: Arc::default(),
            next: AtomicU64::new(1),
            up: Arc::new(AtomicBool::new(true)),
            child: Mutex::new(Some(child)),
        });
        let (waiting, up) = (Arc::clone(&client.waiting), Arc::clone(&client.up));
        let errs = Arc::clone(&errs);
        std::thread::spawn(move || {
            let mut taken = false;
            let mut line = String::new();
            loop {
                line.clear();
                match lines.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(id) = v["id"].as_u64() {
                    let w = waiting.lock().unwrap().remove(&id);
                    if let Some(w) = w {
                        let r = match v.get("err") {
                            Some(e) => Err(e.as_str().unwrap_or("error").to_owned()),
                            None => Ok(v.get("ok").cloned().unwrap_or(Value::Null)),
                        };
                        let _ = w.send(r);
                    }
                } else if let Some(ev) = v["event"].as_str() {
                    if ev == "taken_over" {
                        taken = true;
                    }
                    news(News::Event(ev.to_owned(), v.get("payload").cloned().unwrap_or(Value::Null)));
                }
            }
            up.store(false, Ordering::SeqCst);
            let pending: Vec<Waiter> = waiting.lock().unwrap().drain().map(|(_, w)| w).collect();
            for w in pending {
                let _ = w.send(Err("the link to the server is down".into()));
            }
            let why = if taken { "another window took over".to_owned() } else { why_ended(&errs) };
            news(News::Down { why, taken });
        });
        Ok(client)
    }

    pub fn is_up(&self) -> bool {
        self.up.load(Ordering::SeqCst)
    }

    /// One command, and its answer. It blocks until the answer or the end of the link.
    pub fn call(&self, cmd: &str, args: Value) -> Result<Value, String> {
        if !self.is_up() {
            return Err("the link to the server is down".into());
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.waiting.lock().unwrap().insert(id, tx);
        let line = json!({ "id": id, "cmd": cmd, "args": args }).to_string();
        let sent = {
            let mut i = self.input.lock().unwrap();
            writeln!(i, "{line}").and_then(|_| i.flush())
        };
        if sent.is_err() {
            self.waiting.lock().unwrap().remove(&id);
            return Err("the link to the server is down".into());
        }
        rx.recv().unwrap_or_else(|_| Err("the link to the server is down".into()))
    }

    /// Ends the link.
    pub fn close(&self) {
        if let Some(mut c) = self.child.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_needs_the_same_version() {
        assert_eq!(hello_problem(&hello()), None);
        let old = json!({ "hello": { "proto": PROTO, "version": "0.0.1" } });
        assert!(hello_problem(&old).unwrap().contains("0.0.1"));
        let other = json!({ "hello": { "proto": 99, "version": env!("CARGO_PKG_VERSION") } });
        assert!(hello_problem(&other).is_some());
    }

    #[test]
    fn screen_commands_are_commands() {
        for c in SCREEN {
            assert!(dispatch::COMMANDS.contains(c), "{c}");
        }
    }
}
