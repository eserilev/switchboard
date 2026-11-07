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
    let r = review::local_range(&work, &Default::default()).unwrap();
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
    let r = review::local_range(&wt, &Default::default()).unwrap();
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
    let r = review::local_range(&work, &Default::default()).unwrap();
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
    let r = review::local_range(&d, &Default::default()).unwrap();
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
    let r = review::local_range(&d, &Default::default()).unwrap();
    assert_eq!((r.base.as_str(), r.head.as_str(), r.count), (first.as_str(), head.as_str(), 1));
    assert!(r.note.starts_with("No unpushed commits"), "{}", r.note);
    // Everything pushed: the same.
    let root2 = scratch("pushed");
    let (work, tip) = remote_and_clone(&root2.0);
    let r = review::local_range(&work, &Default::default()).unwrap();
    assert_eq!((r.head.as_str(), r.count), (tip.as_str(), 1));
    assert!(r.note.starts_with("No unpushed commits"), "{}", r.note);
}

#[test]
fn a_root_commit_compares_with_the_empty_tree() {
    let root = scratch("root");
    let d = root.0.clone();
    g(&d, &["init", "-q", "-b", "main"]);
    commit(&d, "only");
    let r = review::local_range(&d, &Default::default()).unwrap();
    assert_eq!((r.base.as_str(), r.count), (EMPTY_TREE, 1));
    // The diff model takes the empty tree as a base.
    let m = switchboard_core::coverage::build(&d, &r.base, &r.head).unwrap();
    assert_eq!(m.files.len(), 1);
}

#[test]
fn no_commit_is_an_error() {
    let root = scratch("empty");
    g(&root.0, &["init", "-q", "-b", "main"]);
    assert!(review::local_range(&root.0, &Default::default()).unwrap_err().contains("no commit"));
}

#[test]
fn a_shallow_clone_never_diffs_the_whole_repo() {
    let root = scratch("shallow");
    let (_, _) = remote_and_clone(&root.0);
    let url = format!("file://{}", root.0.join("up").display());
    g(&root.0, &["clone", "-q", "--depth", "1", &url, "thin"]);
    let err = review::local_range(&root.0.join("thin"), &Default::default()).unwrap_err();
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
    let r = review::local_range(&work, &Default::default()).unwrap();
    assert_eq!(r.base, tip);
    assert!(r.note.contains("sigp/"), "{}", r.note);
}

use switchboard_core::base::{self, BaseRule, Fork};

/// The owner's lighthouse: `origin` is the fork, `upstream` the parent. The fork's
/// `unstable` is 3 commits behind the parent's, and the branch `update-redb` tracks
/// the fork's. The remote refs are written by hand: nothing fetches, and gh never runs.
struct ForkRepo {
    dir: PathBuf,
    old: String,
    parent: String,
}

fn fork_repo(root: &Path) -> ForkRepo {
    let dir = root.join("lighthouse");
    std::fs::create_dir_all(&dir).unwrap();
    g(&dir, &["init", "-q", "-b", "unstable"]);
    let old = commit(&dir, "1");
    commit(&dir, "2");
    commit(&dir, "3");
    let parent = commit(&dir, "4");
    g(&dir, &["remote", "add", "origin", "git@github.com:eserilev/lighthouse.git"]);
    g(&dir, &["remote", "add", "upstream", "https://github.com/sigp/lighthouse"]);
    g(&dir, &["update-ref", "refs/remotes/origin/unstable", &old]);
    g(&dir, &["update-ref", "refs/remotes/upstream/unstable", &parent]);
    g(&dir, &["checkout", "-q", "-b", "update-redb"]);
    g(&dir, &["branch", "-q", "--set-upstream-to=origin/unstable"]);
    commit(&dir, "5");
    ForkRepo { dir, old, parent }
}

fn sigp() -> Fork {
    Fork::Of { repo: "sigp/lighthouse".into(), branch: "unstable".into() }
}

