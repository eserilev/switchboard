//! The base of a review of local work on a real hub (SPEC 11.11): a chosen base,
//! the refusals, the choice kept over a restart and shared by every worktree of the
//! repo, back to the usual rule, and Update base.

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

fn commit(dir: &Path, file: &str) -> String {
    std::fs::write(dir.join(file), file).unwrap();
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
fn a_chosen_base_is_checked_kept_and_shared_by_the_worktrees() {
    let root = std::env::temp_dir().join(format!("sb-base-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    // An origin, a clone on `feat` (2 commits on origin/main), and a detached worktree.
    let up = root.join("up");
    std::fs::create_dir_all(&up).unwrap();
    g(&up, &["init", "-q", "-b", "main"]);
    let a = commit(&up, "a.rs");
    g(&root, &["clone", "-q", &up.to_string_lossy(), "demo"]);
    let clone = root.join("demo");
    g(&clone, &["checkout", "-q", "-b", "feat", "--track", "origin/main"]);
    let b = commit(&clone, "b.rs");
    commit(&clone, "c.rs");
    let wt = root.join("demo-agent");
    g(&clone, &["worktree", "add", "-q", "--detach", &wt.to_string_lossy()]);

    std::env::set_var("SHELL", "/bin/sh");
    std::env::set_var("SB_HOME", &root);
    std::env::set_var("SB_TMUX_SOCKET", format!("sb-base-{}", std::process::id()));
    std::env::remove_var("SB_SOCK");
    let hub = Hub::start(
        switchboard_core::paths::Paths::from_env(),
        PathBuf::from(env!("CARGO_BIN_EXE_sb")),
        Arc::new(Events::default()),
    )
    .unwrap();
    struct Kill(String, PathBuf);
    impl Drop for Kill {
        fn drop(&mut self) {
            switchboard_core::tmux::kill_server(&self.0);
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }
    let _kill = Kill(std::env::var("SB_TMUX_SOCKET").unwrap(), root.clone());
    let open = |tree: &Path| {
        hub.open(OpenReq {
            repo: clone.to_string_lossy().into(),
            place: "tree".into(),
            branch: None,
            tree: Some(tree.to_string_lossy().into()),
            run: "shell".into(),
            title: Some("agent".into()),
            task: None,
            first: None,
        })
        .unwrap()
        .id
    };
    let ready = |id: &str| hub.review_view(id).map(|v| v.status == "ready").unwrap_or(false);

    // The usual rule: the upstream of `feat`.
    let p1 = open(&clone);
    let id = hub.review_open_local(&p1).unwrap();
    wait_for("the commits load", || ready(&id));
    let v = hub.review_view(&id).unwrap();
    assert_eq!((v.base.as_str(), v.base_ref.as_deref(), v.base_pick.as_deref()), (a.as_str(), Some("origin/main"), None));
    assert!(v.bases.iter().any(|o| o.name == "origin/main"), "{:?}", v.bases);

    // Refusals: an option, a ref that is not here, a PR review is not asked.
    assert!(hub.review_base(&id, "-x").unwrap_err().contains("never starts with -"));
    assert!(hub.review_base(&id, "--output=/tmp/x").is_err());
    assert!(hub.review_base(&id, "nope").unwrap_err().contains("not a branch or a commit"));
    assert!(hub.review_base("r999", "main").is_err());
    assert_eq!(hub.review_view(&id).unwrap().base_ref.as_deref(), Some("origin/main"));

    // A chosen base goes first, and survives a restart.
    hub.review_base(&id, "feat~1").unwrap();
    assert!(hub.review_base(&id, "main").unwrap_err().contains("still loading"));
    wait_for("the chosen base loads", || ready(&id));
    let v = hub.review_view(&id).unwrap();
    assert_eq!((v.base.as_str(), v.base_pick.as_deref()), (b.as_str(), Some("feat~1")));
    assert_eq!(v.base_label.as_deref(), Some("feat~1 (your choice)"));
    hub.debug_reload_reviews();
    hub.review_update(&id).unwrap();
    wait_for("the reload loads", || ready(&id));
    assert_eq!(hub.review_view(&id).unwrap().base, b);

    // Another worktree of the repo uses the same choice.
    let p2 = open(&wt);
    let id2 = hub.review_open_local(&p2).unwrap();
    assert_ne!(id2, id);
    wait_for("the worktree loads", || ready(&id2));
    let v2 = hub.review_view(&id2).unwrap();
    assert_eq!((v2.base.as_str(), v2.base_pick.as_deref()), (b.as_str(), Some("feat~1")));

    // Back to the usual rule. The first review keeps its own choice until it clears it too.
    hub.review_base(&id2, "").unwrap();
    wait_for("the usual rule loads", || ready(&id2));
    let v2 = hub.review_view(&id2).unwrap();
    assert_eq!(v2.base_pick, None);
    assert_ne!(v2.base, b);
    hub.review_base(&id, " ").unwrap();
    wait_for("the usual rule loads", || ready(&id));
    assert_eq!(hub.review_view(&id).unwrap().base, a);

    // Update base fetches only the base branch.
    let moved = commit(&up, "d.rs");
    hub.review_base_update(&id).unwrap();
    wait_for("Update base loads", || ready(&id));
    assert_eq!(g(&clone, &["rev-parse", "origin/main"]), moved);
    let v = hub.review_view(&id).unwrap();
    assert!(v.base_fetched.is_some_and(|t| t + 60 > switchboard_core::now()), "{:?}", v.base_fetched);
    assert!(!v.scope_note.unwrap_or_default().contains("failed"));
}
