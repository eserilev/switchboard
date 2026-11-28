//! Remote mode (SPEC 29): the hub runs on a server as `sb serve`, and the window on
//! another computer talks to it through `ssh host sb attach`.
//!
//! The protocol is one JSON object per line, over the window socket:
//! - The window starts with `{"hello":{"proto":1,"version":"0.1.0"}}`. The hub answers
//!   `{"hello":{"ok":true,...}}`, or `{"hello":{"ok":false,"why":...}}` and closes.
//! - A command: `{"id":7,"cmd":"pane_open","args":{...}}`. Its answer:
//!   `{"id":7,"ok":<value>}` or `{"id":7,"err":"..."}`. Answers can come out of order.
//! - News from the hub: `{"event":"panes","payload":...,"seq":[boot,n]}`, and pane
//!   bytes as `{"event":"pane_output","payload":{"pane":"p3","data":"<base64>"}}`.
//!   A window that comes back puts the last `seq` it saw in its hello as `since`.
//!   The hello answer says `resumed: true` when the hub sends it what it missed,
//!   before any new event; else the window loads everything again (SPEC 41.8).
//!
//! Several windows at once (SPEC 41.8). Each has its own screen, its own queue
//! and its own writer; the news goes to all of them. A window that comes back with
//! the instance id of a window still attached replaces it: that old link is dead.
//!
//! Nothing that a hub thread calls does I/O under a lock. The sink only puts lines
//! and bytes in queues, and a writer thread for each window writes them. The pane
//! bytes go through the proved queue of `outq`: past its budget a pane drops its
//! bytes, and the window gets a fresh screen of it. Lines (answers and news) never
//! drop; when too many wait, the window is too slow and the hub closes it.

use crate::dispatch;
use crate::events::EventLog;
use crate::hub::{Hub, Sink};
use crate::screens::Win;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write, Read};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::Duration;

/// The protocol version. A window and a hub with different versions refuse each other.
pub const PROTO: u64 = 1;
/// The bytes that one pane can have waiting before they drop.
pub const BUDGET: usize = 2 << 20;
/// The lines that can wait for a window before the hub closes it.
pub const MAX_LINES: usize = 20_000;

/// The first line of a window.
pub fn hello() -> Value {
    hello_as(instance())
}

/// The first line of a window with its own instance id.
pub fn hello_as(instance: &str) -> Value {
    hello_since(instance, None)
}

/// The first line of a window that comes back: `since` is the last `(boot, n)` it saw.
pub fn hello_since(instance: &str, since: Option<(u64, u64)>) -> Value {
    let mut h = json!({ "proto": PROTO, "version": env!("CARGO_PKG_VERSION"), "instance": instance });
    if let Some((b, n)) = since {
        h["since"] = json!([b, n]);
    }
    json!({ "hello": h })
}

/// The `seq` of an event line: `(boot, n)`.
fn seq_of(v: &Value) -> Option<(u64, u64)> {
    Some((v["seq"][0].as_u64()?, v["seq"][1].as_u64()?))
}

/// The id of this process as a window: the same for each link it makes, and new
/// for each start. A window that comes back with it replaces its old link.
pub fn instance() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        format!("{}-{}", std::process::id(), at.as_nanos())
    })
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

/// The commands that one device window runs at once, at most.
pub const MAX_RUNNING: usize = 4;

/// The device of a window, as its session started.
pub struct Device {
    pub id: String,
    pub generation: u64,
    pub classes: u8,
}

/// The events a device with `read` gets. A device gets no other event: an event of
/// one window (`focus`), and the errors of other windows, are not for it.
const READ_EVENTS: &[&str] = &[
    "connections", "mail", "mail_agents", "pane", "panes", "review", "review_cleanup", "review_closed",
    "review_stream", "tasks", "today",
];

fn device_gets(classes: u8, event: &str) -> bool {
    classes & scope_check::READ != 0 && READ_EVENTS.contains(&event)
}

