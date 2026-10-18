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
        // The writer sends the event, then shuts the socket. A writer that blocks on
        // a dead link never does: shut it after a while anyway.
        old.ending.store(true, Ordering::SeqCst);
        old.ring();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(5));
            old.close();
        });
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
                // Under the owner lock, as a screen command: no collapse comes between
                // the check of `resume_stream` and its stream on, and a window that
                // lost a takeover never pauses a pane of the new one (models/remote.qnt, R6).
                let owner = sink.owner.lock().unwrap();
                let mine = *owner == conn.epoch;
                let screen = match &pane {
                    Some(p) if mine => hub.screen_for_resync(p).ok(),
                    _ => None,
                };
                tracing::info!(target: "sb::remote", pane = ?pane, mine, ok = screen.is_some(), "fresh screen");
                // Always fill: a pane that left the screen is live again, not stuck.
                outq::fill(&mut conn.out.lock().unwrap(), key, screen.unwrap_or_default());
                conn.ring();
                match &pane {
                    Some(p) if mine => hub.resume_stream(p),
                    _ => {}
                }
                drop(owner);
            }
        });
    }

    // The end of the window, also after a panic in a command of the read loop.
    let _leave = Leave { hub: &hub, sink: &sink, conn: &conn };
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        // After a takeover, no more lines of this window run: no late keys.
        if conn.closed.load(Ordering::SeqCst) || conn.ending.load(Ordering::SeqCst) {
            break;
        }
        let Ok(req) = serde_json::from_str::<Value>(&line) else {
            tracing::warn!(target: "sb::remote", "a bad line from the window");
            continue;
        };
        // A command that changes the screen or types runs here, in the order of the
        // lines: in a thread of its own, a collapse can run after the expand that
        // the window sent next (models/remote.qnt, R5).
        if dispatch::ORDERED.contains(&req["cmd"].as_str().unwrap_or("")) {
            answer(&hub, &sink, &conn, &req);
            continue;
        }
        let (hub, sink, conn) = (Arc::clone(&hub), Arc::clone(&sink), Arc::clone(&conn));
        std::thread::spawn(move || answer(&hub, &sink, &conn, &req));
    }
}

/// Ends a window when its read loop ends. The hub keeps running. Its screen goes
/// with the window that owned it.
struct Leave<'a> {
    hub: &'a Hub,
    sink: &'a RemoteSink,
    conn: &'a Conn,
}

impl Drop for Leave<'_> {
    fn drop(&mut self) {
        tracing::info!(target: "sb::remote", epoch = self.conn.epoch, "a window left");
        self.conn.close();
        // A panic under the owner lock poisons it: the value is still good.
        let owner = self.sink.owner.lock().unwrap_or_else(|e| e.into_inner());
        if *owner == self.conn.epoch {
            let mut cur = self.sink.current.lock().unwrap_or_else(|e| e.into_inner());
            if cur.as_ref().map(|c| c.epoch) == Some(self.conn.epoch) {
                *cur = None;
            }
            drop(cur);
            self.hub.collapse();
        }
    }
}

/// Runs one command line and queues its answer.
fn answer(hub: &Hub, sink: &RemoteSink, conn: &Conn, req: &Value) {
    let id = req["id"].clone();
    let cmd = req["cmd"].as_str().unwrap_or("");
    let args = req.get("args").cloned().unwrap_or(json!({}));
    let out = run(hub, sink, conn.epoch, cmd, &args);
    if let Err(err) = &out {
        tracing::warn!(target: "sb::remote", cmd, %err, "command failed");
    }
    conn.line(&match out {
        Ok(v) => json!({ "id": id, "ok": v }),
        Err(e) => json!({ "id": id, "err": e }),
    });
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

/// How long a window waits for the hello answer.
const HELLO_WAIT: Duration = Duration::from_secs(20);

/// The hello answer of the hub. A login shell can print a banner first: lines
/// that are not a hello are skipped, up to 100. `None`: the link ended.
fn hello_answer(lines: &mut impl BufRead) -> Result<Option<Value>, String> {
    let mut line = String::new();
    for _ in 0..100 {
        line.clear();
        if lines.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            return Ok(None);
        }
        match serde_json::from_str::<Value>(&line) {
            Ok(v) if v.get("hello").is_some() => return Ok(Some(v)),
            _ => tracing::info!(target: "sb::remote", "not a hello: {}", line.trim_end()),
        }
    }
    Err("the server sent no hello in its first 100 lines".into())
}

