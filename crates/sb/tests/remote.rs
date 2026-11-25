//! Remote mode (SPEC 29) on a real tmux: `sb serve`, `sb attach` and the window's
//! client, with no ssh. The window side runs `sb attach` as its remote command.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use switchboard_core::remote::{self, Client, News};

fn wait_for(what: &str, secs: u64, f: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out: {what}");
}

/// A home for a hub, its tmux socket, and the env of every program in the test.
struct Home {
    root: PathBuf,
    socket: String,
}

impl Home {
    fn new(name: &str) -> Home {
        let root = std::env::temp_dir().join(format!("sb-remote-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("work/demo")).unwrap();
        assert!(Command::new("git").arg("-C").arg(root.join("work/demo")).args(["init", "-q", "-b", "main"]).status().unwrap().success());
        Home { root, socket: format!("sb-test-remote-{name}-{}", std::process::id()) }
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_sb"));
        c.args(args)
            .env("SB_HOME", &self.root)
            .env("SB_TMUX_SOCKET", &self.socket)
            .env_remove("SB_SOCK")
            .env_remove("SB_PANE")
            .env("SHELL", "/bin/sh")
            .env("NVIM_APPNAME", format!("sb-test-nvim-{}", std::process::id()));
        c
    }

    fn argv(&self) -> Vec<String> {
        // `env` carries the test env into `sb attach`, as ssh would run it on the server.
        vec![
            "env".into(),
            "-u".into(),
            "SB_SOCK".into(),
            "-u".into(),
            "SB_PANE".into(),
            format!("SB_HOME={}", self.root.display()),
            format!("SB_TMUX_SOCKET={}", self.socket),
            "SHELL=/bin/sh".into(),
            format!("NVIM_APPNAME=sb-test-nvim-{}", std::process::id()),
            env!("CARGO_BIN_EXE_sb").into(),
            "attach".into(),
        ]
    }

    fn window_sock(&self) -> PathBuf {
        self.root.join("run/window.sock")
    }

    fn serve_pid(&self) -> Option<i32> {
        std::fs::read_to_string(self.root.join("run/serve.pid")).ok()?.trim().parse().ok()
    }

    fn keep_panes(&self) -> usize {
        let out = Command::new("tmux").args(["-L", &self.socket, "list-panes", "-a", "-F", "#{@sb_keep}"]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).lines().filter(|l| *l == "1").count()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        if let Some(pid) = self.serve_pid() {
            // SAFETY: a plain signal to a process the test started.
            unsafe { libc_kill(pid) };
        }
        std::thread::sleep(Duration::from_millis(300));
        let _ = Command::new("tmux").args(["-L", &self.socket, "kill-server"]).status();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

unsafe fn libc_kill(pid: i32) {
    let _ = Command::new("kill").arg(pid.to_string()).status();
}

/// The events a client got, and a flag for the end of its link.
#[derive(Default)]
struct Got {
    events: Mutex<Vec<(String, Value)>>,
    down: Mutex<Option<(String, bool)>>,
}

impl Got {
    fn output_of(&self, pane: &str) -> String {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(e, p)| e == "pane_output" && p["pane"] == pane)
            .map(|(_, p)| String::from_utf8_lossy(&B64.decode(p["data"].as_str().unwrap()).unwrap()).into_owned())
            .collect()
    }
}

/// A new window: each call is another window, with its own instance id.
fn connect(home: &Home) -> (Arc<Client>, Arc<Got>) {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    connect_as(home, &format!("test-{}-{n}", std::process::id()))
}

fn connect_as(home: &Home, instance: &str) -> (Arc<Client>, Arc<Got>) {
    connect_since(home, instance, None)
}

fn connect_since(home: &Home, instance: &str, since: Option<(u64, u64)>) -> (Arc<Client>, Arc<Got>) {
    let got = Arc::new(Got::default());
    let g = Arc::clone(&got);
    let client = Client::connect_as(&home.argv(), instance, since, move |n| match n {
        News::Event(e, p) => g.events.lock().unwrap().push((e, p)),
        News::Down { why, taken } => *g.down.lock().unwrap() = Some((why, taken)),
    })
    .expect("connect");
    (client, got)
}

fn open_shell(c: &Client, home: &Home) -> String {
    let v = c
        .call("pane_open", json!({ "req": { "repo": home.root.join("work/demo"), "place": "main", "run": "shell" } }))
        .unwrap();
    v["id"].as_str().unwrap().to_owned()
}

#[test]
fn a_remote_window_end_to_end() {
    let home = Home::new("e2e");

    // `sb attach` starts `sb serve` when none runs.
    let (a, got_a) = connect(&home);
    assert!(home.serve_pid().is_some(), "sb attach started sb serve");
    let id = open_shell(&a, &home);
    a.call("pane_resize", json!({ "cols": 100, "rows": 30 })).unwrap();
    let screen = a.call("pane_expand", json!({ "id": id })).unwrap();
    assert!(B64.decode(screen.as_str().unwrap()).unwrap().starts_with(b"\x1b[H"));
    a.call("pane_input", json!({ "id": id, "data": "echo hi-$((1+2))\r", "binary": false })).unwrap();
    wait_for("the output of the shell", 5, || got_a.output_of(&id).contains("hi-3"));
    assert!(got_a.events.lock().unwrap().iter().any(|(e, _)| e == "pane" || e == "panes"), "news comes too");
    // An unknown command is an error, not an end of the link.
    assert!(a.call("no_such_command", json!({})).is_err());
    assert!(a.is_up());

    // A version that does not fit gets a clear no.
    {
        let mut s = UnixStream::connect(home.window_sock()).unwrap();
        writeln!(s, "{}", json!({ "hello": { "proto": remote::PROTO, "version": "0.0.0" } })).unwrap();
        let mut line = String::new();
        BufReader::new(&s).read_line(&mut line).unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["hello"]["ok"], false);
        assert!(v["hello"]["why"].as_str().unwrap().contains("0.0.0"));
    }
    assert!(a.is_up(), "a refused window takes nothing over");

    // A second window: both stay up, each with its own screen (SPEC 41.8).
    let (b, got_b) = connect(&home);
    std::thread::sleep(Duration::from_millis(300));
    assert!(a.is_up(), "the first window stays");
    let panes = b.call("panes", json!({})).unwrap();
    assert!(panes.as_array().unwrap().iter().any(|p| p["id"] == id));
    b.call("pane_expand", json!({ "id": id })).unwrap();
    b.call("pane_input", json!({ "id": id, "data": "echo two-$((1+1))\r", "binary": false })).unwrap();
    wait_for("output in both windows", 5, || got_b.output_of(&id).contains("two-2") && got_a.output_of(&id).contains("two-2"));
    // The first window leaves the pane: the second keeps it, and its bytes.
    a.call("pane_collapse", json!({})).unwrap();
    b.call("pane_input", json!({ "id": id, "data": "echo three-$((1+2))\r", "binary": false })).unwrap();
    wait_for("output in the second window", 5, || got_b.output_of(&id).contains("three-3"));
    std::thread::sleep(Duration::from_millis(300));
    assert!(!got_a.output_of(&id).contains("three-3"), "a window with no screen gets no bytes");
    // The first window's link drops: the second keeps its screen.
    a.close();
    wait_for("the first window is down", 5, || got_a.down.lock().unwrap().is_some());
    assert!(!got_a.down.lock().unwrap().clone().unwrap().1, "no takeover");
    b.call("pane_input", json!({ "id": id, "data": "echo four-$((2+2))\r", "binary": false })).unwrap();
    wait_for("output after the other window left", 5, || got_b.output_of(&id).contains("four-4"));

    // The same window comes back on a new link: the hub ends the old link.
    let (b2, _got_b2) = connect_as(&home, "same-window");
    let (b3, _got_b3) = connect_as(&home, "same-window");
    wait_for("the old link of the window goes", 10, || !b2.is_up());
    assert!(b3.is_up() && b.is_up());
    b3.close();

    // All links drop: the hub and the pane keep running.
    b.close();
    wait_for("the window is down", 5, || got_b.down.lock().unwrap().is_some());
    std::thread::sleep(Duration::from_millis(300));
    let (c, _got_c) = connect(&home);
    let panes = c.call("panes", json!({})).unwrap();
    assert!(panes.as_array().unwrap().iter().any(|p| p["id"] == id), "the shell still runs");
    assert!(!panes.as_array().unwrap().iter().any(|p| p["live"] == true), "a new window starts with no screen");
    c.close();
}

/// A window that comes back gets the events it missed, the newest of each kind and
/// id, before any new event. Another start of the hub means a full load (SPEC 41.8).
#[test]
fn a_window_that_comes_back_gets_what_it_missed() {
    let home = Home::new("since");
    let (a, _got_a) = connect(&home);
    let first = open_shell(&a, &home);
    wait_for("a numbered event", 5, || a.seq().is_some());
    std::thread::sleep(Duration::from_millis(300));
    let seen = a.seq().unwrap();
    assert!(!a.resumed(), "a first link has nothing to resume");
    a.close();

    // Meanwhile another window makes news: a second shell.
    let (b, _) = connect(&home);
    let second = open_shell(&b, &home);
    std::thread::sleep(Duration::from_millis(300));

    let (back, got) = connect_since(&home, "back", Some(seen));
    assert!(back.resumed());
    wait_for("the missed panes", 5, || {
        got.events.lock().unwrap().iter().any(|(e, p)| e == "panes" && p.as_array().unwrap().iter().any(|x| x["id"] == second.as_str()))
    });
    // Nothing it saw comes again: each panes event it got has the second shell.
    let evs = got.events.lock().unwrap().clone();
    for (_, p) in evs.iter().filter(|(e, _)| e == "panes") {
        assert!(p.as_array().unwrap().iter().any(|x| x["id"] == second.as_str()), "an old event came again: {p}");
        assert!(p.as_array().unwrap().iter().any(|x| x["id"] == first.as_str()));
    }
    assert!(back.seq().unwrap().1 > seen.1);

    // Another start of the hub, or a number it never gave: a full load.
    let (other, _) = connect_since(&home, "other", Some((seen.0 ^ 1, seen.1)));
    assert!(!other.resumed());
    let (late, _) = connect_since(&home, "late", Some((seen.0, 1 << 40)));
    assert!(!late.resumed());
    for c in [b, back, other, late] {
        c.close();
    }
}

/// A window that lost bytes gets a fresh screen from the pane's emulator, and the
/// pane never pauses: another window loses nothing meanwhile (SPEC 41.8).
#[test]
fn a_fresh_screen_never_pauses_another_window() {
    let home = Home::new("fresh");
    let (a, got_a) = connect(&home);
    let id = open_shell(&a, &home);
    a.call("pane_resize", json!({ "cols": 100, "rows": 30 })).unwrap();
    a.call("pane_expand", json!({ "id": id })).unwrap();
    a.call("pane_input", json!({ "id": id, "data": "clear; echo marker-$((3+4))\r", "binary": false })).unwrap();
    wait_for("the marker", 5, || got_a.output_of(&id).contains("marker-7"));

    // A raw window that does not read, while numbers flood the pane.
    let mut s = UnixStream::connect(home.window_sock()).unwrap();
    writeln!(s, "{}", remote::hello_as("raw")).unwrap();
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    writeln!(s, "{}", json!({ "id": 1, "cmd": "pane_expand", "args": { "id": id } })).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let flood = "seq 1 400000; echo done-$((5+5))\r";
    a.call("pane_input", json!({ "id": id, "data": flood, "binary": false })).unwrap();
    wait_for("the flood in the window that reads", 30, || got_a.output_of(&id).contains("done-10"));
    // Every number reached the window that reads: no pause cut its stream.
    let out = got_a.output_of(&id);
    for n in ["1\r\n", "199999\r\n", "400000\r\n"] {
        assert!(out.contains(n), "{n:?} is missing");
    }

    // The raw window reads now: a fresh screen comes with the end of the flood.
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let mut fresh = String::new();
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end && !fresh.contains("done-10") {
        line.clear();
        match r.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => continue,
        }
        let v: Value = serde_json::from_str(&line).unwrap();
        if v["event"] == "pane_output" && v["payload"]["pane"] == id.as_str() {
            let b = B64.decode(v["payload"]["data"].as_str().unwrap()).unwrap();
            if b.starts_with(b"\x1b[H\x1b[2J") {
                fresh = String::from_utf8_lossy(&b).into_owned();
            } else if !fresh.is_empty() {
                fresh.push_str(&String::from_utf8_lossy(&b));
            }
        }
    }
    assert!(fresh.contains("done-10"), "a fresh screen came, then the rest");
    a.close();
}

/// Commands that change the screen or type run in the order of the lines, also
/// when the window does not wait for each answer (models/windows.qnt, W4).
#[test]
fn screen_commands_and_keys_keep_their_order() {
    let home = Home::new("order");
    let (c, got) = connect(&home);
    let id = open_shell(&c, &home);
    c.call("pane_resize", json!({ "cols": 100, "rows": 30 })).unwrap();
    c.call("pane_expand", json!({ "id": id })).unwrap();
    // A collapse and then an expand, many times, with no wait between: the pane
    // is on screen at the end.
    let mut answers = vec![];
    for _ in 0..20 {
        answers.push(c.send("pane_collapse", json!({})).unwrap());
        answers.push(c.send("pane_expand", json!({ "id": id })).unwrap());
    }
    for a in answers {
        a.recv().unwrap().unwrap();
    }
    let panes = c.call("panes", json!({})).unwrap();
    let p = panes.as_array().unwrap().iter().find(|p| p["id"] == id).unwrap().clone();
    assert_eq!(p["live"], true, "the last expand won: {p}");
    // Keys one by one, with no wait between: they reach the shell in order.
    let word = "abcdefghijklmnopqrstuvwxyz0123456789";
    let mut answers = vec![];
    for ch in format!("echo {word}-$((2+2))\r").chars() {
        answers.push(c.send("pane_input", json!({ "id": id, "data": ch.to_string(), "binary": false })).unwrap());
    }
    for a in answers {
        a.recv().unwrap().unwrap();
    }
    wait_for("the keys in order", 5, || got.output_of(&id).contains(&format!("{word}-4")));
    c.close();
}

/// A window that does not read: the hub keeps working, the pane drops bytes, and
/// the window gets a fresh screen after it reads again.
#[test]
fn a_flood_drops_bytes_and_never_blocks_the_hub() {
    let home = Home::new("flood");
    let (setup, _) = connect(&home);
    let id = open_shell(&setup, &home);
    setup.close();
    std::thread::sleep(Duration::from_millis(300));

    // A raw window, so the test decides when it reads.
    let mut s = UnixStream::connect(home.window_sock()).unwrap();
    writeln!(s, "{}", remote::hello()).unwrap();
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["hello"]["ok"], true);
    writeln!(s, "{}", json!({ "id": 1, "cmd": "pane_resize", "args": { "cols": 120, "rows": 30 } })).unwrap();
    writeln!(s, "{}", json!({ "id": 2, "cmd": "pane_expand", "args": { "id": id } })).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    // About 60 MB, with no newline, while the window reads nothing.
    let flood = "head -c 60000000 /dev/zero | tr '\\0' x; echo; echo flood-$((4+4))\r";
    writeln!(s, "{}", json!({ "id": 3, "cmd": "pane_input", "args": { "id": id, "data": flood, "binary": false } })).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    // The hub still talks to tmux: `sb open` makes a pane through it, quickly.
    let t0 = Instant::now();
    let out = home.cmd(&["open", home.root.join("work/demo").to_str().unwrap(), "--shell"]).stdin(Stdio::null()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(t0.elapsed() < Duration::from_secs(5), "the hub answered in {:?}", t0.elapsed());
    std::thread::sleep(Duration::from_secs(3));

    // Now read it all.
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let mut bytes = 0usize;
    let mut screens = 0usize;
    let mut text = String::new();
    let end = Instant::now() + Duration::from_secs(60);
    while Instant::now() < end {
        line.clear();
        match r.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => {
                if text.contains("flood-8") {
                    break;
                }
                continue;
            }
        }
        let v: Value = serde_json::from_str(&line).unwrap();
        if v["event"] == "pane_output" && v["payload"]["pane"] == id.as_str() {
            let b = B64.decode(v["payload"]["data"].as_str().unwrap()).unwrap();
            if b.starts_with(b"\x1b[H\x1b[2J") {
                screens += 1;
            }
            bytes += b.len();
            let t = String::from_utf8_lossy(&b);
            text.push_str(&t[t.len().saturating_sub(4000)..]);
            if text.len() > 20_000 {
                text.drain(..text.len() - 8000);
            }
        }
    }
    assert!(text.contains("flood-8"), "the end of the flood came through");
    assert!(bytes < 60_000_000, "bytes dropped: {bytes}");
    assert!(screens >= 1, "a fresh screen came after the drop");
}

