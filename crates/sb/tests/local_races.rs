//! The races of the local review load cycle on a real hub (SPEC 26.1): a second
//! Retry while a load with no head runs, the head ref and New changes after a
//! quit, and the start sweep of the refs that a quit during a close left.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use switchboard_core::hub::{Hub, OpenReq, Sink};
use switchboard_core::store::ReviewRow;

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

/// The commit a ref names, or "" with no such ref.
fn at(dir: &Path, r: &str) -> String {
    g(dir, &["for-each-ref", "--format=%(objectname)", r])
}

#[test]
fn local_load_races() {
    let root = std::env::temp_dir().join(format!("sb-races-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let up = root.join("up");
    std::fs::create_dir_all(&up).unwrap();
    g(&up, &["init", "-q", "-b", "main"]);
    commit(&up, "a.rs", "fn a() {}\n");
    g(&root, &["clone", "-q", &up.to_string_lossy(), "demo"]);
    let clone = root.join("demo");
    let wt = root.join("demo-agent");
    g(&clone, &["worktree", "add", "-q", "--detach", &wt.to_string_lossy()]);
    let head = commit(&wt, "b.rs", "fn b() {}\n");

    std::env::set_var("SB_HOME", &root);
    std::env::set_var("SB_TMUX_SOCKET", format!("sb-races-{}", std::process::id()));
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
    let ready = |id: &str| hub.review_view(id).map(|v| v.status == "ready").unwrap_or(false);
    let head_ref = |id: &str| at(&clone, &format!("refs/sb/review/{id}/head"));

    // A first load claims the review: a Retry during it is refused.
    let id = hub.review_open_local(&pane).unwrap();
    let err = hub.review_retry(&id).unwrap_err();
    assert!(err.contains("still loading"), "{err}");
    wait_for("the commits load", || ready(&id));
    let shown = hub.review_view(&id).unwrap().head;
    assert_eq!(head_ref(&id), shown);

    // A quit during a load left the head ref on another commit than the saved row:
    // the start points it back at the view.
    g(&clone, &["update-ref", &format!("refs/sb/review/{id}/head"), &head]);
    std::fs::write(wt.join("c.rs"), "fn c() {}\n").unwrap();
    hub.debug_reload_reviews();
    assert_eq!(head_ref(&id), shown, "the ref keeps the snapshot that the view shows");
    // New changes is in memory only: the start looks once, and the new file shows it.
    wait_for("New changes after a start", || hub.review_view(&id).unwrap().new_head.is_some());

    // A local review whose first load never ended: after a start it has no head
    // and says Retry. Two Retry clicks start one load.
    hub.debug_add_review(ReviewRow {
        id: "lx".into(),
        url: String::new(),
        repo: String::new(),
        number: 0,
        title: String::new(),
        head: String::new(),
        base: String::new(),
        tree: wt.to_string_lossy().into(),
        connection: None,
        guide_session: None,
        guide: None,
        round: 1,
        since: None,
        scope_note: None,
        kind: "local".into(),
        pane: Some(pane.clone()),
    });
    hub.debug_reload_reviews();
    assert_eq!(hub.review_view("lx").unwrap().status, "error");
    hub.review_retry("lx").unwrap();
    for again in [hub.review_retry("lx"), hub.review_update("lx"), hub.review_restart("lx"), hub.review_range("lx", "all")] {
        let err = again.unwrap_err();
        assert!(err.contains("still loading"), "{err}");
    }
    wait_for("the Retry loads", || ready("lx"));
    assert_eq!(head_ref("lx"), hub.review_view("lx").unwrap().head);

    // A quit between the store write of a close and the ref drop leaves the refs.
    // The start sweep drops them, and only them: not the refs of an open review,
    // not a ref of an id the store does not know, and nothing of the agent.
    assert!(hub.review_close(&id, false, None).unwrap().closed);
    assert_eq!(head_ref(&id), "");
    g(&clone, &["update-ref", &format!("refs/sb/review/{id}/head"), &head]);
    g(&clone, &["update-ref", &format!("refs/sb/review/{id}/reviewed"), &head]);
    g(&clone, &["update-ref", "refs/sb/review/zz9/head", &head]);
    let open_ref = head_ref("lx");
    let status = g(&wt, &["status", "--porcelain"]);
    hub.debug_sweep();
    assert_eq!(g(&clone, &["for-each-ref", &format!("refs/sb/review/{id}/")]), "", "the refs of the closed review go");
    assert_eq!(head_ref("lx"), open_ref, "the ref of an open review stays");
    assert_eq!(at(&clone, "refs/sb/review/zz9/head"), head, "a ref of an unknown id stays");
    assert!(wt.exists());
    assert_eq!(g(&wt, &["rev-parse", "HEAD"]), head);
    assert_eq!(g(&wt, &["status", "--porcelain"]), status, "the agent's work stays");
}