/// Kills and reaps a child when it goes out of scope.
struct KillOnDrop(Option<std::process::Child>);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
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
        let child = std::process::Command::new(prog)
            .args(rest)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("{prog}: {e}"))?;
        // Until the hello is good, any way out kills the program.
        let mut child = KillOnDrop(Some(child));
        let c = child.0.as_mut().ok_or("no child")?;
        let mut stdin = c.stdin.take().ok_or("no stdin")?;
        let stdout = c.stdout.take().ok_or("no stdout")?;
        let stderr = c.stderr.take().ok_or("no stderr")?;
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
        // The hello answer, in a thread: a server that never answers ends in a
        // timeout. The kill on the way out ends that thread too.
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut lines = BufReader::new(stdout);
            let got = hello_answer(&mut lines);
            let _ = tx.send((got, lines));
        });
        let Ok((got, mut lines)) = rx.recv_timeout(HELLO_WAIT) else {
            return Err(format!("no answer from the server in {} s", HELLO_WAIT.as_secs()));
        };
        let Some(answer) = got? else {
            return Err(why_ended(&errs));
        };
        if answer["hello"]["ok"] != true {
            return Err(answer["hello"]["why"].as_str().unwrap_or("the hub refused the window").to_owned());
        }
        let client = Arc::new(Client {
            input: Mutex::new(Box::new(stdin)),
            waiting: Arc::default(),
            next: AtomicU64::new(1),
            up: Arc::new(AtomicBool::new(true)),
            child: Mutex::new(child.0.take()),
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
        let rx = self.send(cmd, args)?;
        rx.recv().unwrap_or_else(|_| Err("the link to the server is down".into()))
    }

    /// Writes one command and returns at once. The answer comes on the receiver.
    /// Lines go out in the order of the calls.
    pub fn send(&self, cmd: &str, args: Value) -> Result<mpsc::Receiver<Result<Value, String>>, String> {
        if !self.is_up() {
            return Err("the link to the server is down".into());
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.waiting.lock().unwrap().insert(id, tx);
        // The reader can end and drain the waiters before the insert: then no
        // answer ever comes. It marks the link down first, so look again.
        if !self.is_up() {
            self.waiting.lock().unwrap().remove(&id);
            return Err("the link to the server is down".into());
        }
        let line = json!({ "id": id, "cmd": cmd, "args": args }).to_string();
        let sent = {
            let mut i = self.input.lock().unwrap();
            writeln!(i, "{line}").and_then(|_| i.flush())
        };
        if sent.is_err() {
            self.waiting.lock().unwrap().remove(&id);
            return Err("the link to the server is down".into());
        }
        Ok(rx)
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
    fn a_banner_before_the_hello_is_skipped() {
        let mut r = std::io::Cursor::new(b"Welcome to box\n{\"not\":1}\n{\"hello\":{\"ok\":true}}\n".to_vec());
        assert_eq!(hello_answer(&mut r).unwrap().unwrap()["hello"]["ok"], true);
        let mut r = std::io::Cursor::new(b"only a banner\n".to_vec());
        assert!(hello_answer(&mut r).unwrap().is_none());
    }

    /// A refused hello ends the link program: no ssh stays behind (SPEC 29.8, V1).
    #[test]
    fn a_failed_connect_kills_its_program() {
        let dir = std::env::temp_dir().join(format!("sb-connect-kill-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pid_file = dir.join("pid");
        let script = format!(
            "echo $$ > {}; echo '{{\"hello\":{{\"ok\":false,\"why\":\"no\"}}}}'; exec sleep 30",
            pid_file.display()
        );
        let argv = ["sh".to_owned(), "-c".to_owned(), script];
        let err = Client::connect(&argv, |_| {}).err().unwrap();
        assert_eq!(err, "no");
        let pid: i32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
        // SAFETY: kill with signal 0 only asks whether the process exists.
        assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "the program still runs");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn screen_commands_are_commands() {
        for c in SCREEN {
            assert!(dispatch::COMMANDS.contains(c), "{c}");
            // A screen command runs in order, under the owner lock.
            assert!(dispatch::ORDERED.contains(c), "{c} is not ordered");
        }
        for c in dispatch::ORDERED {
            assert!(dispatch::COMMANDS.contains(c), "{c}");
        }
    }
}
