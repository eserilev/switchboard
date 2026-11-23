//! Handover (SPEC 38), end to end on real hubs, tmux and git: an agent on the
//! "server" hub continues on "this computer". git fetches from a folder in place
//! of ssh. The test sets PATH and other process-wide variables: it is its own binary.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use switchboard_core::config::Config;
use switchboard_core::handover::{self, Plan};
use switchboard_core::hub::{Hub, OpenReq, Sink};
use switchboard_core::paths::Paths;

struct Quiet;

impl Sink for Quiet {
    fn emit(&self, _event: &str, _payload: Value) {}
    fn output(&self, _pane: &str, _data: &[u8]) {}
}

fn g(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=t", "-c", "user.email=t@t"]).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn wait_for(what: &str, f: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < end, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

struct Clean(Vec<String>, PathBuf);
impl Drop for Clean {
    fn drop(&mut self) {
        for s in &self.0 {
            switchboard_core::tmux::kill_server(s);
        }
        let _ = std::fs::remove_dir_all(&self.1);
    }
}

fn start_hub(root: &Path, socket: &str) -> Arc<Hub> {
    std::env::set_var("SB_HOME", root);
    std::env::set_var("SB_TMUX_SOCKET", socket);
    std::env::remove_var("SB_SOCK");
    Hub::start(Paths::from_env(), PathBuf::from(env!("CARGO_BIN_EXE_sb")), Arc::new(Quiet)).unwrap()
}

#[test]
fn an_agent_on_the_server_continues_here_with_its_work_and_its_conversation() {
    let pid = std::process::id();
    let root = std::env::temp_dir().join(format!("sb-handover-{pid}"));
    let _ = std::fs::remove_dir_all(&root);
    let (sa, sb) = (format!("sb-test-hand-a-{pid}"), format!("sb-test-hand-b-{pid}"));
    let _clean = Clean(vec![sa.clone(), sb.clone()], root.clone());

    // One repo, cloned on the server and here.
    let up = root.join("up.git");
    std::fs::create_dir_all(&up).unwrap();
    g(&up, &["init", "-q", "--bare", "-b", "main"]);
    let seed = root.join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    g(&seed, &["init", "-q", "-b", "main"]);
    std::fs::write(seed.join("a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(seed.join("gone.rs"), "fn gone() {}\n").unwrap();
    g(&seed, &["add", "-A"]);
    g(&seed, &["commit", "-q", "-m", "seed"]);
    g(&seed, &["push", "-q", up.to_str().unwrap(), "main"]);
    let server = root.join("server/demo");
    let here = root.join("here/demo");
    for d in [&server, &here] {
        std::fs::create_dir_all(d.parent().unwrap()).unwrap();
        g(d.parent().unwrap(), &["clone", "-q", up.to_str().unwrap(), "demo"]);
    }

    // On the server: the agent's worktree, a commit, and work that is not committed.
    let tree = root.join("server/demo-feat");
    g(&server, &["worktree", "add", "-q", "-b", "feat", tree.to_str().unwrap()]);
    std::fs::write(tree.join("a.rs"), "fn a() { 1 }\n").unwrap();
    g(&tree, &["commit", "-q", "-am", "feat: a returns 1"]);
    let c1 = g(&tree, &["rev-parse", "HEAD"]);
    std::fs::write(tree.join("a.rs"), "fn a() { 2 }\n").unwrap();
    std::fs::write(tree.join("new.rs"), "fn new() {}\n").unwrap();
    std::fs::remove_file(tree.join("gone.rs")).unwrap();
    std::fs::write(tree.join(".gitignore"), "target/\n").unwrap();
    std::fs::create_dir_all(tree.join("target")).unwrap();
    std::fs::write(tree.join("target/big.bin"), "built").unwrap();
    let status_before = g(&tree, &["status", "--porcelain"]);

    // The server hub: an agent pane with a session and its file.
    let a_root = root.join("a");
    let hub_a = start_hub(&a_root, &sa);
    let id = hub_a
        .open(OpenReq {
            repo: server.to_string_lossy().into(),
            place: "tree".into(),
            branch: None,
            tree: Some(tree.to_string_lossy().into()),
            run: "shell".into(),
            title: Some("gloas fix".into()),
            task: None,
            first: None,
        })
        .unwrap()
        .id;
    hub_a.debug_set_kind(&id, "claude").unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_sb"))
        .args(["state", "session"])
        .env("SB_HOME", &a_root)
        .env("SB_PANE", &id)
        .env_remove("SB_SOCK")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut c.stdin.take().unwrap(), br#"{"session_id": "sess-1"}"#).unwrap();
    c.wait().unwrap();
    wait_for("the session", || hub_a.views().iter().any(|v| v.id == id && v.session.as_deref() == Some("sess-1")));
    let a_paths = Paths::from_env();
    let sdir = handover::project_dir(&a_paths.claude, &tree);
    std::fs::create_dir_all(&sdir).unwrap();
    std::fs::write(sdir.join("sess-1.jsonl"), "{\"type\":\"user\"}\n").unwrap();
    // An old snapshot ref goes at the next handover.
    g(&tree, &["update-ref", "refs/switchboard/handover/p0-1", &c1]);

    let plan: Plan = hub_a.handover_out(&id).unwrap();
    assert_eq!((plan.head.as_str(), plan.branch.as_deref(), plan.title.as_str()), (c1.as_str(), Some("feat"), "gloas fix"));
    assert_ne!(plan.snap, plan.head);
    let mut changes = plan.changes.clone();
    changes.sort();
    assert_eq!(changes, ["A\t.gitignore", "A\tnew.rs", "D\tgone.rs", "M\ta.rs"]);
    assert_eq!((plan.session.as_deref(), plan.session_data.as_deref()), (Some("sess-1"), Some("{\"type\":\"user\"}\n")));
    // The snapshot changes nothing on the server: not the files, not the index, not the branch.
    assert_eq!(g(&tree, &["status", "--porcelain"]), status_before);
    assert_eq!(g(&tree, &["rev-parse", "HEAD"]), c1);
    assert_eq!(g(&tree, &["for-each-ref", "--format=%(refname)", "refs/switchboard/handover/"]), plan.refname);

    // This computer: the dry run, then the handover. git fetches from the folder.
    let b_root = root.join("b");
    let b_paths = Paths::under(b_root.clone());
    let config = Config { scan: vec![root.join("here").to_string_lossy().into()], ..Config::default() };
    let source = tree.to_string_lossy().into_owned();
    let mut bad = plan.clone();
    bad.branch = Some("-x".into());
    assert!(handover::target(&b_paths, &config, &source, &bad).is_err());
    bad = plan.clone();
    bad.refname = "refs/heads/main".into();
    assert!(handover::target(&b_paths, &config, &source, &bad).is_err());
    let t = handover::target(&b_paths, &config, &source, &plan).unwrap();
    let want = root.join("here/demo-feat");
    assert_eq!((t.repo.as_str(), t.tree.as_str(), t.branch.as_str(), t.note.as_deref()), (here.to_str().unwrap(), want.to_str().unwrap(), "feat", None));
    assert!(!want.exists(), "the dry run makes nothing");
    assert_eq!(g(&here, &["for-each-ref", "refs/switchboard/"]), "");

    let p = handover::bring_in_here(&b_paths, &config, &source, "me@box", &plan).unwrap();
    assert_eq!((p.tree.as_str(), p.branch.as_str(), p.session.as_deref(), p.from.as_str()), (want.to_str().unwrap(), "feat", Some("sess-1"), "me@box"));
    assert_eq!(g(&want, &["rev-parse", "HEAD"]), c1);
    assert_eq!(g(&want, &["rev-parse", "--abbrev-ref", "HEAD"]), "feat");
    assert_eq!(std::fs::read_to_string(want.join("a.rs")).unwrap(), "fn a() { 2 }\n");
    assert_eq!(std::fs::read_to_string(want.join("new.rs")).unwrap(), "fn new() {}\n");
    assert!(!want.join("gone.rs").exists() && !want.join("target").exists());
    // The changes are back as changes, not staged, as on the server.
    let mut st: Vec<String> = g(&want, &["status", "--porcelain"]).lines().map(|l| l.trim().to_owned()).collect();
    st.sort();
    assert_eq!(st, ["?? .gitignore", "?? new.rs", "D gone.rs", "M a.rs"]);
    assert_eq!(g(&want, &["diff", "--cached", "--name-only"]), "", "nothing is staged");
    assert_eq!(g(&here, &["for-each-ref", "refs/switchboard/"]), "");

    // The branch is checked out here now: a second dry run takes the next name, and says why.
    let t2 = handover::target(&b_paths, &config, &source, &plan).unwrap();
    assert_eq!((t2.branch.as_str(), t2.note.as_deref()), ("feat-2", Some("feat is checked out here already")));

    // The hub here: the agent starts in the worktree with --resume, once.
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        bin.join("claude"),
        format!("#!/bin/sh\necho \"$PWD $*\" >> '{}/args.txt'\nexec cat > /dev/null\n", root.display()),
    )
    .unwrap();
    Command::new("chmod").args(["+x", bin.join("claude").to_str().unwrap()]).status().unwrap();
    std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
    let hub_b = start_hub(&b_root, &sb);
    let a = hub_b.handover_take().unwrap().expect("the pending handover");
    assert_eq!((a.from.as_str(), a.branch.as_str(), a.changes, a.resumed), ("me@box", "feat", 4, true));
    let v = a.pane;
    assert_eq!((v.title.as_deref(), v.kind.as_str()), (Some("gloas fix"), "claude"));
    let args = || std::fs::read_to_string(root.join("args.txt")).unwrap_or_default();
    wait_for("claude --resume", || args().contains("--resume sess-1"));
    assert!(args().starts_with(want.to_str().unwrap()), "{}", args());
    let copied = handover::project_dir(&b_paths.claude, &want).join("sess-1.jsonl");
    assert_eq!(std::fs::read_to_string(copied).unwrap(), "{\"type\":\"user\"}\n");
    assert!(hub_b.handover_take().unwrap().is_none(), "a handover opens once");
    assert!(!handover::pending_dir(&b_paths.home).join("sess-1.jsonl").exists());
}
