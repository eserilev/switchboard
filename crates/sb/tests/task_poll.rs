//! The "PR merged" poll on a real hub (SPEC 27.6), with a stand-in `gh` that writes its
//! arguments to a file. A file of its own: the test sets PATH.

use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use switchboard_core::hub::{Hub, Sink};
use switchboard_core::tasks::{NewTask, TaskFields};

#[derive(Default)]
struct Events(Mutex<Vec<(String, Value)>>);

impl Sink for Events {
    fn emit(&self, event: &str, payload: Value) {
        self.0.lock().unwrap().push((event.to_owned(), payload));
    }
    fn output(&self, _pane: &str, _data: &[u8]) {}
}

impl Events {
    fn merged(&self) -> Option<Vec<String>> {
        let e = self.0.lock().unwrap();
        let last = e.iter().rev().find(|(k, _)| k == "tasks")?;
        Some(last.1["merged"].as_array()?.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
    }
}

fn wait_for(what: &str, f: impl Fn() -> bool) {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while std::time::Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("timed out: {what}");
}

#[test]
fn a_merged_pr_link_gets_the_hint_and_gh_only_reads() {
    let root = std::env::temp_dir().join(format!("sb-taskpoll-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("bin")).unwrap();
    // PR 1 is merged, PR 2 is open. Any other call fails.
    let gh = root.join("bin/gh");
    std::fs::write(
        &gh,
        format!(
            "#!/bin/sh\necho \"$*\" >> '{}/gh.txt'\ncase \"$*\" in\n  *pull/1\\ *) echo MERGED ;;\n  *pull/2\\ *) echo OPEN ;;\n  *) exit 1 ;;\nesac\n",
            root.display()
        ),
    )
    .unwrap();
    std::process::Command::new("chmod").args(["+x", &gh.to_string_lossy()]).status().unwrap();
    std::env::set_var("PATH", format!("{}:{}", root.join("bin").display(), std::env::var("PATH").unwrap_or_default()));
    std::env::set_var("SB_HOME", &root);
    std::env::set_var("SB_TMUX_SOCKET", format!("sb-taskpoll-{}", std::process::id()));
    std::env::set_var("SB_PR_POLL_SECS", "1");
    std::env::remove_var("SB_SOCK");
    struct Kill(String, PathBuf);
    impl Drop for Kill {
        fn drop(&mut self) {
            switchboard_core::tmux::kill_server(&self.0);
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }
    let _kill = Kill(std::env::var("SB_TMUX_SOCKET").unwrap(), root.clone());
    let events = Arc::new(Events::default());
    let hub = Hub::start(switchboard_core::paths::Paths::from_env(), PathBuf::from(env!("CARGO_BIN_EXE_sb")), events.clone()).unwrap();

    let pr1 = "https://github.com/o/r/pull/1";
    let pr2 = "https://github.com/o/r/pull/2";
    let fields = |links: &[&str]| TaskFields {
        title: "t".into(),
        notes: "".into(),
        priority: "normal".into(),
        repo: None,
        links: links.iter().map(|l| l.to_string()).collect(),
        ..TaskFields::default()
    };
    let a = hub.task_add(NewTask { title: "t".into(), ..NewTask::default() }).unwrap();
    hub.task_edit(&a.id, fields(&[&format!("{pr1}/files"), pr2, "https://example.com/x"])).unwrap();
    // A done task is not read.
    let done = hub.task_add(NewTask { title: "done".into(), ..NewTask::default() }).unwrap();
    hub.task_edit(&done.id, TaskFields { title: "done".into(), ..fields(&["https://github.com/o/r/pull/3"]) }).unwrap();
    hub.task_status(&done.id, "done", None).unwrap();

    wait_for("the poll finds PR 1 merged", || events.merged() == Some(vec![pr1.to_owned()]));
    assert_eq!(hub.tasks().unwrap().merged, [pr1]);
    // Wait for a few more polls: a merged PR is not read again.
    let calls = || std::fs::read_to_string(root.join("gh.txt")).unwrap_or_default();
    wait_for("more polls", || calls().lines().filter(|l| l.contains("pull/2")).count() >= 3);
    let lines: Vec<String> = calls().lines().map(str::to_owned).collect();
    assert_eq!(lines.iter().filter(|l| l.contains("pull/1")).count(), 1, "{lines:?}");
    assert!(lines.iter().all(|l| l.starts_with("pr view https://github.com/o/r/pull/") && l.ends_with("--json state --jq .state")), "gh only reads: {lines:?}");
    assert!(!lines.iter().any(|l| l.contains("pull/3")), "a done task is not read");
    // A task with the link removed loses the hint.
    hub.task_edit(&a.id, fields(&[pr2])).unwrap();
    assert!(hub.tasks().unwrap().merged.is_empty());
    std::env::remove_var("SB_PR_POLL_SECS");
}
