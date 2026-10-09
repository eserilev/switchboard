//! A live switch of a Claude pane to another connection, with a real `claude`.
//! It costs tokens, so it is ignored by default:
//!   cargo test -p sb --test live_switch -- --ignored --nocapture

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use switchboard_core::hub::{Hub, OpenReq, PaneView, Sink};
use switchboard_core::lamp::Lamp;

struct Quiet;
impl Sink for Quiet {
    fn emit(&self, _: &str, _: Value) {}
    fn output(&self, _: &str, _: &[u8]) {}
}

fn until(hub: &Hub, id: &str, what: &str, secs: u64, f: impl Fn(&PaneView) -> bool) -> PaneView {
    let t0 = Instant::now();
    loop {
        if let Some(v) = hub.views().into_iter().find(|v| v.id == id) {
            if f(&v) {
                return v;
            }
            assert!(
                t0.elapsed() < Duration::from_secs(secs),
                "timed out: {what}: {:?} {:?} {:?}",
                v.lamp,
                v.summary,
                v.tail
            );
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[test]
#[ignore]
fn live_switch_keeps_the_session() {
    let root = std::env::temp_dir().join(format!("sb-switch-{}", std::process::id()));
    let work = root.join("work/demo");
    std::fs::create_dir_all(&work).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&work)
        .status()
        .unwrap()
        .success());
    // Two connections on the same login, so the switch needs no second account.
    std::fs::write(
        root.join("config.toml"),
        "[connections.a]\nkind = \"claude\"\n[connections.b]\nkind = \"claude\"\n",
    )
    .unwrap();
    std::env::set_var("SB_HOME", &root);
    std::env::set_var(
        "SB_TMUX_SOCKET",
        format!("sb-switch-{}", std::process::id()),
    );
    let hub = Hub::start(
        switchboard_core::paths::Paths::from_env(),
        PathBuf::from(env!("CARGO_BIN_EXE_sb")),
        Arc::new(Quiet),
    )
    .unwrap();
    struct Kill;
    impl Drop for Kill {
        fn drop(&mut self) {
            let _ = Command::new("tmux")
                .args([
                    "-L",
                    &std::env::var("SB_TMUX_SOCKET").unwrap(),
                    "kill-server",
                ])
                .status();
        }
    }
    let _kill = Kill;

    let v = hub
        .open(OpenReq {
            repo: work.to_string_lossy().into(),
            place: "main".into(),
            branch: None,
            tree: None,
            run: "claude".into(),
            title: None,
        })
        .unwrap();
    let id = v.id.clone();
    assert_eq!(v.connection.as_deref(), Some("a"));
    let v = until(&hub, &id, "trust dialog or prompt", 30, |v| {
        v.trust || v.tail.iter().any(|l| l.contains('❯'))
    });
    if v.trust {
        println!("trust dialog seen; trusting");
        hub.trust(&id, true).unwrap();
    }
    std::thread::sleep(Duration::from_secs(4));
    hub.input(&id, b"Remember the code word: lantern. Reply with OK only.")
        .unwrap();
    std::thread::sleep(Duration::from_millis(400));
    hub.input(&id, b"\r").unwrap();
    let v = until(&hub, &id, "first turn", 120, |v| v.lamp == Lamp::Turn);
    println!("turn 1: {:?} session={:?}", v.summary, v.session);
    assert!(v.session.is_some());

    let next = hub.connection_resume(&id).unwrap();
    println!("switched to {next}");
    assert_eq!(next, "b");
    assert_eq!(hub.connection_view().active, "b");
    let v = until(&hub, &id, "resumed pane", 30, |v| {
        v.connection.as_deref() == Some("b") && v.tail.iter().any(|l| l.contains('❯'))
    });
    println!("resumed: {:?}", v.summary);
    std::thread::sleep(Duration::from_secs(3));
    hub.input(&id, b"What was the code word? Reply with the word only.")
        .unwrap();
    std::thread::sleep(Duration::from_millis(400));
    hub.input(&id, b"\r").unwrap();
    let v = until(&hub, &id, "second turn", 120, |v| {
        v.lamp == Lamp::Turn
            && v.summary
                .as_deref()
                .map(|s| !s.contains("Resumed"))
                .unwrap_or(false)
    });
    println!("turn 2: {:?}", v.summary);
    assert!(v.summary.unwrap().to_lowercase().contains("lantern"));
    let _ = std::fs::remove_dir_all(&root);
}
