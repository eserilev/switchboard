//! Start agent from a task on a real hub and tmux (SPEC 27.6, 27.7). A file of its
//! own: the test sets PATH and other process-wide variables.

use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use switchboard_core::hub::{Hub, Sink};
use switchboard_core::tasks::NewTask;

#[derive(Default)]
struct Events(Mutex<Vec<(String, Value)>>);

impl Sink for Events {
    fn emit(&self, event: &str, payload: Value) {
        self.0.lock().unwrap().push((event.to_owned(), payload));
    }
    fn output(&self, _pane: &str, _data: &[u8]) {}
}

fn g(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn wait_for(what: &str, f: impl Fn() -> bool) {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("timed out: {what}");
}

/// Runs `sb` as an agent pane does, and returns (exit code, stdout, stderr).
fn sb(root: &std::path::Path, pane: Option<&str>, args: &[&str]) -> (i32, String, String) {
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_sb"));
    c.args(args).env("SB_HOME", root).env_remove("SB_SOCK").env_remove("SB_PANE");
    if let Some(p) = pane {
        c.env("SB_PANE", p);
    }
    let out = c.output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into(), String::from_utf8_lossy(&out.stderr).into())
}

/// Start agent from a task on a real hub and tmux (SPEC 27.6, 27.7), with a stand-in
/// `claude` that writes its arguments and what it gets typed to files.
#[test]
fn start_an_agent_from_a_task() {
    use switchboard_core::hub::OpenReq;
    use switchboard_core::store::Store;
    let root = std::env::temp_dir().join(format!("sb-taskagent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("bin")).unwrap();
    let repo = root.join("demo");
    std::fs::create_dir_all(&repo).unwrap();
    g(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.rs"), "fn a() {}\n").unwrap();
    g(&repo, &["add", "-A"]);
    g(&repo, &["commit", "-q", "-m", "a"]);
    // The branch name of the task is taken: Start picks the next one.
    g(&repo, &["branch", "sb/write-the-tests"]);
    let claude = root.join("bin/claude");
    std::fs::write(
        &claude,
        format!(
            "#!/bin/sh\necho \"$SB_PANE $*\" >> '{0}/args.txt'\nstty raw -echo 2>/dev/null\nexec cat >> \"{0}/typed-$SB_PANE.txt\"\n",
            root.display()
        ),
    )
    .unwrap();
    std::process::Command::new("chmod").args(["+x", &claude.to_string_lossy()]).status().unwrap();
    std::env::set_var("PATH", format!("{}:{}", root.join("bin").display(), std::env::var("PATH").unwrap_or_default()));
    std::env::set_var("SB_HOME", &root);
    std::env::set_var("SB_TMUX_SOCKET", format!("sb-taskagent-{}", std::process::id()));
    std::env::set_var("SB_FIRST_WAIT_MS", "4000");
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

    let t = hub
        .task_add(NewTask { title: "Write the tests".into(), notes: "Cover slot 0.".into(), repo: Some(repo.to_string_lossy().into()), ..NewTask::default() })
        .unwrap();
    let plan = hub.task_start_plan(&t.id).unwrap();
    assert_eq!((plan.branch.as_str(), plan.first.as_str()), ("sb/write-the-tests-2", "Write the tests\n\nCover slot 0."));
    let typed = |pane: &str| std::fs::read_to_string(root.join(format!("typed-{pane}.txt"))).unwrap_or_default();

    // Start: a new worktree, the pane links to the task, and To do goes to In progress.
    let pane = hub
        .open(OpenReq {
            repo: repo.to_string_lossy().into(),
            place: "new".into(),
            branch: Some(plan.branch.clone()),
            tree: None,
            run: "claude".into(),
            title: Some(plan.title.clone()),
            task: Some(t.id.clone()),
            first: Some(plan.first.clone()),
        })
        .unwrap()
        .id;
    let v = hub.tasks().unwrap();
    assert_eq!(v.tasks[0].status, "doing");
    assert_eq!((v.links.len(), v.links[0].pane.as_str(), v.links[0].closed), (1, pane.as_str(), false));
    assert_eq!(g(&repos_tree(&v.links[0]), &["rev-parse", "--abbrev-ref", "HEAD"]), "sb/write-the-tests-2");
    // No paste before the agent has its session.
    std::thread::sleep(std::time::Duration::from_millis(800));
    assert_eq!(typed(&pane), "");
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_sb"))
        .args(["state", "session"])
        .env("SB_HOME", &root)
        .env("SB_PANE", &pane)
        .env_remove("SB_SOCK")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut c.stdin.take().unwrap(), br#"{"session_id": "sess-1"}"#).unwrap();
    c.wait().unwrap();
    wait_for("the first message", || typed(&pane).contains("Cover slot 0."));
    assert_eq!(typed(&pane), "\x1b[200~Write the tests\n\nCover slot 0.\x1b[201~", "one paste, no Enter");

    // A pane that never gets a session: after the wait, the window gets the text.
    let other = hub
        .open(OpenReq {
            repo: repo.to_string_lossy().into(),
            place: "main".into(),
            branch: None,
            tree: None,
            run: "claude".into(),
            title: None,
            task: None,
            first: Some("hello".into()),
        })
        .unwrap()
        .id;
    wait_for("paste_failed", || events.0.lock().unwrap().iter().any(|(e, p)| e == "paste_failed" && p["pane"] == other.as_str() && p["text"] == "hello"));
    assert_eq!(typed(&other), "");

    // `sb task` from the agent pane: show, note, status; never done.
    let (code, out, _) = sb(&root, Some(&pane), &["task", "show"]);
    assert_eq!(code, 0);
    assert!(out.contains("Write the tests") && out.contains("doing"), "{out}");
    assert_eq!(sb(&root, Some(&pane), &["task", "note", &t.id, "Tests pass."]).0, 0);
    assert_eq!(sb(&root, Some(&pane), &["task", "status", &t.id, "review"]).0, 0);
    for p in [Some(pane.as_str()), None] {
        let (code, _, err) = sb(&root, p, &["task", "status", &t.id, "done"]);
        assert!(code == 1 && err.contains("only the Switchboard window"), "{err}");
    }
    let (code, out, _) = sb(&root, None, &["task", "add", "From the shell", "--sprint", "backlog"]);
    assert!(code == 0 && out.contains("From the shell"), "{out}");
    let (_, out, _) = sb(&root, None, &["task", "list", "--json"]);
    let list: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(list["tasks"].as_array().unwrap().len(), 2);
    let h = Store::open(&switchboard_core::paths::Paths::from_env().store()).unwrap().task_history(&t.id).unwrap();
    let rows: Vec<(String, String)> = h.iter().map(|x| (x.by.clone(), x.kind.clone())).collect();
    assert!(rows.contains(&(pane.clone(), "note".into())) && rows.contains(&(pane.clone(), "status".into())), "{rows:?}");
    assert_eq!(hub.tasks().unwrap().tasks[0].status, "review");

    // A resume never restarts an agent that runs.
    assert!(hub.task_resume(&pane).unwrap_err().contains("still runs"));
    // Close the pane: the link stays and shows ended. Resume starts the agent again
    // with its session, in its folder, and the link moves to the new pane.
    hub.close(&pane).unwrap();
    assert!(hub.tasks().unwrap().links[0].closed);
    let again = hub.task_resume(&pane).unwrap();
    assert_ne!(again, pane);
    wait_for("the resumed agent", || std::fs::read_to_string(root.join("args.txt")).unwrap_or_default().lines().any(|l| l.starts_with(&again) && l.contains("--resume sess-1")));
    let v = hub.tasks().unwrap();
    assert_eq!((v.links.len(), v.links[0].pane.as_str(), v.links[0].closed), (1, again.as_str(), false));
}

fn repos_tree(l: &switchboard_core::tasks::TaskLink) -> PathBuf {
    PathBuf::from(l.tree.clone().unwrap())
}
