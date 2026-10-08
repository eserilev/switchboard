//! A live review against a real GitHub PR and a real `claude`. It costs tokens,
//! so it is ignored by default. Run it with:
//!   cargo test -p sb --test live_review -- --ignored --nocapture

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use switchboard_core::hub::{Hub, Sink};

struct Quiet;
impl Sink for Quiet {
    fn emit(&self, _: &str, _: Value) {}
    fn output(&self, _: &str, _: &[u8]) {}
}

#[test]
#[ignore]
fn live_review_of_a_tiny_pr() {
    let url = std::env::var("SB_LIVE_PR")
        .unwrap_or_else(|_| "https://github.com/octocat/Hello-World/pull/11471".into());
    let root = std::env::temp_dir().join(format!("sb-live-{}", std::process::id()));
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    assert!(Command::new("git")
        .args(["clone", "-q", "https://github.com/octocat/Hello-World.git"])
        .current_dir(&work)
        .status()
        .unwrap()
        .success());
    std::fs::write(
        root.join("config.toml"),
        format!("scan = [\"{}\"]\n", work.display()),
    )
    .unwrap();
    std::env::set_var("SB_HOME", &root);
    std::env::set_var("SB_TMUX_SOCKET", format!("sb-live-{}", std::process::id()));
    let hub = Hub::start(
        switchboard_core::paths::Paths::from_env(),
        PathBuf::from(env!("CARGO_BIN_EXE_sb")),
        Arc::new(Quiet),
    )
    .unwrap();

    let id = hub.review_open(&url).unwrap();
    let t0 = Instant::now();
    let v = loop {
        let v = hub.review_view(&id).unwrap();
        if v.status == "ready" || v.status == "error" || t0.elapsed() > Duration::from_secs(480) {
            break v;
        }
        std::thread::sleep(Duration::from_secs(2));
    };
    println!(
        "status={} after {:?} error={:?}",
        v.status,
        t0.elapsed(),
        v.error
    );
    assert_eq!(v.status, "ready", "{:?}", v.error);
    let guide = v.guide.unwrap();
    let steps = guide["steps"].as_array().unwrap();
    for s in steps {
        println!(
            "step {} {} {} {:?}",
            s["id"], s["title"], s["file"], s["lines"]
        );
    }
    let step = steps
        .iter()
        .find(|s| s["file"].is_string())
        .expect("a step with a file");
    let sid = step["id"].as_str().unwrap();
    let diff = hub.review_diff(&id, sid).unwrap();
    println!("diff rows={} context={}", diff.rows.len(), diff.context);
    assert!(!diff.rows.is_empty());

    let tid = hub
        .review_ask(
            &id,
            sid,
            None,
            None,
            "What does this change add, in one sentence?",
        )
        .unwrap();
    let t0 = Instant::now();
    let answer = loop {
        let v = hub.review_view(&id).unwrap();
        let t = v.threads.iter().find(|t| t.id == tid).unwrap();
        if !t.busy && t.messages.len() >= 2 {
            break t.messages[1].text.clone();
        }
        assert!(t0.elapsed() < Duration::from_secs(240), "no answer");
        std::thread::sleep(Duration::from_secs(2));
    };
    println!("answer: {answer}");
    hub.review_ask(
        &id,
        sid,
        None,
        Some(tid.clone()),
        "Is there a test for it? Answer yes or no.",
    )
    .unwrap();
    let t0 = Instant::now();
    loop {
        let v = hub.review_view(&id).unwrap();
        let t = v.threads.iter().find(|t| t.id == tid).unwrap();
        if !t.busy && t.messages.len() >= 4 {
            println!("follow-up: {}", t.messages[3].text);
            break;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(240),
            "no follow-up answer"
        );
        std::thread::sleep(Duration::from_secs(2));
    }
    hub.review_pin(&id, &tid).unwrap();
    hub.review_draft(&id, &tid).unwrap();
    let v = hub.review_view(&id).unwrap();
    println!("pin: {}", v.pins[0].text);
    println!("draft: {}", hub.review_drafts_text(&id).unwrap());
    assert_eq!(v.pins.len(), 1);
    assert_eq!(v.drafts.len(), 1);

    let _ = Command::new("tmux")
        .args([
            "-L",
            &std::env::var("SB_TMUX_SOCKET").unwrap(),
            "kill-server",
        ])
        .status();
    let _ = std::fs::remove_dir_all(&root);
}
