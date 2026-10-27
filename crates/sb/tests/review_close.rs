//! Close of a review on a real hub (SPEC 11.10): a tmux server, panes in the review
//! worktree, a local git remote that serves `pull/7/head`, and no gh or agent.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use switchboard_core::hub::{Hub, OpenReq, Sink};
use switchboard_core::review::PrInfo;
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

const URL: &str = "https://github.com/demo/demo/pull/7";

fn row(id: &str, head: &str) -> ReviewRow {
    ReviewRow {
        id: id.into(),
        url: URL.into(),
        repo: "demo/demo".into(),
        number: 7,
        title: "PR 7".into(),
        head: head.into(),
        base: String::new(),
        tree: String::new(),
        connection: None,
        guide_session: None,
        guide: Some(r#"{"steps":[]}"#.into()),
        round: 1,
        since: None,
        scope_note: None,
        kind: "pr".into(),
        pane: None,
    }
}

#[test]
fn close_cleans_up_and_never_races_a_checkout() {
    let root = std::env::temp_dir().join(format!("sb-close-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    // The "GitHub" repo: a base and a PR head under refs/pull/7/head.
    let up = root.join("up");
    std::fs::create_dir_all(&up).unwrap();
    g(&up, &["init", "-q", "-b", "main"]);
    std::fs::write(up.join("a.rs"), "fn a() {}\n").unwrap();
    g(&up, &["add", "-A"]);
    g(&up, &["commit", "-q", "-m", "base"]);
    let base = g(&up, &["rev-parse", "HEAD"]);
    g(&up, &["checkout", "-q", "-b", "pr"]);
    std::fs::write(up.join("b.rs"), "fn b() {}\n").unwrap();
    g(&up, &["add", "-A"]);
    g(&up, &["commit", "-q", "-m", "pr"]);
    let head = g(&up, &["rev-parse", "HEAD"]);
    g(&up, &["update-ref", "refs/pull/7/head", &head]);
    g(&up, &["checkout", "-q", "main"]);
    // The local clone.
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    g(&work, &["clone", "-q", &up.to_string_lossy(), "demo"]);
    let clone = work.join("demo");
    let pr = PrInfo {
        number: 7,
        title: "PR 7".into(),
        body: String::new(),
        head_ref_oid: head.clone(),
        head_ref_name: "pr".into(),
        base_ref_name: "main".into(),
        base_ref_oid: base.clone(),
        files: vec![],
    };

    std::env::set_var("SB_HOME", &root);
    std::env::set_var("SB_TMUX_SOCKET", format!("sb-close-{}", std::process::id()));
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
    let notes = |review: &str| -> Vec<String> {
        events
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(e, p)| e == "review_cleanup" && p["review"] == review)
            .map(|(_, p)| p["note"].as_str().unwrap_or("").to_owned())
            .collect()
    };
    let shell = |tree: &Path, title: &str| {
        hub.open(OpenReq {
            repo: clone.to_string_lossy().into(),
            place: "tree".into(),
            branch: None,
            tree: Some(tree.to_string_lossy().into()),
            run: "shell".into(),
            title: Some(title.into()),
            task: None,
            first: None,
        })
        .unwrap()
        .id
    };
    let pane_ids = || hub.views().into_iter().map(|v| v.id).collect::<Vec<_>>();

    // 1. A review with its Claude pane, your own shell, and a file of yours. The
    // clone is reached through a symlink, and your shell opens at the real path.
    let link = root.join("link");
    std::os::unix::fs::symlink(&work, &link).unwrap();
    hub.debug_add_review(row("ra", &head));
    let tree = hub.debug_fetch("ra", &link.join("demo"), "origin", &pr, 1).unwrap();
    assert!(tree.starts_with(&link) && tree.join("b.rs").exists(), "{}", tree.display());
    let ask = shell(&tree, "Ask #7");
    hub.debug_set_kind(&ask, "claude").unwrap();
    let mine = shell(&tree.canonicalize().unwrap(), "mine");
    std::fs::write(tree.join("notes.txt"), "mine\n").unwrap();

    // A first close finds the change, asks, and changes nothing.
    let r = hub.review_close("ra", false, None).unwrap();
    assert!(!r.closed && r.changes.iter().any(|c| c.contains("notes.txt")), "{r:?}");
    assert!(hub.reviews().iter().any(|v| v.id == "ra"));
    assert!(pane_ids().contains(&ask));
    // Discard with a list that misses a change: it asks again.
    let r2 = hub.review_close("ra", false, Some(vec![])).unwrap();
    assert!(!r2.closed);

    // Discard what you saw: the review closes and its Claude pane goes. Your shell
    // is in the folder, by its real path, so the folder stays with a note.
    let r = hub.review_close("ra", false, Some(r.changes)).unwrap();
    assert!(r.closed);
    assert!(!hub.reviews().iter().any(|v| v.id == "ra"));
    wait_for("a note on the kept folder", || !notes("ra").is_empty());
    assert!(!pane_ids().contains(&ask), "the review's Claude pane closes");
    assert!(pane_ids().contains(&mine), "your own pane stays");
    assert!(tree.join("notes.txt").exists(), "{:?}", notes("ra"));
    assert!(notes("ra")[0].contains("stays"), "{:?}", notes("ra"));
    // The head is pinned for a later round.
    assert_eq!(g(&clone, &["rev-parse", "refs/switchboard/ra/head"]), head);

    // 2. With your pane closed and your file gone, the sweep at start removes it.
    hub.close(&mine).unwrap();
    std::fs::remove_file(tree.join("notes.txt")).unwrap();
    hub.debug_sweep();
    assert!(!tree.exists(), "the sweep removes a clean closed worktree");
    assert!(g(&clone, &["branch", "--list", "sb-pr-7"]).is_empty(), "the PR branch goes");
    assert_eq!(g(&clone, &["rev-parse", "refs/switchboard/ra/head"]), head);

    // 3. A review closed during its network fetch gets no worktree.
    hub.debug_add_review(row("rb", &head));
    assert!(hub.review_close("rb", false, None).unwrap().closed);
    let err = hub.debug_fetch("rb", &clone, "origin", &pr, 1).unwrap_err();
    assert!(err.contains("closed"), "{err}");
    assert!(!tree.exists());

    // 4. The PR has closed rounds: the next round starts from the last one.
    let prior = hub.review_prior(URL).unwrap().expect("a closed round");
    assert_eq!((prior.round, prior.head.as_str()), (1, head.as_str()));
    let next = hub.review_open(URL, Some(&prior.id)).unwrap();
    let v = hub.review_view(&next).unwrap();
    assert_eq!((v.round, v.since.as_deref()), (2, Some(head.as_str())));
    // With a review of the PR open, there is no prior to offer.
    assert!(hub.review_prior(URL).unwrap().is_none());
    // A round of another PR is refused.
    assert!(hub
        .review_open("https://github.com/demo/demo/pull/8", Some(&prior.id))
        .is_err());
    // The round fails in the background with no gh; close it.
    wait_for("the round fails with no gh", || {
        hub.review_view(&next).map(|v| v.status == "error").unwrap_or(true)
    });
    assert!(hub.review_close(&next, false, None).unwrap().closed);

    // 5. Close and checkout race: a close at a random time during the fetch. When
    // both are done, no worktree is left, and no folder of an open review goes.
    // The delays spread over 0 to 300 ms, and the rounds go on until both orders
    // ran: how long a fetch takes depends on the machine.
    let hub = Arc::new(hub);
    let (mut before, mut after) = (0, 0);
    for i in 0..100 {
        if i >= 25 && before > 0 && after > 0 {
            break;
        }
        let id = format!("rs{i}");
        hub.debug_add_review(row(&id, &head));
        let (h1, h2) = (hub.clone(), hub.clone());
        let (id1, id2) = (id.clone(), id.clone());
        let (c1, p1) = (clone.clone(), pr.clone());
        let fetch = std::thread::spawn(move || h1.debug_fetch(&id1, &c1, "origin", &p1, 1));
        let delay = Duration::from_millis((i * 37 % 300) as u64);
        let close = std::thread::spawn(move || {
            std::thread::sleep(delay);
            h2.review_close(&id2, false, None).unwrap()
        });
        let fetched = fetch.join().unwrap();
        assert!(close.join().unwrap().closed);
        match &fetched {
            Ok(_) => before += 1,
            Err(e) if e.contains("closed") => after += 1,
            Err(e) => panic!("round {i}: {e}"),
        }
        wait_for(&format!("round {i}: no worktree left ({fetched:?})"), || !tree.exists());
    }
    assert!(before > 0 && after > 0, "both orders ran: {before} {after}");
    // An open review's folder never goes: a sweep with a review open on it.
    hub.debug_add_review(row("rc", &head));
    let t = hub.debug_fetch("rc", &clone, "origin", &pr, 1).unwrap();
    hub.debug_sweep();
    assert!(t.exists(), "the sweep keeps the folder of an open review");
    // An Update of an open review keeps working when its own folder has your edit,
    // for example from its Claude pane. The edit comes along.
    std::fs::write(t.join("scratch.txt"), "a note\n").unwrap();
    assert_eq!(hub.debug_fetch("rc", &clone, "origin", &pr, 1).unwrap(), t);
    assert!(t.join("scratch.txt").exists());
    // Another review of the same folder refuses it while that work is there.
    let r = hub.review_close("rc", true, None).unwrap();
    assert!(r.closed);
    wait_for("a note on the kept folder", || !notes("rc").is_empty());
    hub.debug_add_review(row("rd", &head));
    let err = hub.debug_fetch("rd", &clone, "origin", &pr, 1).unwrap_err();
    assert!(err.contains("scratch.txt"), "{err}");
    assert!(t.join("scratch.txt").exists());
}
