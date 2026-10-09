//! Findings of the model check of the permission flow (SPEC 25), on the real
//! hub. Each test failed before its fix.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use switchboard_core::hub::{Hub, OpenReq, PermitView, Sink};
use switchboard_core::lamp::Lamp;

/// The tests set process environment variables, so they run one at a time.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

struct Quiet;

impl Sink for Quiet {
    fn emit(&self, _event: &str, _payload: Value) {}
    fn output(&self, _pane: &str, _data: &[u8]) {}
}

struct Board {
    root: PathBuf,
    socket: String,
    hub: Arc<Hub>,
    pane: String,
}

impl Drop for Board {
    fn drop(&mut self) {
        switchboard_core::tmux::kill_server(&self.socket);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A hub on its own tmux server, with one shell pane that has the Claude kind.
fn board(name: &str) -> Board {
    let root = std::env::temp_dir().join(format!("sb-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("work/demo");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        assert!(Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .status()
            .unwrap()
            .success())
    };
    git(&["init", "-q", "-b", "main"]);
    git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "x",
    ]);
    let socket = format!("sb-test-{name}-{}", std::process::id());
    std::env::set_var("SB_HOME", &root);
    std::env::set_var("SB_TMUX_SOCKET", &socket);
    std::env::remove_var("SB_SOCK");
    let hub = Hub::start(
        switchboard_core::paths::Paths::from_env(),
        PathBuf::from(env!("CARGO_BIN_EXE_sb")),
        Arc::new(Quiet),
    )
    .unwrap();
    let v = hub
        .open(OpenReq {
            repo: repo.to_string_lossy().into(),
            place: "main".into(),
            branch: None,
            tree: None,
            run: "shell".into(),
            title: None,
        })
        .unwrap();
    hub.debug_set_kind(&v.id, "claude").unwrap();
    Board {
        root,
        socket,
        hub,
        pane: v.id,
    }
}

impl Board {
    /// Starts `sb permit` for the pane, as the `PermissionRequest` hook does.
    fn ask(&self, command: &str) -> JoinHandle<std::process::Output> {
        let (root, pane) = (self.root.clone(), self.pane.clone());
        let input = json!({"tool_name":"Bash","tool_input":{"command":command}});
        std::thread::spawn(move || {
            let mut c = Command::new(env!("CARGO_BIN_EXE_sb"))
                .arg("permit")
                .env("SB_HOME", &root)
                .env("SB_PANE", &pane)
                .env_remove("SB_SOCK")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            c.stdin
                .take()
                .unwrap()
                .write_all(input.to_string().as_bytes())
                .unwrap();
            c.wait_with_output().unwrap()
        })
    }

    fn permit(&self) -> Option<PermitView> {
        self.hub
            .views()
            .into_iter()
            .find(|p| p.id == self.pane)
            .and_then(|p| p.permit)
    }

    fn lamp(&self) -> Lamp {
        self.hub
            .views()
            .into_iter()
            .find(|p| p.id == self.pane)
            .unwrap()
            .lamp
    }

    /// Ends the process in the pane with tmux, not with `Hub::input`, which
    /// clears the permits.
    fn end_process(&self) {
        let out = Command::new("tmux")
            .args(["-L", &self.socket, "list-panes", "-a", "-F"])
            .arg("#{pane_id} #{@sb_pane}")
            .output()
            .unwrap();
        let list = String::from_utf8_lossy(&out.stdout);
        let tid = list
            .lines()
            .find_map(|l| l.strip_suffix(&format!(" {}", self.pane)))
            .unwrap()
            .to_owned();
        assert!(Command::new("tmux")
            .args(["-L", &self.socket, "send-keys", "-t", &tid, "exit", "Enter"])
            .status()
            .unwrap()
            .success());
    }
}

fn wait_for(what: &str, f: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out: {what}");
}

fn decision(out: &std::process::Output) -> Option<String> {
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    v["hookSpecificOutput"]["decision"]["behavior"]
        .as_str()
        .map(str::to_owned)
}

