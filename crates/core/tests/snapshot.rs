//! Snapshots of a work tree (SPEC 11.14), against real git: what goes in, what
//! stays out, and that the files, the index and HEAD never change.

use std::path::{Path, PathBuf};
use std::process::Command;
use switchboard_core::review::{self, SNAP_MAX};

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(name: &str) -> Dir {
    let d = std::env::temp_dir().join(format!("sb-snap-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    Dir(d)
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

/// A repo with a.txt, b.txt and a .gitignore that ignores *.log, in one commit.
fn repo(name: &str) -> Dir {
    let d = scratch(name);
    g(&d.0, &["init", "-q", "-b", "main"]);
    std::fs::write(d.0.join("a.txt"), "a1\n").unwrap();
    std::fs::write(d.0.join("b.txt"), "b1\n").unwrap();
    std::fs::write(d.0.join(".gitignore"), "*.log\n").unwrap();
    g(&d.0, &["add", "-A"]);
    g(&d.0, &["commit", "-q", "-m", "one"]);
    d
}

/// What a commit holds: `path:content` for each file, sorted.
fn files(dir: &Path, commit: &str) -> Vec<String> {
    let mut out: Vec<String> = g(dir, &["ls-tree", "-r", "--name-only", commit])
        .lines()
        .map(|p| format!("{p}:{}", g(dir, &["show", &format!("{commit}:{p}")])))
        .collect();
    out.sort();
    out
}

/// The state that a snapshot must never change.
fn state(dir: &Path) -> (String, String, Vec<u8>) {
    let index = PathBuf::from(g(dir, &["rev-parse", "--path-format=absolute", "--git-path", "index"]));
    (g(dir, &["rev-parse", "HEAD"]), g(dir, &["status", "--porcelain=v2", "--untracked-files=all"]), std::fs::read(index).unwrap())
}

#[test]
fn a_clean_tree_is_head() {
    let d = repo("clean");
    let s = review::snapshot(&d.0).unwrap();
    assert_eq!(s.commit, g(&d.0, &["rev-parse", "HEAD"]));
    assert_eq!(s.head, s.commit);
    assert!(s.skipped.is_empty());
}

#[test]
fn the_snapshot_holds_the_work_and_changes_nothing() {
    let d = repo("work");
    // A change, a deletion, a rename, a new file, a staged change, an ignored file.
    std::fs::write(d.0.join("a.txt"), "a2\n").unwrap();
    std::fs::remove_file(d.0.join("b.txt")).unwrap();
    std::fs::create_dir_all(d.0.join("src")).unwrap();
    std::fs::write(d.0.join("src/new.rs"), "fn main() {}\n").unwrap();
    std::fs::write(d.0.join("c.txt"), "c1\n").unwrap();
    g(&d.0, &["add", "c.txt"]);
    std::fs::write(d.0.join("c.txt"), "c2\n").unwrap();
    std::fs::write(d.0.join("debug.log"), "noise\n").unwrap();
    let before = state(&d.0);
    let s = review::snapshot(&d.0).unwrap();
    assert_eq!(state(&d.0), before, "the files, the index or HEAD changed");
    assert_ne!(s.commit, s.head);
    // The work tree as it is, not the index: c.txt is c2.
    assert_eq!(files(&d.0, &s.commit), [".gitignore:*.log", "a.txt:a2", "c.txt:c2", "src/new.rs:fn main() {}"]);
    assert_eq!(g(&d.0, &["rev-parse", &format!("{}^", s.commit)]), s.head);
    // A second snapshot of the same work is the same tree.
    let again = review::snapshot(&d.0).unwrap();
    assert_eq!(g(&d.0, &["rev-parse", &format!("{}^{{tree}}", again.commit)]), g(&d.0, &["rev-parse", &format!("{}^{{tree}}", s.commit)]));
}

#[test]
fn a_big_new_file_stays_out() {
    let d = repo("big");
    std::fs::write(d.0.join("data.bin"), vec![7u8; SNAP_MAX as usize + 1]).unwrap();
    std::fs::write(d.0.join("small.txt"), "s\n").unwrap();
    let s = review::snapshot(&d.0).unwrap();
    assert_eq!(s.skipped, ["data.bin"]);
    let names = g(&d.0, &["ls-tree", "-r", "--name-only", &s.commit]);
    assert!(names.contains("small.txt") && !names.contains("data.bin"), "{names}");
}

#[test]
fn a_linked_worktree_uses_its_own_index() {
    let d = repo("linked");
    let wt = d.0.with_extension("wt");
    let _wt = Dir(wt.clone());
    g(&d.0, &["worktree", "add", "-q", "-b", "agent", wt.to_str().unwrap()]);
    std::fs::write(wt.join("a.txt"), "agent\n").unwrap();
    std::fs::write(d.0.join("a.txt"), "main\n").unwrap();
    let before = (state(&d.0), state(&wt));
    let s = review::snapshot(&wt).unwrap();
    assert_eq!((state(&d.0), state(&wt)), before);
    assert!(files(&d.0, &s.commit).contains(&"a.txt:agent".to_owned()));
}

#[test]
fn a_sparse_checkout_keeps_the_files_outside_it() {
    let d = repo("sparse");
    std::fs::create_dir_all(d.0.join("in")).unwrap();
    std::fs::create_dir_all(d.0.join("out")).unwrap();
    std::fs::write(d.0.join("in/x.txt"), "x\n").unwrap();
    std::fs::write(d.0.join("out/y.txt"), "y\n").unwrap();
    g(&d.0, &["add", "-A"]);
    g(&d.0, &["commit", "-q", "-m", "two"]);
    g(&d.0, &["sparse-checkout", "set", "in"]);
    assert!(!d.0.join("out/y.txt").exists());
    std::fs::write(d.0.join("in/x.txt"), "x2\n").unwrap();
    let s = review::snapshot(&d.0).unwrap();
    let all = files(&d.0, &s.commit);
    assert!(all.contains(&"out/y.txt:y".to_owned()) && all.contains(&"in/x.txt:x2".to_owned()), "{all:?}");
}

#[test]
fn a_held_index_lock_does_not_stop_it() {
    let d = repo("lock");
    std::fs::write(d.0.join("a.txt"), "a2\n").unwrap();
    // Another git command runs in the tree.
    let lock = PathBuf::from(g(&d.0, &["rev-parse", "--path-format=absolute", "--git-path", "index.lock"]));
    std::fs::write(&lock, "").unwrap();
    let s = review::snapshot(&d.0);
    std::fs::remove_file(&lock).unwrap();
    assert!(files(&d.0, &s.unwrap().commit).contains(&"a.txt:a2".to_owned()));
}

#[test]
fn a_snapshot_ref_does_not_hide_unpushed_commits_from_the_work_check() {
    // `local_changes` leaves out commits under refs/switchboard. A snapshot's parent
    // is the agent's HEAD, so its ref must live elsewhere (refs/sb).
    let d = repo("hide");
    let wt = d.0.with_extension("wt");
    let _wt = Dir(wt.clone());
    g(&d.0, &["worktree", "add", "-q", "--detach", wt.to_str().unwrap()]);
    std::fs::write(wt.join("a.txt"), "agent\n").unwrap();
    g(&wt, &["commit", "-q", "-am", "agent work"]);
    std::fs::write(wt.join("a.txt"), "more\n").unwrap();
    let s = review::snapshot(&wt).unwrap();
    g(&d.0, &["update-ref", &review::snap_ref("r1", "head"), &s.commit]);
    let changes = review::local_changes(&d.0, &wt).unwrap();
    assert!(changes.iter().any(|c| c.starts_with("commit ")), "{changes:?}");
}
