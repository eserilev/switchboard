//! The chat of each step and the take commands on a real hub (SPEC 11.5, 11.12): a
//! review, its Claude pane with a session from a hook, Claude's session file, and the
//! steps you viewed.

use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use switchboard_core::hub::{Hub, OpenReq, Sink};
use switchboard_core::store::ReviewRow;

/// An RFC 3339 UTC time, `2026-10-15T04:24:39.000Z`, made here: `date -d` is GNU only,
/// and macOS `date` gives nothing for it.
fn rfc3339(t: u64) -> String {
    let (days, secs) = (t / 86400, t % 86400);
    // The civil-from-days rule.
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z", secs / 3600, secs / 60 % 60, secs % 60)
}

#[derive(Default)]
struct Events(Mutex<Vec<(String, Value)>>);

impl Sink for Events {
    fn emit(&self, event: &str, payload: Value) {
        self.0.lock().unwrap().push((event.to_owned(), payload));
    }
    fn output(&self, _pane: &str, _data: &[u8]) {}
}

fn sb(root: &Path, pane: &str, args: &[&str], stdin: Value) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_sb"))
        .args(args)
        .env("SB_HOME", root)
        .env("SB_PANE", pane)
        .env_remove("SB_SOCK")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(stdin.to_string().as_bytes()).unwrap();
    c.wait_with_output().unwrap();
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
fn the_chat_of_each_step_and_dismissed_notes() {
    let root = std::env::temp_dir().join(format!("sb-chat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let tree = root.join("work/demo-pr7");
    std::fs::create_dir_all(&tree).unwrap();
    std::env::set_var("SB_HOME", &root);
    std::env::set_var("SB_TMUX_SOCKET", format!("sb-chat-{}", std::process::id()));
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

    hub.debug_add_review(ReviewRow {
        id: "r1".into(),
        url: "https://github.com/demo/demo/pull/7".into(),
        repo: "demo/demo".into(),
        number: 7,
        title: "PR 7".into(),
        head: "h".into(),
        base: "b".into(),
        tree: tree.to_string_lossy().into(),
        connection: None,
        guide_session: None,
        guide: Some(json!({"steps": [{"id": "s1", "title": "One"}, {"id": "s2", "title": "Two"}]}).to_string()),
        round: 1,
        since: None,
        scope_note: None,
        kind: "pr".into(),
        pane: None,
    });
    // No Claude pane yet: no chat.
    assert!(hub.review_chat("r1").unwrap().is_empty());

    // The review's Claude pane, and its session from the SessionStart hook.
    let ask = hub
        .open(OpenReq {
            repo: tree.to_string_lossy().into(),
            place: "tree".into(),
            branch: None,
            tree: Some(tree.to_string_lossy().into()),
            run: "shell".into(),
            title: Some("Ask #7".into()),
            task: None,
            first: None,
        })
        .unwrap()
        .id;
    hub.debug_set_kind(&ask, "claude").unwrap();
    sb(&root, &ask, &["state", "session"], json!({"session_id": "5e55-1d"}));
    wait_for("the session id", || hub.views().iter().any(|v| v.id == ask && v.session.as_deref() == Some("5e55-1d")));

    // Claude's session file, with the guide session copied in before the first message.
    let now = |offset: i64| {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64 + offset;
        rfc3339(t as u64)
    };
    for t in [0, 951782400, 1709210096, 1793075479] {
        assert_eq!(switchboard_core::chat::epoch(&rfc3339(t)), Some(t), "{}", rfc3339(t));
    }
    let user = |text: &str, at: &str| json!({"type": "user", "message": {"role": "user", "content": text}, "timestamp": at}).to_string();
    let said = |text: &str| json!({"type": "assistant", "message": {"content": [{"type": "text", "text": text}]}}).to_string();
    hub.review_step_seen("r1", "s1").unwrap();
    std::thread::sleep(Duration::from_millis(1100));
    let t_s2 = now(0);
    hub.review_step_seen("r1", "s2").unwrap();
    // The same step again adds nothing.
    hub.review_step_seen("r1", "s2").unwrap();
    let lines = [
        user("You are guiding a human through a review", &now(-100)),
        said("2 steps"),
        user("I am on step s1 \"One\". Wait for my question. Reply only: Ready.", &now(-50)),
        said("Ready."),
        user("first question", &now(-40)),
        said("first answer"),
        user("second question", &t_s2),
        said("second answer"),
    ]
    .join("\n");
    let dir = root.join("claude/projects/-work-demo-pr7");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("5e55-1d.jsonl"), lines).unwrap();
    let chat: Vec<(Option<String>, String, String)> =
        hub.review_chat("r1").unwrap().into_iter().map(|t| (t.step, t.question, t.answer)).collect();
    assert_eq!(
        chat,
        vec![
            (Some("s1".into()), "first question".into(), "first answer".into()),
            (Some("s2".into()), "second question".into(), "second answer".into()),
        ]
    );

    // Dismiss and undo a take note: the view lists the dismissed keys.
    hub.review_dismiss("r1", "s2|must|a.rs:3|bad", true).unwrap();
    hub.review_dismiss("r1", "s2|must|a.rs:3|bad", true).unwrap();
    assert_eq!(hub.review_view("r1").unwrap().dismissed, vec!["s2|must|a.rs:3|bad".to_owned()]);
    hub.review_dismiss("r1", "s2|must|a.rs:3|bad", false).unwrap();
    assert!(hub.review_view("r1").unwrap().dismissed.is_empty());
    // A re-run needs the guide session.
    assert!(hub.review_retake("r1", Some("s2")).unwrap_err().contains("guide session"));

    // Restart (SPEC 11.13): the check marks go, the Claude pane closes, and the
    // review fetches again. Here the fetch fails (no clone), so it ends in an error.
    hub.review_mark("r1", "s1", true).unwrap();
    assert!(hub.review_view("r1").unwrap().steps.iter().any(|s| s.checked));
    hub.review_restart("r1").unwrap();
    assert!(hub.review_view("r1").unwrap().steps.iter().all(|s| !s.checked), "the marks go");
    assert!(!hub.views().iter().any(|v| v.id == ask), "the Claude pane closes");
    wait_for("the restart ends", || hub.review_view("r1").unwrap().status == "error");
    hub.debug_add_review(ReviewRow {
        id: "l1".into(),
        url: String::new(),
        repo: "demo".into(),
        number: 0,
        title: "Local".into(),
        head: "h".into(),
        base: "b".into(),
        tree: tree.to_string_lossy().into(),
        connection: None,
        guide_session: None,
        guide: None,
        round: 1,
        since: None,
        scope_note: None,
        kind: "local".into(),
        pane: None,
    });
    // A local review fetches its remotes and reads its range again. This folder is no
    // git checkout, so that ends in an error.
    hub.review_restart("l1").unwrap();
    wait_for("the local restart ends", || hub.review_view("l1").unwrap().status == "error");
}