/// F2: Claude exits while a request waits. The request is gone, so the tile
/// must lose its Allow and Deny buttons.
#[test]
fn buttons_go_when_the_process_ends() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let b = board("f2");
    let asker = b.ask("cargo test");
    wait_for("permit on the tile", || b.permit().is_some());
    b.end_process();
    wait_for("ended lamp", || b.lamp() == Lamp::Ended);
    let left = b.permit();
    // Clean up first: a close answers the waiting `sb permit`.
    b.hub.close(&b.pane).unwrap();
    asker.join().unwrap();
    assert_eq!(left, None, "the ended tile still shows Allow and Deny");
}

/// F6: two requests of one pane wait at the same time (an old one whose
/// process is gone, and a new one). A click on the old buttons must not take
/// the buttons of the new request.
#[test]
fn an_old_click_keeps_the_new_buttons() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let b = board("f6");
    let first = b.ask("ls");
    wait_for("first permit", || b.permit().is_some());
    let old = b.permit().unwrap();
    let second = b.ask("rm -rf target");
    wait_for("second permit", || {
        b.permit().map(|p| p.id != old.id).unwrap_or(false)
    });
    let new = b.permit().unwrap();

    b.hub.permit_answer(&old.id, true).unwrap();
    assert_eq!(decision(&first.join().unwrap()).as_deref(), Some("allow"));
    let left = b.permit();
    let lamp = b.lamp();

    // Clean up: answer the second request so that its `sb permit` exits.
    let _ = b.hub.permit_answer(&new.id, false);
    assert_eq!(decision(&second.join().unwrap()).as_deref(), Some("deny"));
    assert_eq!(
        left,
        Some(new),
        "the click on the old request took the new buttons"
    );
    assert_eq!(lamp, Lamp::Needs);
}

/// F1: a `working` line that the hub reads while `on_permit` runs. When both
/// are done, a request that still waits has its buttons on the tile.
#[test]
fn a_waiting_request_keeps_its_buttons() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let b = board("f1");
    let sock = switchboard_core::paths::Paths::from_env().sock();
    let line = |req: Value| {
        let mut s = UnixStream::connect(&sock).unwrap();
        writeln!(s, "{req}").unwrap();
        s
    };
    let working = || json!({"kind":"state","v":1,"pane":b.pane,"event":"working","time":0});
    for round in 0..200 {
        let s = line(json!({"kind":"permit","pane":b.pane,"tool":"Bash","input":{"command":"ls"}}));
        drop(line(working()));
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut reply = String::new();
            let _ = BufReader::new(s).read_line(&mut reply);
            let _ = tx.send(reply);
        });
        std::thread::sleep(Duration::from_millis(100));
        let waits = rx.try_recv().is_err();
        let shown = b.permit().is_some();
        // Clean up: one more line ends a request that still waits.
        drop(line(working()));
        reader.join().unwrap();
        assert!(
            !waits || shown,
            "round {round}: the request waits with no buttons"
        );
    }
}

/// The guard of P2 on the real hub: a click with the id of the first request
/// answers only the first `sb permit`, and never the second.
#[test]
fn a_click_answers_only_its_own_request() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let b = board("p2");
    let first = b.ask("ls");
    wait_for("first permit", || b.permit().is_some());
    let old = b.permit().unwrap();
    let second = b.ask("rm -rf target");
    wait_for("second permit", || {
        b.permit().map(|p| p.id != old.id).unwrap_or(false)
    });
    let new = b.permit().unwrap();

    b.hub.permit_answer(&old.id, true).unwrap();
    assert_eq!(decision(&first.join().unwrap()).as_deref(), Some("allow"));
    assert!(!second.is_finished());
    // The same id again: the request is gone.
    assert!(b.hub.permit_answer(&old.id, true).is_err());
    b.hub.permit_answer(&new.id, false).unwrap();
    assert_eq!(decision(&second.join().unwrap()).as_deref(), Some("deny"));
}
