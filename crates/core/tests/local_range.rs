//! The range of a review of local commits (SPEC 11.11), against real git: each rule
//! of `review::local_range`.

use std::path::{Path, PathBuf};
use std::process::Command;
use switchboard_core::review::{self, EMPTY_TREE};

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(name: &str) -> Dir {
    let d = std::env::temp_dir().join(format!("sb-range-{name}-{}", std::process::id()));
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

fn commit(dir: &Path, text: &str) -> String {
    std::fs::write(dir.join("f.txt"), text).unwrap();
    g(dir, &["add", "-A"]);
    g(dir, &["commit", "-q", "-m", text]);
    g(dir, &["rev-parse", "HEAD"])
}

/// An "origin" with two commits on main, and a clone of it.
fn remote_and_clone(root: &Path) -> (PathBuf, String) {
    let up = root.join("up");
    std::fs::create_dir_all(&up).unwrap();
    g(&up, &["init", "-q", "-b", "main"]);
    commit(&up, "1");
    let tip = commit(&up, "2");
    g(root, &["clone", "-q", &up.to_string_lossy(), "work"]);
    (root.join("work"), tip)
}

#[test]
fn a_branch_with_an_upstream_shows_its_unpushed_commits() {
    let root = scratch("up");
    let (work, tip) = remote_and_clone(&root.0);
    commit(&work, "3");
    let head = commit(&work, "4");
    let r = review::local_range(&work).unwrap();
    assert_eq!((r.base.as_str(), r.head.as_str(), r.count), (tip.as_str(), head.as_str(), 2));
    assert!(r.note.contains("origin/main"), "{}", r.note);
}

#[test]
fn a_detached_agent_worktree_shows_the_commits_no_remote_has() {
    let root = scratch("detached");
    let (work, tip) = remote_and_clone(&root.0);
    let wt = root.0.join("agent");
    g(&work, &["worktree", "add", "-q", "--detach", &wt.to_string_lossy()]);
    let head = commit(&wt, "agent work");
    let r = review::local_range(&wt).unwrap();
    assert_eq!((r.base.as_str(), r.head.as_str(), r.count), (tip.as_str(), head.as_str(), 1));
    assert!(r.note.contains("no remote"), "{}", r.note);
}

#[test]
fn a_new_branch_with_a_merge_of_remote_work_uses_the_default_branch() {
    let root = scratch("merge");
    let (work, tip) = remote_and_clone(&root.0);
    g(&work, &["checkout", "-q", "-b", "feat"]);
    commit(&work, "feat 1");
    // Remote work that is not on main yet, merged into the branch: two edges.
    let up = root.0.join("up");
    g(&up, &["checkout", "-q", "-b", "other"]);
    std::fs::write(up.join("other.txt"), "other").unwrap();
    g(&up, &["add", "-A"]);
    g(&up, &["commit", "-q", "-m", "other"]);
    g(&up, &["checkout", "-q", "main"]);
    g(&work, &["fetch", "-q"]);
    g(&work, &["merge", "-q", "--no-edit", "origin/other"]);
    let r = review::local_range(&work).unwrap();
    assert_eq!(r.base, tip);
    assert!(r.note.contains("origin/main") || r.note.contains("origin/HEAD"), "{}", r.note);
}

#[test]
fn with_no_remote_a_branch_compares_with_main() {
    let root = scratch("noremote");
    let d = root.0.clone();
    g(&d, &["init", "-q", "-b", "main"]);
    let base = commit(&d, "1");
    g(&d, &["checkout", "-q", "-b", "feat"]);
    commit(&d, "2");
    let head = commit(&d, "3");
    let r = review::local_range(&d).unwrap();
    assert_eq!((r.base.as_str(), r.head.as_str(), r.count), (base.as_str(), head.as_str(), 2));
    assert!(r.note.contains("main"), "{}", r.note);
}

#[test]
fn nothing_unpushed_shows_the_last_commit() {
    // On main with no remote: nothing to compare with.
    let root = scratch("nothing");
    let d = root.0.clone();
    g(&d, &["init", "-q", "-b", "main"]);
    let first = commit(&d, "1");
    let head = commit(&d, "2");
    let r = review::local_range(&d).unwrap();
    assert_eq!((r.base.as_str(), r.head.as_str(), r.count), (first.as_str(), head.as_str(), 1));
    assert!(r.note.starts_with("No unpushed commits"), "{}", r.note);
    // Everything pushed: the same.
    let root2 = scratch("pushed");
    let (work, tip) = remote_and_clone(&root2.0);
    let r = review::local_range(&work).unwrap();
    assert_eq!((r.head.as_str(), r.count), (tip.as_str(), 1));
    assert!(r.note.starts_with("No unpushed commits"), "{}", r.note);
}

#[test]
fn a_root_commit_compares_with_the_empty_tree() {
    let root = scratch("root");
    let d = root.0.clone();
    g(&d, &["init", "-q", "-b", "main"]);
    commit(&d, "only");
    let r = review::local_range(&d).unwrap();
    assert_eq!((r.base.as_str(), r.count), (EMPTY_TREE, 1));
    // The diff model takes the empty tree as a base.
    let m = switchboard_core::coverage::build(&d, &r.base, &r.head).unwrap();
    assert_eq!(m.files.len(), 1);
}

#[test]
fn no_commit_is_an_error() {
    let root = scratch("empty");
    g(&root.0, &["init", "-q", "-b", "main"]);
    assert!(review::local_range(&root.0).unwrap_err().contains("no commit"));
}

#[test]
fn a_shallow_clone_never_diffs_the_whole_repo() {
    let root = scratch("shallow");
    let (_, _) = remote_and_clone(&root.0);
    let url = format!("file://{}", root.0.join("up").display());
    g(&root.0, &["clone", "-q", "--depth", "1", &url, "thin"]);
    let err = review::local_range(&root.0.join("thin")).unwrap_err();
    assert!(err.contains("shallow"), "{err}");
}

#[test]
fn a_remote_not_named_origin_gives_the_default_branch() {
    let root = scratch("sigp");
    let (work, tip) = remote_and_clone(&root.0);
    g(&work, &["remote", "rename", "origin", "sigp"]);
    g(&work, &["checkout", "-q", "-b", "feat"]);
    commit(&work, "feat 1");
    let up = root.0.join("up");
    g(&up, &["checkout", "-q", "-b", "other"]);
    std::fs::write(up.join("other.txt"), "other").unwrap();
    g(&up, &["add", "-A"]);
    g(&up, &["commit", "-q", "-m", "other"]);
    g(&up, &["checkout", "-q", "main"]);
    g(&work, &["fetch", "-q", "sigp"]);
    g(&work, &["merge", "-q", "--no-edit", "sigp/other"]);
    let r = review::local_range(&work).unwrap();
    assert_eq!(r.base, tip);
    assert!(r.note.contains("sigp/"), "{}", r.note);
}
