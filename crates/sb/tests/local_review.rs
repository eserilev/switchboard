//! A review of an agent's local commits on a real hub (SPEC 11.11): a tmux server,
//! an agent pane in a detached worktree, Files, a comment, Send to agent, Refresh
//! after a new commit, and a close that leaves the agent's worktree alone.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use switchboard_core::hub::{Hub, OpenReq, Sink};
use switchboard_core::hub_review::NewComment;

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
fn review_local_commits_send_to_the_agent_and_keep_its_worktree() {
    let root = std::env::temp_dir().join(format!("sb-local-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    // An origin, a clone, and the agent's detached worktree with two commits.
    let up = root.join("up");
    std::fs::create_dir_all(&up).unwrap();
    g(&up, &["init", "-q", "-b", "main"]);
    let base = commit(&up, "a.rs", "fn a() {}\n");
    g(&root, &["clone", "-q", &up.to_string_lossy(), "demo"]);
    let clone = root.join("demo");
    let wt = root.join("demo-agent");
    g(&clone, &["worktree", "add", "-q", "--detach", &wt.to_string_lossy()]);
    commit(&wt, "b.rs", "fn b() {\n    let x = 1;\n}\n");
    let head1 = commit(&wt, "a.rs", "fn a() {}\nfn a2() {}\n");

    // The agent pane: a stand-in shell that writes what it gets to a file.
    let got = root.join("typed.txt");
    let agent = root.join("agent.sh");
    std::fs::write(&agent, format!("#!/bin/sh\nstty raw -echo 2>/dev/null\nexec cat > '{}'\n", got.display())).unwrap();
    Command::new("chmod").args(["+x", &agent.to_string_lossy()]).status().unwrap();
    std::env::set_var("SHELL", &agent);
    std::env::set_var("SB_HOME", &root);
    std::env::set_var("SB_TMUX_SOCKET", format!("sb-local-{}", std::process::id()));
    std::env::remove_var("SB_SOCK");
    let events = Arc::new(Events::default());
    let hub = Hub::start(
        switchboard_core::paths::Paths::from_env(),
        PathBuf::from(env!("CARGO_BIN_EXE_sb")),
        events.clone(),
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
    let pane = hub
        .open(OpenReq {
            repo: clone.to_string_lossy().into(),
            place: "tree".into(),
            branch: None,
            tree: Some(wt.to_string_lossy().into()),
            run: "shell".into(),
            title: Some("agent".into()),
        })
        .unwrap()
        .id;

    // Open: the two unpushed commits, in Files, with no guide and no gh.
    let id = hub.review_open_local(&pane).unwrap();
    wait_for("the commits load", || hub.review_view(&id).map(|v| v.status == "ready").unwrap_or(false));
    let v = hub.review_view(&id).unwrap();
    assert_eq!((v.kind.as_str(), v.base.as_str(), v.head.as_str()), ("local", base.as_str(), head1.as_str()));
    assert!(v.title.starts_with("2 commits on a detached HEAD"), "{}", v.title);
    let files: Vec<String> = hub.review_files(&id).unwrap().into_iter().map(|f| f.path).collect();
    assert_eq!(files, vec!["a.rs", "b.rs"]);
    // The same folder opens the same review.
    assert_eq!(hub.review_open_local(&pane).unwrap(), id);
    // PR-only commands refuse.
    assert!(hub.review_round(&id).is_err());
    assert!(hub.review_post_preview(&id).is_err());
    // The review shows the agent pane itself, and never starts a Claude pane.
    assert_eq!(hub.review_term(&id, None, true).unwrap().as_deref(), Some(pane.as_str()));

    // A comment on `let x = 1;`, then Send to agent.
    hub.review_comment(
        &id,
        NewComment {
            path: "b.rs".into(),
            side: "new".into(),
            line: 2,
            start_line: None,
            text: "Name this better.".into(),
            head: head1.clone(),
            line_text: "    let x = 1;".into(),
            agent: false,
        },
    )
    .unwrap();
    assert_eq!(hub.review_send_agent(&id, "").unwrap(), 1);
    wait_for("the agent gets the paste", || {
        std::fs::read_to_string(&got).map(|t| t.contains("Name this better.")).unwrap_or(false)
    });
    let typed = std::fs::read_to_string(&got).unwrap();
    assert!(typed.starts_with("\x1b[200~Review comments on commit"), "{typed:?}");
    assert!(typed.contains("- b.rs:2: Name this better.") && typed.ends_with("\x1b[201~"), "{typed:?}");
    assert!(!typed.contains('\r'), "no Enter: you send it");
    let v = hub.review_view(&id).unwrap();
    assert!(v.drafts.is_empty() && v.posted.len() == 1 && v.posted[0].event == "AGENT");
    assert!(hub.review_send_agent(&id, "").is_err(), "nothing left to send");

    // The agent commits a fix above the line of a new comment: Refresh moves it.
    hub.review_comment(
        &id,
        NewComment {
            path: "b.rs".into(),
            side: "new".into(),
            line: 2,
            start_line: None,
            text: "Still here?".into(),
            head: head1.clone(),
            line_text: "    let x = 1;".into(),
            agent: false,
        },
    )
    .unwrap();
    let head2 = commit(&wt, "b.rs", "// moved\nfn b() {\n    let x = 1;\n}\n");
    hub.review_update(&id).unwrap();
    wait_for("the new head loads", || hub.review_view(&id).map(|v| v.head == head2 && v.status == "ready").unwrap_or(false));
    let d = &hub.review_view(&id).unwrap().drafts[0];
    assert_eq!((d.line, d.stale, d.head.as_deref()), (Some(3), false, Some(head2.as_str())));

    // Close: the agent's worktree, its commits and its pane stay. No pin is written.
    assert!(hub.review_close(&id, false, None).unwrap().closed);
    std::thread::sleep(Duration::from_millis(300));
    assert!(hub.views().iter().any(|p| p.id == pane), "the agent pane stays");
    assert!(wt.join("b.rs").exists());
    assert!(g(&clone, &["for-each-ref", "refs/switchboard"]).is_empty(), "no pin for a local review");
    // Even with the pane closed, the folder clean and detached, the start sweep keeps it.
    hub.close(&pane).unwrap();
    hub.debug_sweep();
    assert!(wt.exists(), "the sweep never removes an agent's worktree");
    assert_eq!(g(&wt, &["rev-parse", "HEAD"]), head2);
    // And its commits still count as work: no ref of ours hides them.
    assert!(!switchboard_core::review::local_changes(&clone, &wt).unwrap().is_empty());
}