/// One window on the socket.
struct Conn {
    epoch: u64,
    /// The instance id of its hello, or empty.
    instance: String,
    /// The device behind the window. With none, the window is trusted (SPEC 41.7).
    device: Option<Device>,
    /// The classes of the device, as of its last check.
    classes: AtomicU8,
    /// The commands of a device that run now.
    running: AtomicUsize,
    stream: UnixStream,
    lines: Mutex<VecDeque<String>>,
    out: Mutex<outq::Queue>,
    wake: Condvar,
    /// Set with `lines`: the writer waits on it.
    pending: Mutex<bool>,
    /// The socket is shut.
    closed: AtomicBool,
    /// The window leaves: the writer sends what waits, then shuts the socket.
    ending: AtomicBool,
}

impl Conn {
    fn ring(&self) {
        *self.pending.lock().unwrap() = true;
        self.wake.notify_one();
    }

    fn gets(&self, event: &str) -> bool {
        self.device.is_none() || device_gets(self.classes.load(Ordering::SeqCst), event)
    }

    /// Whether the window may go on: a trusted one always, a device while it is on
    /// the list with the generation its session started with.
    fn listed(&self, hub: &Hub) -> bool {
        let Some(d) = &self.device else { return true };
        match hub.device(&d.id).filter(|r| r.revoked_at.is_none() && r.generation == d.generation) {
            Some(r) => {
                self.classes.store(r.classes, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    fn line(&self, v: &Value) {
        self.line_str(v.to_string())
    }

    fn line_str(&self, line: String) {
        let n = {
            let mut l = self.lines.lock().unwrap();
            l.push_back(line);
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
    /// The numbered events, for windows that come back. Lock order: `log`, then
    /// `conns`: an event gets its number and reaches the windows in one step, so
    /// each window gets the events in the order of their numbers.
    log: Mutex<EventLog>,
    /// The windows, by epoch. Nothing else is locked under it.
    conns: Mutex<BTreeMap<Win, Arc<Conn>>>,
    next_epoch: AtomicU64,
    /// The `outq` key of each pane id.
    keys: Mutex<HashMap<String, u32>>,
}

impl RemoteSink {
    fn conn(&self, win: Win) -> Option<Arc<Conn>> {
        self.conns.lock().unwrap().get(&win).cloned()
    }

    fn all(&self) -> Vec<Arc<Conn>> {
        self.conns.lock().unwrap().values().cloned().collect()
    }

    fn key(&self, pane: &str) -> u32 {
        let mut k = self.keys.lock().unwrap();
        let n = k.len() as u32;
        *k.entry(pane.to_owned()).or_insert(n)
    }

    fn pane_of(&self, key: u32) -> Option<String> {
        self.keys.lock().unwrap().iter().find(|(_, k)| **k == key).map(|(p, _)| p.clone())
    }

    /// Closes the sessions of devices that are not on the list any more.
    pub fn end_unlisted(&self, hub: &Hub) {
        for c in self.all() {
            if !c.listed(hub) {
                tracing::info!(target: "sb::remote", epoch = c.epoch, "the device is not listed any more");
                c.close();
            }
        }
    }

    /// The number of windows attached.
    pub fn windows(&self) -> usize {
        self.conns.lock().unwrap().len()
    }
}

impl Sink for RemoteSink {
    fn emit(&self, event: &str, payload: Value) {
        let mut log = self.log.lock().unwrap();
        let boot = log.boot;
        let (_, line) = log.record(event, &payload, |n| json!({ "event": event, "payload": payload, "seq": [boot, n] }).to_string());
        for c in self.all().into_iter().filter(|c| c.gets(event)) {
            c.line_str(line.clone());
        }
    }

    fn output(&self, pane: &str, data: &[u8]) {
        let key = self.key(pane);
        // A device gets the bytes of its own screen only (`output_to`).
        for c in self.all().into_iter().filter(|c| c.device.is_none()) {
            outq::push(&mut c.out.lock().unwrap(), key, data);
            c.ring();
        }
    }

    fn reset(&self, pane: &str) {
        let key = self.key(pane);
        for c in self.all() {
            outq::forget(&mut c.out.lock().unwrap(), key);
        }
    }

    fn output_to(&self, win: Win, pane: &str, data: &[u8]) {
        let key = self.key(pane);
        if let Some(c) = self.conn(win) {
            outq::push(&mut c.out.lock().unwrap(), key, data);
            c.ring();
        }
    }

    fn reset_for(&self, win: Win, pane: &str) {
        let key = self.key(pane);
        if let Some(c) = self.conn(win) {
            outq::forget(&mut c.out.lock().unwrap(), key);
        }
    }
}

/// Ends a window's link: the writer sends what waits, then shuts the socket. A
/// writer that blocks on a dead link never does: shut it after a while anyway.
fn end_link(conn: &Arc<Conn>) {
    conn.ending.store(true, Ordering::SeqCst);
    conn.ring();
    let conn = Arc::clone(conn);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        conn.close();
    });
}

/// Serves remote windows on `listener` until the hub goes. One thread for each
/// window, one writer for each window, and one thread for each command.
pub fn serve(listener: UnixListener, hub: Weak<Hub>, sink: Arc<RemoteSink>) {
    for stream in listener.incoming().flatten() {
        let Some(h) = hub.upgrade() else { break };
        let sink = Arc::clone(&sink);
        std::thread::spawn(move || window(h, sink, stream, None));
    }
}

/// One window, until its link ends. `device` is the device behind it, or none for
/// the hub's own socket: whoever reaches that runs as you already.
pub(crate) fn window(hub: Arc<Hub>, sink: Arc<RemoteSink>, stream: UnixStream, device: Option<Device>) {
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
    } else if device.is_some() {
        // A phone updates on its own: only the protocol has to fit.
        (hello["hello"]["proto"].as_u64() != Some(PROTO)).then(|| "the hub speaks another protocol".to_owned())
    } else {
        hello_problem(&hello)
    };
    let mut w = &stream;
    if let Some(why) = problem {
        tracing::warn!(target: "sb::remote", %why, "a window does not fit");
        let _ = writeln!(w, "{}", json!({ "hello": { "ok": false, "why": why } }));
        return;
    }
    let instance = hello["hello"]["instance"].as_str().unwrap_or("").to_owned();
    let conn = Arc::new(Conn {
        epoch: sink.next_epoch.fetch_add(1, Ordering::SeqCst) + 1,
        instance,
        classes: AtomicU8::new(device.as_ref().map_or(0, |d| d.classes)),
        device,
        running: AtomicUsize::new(0),
        stream,
        lines: Mutex::default(),
        out: Mutex::new(outq::new(BUDGET)),
        wake: Condvar::new(),
        pending: Mutex::new(false),
        closed: AtomicBool::new(false),
        ending: AtomicBool::new(false),
    });
    let since = seq_of(&json!({ "seq": hello["hello"]["since"] }));
    // A window that comes back: its old link is dead, and it goes. Other windows stay.
    let stale: Vec<Arc<Conn>> = {
        // Under the log lock: no event comes between the replay and the join.
        let log = sink.log.lock().unwrap();
        let missed = since.and_then(|(b, n)| log.since(b, n));
        // The answer goes first, then what the window missed, then new news.
        conn.line(&json!({ "hello": {
            "ok": true, "proto": PROTO, "version": env!("CARGO_PKG_VERSION"),
            "boot": log.boot, "resumed": missed.is_some(),
        } }));
        tracing::info!(target: "sb::remote", ?since, missed = missed.as_ref().map(Vec::len), "hello");
        for l in missed.unwrap_or_default() {
            let event = serde_json::from_str::<Value>(&l).ok();
            if conn.gets(event.as_ref().and_then(|v| v["event"].as_str()).unwrap_or("")) {
                conn.line_str(l);
            }
        }
        let mut conns = sink.conns.lock().unwrap();
        let stale = if conn.instance.is_empty() {
            vec![]
        } else {
            // A device never ends the link of another device or of your computer.
            let of = |c: &Conn| c.device.as_ref().map(|d| d.id.clone());
            conns.values().filter(|c| c.instance == conn.instance && of(c) == of(&conn)).cloned().collect()
        };
        conns.insert(conn.epoch, Arc::clone(&conn));
        stale
    };
    for old in &stale {
        tracing::info!(target: "sb::remote", old = old.epoch, new = conn.epoch, "a window comes back: its old link goes");
        end_link(old);
    }
    tracing::info!(target: "sb::remote", epoch = conn.epoch, windows = sink.windows(), "a window attached");

    // The window is in the list: a revoke from now on closes it. One that came
    // before is seen here, and nothing was written yet.
    if !conn.listed(&hub) {
        conn.close();
        sink.conns.lock().unwrap().remove(&conn.epoch);
        return;
    }
    let (screens, screen_rx) = mpsc::channel::<u32>();
    {
        let (conn, sink) = (Arc::clone(&conn), Arc::clone(&sink));
        std::thread::spawn(move || writer(conn, sink, screens));
    }
    {
        // Fresh screens, one at a time, away from the writer.
        let (conn, sink, hub) = (Arc::clone(&conn), Arc::clone(&sink), Arc::downgrade(&hub));
        std::thread::spawn(move || {
            for key in screen_rx {
                let Some(hub) = hub.upgrade() else { break };
                let pane = sink.pane_of(key);
                // Always fill: a pane that left the screen is live again, not stuck.
                let fill = |screen: Option<Vec<u8>>| {
                    let ok = screen.is_some();
                    outq::fill(&mut conn.out.lock().unwrap(), key, screen.unwrap_or_default());
                    ok
                };
                let ok = match &pane {
                    Some(p) => hub.fresh_screen(conn.epoch, p, fill),
                    None => fill(None),
                };
                tracing::info!(target: "sb::remote", pane = ?pane, ok, "fresh screen");
                conn.ring();
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
        // After the window's end, no more of its lines run: no late keys.
        if conn.closed.load(Ordering::SeqCst) || conn.ending.load(Ordering::SeqCst) {
            break;
        }
        let Ok(req) = serde_json::from_str::<Value>(&line) else {
            tracing::warn!(target: "sb::remote", "a bad line from the window");
            continue;
        };
        // A command that changes the screen or types runs here, in the order of the
        // lines: in a thread of its own, a collapse can run after the expand that
        // the window sent next (models/windows.qnt, W4).
        if conn.device.is_some() && conn.running.load(Ordering::SeqCst) >= MAX_RUNNING {
            conn.line(&json!({ "id": req["id"], "err": "too many commands at once" }));
            continue;
        }
        conn.running.fetch_add(1, Ordering::SeqCst);
        if dispatch::ORDERED.contains(&req["cmd"].as_str().unwrap_or("")) {
            answer(&hub, &conn, &req);
            continue;
        }
        let (hub, conn) = (Arc::clone(&hub), Arc::clone(&conn));
        std::thread::spawn(move || answer(&hub, &conn, &req));
    }
}

/// Ends a window when its read loop ends. The hub keeps running. The window's
/// screen goes; the screens of other windows stay.
struct Leave<'a> {
    hub: &'a Hub,
    sink: &'a RemoteSink,
    conn: &'a Arc<Conn>,
}

impl Drop for Leave<'_> {
    fn drop(&mut self) {
        tracing::info!(target: "sb::remote", epoch = self.conn.epoch, "a window left");
        end_link(self.conn);
        // A panic under the lock poisons it: the value is still good.
        self.sink.conns.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.conn.epoch);
        self.hub.collapse_in(self.conn.epoch);
    }
}

/// Runs one command line and queues its answer.
fn answer(hub: &Hub, conn: &Conn, req: &Value) {
    // Also after a panic in the command.
    struct Done<'a>(&'a Conn);
    impl Drop for Done<'_> {
        fn drop(&mut self) {
            self.0.running.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let _done = Done(conn);
    if !conn.listed(hub) {
        tracing::info!(target: "sb::remote", epoch = conn.epoch, "the device is not listed any more");
        conn.close();
        return;
    }
    let origin = if conn.device.is_some() { scope_check::DEVICE } else { scope_check::TRUSTED };
    let id = req["id"].clone();
    let cmd = req["cmd"].as_str().unwrap_or("");
    let args = req.get("args").cloned().unwrap_or(json!({}));
    let out = match gate(origin, conn.classes.load(Ordering::SeqCst), cmd, &args) {
        Ok(()) => dispatch::dispatch_in(hub, conn.epoch, cmd, &args),
        Err(e) => Err(e),
    };
    if let Err(err) = &out {
        tracing::warn!(target: "sb::remote", cmd, %err, "command failed");
    }
    conn.line(&match out {
        Ok(v) => json!({ "id": id, "ok": v }),
        Err(e) => json!({ "id": id, "err": e }),
    });
}

/// The check of SPEC 41.7 before a command runs: the command table, the classes
/// of a device, and the arguments. `answer` is the only way to `dispatch` from a
/// window, and it calls this first.
pub fn gate(origin: u8, classes: u8, cmd: &str, args: &Value) -> Result<(), String> {
    let url = args.get("url").and_then(Value::as_str).unwrap_or("");
    match scope_check::check(origin, classes, cmd.as_bytes(), url.as_bytes()) {
        scope_check::OK => Ok(()),
        scope_check::UNKNOWN => Err(format!("unknown command {cmd}")),
        scope_check::NEVER => Err(format!("{cmd} runs only on the computer")),
        scope_check::CLASS => Err(format!("this device may not run {cmd}")),
        _ => Err(format!("{cmd}: the arguments are refused")),
    }
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
    /// The last `(boot, n)` of an event, for the hello of the next link.
    seq: Arc<Mutex<Option<(u64, u64)>>>,
    /// The hub sent what this window missed since the `since` of its hello.
    resumed: bool,
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
        Self::connect_as(argv, instance(), None, news)
    }

    /// [`Client::connect`] as the window `instance`. Two links with one instance
    /// are one window: the hub keeps the newer link only. `since` is the last
    /// `(boot, n)` the window saw, from [`Client::seq`] of its old link.
    pub fn connect_as(
        argv: &[String],
        instance: &str,
        since: Option<(u64, u64)>,
        news: impl Fn(News) + Send + 'static,
    ) -> Result<Arc<Client>, String> {
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
        let stdin = c.stdin.take().ok_or("no stdin")?;
        let stdout = c.stdout.take().ok_or("no stdout")?;
        let stderr = c.stderr.take().ok_or("no stderr")?;
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
        Self::over(Box::new(stdin), stdout, errs, child, instance, since, news)
    }

    /// A window's link over a stream pair that is already open: `input` takes the
    /// lines to the hub, `output` gives its lines. A phone's encrypted session
    /// (SPEC 41.5) comes in here; so does the program of [`Client::connect_as`].
    pub fn over_streams(
        input: Box<dyn Write + Send>,
        output: impl Read + Send + 'static,
        instance: &str,
        since: Option<(u64, u64)>,
        news: impl Fn(News) + Send + 'static,
    ) -> Result<Arc<Client>, String> {
        Self::over(input, output, Arc::default(), KillOnDrop(None), instance, since, news)
    }

    fn over(
        mut stdin: Box<dyn Write + Send>,
        stdout: impl Read + Send + 'static,
        errs: Arc<Mutex<VecDeque<String>>>,
        mut child: KillOnDrop,
        instance: &str,
        since: Option<(u64, u64)>,
        news: impl Fn(News) + Send + 'static,
    ) -> Result<Arc<Client>, String> {
        writeln!(stdin, "{}", hello_since(instance, since)).map_err(|e| e.to_string())?;
        let _ = stdin.flush();
        // The hello answer, in a thread: a server that never answers ends in a
        // timeout. The kill on the way out ends that thread too.
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut lines = BufReader::new(stdout);
            let got = hello_answer(&mut lines);
            let _ = tx.send((got, lines));
        });
        let Ok((got, mut lines)) = rx.recv_timeout(HELLO_WAIT) else {
            // What ssh said, if anything, tells why.
            let said = errs.lock().unwrap().iter().cloned().collect::<Vec<_>>().join(" ");
            let wait = HELLO_WAIT.as_secs();
            return Err(if said.is_empty() { format!("no answer from the server in {wait} s") } else { format!("no answer from the server in {wait} s: {said}") });
        };
        let Some(answer) = got? else {
            return Err(why_ended(&errs));
        };
        if answer["hello"]["ok"] != true {
            return Err(answer["hello"]["why"].as_str().unwrap_or("the hub refused the window").to_owned());
        }
        let client = Arc::new(Client {
            seq: Arc::new(Mutex::new(since)),
            resumed: answer["hello"]["resumed"] == true,
            input: Mutex::new(stdin),
            waiting: Arc::default(),
            next: AtomicU64::new(1),
            up: Arc::new(AtomicBool::new(true)),
            child: Mutex::new(child.0.take()),
        });
        let (waiting, up, seq) = (Arc::clone(&client.waiting), Arc::clone(&client.up), Arc::clone(&client.seq));
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
                    if let Some(sq) = seq_of(&v) {
                        *seq.lock().unwrap() = Some(sq);
                    }
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

    /// The last `(boot, n)` of an event this window got, on this link or before.
    pub fn seq(&self) -> Option<(u64, u64)> {
        *self.seq.lock().unwrap()
    }

    /// True when the hub sent what the window missed since its last link: no full load needed.
    pub fn resumed(&self) -> bool {
        self.resumed
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

    /// The table of SPEC 41.7 names every command of `dispatch`, and only those.
    #[test]
    fn the_table_names_every_command() {
        for c in dispatch::COMMANDS {
            assert_ne!(scope_check::class_of(c.as_bytes()), 0, "{c} is not in the table");
        }
        let lists = [
            scope_check::READ_CMDS,
            scope_check::REVIEW_CMDS,
            scope_check::ASK_CMDS,
            scope_check::ACT_CMDS,
            scope_check::TERMINAL_CMDS,
            scope_check::ADMIN_CMDS,
        ];
        for l in lists {
            for n in std::str::from_utf8(l).unwrap().split(' ') {
                assert!(dispatch::COMMANDS.contains(&n), "{n} is in the table but not a command");
            }
        }
    }

    /// `answer` calls `gate` before `dispatch_in`, and nothing else in this file calls `dispatch`.
    #[test]
    fn every_command_goes_through_the_gate() {
        let src = include_str!("remote.rs");
        let body = src.split("fn answer(").nth(1).unwrap().split("\n}\n").next().unwrap();
        let g = body.find("gate(").expect("answer calls gate");
        let d = body.find(concat!("dispatch::", "dispatch_in(")).expect("answer calls dispatch_in");
        assert!(g < d);
        let calls = |pat: &str| src.matches(pat).count();
        // One call, in `answer`. The test builds its patterns, so they do not count.
        assert_eq!(calls(concat!("dispatch::", "dispatch_in(")), 1);
        assert_eq!(calls(concat!("dispatch::", "dispatch(")), 0);
    }

    #[test]
    fn a_device_is_refused_with_a_reason() {
        let d = scope_check::DEVICE;
        assert!(gate(d, scope_check::PHONE, "panes", &json!({})).is_ok());
        assert!(gate(d, scope_check::PHONE, "hub_restart", &json!({})).unwrap_err().contains("only on the computer"));
        assert!(gate(d, scope_check::PHONE, "pane_input", &json!({})).unwrap_err().contains("may not"));
        let bad = json!({ "url": "https://evil.example/o/r/pull/1" });
        assert!(gate(d, scope_check::PHONE, "review_open", &bad).unwrap_err().contains("arguments"));
        let good = json!({ "url": "https://github.com/o/r/pull/1" });
        assert!(gate(d, scope_check::PHONE, "review_open", &good).is_ok());
        assert!(gate(scope_check::TRUSTED, 0, "hub_restart", &json!({})).is_ok());
        assert!(gate(scope_check::TRUSTED, 0, "nope", &json!({})).unwrap_err().contains("unknown"));
    }

    #[test]
    fn ordered_commands_are_commands() {
        for c in dispatch::ORDERED {
            assert!(dispatch::COMMANDS.contains(c), "{c}");
        }
    }
}