#[test]
fn a_branch_of_a_fork_compares_with_the_parent() {
    let root = scratch("fork");
    let f = fork_repo(&root.0);
    let r = review::local_range(&f.dir, &BaseRule { pick: None, fork: sigp() }).unwrap();
    assert_eq!((r.base.as_str(), r.count), (f.parent.as_str(), 1));
    assert_eq!(r.base_ref.as_deref(), Some("upstream/unstable"));
    assert_eq!(r.note, "Commits not on upstream/unstable (sigp/lighthouse, the parent of this fork).");
    assert_eq!(r.label.as_deref(), Some("upstream/unstable (sigp/lighthouse, the parent of this fork)"));
    assert_eq!(base::head_github(&f.dir).as_deref(), Some("eserilev/lighthouse"));
    // Not a fork: the upstream of the branch, as before.
    let r = review::local_range(&f.dir, &BaseRule { pick: None, fork: Fork::No }).unwrap();
    assert_eq!((r.base.as_str(), r.count), (f.old.as_str(), 4));
    assert!(r.note.contains("origin/unstable"), "{}", r.note);
}

#[test]
fn with_no_answer_from_github_a_remote_named_upstream_is_the_parent() {
    let root = scratch("forkname");
    let f = fork_repo(&root.0);
    let r = review::local_range(&f.dir, &BaseRule::default()).unwrap();
    assert_eq!((r.base.as_str(), r.count), (f.parent.as_str(), 1));
    assert!(r.note.contains("upstream/unstable") && r.note.contains("the remote named upstream"), "{}", r.note);
    // With no `unstable` there, its default branch.
    g(&f.dir, &["update-ref", "-d", "refs/remotes/upstream/unstable"]);
    g(&f.dir, &["update-ref", "refs/remotes/upstream/main", &f.parent]);
    let r = review::local_range(&f.dir, &BaseRule::default()).unwrap();
    assert_eq!(r.base_ref.as_deref(), Some("upstream/main"));
    // The parent named by GitHub, with no local remote for it: the branch's upstream.
    let other = Fork::Of { repo: "someone/else".into(), branch: "unstable".into() };
    let r = review::local_range(&f.dir, &BaseRule { pick: None, fork: other }).unwrap();
    assert_eq!(r.base_ref.as_deref(), Some("origin/unstable"));
}

#[test]
fn a_chosen_base_goes_first_and_a_lost_one_falls_back_with_a_note() {
    let root = scratch("forkpick");
    let f = fork_repo(&root.0);
    let rule = BaseRule { pick: Some("origin/unstable".into()), fork: sigp() };
    let r = review::local_range(&f.dir, &rule).unwrap();
    assert_eq!((r.base.as_str(), r.count), (f.old.as_str(), 4));
    assert_eq!(r.note, "Commits not on origin/unstable (your choice).");
    // A commit works too.
    let rule = BaseRule { pick: Some(f.parent.clone()), fork: Fork::No };
    assert_eq!(review::local_range(&f.dir, &rule).unwrap().count, 1);
    let rule = BaseRule { pick: Some("upstream/gone".into()), fork: sigp() };
    let r = review::local_range(&f.dir, &rule).unwrap();
    assert_eq!(r.base, f.parent);
    assert!(r.note.starts_with("The base you chose, upstream/gone, is not in this repo any more"), "{}", r.note);
    assert!(r.note.contains("Commits not on upstream/unstable"), "{}", r.note);
    // A pick that looks like an option is never passed to git as one.
    let rule = BaseRule { pick: Some("--all".into()), fork: sigp() };
    assert!(review::local_range(&f.dir, &rule).unwrap().note.starts_with("The base you chose, --all"));
}

#[test]
fn the_base_menu_lists_the_refs_once() {
    let root = scratch("forkmenu");
    let f = fork_repo(&root.0);
    g(&f.dir, &["update-ref", "refs/remotes/upstream/main", &f.parent]);
    g(&f.dir, &["update-ref", "refs/remotes/origin/feature", &f.old]);
    g(&f.dir, &["symbolic-ref", "refs/remotes/upstream/HEAD", "refs/remotes/upstream/unstable"]);
    let rule = BaseRule { pick: Some(f.old.clone()), fork: sigp() };
    let o = base::options(&f.dir, &rule, Some("upstream/unstable"));
    let names: Vec<&str> = o.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, [f.old.as_str(), "upstream/unstable", "origin/unstable", "upstream/main"], "{o:?}");
    assert_eq!((o[0].why.as_str(), o[1].why.as_str()), ("your choice", "in use"));
    assert!(!names.contains(&"origin/feature"));
}