/// Two `sb serve` on one home at the same time: one runs, and only one hub touched tmux.
#[test]
fn two_hubs_never_share_a_home() {
    for round in 0..8 {
        let home = Home::new(&format!("race{round}"));
        let mut a = home.cmd(&["serve"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap();
        let mut b = home.cmd(&["serve"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap();
        wait_for("one serve listens", 10, || home.window_sock().exists() && home.serve_pid().is_some());
        std::thread::sleep(Duration::from_millis(500));
        let alive = [a.try_wait().unwrap().is_none(), b.try_wait().unwrap().is_none()];
        assert_eq!(alive.iter().filter(|x| **x).count(), 1, "round {round}: exactly one hub runs");
        assert_eq!(home.keep_panes(), 1, "round {round}: one keep pane, from one hub");
        let loser = if alive[0] { &mut b } else { &mut a };
        let mut err = String::new();
        std::io::Read::read_to_string(loser.stderr.as_mut().unwrap(), &mut err).unwrap();
        assert!(err.contains("another Switchboard runs"), "{err}");
        let _ = Path::new(&home.root);
    }
}

/// Restart (SPEC 29.12): `hub_restart` makes `sb serve` start again in place, with
/// the same pid. The link goes down, not as a takeover, and a new window finds the
/// pane still there.
#[test]
fn a_restart_keeps_the_pid_and_the_panes() {
    let home = Home::new("restart");
    let (a, got_a) = connect(&home);
    let pid = home.serve_pid().unwrap();
    let id = open_shell(&a, &home);
    a.call("hub_restart", json!({})).unwrap();
    wait_for("the link goes down", 10, || got_a.down.lock().unwrap().is_some());
    let (why, taken) = got_a.down.lock().unwrap().clone().unwrap();
    assert!(!taken, "a restart is no takeover: {why}");
    wait_for("sb serve listens again", 15, || UnixStream::connect(home.window_sock()).is_ok() && home.serve_pid().is_some());
    assert_eq!(home.serve_pid(), Some(pid), "the same process, started again in place");
    let (b, _got_b) = connect(&home);
    let panes = b.call("panes", json!({})).unwrap();
    assert!(panes.as_array().unwrap().iter().any(|p| p["id"] == id), "the shell still runs: {panes}");
    b.close();
}

/// A fake phone: a session over TCP, and the lines of the window protocol in it.
struct Phone {
    tcp: std::net::TcpStream,
    seal: session::Sealer,
    open: session::Opener,
    n: u64,
    events: Vec<Value>,
}

impl Phone {
    fn connect(port: u16, me: &session::KeyPair, id: &str, hub: &[u8], instance: &str) -> std::io::Result<Phone> {
        let mut tcp = std::net::TcpStream::connect(("127.0.0.1", port))?;
        tcp.set_read_timeout(Some(Duration::from_secs(5)))?;
        let (seal, open) = session::connect(&mut tcp, me, id, hub)?;
        let mut p = Phone { tcp, seal, open, n: 0, events: vec![] };
        p.send(&json!({ "hello": { "proto": remote::PROTO, "version": "9.9.9", "instance": instance } }))?;
        let h = p.recv()?;
        assert_eq!(h["hello"]["ok"], true, "{h}");
        Ok(p)
    }

    fn send(&mut self, v: &Value) -> std::io::Result<()> {
        self.seal.send(&mut self.tcp, v.to_string().as_bytes())
    }

    fn recv(&mut self) -> std::io::Result<Value> {
        let f = self.open.recv(&mut self.tcp, session::MAX_NEWS)?;
        Ok(serde_json::from_slice(&f).unwrap())
    }

    /// The answer of a command. The events before it are kept.
    fn call(&mut self, cmd: &str, args: Value) -> std::io::Result<Result<Value, String>> {
        self.n += 1;
        let id = self.n;
        self.send(&json!({ "id": id, "cmd": cmd, "args": args }))?;
        loop {
            let v = self.recv()?;
            if v["id"] == id {
                return Ok(match v["err"].as_str() {
                    Some(e) => Err(e.to_owned()),
                    None => Ok(v["ok"].clone()),
                });
            }
            self.events.push(v);
        }
    }
}

#[test]
fn a_device_runs_only_its_classes_and_a_revoke_ends_it() {
    use switchboard_core::{paths::Paths, store};
    let home = Home::new("device");
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let paths = Paths::under(home.root.clone());
    std::fs::write(&paths.config, format!("[devices]\nlisten = \"127.0.0.1:{port}\"\n")).unwrap();
    let instance = format!("desktop-{}", std::process::id());
    let (desk, desk_got) = connect_as(&home, &instance);
    let hub = switchboard_core::devices::hub_key(&paths).unwrap().public;
    let (me, stranger) = (session::keypair().unwrap(), session::keypair().unwrap());
    let db = store::Store::open(&paths.store()).unwrap();
    db.device_add(&store::Device {
        id: "d1".into(),
        name: "phone".into(),
        platform: "ios".into(),
        session_key: me.public.clone(),
        push_key: None,
        classes: scope_check::PHONE,
        paired_at: 1,
        last_seen: None,
        revoked_at: None,
        generation: 0,
    })
    .unwrap();
    wait_for("the listener", 10, || std::net::TcpStream::connect(("127.0.0.1", port)).is_ok());

    // A key that is not listed, and a listed key with another id, get no session.
    assert!(Phone::connect(port, &stranger, "d1", &hub, "x").is_err());
    assert!(Phone::connect(port, &me, "d2", &hub, "x").is_err());

    // With the desktop's instance id, the phone does not cut the desktop.
    let mut p = Phone::connect(port, &me, "d1", &hub, &instance).unwrap();
    assert!(p.call("panes", json!({})).unwrap().is_ok());
    assert!(p.call("pane_open", json!({})).unwrap().unwrap_err().contains("only on the computer"));
    assert!(p.call("pane_input", json!({})).unwrap().unwrap_err().contains("may not"));
    assert!(p.call("review_open", json!({ "url": "https://evil.example/o/r/pull/1" })).unwrap().unwrap_err().contains("arguments"));
    assert!(db.device("d1").unwrap().unwrap().last_seen.is_some());

    // The desktop opens and shows a pane: the phone gets the pane event and no bytes.
    let pane = open_shell(&desk, &home);
    desk.call("pane_expand", json!({ "id": pane })).unwrap();
    desk.call("pane_input", json!({ "id": pane, "data": "echo se''cret\r", "binary": false })).unwrap();
    wait_for("the desktop sees the bytes", 10, || desk_got.output_of(&pane).contains("secret"));
    assert!(p.call("panes", json!({})).unwrap().is_ok());
    assert!(p.events.iter().any(|e| e["event"] == "panes" || e["event"] == "pane"), "{:?}", p.events);
    assert!(!p.events.iter().any(|e| e["event"] == "pane_output"), "{:?}", p.events);

    // A class that goes away on the computer holds at the next command.
    assert!(db.device_classes_set("d1", scope_check::REVIEW).unwrap());
    assert!(p.call("panes", json!({})).unwrap().unwrap_err().contains("may not"));

    // After a remove, the open session closes with no command from the phone, and no new one starts.
    assert!(home.cmd(&["devices", "remove", "d1"]).status().unwrap().success());
    let end = Instant::now() + Duration::from_secs(4);
    let err = loop {
        assert!(Instant::now() < end, "the session stayed open");
        if let Err(e) = p.recv() {
            break e;
        }
    };
    assert!(!matches!(err.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut), "{err}");
    assert!(Phone::connect(port, &me, "d1", &hub, "y").is_err());
    assert!(desk.is_up() && desk.call("panes", json!({})).is_ok());
}

#[test]
fn a_device_that_gives_two_lines_in_one_frame_is_cut() {
    use switchboard_core::{paths::Paths, store};
    let home = Home::new("device2");
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let paths = Paths::under(home.root.clone());
    std::fs::write(&paths.config, format!("[devices]\nlisten = \"127.0.0.1:{port}\"\n")).unwrap();
    let (_desk, _got) = connect(&home);
    let hub = switchboard_core::devices::hub_key(&paths).unwrap().public;
    let me = session::keypair().unwrap();
    let db = store::Store::open(&paths.store()).unwrap();
    let d = store::Device {
        id: "d1".into(),
        name: "phone".into(),
        platform: String::new(),
        session_key: me.public.clone(),
        push_key: None,
        classes: scope_check::PHONE,
        paired_at: 1,
        last_seen: None,
        revoked_at: None,
        generation: 0,
    };
    db.device_add(&d).unwrap();
    wait_for("the listener", 10, || std::net::TcpStream::connect(("127.0.0.1", port)).is_ok());
    let mut p = Phone::connect(port, &me, "d1", &hub, "a").unwrap();
    p.seal.send(&mut p.tcp, b"{\"id\":1,\"cmd\":\"panes\"}\n{\"id\":2,\"cmd\":\"panes\"}").unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        match p.recv() {
            Ok(v) => assert!(v.get("id").is_none(), "{v}"),
            Err(e) => {
                assert!(!matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut), "the link stayed open");
                return;
            }
        }
    }
    panic!("the link stayed open");
}

/// `sb pair` needs your terminal: an agent, or a program with no terminal, gets none.
#[test]
fn pairing_needs_a_terminal() {
    use std::os::unix::process::CommandExt;
    let home = Home::new("pair");
    let mut c = home.cmd(&["pair"]);
    c.env("CLAUDECODE", "1");
    let out = c.output().unwrap();
    assert!(!out.status.success() && String::from_utf8_lossy(&out.stderr).contains("your own terminal"));
    let mut c = home.cmd(&["pair"]);
    c.env_remove("CLAUDECODE").stdin(Stdio::null());
    // SAFETY: setsid is async-signal-safe. With a new session there is no terminal.
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let out = c.output().unwrap();
    assert!(!out.status.success() && String::from_utf8_lossy(&out.stderr).contains("needs a terminal"), "{out:?}");
}
