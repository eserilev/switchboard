//! A review of an agent's uncommitted work on a real hub (SPEC 11.14): snapshots,
//! the New and All views, Mark reviewed, New changes after a turn, the poll, and
//! a close that drops the refs of the review and nothing of the agent.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use switchboard_core::hub::{Hub, OpenReq, Sink};

#[derive(Default)]
struct Events(Mutex<Vec<(String, Value)>>);

impl Sink for Events {
    fn emit(&self, event: &str, payload: Value) {
        self.0.lock().unwrap().push((event.to_owned(), payload));
    }
    fn output(&self, _pane: &str, _data: &[u8]) {}
}

fn g(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn commit(dir: &Path, file: &str, text: &str) -> String {
    std::fs::write(dir.join(file), text).unwrap();
    g(dir, &["add", "-A"]);
    g(dir, &["commit", "-q", "-m", file]);
    g(dir, &["rev-parse", "HEAD"])
}

fn wait_for(what: &str, f: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out: {what}");
}

#[test]
fn review_uncommitted_work_in_steps() {
    let root = std::env::temp_dir().join(format!("sb-work-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    // An origin, a clone, and the agent's detached worktree with one commit and
    // uncommitted work: a change and a new file.
    let up = root.join("up");
    std::fs::create_dir_all(&up).unwrap();
    g(&up, &["init", "-q", "-b", "main"]);
    let base = commit(&up, "a.rs", "fn a() {}\n");
    g(&root, &["clone", "-q", &up.to_string_lossy(), "demo"]);
    let clone = root.join("demo");
    let wt = root.join("demo-agent");
    g(&clone, &["worktree", "add", "-q", "--detach", &wt.to_string_lossy()]);
    let head = commit(&wt, "b.rs", "fn b() {}\n");
    std::fs::write(wt.join("a.rs"), "fn a() {}\nfn a2() {}\n").unwrap();
    std::fs::write(wt.join("c.rs"), "fn c() {}\n").unwrap();
    let status = g(&wt, &["status", "--porcelain"]);

    std::env::set_var("SB_HOME", &root);
    std::env::set_var("SB_TMUX_SOCKET", format!("sb-work-{}", std::process::id()));
    std::env::remove_var("SB_SOCK");
    let hub = Hub::start(switchboard_core::paths::Paths::from_env(), PathBuf::from(env!("CARGO_BIN_EXE_sb")), Arc::new(Events::default())).unwrap();
    struct Kill(String, PathBuf);
    impl Drop for Kill {
        fn drop(&mut self) {
            switchboard_core::tmux::kill_server(&self.0);
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }
    let _kill = Kill(std::env::var("SB_TMUX_SOCKET").unwrap(), root.clone());
    let pane = hub
        .open(OpenReq {
            repo: clone.to_string_lossy().into(),
            place: "tree".into(),
            branch: None,
            tree: Some(wt.to_string_lossy().into()),
            run: "shell".into(),
            title: Some("agent".into()),
            task: None,
            first: None,
        })
        .unwrap()
        .id;
    hub.debug_set_kind(&pane, "claude").unwrap();
    let files = |id: &str| -> Vec<String> { hub.review_files(id).unwrap().into_iter().map(|f| f.path).collect() };
    let ready = |id: &str| hub.review_view(id).map(|v| v.status == "ready").unwrap_or(false);

    // Open: the commit and the uncommitted work, as one view on a snapshot.
    let id = hub.review_open_local(&pane).unwrap();
    wait_for("the work loads", || ready(&id));
    let v = hub.review_view(&id).unwrap();
    assert_eq!((v.base.as_str(), v.range.as_str(), v.reviewed), (base.as_str(), "new", false));
    assert_ne!(v.head, head, "the head is a snapshot of the work tree");
    assert!(v.title.starts_with("1 commit and uncommitted work on a detached HEAD"), "{}", v.title);
    assert_eq!(files(&id), ["a.rs", "b.rs", "c.rs"]);
    assert_eq!(g(&wt, &["status", "--porcelain"]), status, "the snapshot changed the agent's files or index");
    assert_eq!(g(&wt, &["rev-parse", "HEAD"]), head);

    // The poll compares HEAD with the HEAD the snapshot was made on: no false alarm.
    hub.debug_tick(30);
    assert_eq!(hub.review_view(&id).unwrap().new_head, None);

    // Mark reviewed: New is empty until the agent changes more.
    hub.review_mark_reviewed(&id).unwrap();
    wait_for("the empty view loads", || ready(&id) && hub.review_view(&id).unwrap().reviewed);
    let v = hub.review_view(&id).unwrap();
    assert!(v.title.starts_with("Since you marked reviewed"), "{}", v.title);
    assert!(files(&id).is_empty(), "{:?}", files(&id));

    // The agent's turn ends with a new file: the review shows New changes.
    std::fs::write(wt.join("d.rs"), "fn d() {}\n").unwrap();
    hub.on_state(&switchboard_core::proto::StateMsg {
        v: switchboard_core::proto::VERSION,
        pane: pane.clone(),
        event: "turn".into(),
        session: None,
        cwd: None,
        time: 1,
        summary: None,
        detail: Value::Null,
    });
    wait_for("New changes after the turn", || hub.review_view(&id).unwrap().new_head.is_some());
    hub.review_update(&id).unwrap();
    wait_for("the new work loads", || ready(&id) && hub.review_view(&id).unwrap().new_head.is_none());
    assert_eq!(files(&id), ["d.rs"], "only the change since Mark reviewed");

    // All: everything since the base again.
    hub.review_range(&id, "all").unwrap();
    wait_for("All loads", || ready(&id) && hub.review_view(&id).unwrap().range == "all");
    assert_eq!(files(&id), ["a.rs", "b.rs", "c.rs", "d.rs"]);

    // The agent commits: the poll sees a new HEAD.
    g(&wt, &["add", "-A"]);
    g(&wt, &["commit", "-q", "-m", "work"]);
    hub.debug_tick(60);
    assert!(hub.review_view(&id).unwrap().new_head.is_some(), "a commit is new");

    // The refs are the review's own, under refs/sb, and go with it.
    let refs = g(&clone, &["for-each-ref", "--format=%(refname)", "refs/sb"]);
    assert_eq!(refs.lines().count(), 2, "{refs}");
    assert!(hub.review_close(&id, false, None).unwrap().closed);
    assert_eq!(g(&clone, &["for-each-ref", "refs/sb"]), "");
    assert!(g(&clone, &["for-each-ref", "refs/switchboard"]).is_empty(), "no pin for a local review");
    assert!(wt.join("d.rs").exists(), "the agent's work stays");
    assert!(!switchboard_core::review::local_changes(&clone, &wt).unwrap().is_empty());
}
