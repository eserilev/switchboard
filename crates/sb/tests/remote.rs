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

fn connect(home: &Home) -> (Arc<Client>, Arc<Got>) {
    let got = Arc::new(Got::default());
    let g = Arc::clone(&got);
    let client = Client::connect(&home.argv(), move |n| match n {
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

    // A second window takes over: the first one hears so and goes down.
    let (b, got_b) = connect(&home);
    wait_for("the first window goes down", 5, || got_a.down.lock().unwrap().is_some());
    let (why, taken) = got_a.down.lock().unwrap().clone().unwrap();
    assert!(taken, "{why}");
    assert!(a.call("panes", json!({})).is_err());
    // The new window gets the panes, and its own screen.
    let panes = b.call("panes", json!({})).unwrap();
    assert!(panes.as_array().unwrap().iter().any(|p| p["id"] == id));
    b.call("pane_expand", json!({ "id": id })).unwrap();
    b.call("pane_input", json!({ "id": id, "data": "echo two-$((1+1))\r", "binary": false })).unwrap();
    wait_for("output in the new window", 5, || got_b.output_of(&id).contains("two-2"));
    assert!(!got_a.output_of(&id).contains("two-2"));

    // The link drops in the middle: the hub and the pane keep running.
    b.close();
    wait_for("the window is down", 5, || got_b.down.lock().unwrap().is_some());
    std::thread::sleep(Duration::from_millis(300));
    let (c, _got_c) = connect(&home);
    let panes = c.call("panes", json!({})).unwrap();
    assert!(panes.as_array().unwrap().iter().any(|p| p["id"] == id), "the shell still runs");
    assert!(!panes.as_array().unwrap().iter().any(|p| p["live"] == true), "a new window starts with no screen");
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