#[test]
fn a_far_base_says_it_may_be_wrong() {
    let root = scratch("far");
    let d = root.0.clone();
    g(&d, &["init", "-q", "-b", "main"]);
    let base = commit(&d, "1");
    g(&d, &["checkout", "-q", "-b", "feat"]);
    // 501 commits in one fast-import.
    let mut stream = String::new();
    for i in 0..=review::FAR_BASE {
        let msg = format!("c{i}");
        stream += &format!("commit refs/heads/feat\ncommitter t <t@t> 1 +0000\ndata {}\n{msg}\n", msg.len());
        if i == 0 {
            stream += &format!("from {base}\n");
        }
        stream += "\n";
    }
    let mut child = Command::new("git")
        .arg("-C")
        .arg(&d)
        .args(["fast-import", "--quiet", "--force"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(stream.as_bytes()).unwrap();
    assert!(child.wait().unwrap().success());
    g(&d, &["reset", "-q", "--hard", "feat"]);
    let r = review::local_range(&d, &BaseRule::default()).unwrap();
    assert_eq!((r.base.as_str(), r.count), (base.as_str(), review::FAR_BASE + 1));
    assert!(r.note.contains("may be the wrong base"), "{}", r.note);
    // 500 is fine.
    g(&d, &["reset", "-q", "--hard", "feat~1"]);
    let r = review::local_range(&d, &BaseRule::default()).unwrap();
    assert!(!r.note.contains("wrong base"), "{}", r.note);
}

#[test]
fn update_base_fetches_one_branch_and_knows_when() {
    let root = scratch("fetchone");
    let (work, _) = remote_and_clone(&root.0);
    let up = root.0.join("up");
    g(&work, &["remote", "rename", "origin", "upstream"]);
    let t0 = switchboard_core::now();
    let moved = commit(&up, "3");
    assert_ne!(g(&work, &["rev-parse", "upstream/main"]), moved);
    base::fetch_one(&work, "upstream/main").unwrap();
    assert_eq!(g(&work, &["rev-parse", "upstream/main"]), moved);
    let at = base::fetched_at(&work, "upstream/main").unwrap();
    assert!(at + 5 >= t0 && at <= switchboard_core::now() + 5, "{at} {t0}");
    // A fetch that finds the branch as it was still counts, through FETCH_HEAD.
    base::fetch_one(&work, "upstream/main").unwrap();
    assert!(base::fetched_at(&work, "upstream/main").unwrap() >= at);
    // Only a branch of a remote.
    assert!(base::fetch_one(&work, "main").is_err());
    assert!(base::fetch_one(&work, "upstream/-x").is_err());
    assert_eq!(base::fetched_at(&work, "main"), None);
}

#[test]
fn a_typed_ref_must_be_a_commit_and_never_an_option() {
    let root = scratch("verify");
    let f = fork_repo(&root.0);
    assert_eq!(base::verify(&f.dir, "upstream/unstable").unwrap(), f.parent);
    assert!(base::verify(&f.dir, "-x").unwrap_err().contains("never starts with -"));
    assert!(base::verify(&f.dir, "--output=/tmp/x").is_err());
    assert!(base::verify(&f.dir, "nope").unwrap_err().contains("not a branch or a commit"));
    assert!(base::verify(&f.dir, "a b").is_err());
    assert!(base::verify(&f.dir, "").is_err());
}

#[test]
fn every_worktree_of_a_repo_has_one_root() {
    let root = scratch("roots");
    let f = fork_repo(&root.0);
    let wt = root.0.join("agent");
    g(&f.dir, &["worktree", "add", "-q", "--detach", &wt.to_string_lossy()]);
    let main = base::repo_root(&f.dir).unwrap();
    assert_eq!(base::repo_root(&wt).unwrap(), main);
    assert_eq!(Path::new(&main), std::fs::canonicalize(&f.dir).unwrap());
}
