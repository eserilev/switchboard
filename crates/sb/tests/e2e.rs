//! The hub and `sb` together: a real tmux server, the socket, hooks, permits.

use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use switchboard_core::hub::{Hub, OpenReq, Sink};
use switchboard_core::lamp::Lamp;

#[derive(Default)]
struct Events(Mutex<Vec<(String, Value)>>);

impl Sink for Events {
    fn emit(&self, event: &str, payload: Value) {
        self.0.lock().unwrap().push((event.to_owned(), payload));
    }
    fn output(&self, _pane: &str, _data: &[u8]) {}
}

fn sb(root: &Path, pane: &str, args: &[&str], stdin: Value) -> std::process::Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_sb"))
        .args(args)
        .env("SB_HOME", root)
        .env("SB_PANE", pane)
        .env_remove("SB_SOCK")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin
        .take()
        .unwrap()
        .write_all(stdin.to_string().as_bytes())
        .unwrap();
    c.wait_with_output().unwrap()
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

#[test]
fn hooks_permits_and_panes_end_to_end() {
    let root = std::env::temp_dir().join(format!("sb-e2e-{}", std::process::id()));
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

    // Every path of this hub lives under `root`, and tmux runs on its own socket.
    std::env::set_var("SB_HOME", &root);
    std::env::set_var("SB_TMUX_SOCKET", format!("sb-e2e-{}", std::process::id()));
    std::env::remove_var("SB_SOCK");
    // nvim with an empty config: a user config can stop at a "Press ENTER" prompt.
    std::env::set_var(
        "NVIM_APPNAME",
        format!("sb-test-nvim-{}", std::process::id()),
    );
    let events = Arc::new(Events::default());
    let hub = Hub::start(
        switchboard_core::paths::Paths::from_env(),
        PathBuf::from(env!("CARGO_BIN_EXE_sb")),
        events.clone(),
    )
    .unwrap();
    struct Kill(String);
    impl Drop for Kill {
        fn drop(&mut self) {
            switchboard_core::tmux::kill_server(&self.0);
        }
    }
    let _kill = Kill(std::env::var("SB_TMUX_SOCKET").unwrap());

    // The first window of the session is adopted as a shell.
    assert_eq!(hub.views().len(), 1);
    assert_eq!(hub.views()[0].kind, "shell");

    let v = hub
        .open(OpenReq {
            repo: repo.to_string_lossy().into(),
            place: "main".into(),
            branch: None,
            tree: None,
            run: "shell".into(),
            title: Some("demo".into()),
        })
        .unwrap();
    let id = v.id.clone();
    assert_eq!(v.branch, "main");

    // A shell pane has no lamp: hook events do not light it.
    sb(
        &root,
        &id,
        &["state", "turn"],
        json!({"last_assistant_message":"x"}),
    );
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        hub.views().iter().find(|p| p.id == id).unwrap().lamp,
        Lamp::None
    );

    // A new worktree, run as a Claude-kind pane but with no claude: test the lamps through sb.
    let wt = hub
        .open(OpenReq {
            repo: repo.to_string_lossy().into(),
            place: "new".into(),
            branch: Some("feat".into()),
            tree: None,
            run: "shell".into(),
            title: None,
        })
        .unwrap();
    assert!(wt.tree.ends_with("demo-feat"), "{}", wt.tree);
    assert_eq!(wt.branch, "feat");

    // Make a lamp pane by hand: the store row kind decides the lamp rules.
    let claude_id = {
        let v = hub.views();
        v.iter().find(|p| p.id == wt.id).unwrap().id.clone()
    };
    hub.debug_set_kind(&claude_id, "claude").unwrap();

    let out = sb(
        &root,
        &claude_id,
        &["state", "turn"],
        json!({"session_id":"s-1","last_assistant_message":"\nMoved links to SQLite.\n"}),
    );
    assert!(out.status.success());
    wait_for("turn lamp", || {
        hub.views()
            .iter()
            .any(|p| p.id == claude_id && p.lamp == Lamp::Turn && p.unseen)
    });
    let p = hub.views().into_iter().find(|p| p.id == claude_id).unwrap();
    assert_eq!(p.summary.as_deref(), Some("Moved links to SQLite."));
    assert_eq!(p.session.as_deref(), Some("s-1"));
    assert!(root
        .join("home/state")
        .join(format!("{claude_id}.json"))
        .exists());
    assert!(events
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|(e, v)| e == "pane" && v["alert"] == true));

    // Typing clears unseen.
    hub.input(&claude_id, b"").unwrap();
    hub.seen(&claude_id).unwrap();
    assert!(
        !hub.views()
            .iter()
            .find(|p| p.id == claude_id)
            .unwrap()
            .unseen
    );

    // A permission request waits for the tile.
    let r2 = root.clone();
    let cid = claude_id.clone();
    let asker = std::thread::spawn(move || {
        sb(
            &r2,
            &cid,
            &["permit"],
            json!({"tool_name":"Bash","tool_input":{"command":"cargo test"}}),
        )
    });
    wait_for("permit on the tile", || {
        hub.views()
            .iter()
            .any(|p| p.id == claude_id && p.permit.is_some())
    });
    let p = hub.views().into_iter().find(|p| p.id == claude_id).unwrap();
    assert_eq!(
        (p.lamp, p.summary.as_deref()),
        (Lamp::Needs, Some("Bash: cargo test"))
    );
    hub.permit_answer(&p.permit.unwrap().id, true).unwrap();
    let out = asker.join().unwrap();
    let decision: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        decision["hookSpecificOutput"]["decision"]["behavior"],
        "allow"
    );

    // A request answered in the pane: the next hook event ends it with no decision.
    let r3 = root.clone();
    let cid = claude_id.clone();
    let asker = std::thread::spawn(move || {
        sb(
            &r3,
            &cid,
            &["permit"],
            json!({"tool_name":"Edit","tool_input":{"file_path":"/a.rs"}}),
        )
    });
    wait_for("second permit", || {
        hub.views()
            .iter()
            .any(|p| p.id == claude_id && p.permit.is_some())
    });
    sb(&root, &claude_id, &["state", "working"], json!({}));
    let out = asker.join().unwrap();
    assert!(
        out.stdout.is_empty(),
        "no decision: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    wait_for("permit cleared", || {
        hub.views()
            .iter()
            .any(|p| p.id == claude_id && p.permit.is_none() && p.lamp == Lamp::Working)
    });

    // A key typed in the pane answers there: the tile buttons go, with no decision.
    let r4 = root.clone();
    let cid = claude_id.clone();
    let asker = std::thread::spawn(move || {
        sb(
            &r4,
            &cid,
            &["permit"],
            json!({"tool_name":"Bash","tool_input":{"command":"ls"}}),
        )
    });
    wait_for("third permit", || {
        hub.views()
            .iter()
            .any(|p| p.id == claude_id && p.permit.is_some())
    });
    hub.input(&claude_id, b"1").unwrap();
    let out = asker.join().unwrap();
    assert!(out.stdout.is_empty());
    assert!(hub
        .views()
        .iter()
        .any(|p| p.id == claude_id && p.permit.is_none()));

    // A rate limit sets Limit and marks the connection.
    sb(
        &root,
        &claude_id,
        &["state", "failure"],
        json!({"error":"rate_limit","last_assistant_message":"API Error: Rate limit reached"}),
    );
    wait_for("limit lamp", || {
        hub.views()
            .iter()
            .any(|p| p.id == claude_id && p.lamp == Lamp::Limit)
    });
    assert!(hub.connection_view().list[0].limit_until.is_some());

    // Expand gives the screen and makes the pane live; collapse pauses it again.
    let screen = hub.expand(&id).unwrap();
    assert!(screen.starts_with(b"\x1b[H"));
    assert!(hub.views().iter().find(|p| p.id == id).unwrap().live);
    hub.collapse();
    assert!(!hub.views().iter().any(|p| p.live));

    // Closing removes the pane. A shell that exits removes itself.
    hub.close(&id).unwrap();
    assert!(!hub.views().iter().any(|p| p.id == id));
    let sh = hub
        .open(OpenReq {
            repo: repo.to_string_lossy().into(),
            place: "main".into(),
            branch: None,
            tree: None,
            run: "shell".into(),
            title: None,
        })
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    hub.input(&sh.id, b"exit\r").unwrap();
    wait_for("shell exit removes the tile", || {
        !hub.views().iter().any(|p| p.id == sh.id)
    });
    // A Claude pane that crashes shows Error and alerts once, not every tick.
    let alerts = |id: &str| {
        events
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(e, v)| e == "pane" && v["id"] == id && v["alert"] == true)
            .count()
    };
    let before = alerts(&claude_id);
    hub.input(&claude_id, b"\x15exit 3\r").unwrap();
    wait_for("crash shows Error", || {
        hub.views()
            .iter()
            .any(|p| p.id == claude_id && p.lamp == Lamp::Error && p.unseen)
    });
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(alerts(&claude_id) - before, 1, "one alert for one crash");
    assert_eq!(
        hub.views()
            .iter()
            .find(|p| p.id == claude_id)
            .unwrap()
            .summary
            .as_deref(),
        Some("Exited with 3")
    );

    // nvim: one per worktree, opened at a line, then reused.
    if switchboard_core::nvim::installed() {
        // The tree of the Claude-kind pane, so its edit event reaches this nvim.
        let tree = PathBuf::from(&wt.tree);
        let file = tree.join("notes.txt");
        std::fs::write(&file, "1\n2\n3\n4\n5\n").unwrap();
        let f = file.to_string_lossy().into_owned();
        let n1 = hub.nvim_open(&tree, Some((&f, 2))).unwrap();
        let sock = switchboard_core::nvim::socket(&root.join("run/nvim"), &tree);
        wait_for("nvim socket", || switchboard_core::nvim::alive(&sock));
        let n2 = hub.nvim_open(&tree, Some((&f, 4))).unwrap();
        assert_eq!(n1, n2, "the second open reuses the nvim");
        assert_eq!(
            switchboard_core::nvim::expr(&sock, "line('.')").unwrap(),
            "4"
        );
        // An agent edit in the same tree reloads the buffer.
        std::fs::write(&file, "changed\n").unwrap();
        sb(
            &root,
            &claude_id,
            &["state", "edited"],
            json!({"tool_input":{"file_path": f}}),
        );
        wait_for("checktime reload", || {
            switchboard_core::nvim::expr(&sock, "getline(1)")
                .map(|l| l == "changed")
                .unwrap_or(false)
        });
        assert_eq!(
            hub.views().iter().find(|p| p.id == n1).unwrap().kind,
            "nvim"
        );
        hub.close(&n1).unwrap();
    }

    // Layouts.
    hub.layout_save("day").unwrap();
    assert_eq!(hub.layouts(), vec!["day"]);

    // Close every tile. The hidden keeper pane keeps the tmux session, so a new pane still opens.
    for v in hub.views() {
        hub.close(&v.id).unwrap();
    }
    assert!(hub.views().is_empty());
    std::thread::sleep(Duration::from_millis(300));
    let again = hub
        .open(OpenReq {
            repo: repo.to_string_lossy().into(),
            place: "main".into(),
            branch: None,
            tree: None,
            run: "shell".into(),
            title: None,
        })
        .unwrap();
    assert!(hub.views().iter().any(|p| p.id == again.id));

    // `sb state` with no SB_PANE does nothing and exits 0.
    let out = Command::new(env!("CARGO_BIN_EXE_sb"))
        .args(["state", "turn"])
        .env_remove("SB_PANE")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success());

    let _ = std::fs::remove_dir_all(&root);
}
