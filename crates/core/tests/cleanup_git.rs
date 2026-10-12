//! Review worktree cleanup against real git (SPEC 11.10): random states of a review
//! worktree, the close rule, a removal, and `git gc`. Work is never lost without a
//! choice, and the pinned commits stay for a later round.

use std::path::{Path, PathBuf};
use std::process::Command;
use switchboard_core::cleanup::{verdict, Facts, Verdict};
use switchboard_core::repos::git;
use switchboard_core::review;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn bit(&mut self) -> bool {
        self.next() & 1 == 1
    }
}

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(name: &str) -> Dir {
    let d = std::env::temp_dir().join(format!("sb-cleanup-{name}-{}", std::process::id()));
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

fn commit(dir: &Path, file: &str, text: &str) -> String {
    std::fs::write(dir.join(file), text).unwrap();
    g(dir, &["add", "-A"]);
    g(dir, &["commit", "-q", "-m", text]);
    g(dir, &["rev-parse", "HEAD"])
}

fn exists(dir: &Path, oid: &str) -> bool {
    git(dir, &["cat-file", "-e", oid]).is_ok()
}

/// Drops every object that no ref keeps, as a gc two weeks later would.
fn gc(dir: &Path) {
    g(dir, &["reflog", "expire", "--expire=now", "--all"]);
    g(dir, &["gc", "-q", "--prune=now"]);
}

/// A main checkout with a base commit and a PR head on `sb-pr-1`, and the review
/// worktree of that head.
fn setup(root: &Path) -> (PathBuf, String, String, PathBuf) {
    let main = root.join("repo");
    std::fs::create_dir_all(&main).unwrap();
    g(&main, &["init", "-q", "-b", "main"]);
    std::fs::write(main.join(".gitignore"), "target/\n").unwrap();
    let base = commit(&main, "a.rs", "fn a() {}\n");
    g(&main, &["checkout", "-q", "-b", "sb-pr-1"]);
    let head = commit(&main, "b.rs", "fn b() {}\n");
    g(&main, &["checkout", "-q", "main"]);
    let tree = review::checkout_tree(&main, "pr1", &head).unwrap();
    (main, base, head, tree)
}

#[test]
fn random_worktrees_never_lose_work_and_pins_survive_gc() {
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let mut seen = [0usize; 4];
    for i in 0..40 {
        let root = scratch(&format!("rand{i}"));
        let (main, base, head, tree) = setup(&root.0);
        review::pin(&main, &format!("r{i}"), &head, &base).unwrap();
        // Random work in the worktree.
        let edit = rng.bit();
        let untracked = rng.bit();
        let ignored = rng.bit();
        let local_commit = rng.next().is_multiple_of(4);
        if edit {
            std::fs::write(tree.join("b.rs"), "fn b() { edited }\n").unwrap();
        }
        if untracked {
            std::fs::write(tree.join("notes.txt"), "mine\n").unwrap();
        }
        if ignored {
            std::fs::create_dir_all(tree.join("target")).unwrap();
            std::fs::write(tree.join("target/big"), "build output\n").unwrap();
        }
        let mut made = None;
        if local_commit {
            made = Some(commit(&tree, "c.rs", "fn c() {}\n"));
        }
        // A force-push moves the PR branch away from the reviewed head.
        if rng.bit() {
            g(&main, &["branch", "-f", "sb-pr-1", &base]);
        }
        let state = review::tree_state(&main, &tree).unwrap();
        assert!(state.exists && state.worktree && state.detached && !state.locked, "{state:?}");
        let work0 = edit || untracked || local_commit;
        assert_eq!(!state.changes.is_empty(), work0, "ignored files are not work: {state:?}");
        // You look at the list, and maybe choose Discard. Then maybe a new file comes.
        let discard = rng.bit();
        let seen_list = if discard { state.changes.clone() } else { vec![] };
        let late = rng.next().is_multiple_of(3);
        if late {
            std::fs::write(tree.join("late.txt"), "after you looked\n").unwrap();
        }
        let state = review::tree_state(&main, &tree).unwrap();
        let work = work0 || late;
        let facts = Facts {
            tree: state,
            in_store: true,
            open_review: false,
            panes: vec![],
            discard: seen_list,
        };
        let v = verdict(&facts);
        match &v {
            Verdict::Remove { force } => {
                review::remove_tree(&main, &tree, *force).unwrap();
                assert!(!tree.exists());
                assert!(!work || (discard && !late), "work went without a choice");
                seen[if *force { 1 } else { 0 }] += 1;
            }
            Verdict::Ask(c) => {
                assert!(work && (!discard || late) && !c.is_empty());
                if discard && late {
                    assert_eq!(c, &vec!["?? late.txt".to_owned()], "only the change you did not see");
                }
                assert!(tree.exists());
                seen[2] += 1;
            }
            other => panic!("unexpected {other:?}"),
        }
        gc(&main);
        // The pinned head and base survive the removal, the force-push and the gc.
        assert!(exists(&main, &head) && exists(&main, &base), "pin lost a commit");
        // Kept work is still there.
        if let Verdict::Ask(_) = v {
            if edit {
                assert!(std::fs::read_to_string(tree.join("b.rs")).unwrap().contains("edited"));
            }
            if untracked {
                assert!(tree.join("notes.txt").exists());
            }
            if let Some(c) = &made {
                assert!(exists(&main, c), "a kept worktree keeps its commit");
            }
            seen[3] += 1;
        }
    }
    assert!(seen.iter().all(|n| *n > 0), "every path ran: {seen:?}");
}

/// A round after its round before was closed: the old worktree is gone, the author
/// rebased and force-pushed, and gc ran. The pinned head still gives the round base.
#[test]
fn a_round_after_close_force_push_and_gc() {
    let root = scratch("round");
    let (main, base, head1, tree) = setup(&root.0);
    review::pin(&main, "r1", &head1, &base).unwrap();
    review::remove_tree(&main, &tree, false).unwrap();
    g(&main, &["branch", "-D", "sb-pr-1"]);
    // The base branch moves on, and the author rebases onto it and adds a fix.
    let new_base = commit(&main, "a.rs", "fn a() { new }\n");
    g(&main, &["checkout", "-q", "-b", "sb-pr-1"]);
    commit(&main, "b.rs", "fn b() {}\n");
    let head2 = commit(&main, "b.rs", "fn b() { fixed }\n");
    g(&main, &["checkout", "-q", "main"]);
    gc(&main);
    assert!(exists(&main, &head1), "the pin keeps the old head");
    let (b, note) = review::round_base(&main, &head1, &new_base, &head2).unwrap();
    assert!(note.is_some(), "a rebase replays the old head");
    // The round shows only the fix: b.rs, not the base branch change in a.rs.
    let files = git(&main, &["diff", "--name-only", &b, &head2]).unwrap();
    assert_eq!(files, "b.rs");
    // The replayed base is a tree with no ref: pin it, and it survives a gc too.
    review::pin(&main, "r2", &head2, &b).unwrap();
    gc(&main);
    assert!(exists(&main, &b));
}

/// The tree state of folders that are not a review worktree.
#[test]
fn only_detached_worktrees_count() {
    let root = scratch("kinds");
    let (main, _base, _head, tree) = setup(&root.0);
    // The main checkout is never a worktree to remove.
    let s = review::tree_state(&main, &main).unwrap();
    assert!(!s.worktree);
    // A worktree with a branch is not detached.
    let feat = root.0.join("feat");
    g(&main, &["worktree", "add", "-q", "-b", "feat", &feat.to_string_lossy()]);
    let s = review::tree_state(&main, &feat).unwrap();
    assert!(s.worktree && !s.detached);
    assert!(matches!(
        verdict(&Facts { tree: s, in_store: true, ..Facts::default() }),
        Verdict::Keep(_)
    ));
    // A removed folder is gone.
    review::remove_tree(&main, &tree, false).unwrap();
    let s = review::tree_state(&main, &tree).unwrap();
    assert!(!s.exists && !s.worktree);
    assert_eq!(verdict(&Facts { tree: s, in_store: true, ..Facts::default() }), Verdict::Gone);
    // A path through a symlink finds the same worktree (macOS /var).
    let link = root.0.join("link");
    std::os::unix::fs::symlink(&root.0, &link).unwrap();
    let s = review::tree_state(&main, &link.join("feat")).unwrap();
    assert!(s.worktree);
}

/// A locked worktree stays, also with Discard. A kept folder with your work, or a
/// folder of yours with the same name, refuses a new checkout.
#[test]
fn locks_kept_work_and_foreign_folders_are_safe() {
    let root = scratch("lock");
    let (main, base, head, tree) = setup(&root.0);
    review::pin(&main, "r1", &head, &base).unwrap();
    // Locked: Keep, and a forced removal is refused by git too.
    g(&main, &["worktree", "lock", &tree.to_string_lossy()]);
    std::fs::write(tree.join("notes.txt"), "mine\n").unwrap();
    let s = review::tree_state(&main, &tree).unwrap();
    assert!(s.locked);
    let f = Facts { tree: s.clone(), in_store: true, discard: s.changes.clone(), ..Facts::default() };
    assert!(matches!(verdict(&f), Verdict::Keep(_)));
    assert!(review::remove_tree(&main, &tree, true).is_err(), "one --force keeps a lock");
    g(&main, &["worktree", "unlock", &tree.to_string_lossy()]);
    // A kept folder with work: a new review of the same round refuses to check out.
    let err = review::checkout_tree(&main, "pr1", &head).unwrap_err();
    assert!(err.contains("notes.txt"), "{err}");
    assert!(tree.join("notes.txt").exists());
    // A commit made in it counts as work until a ref reaches it.
    std::fs::remove_file(tree.join("notes.txt")).unwrap();
    let c = commit(&tree, "c.rs", "fn c() {}\n");
    let work = review::local_changes(&main, &tree).unwrap();
    assert!(work.len() == 1 && work[0].contains(&c[..12]), "{work:?}");
    g(&main, &["branch", "keep-c", &c]);
    assert!(review::local_changes(&main, &tree).unwrap().is_empty(), "a branch keeps the commit");
    // A pinned head that is not on any branch is not work either.
    g(&main, &["branch", "-D", "sb-pr-1"]);
    g(&tree, &["checkout", "-q", "--detach", &head]);
    assert!(review::local_changes(&main, &tree).unwrap().is_empty(), "the pin reaches the head");
    // With no work, the checkout goes on.
    assert_eq!(review::checkout_tree(&main, "pr1", &base).unwrap(), tree);
    // A folder of yours with the review's name, with a branch: refused, untouched.
    review::remove_tree(&main, &tree, false).unwrap();
    g(&main, &["worktree", "add", "-q", "-b", "mine", &tree.to_string_lossy()]);
    let err = review::checkout_tree(&main, "pr1", &head).unwrap_err();
    assert!(err.contains("not a review worktree"), "{err}");
    assert_eq!(g(&tree, &["rev-parse", "--abbrev-ref", "HEAD"]), "mine");
}

#[test]
fn same_path_sees_through_symlinks() {
    let root = scratch("same");
    let real = root.0.join("real");
    std::fs::create_dir_all(&real).unwrap();
    let link = root.0.join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(review::same_path(&real, &link));
    assert!(review::same_path(&root.0.join("gone"), &root.0.join("gone")));
    assert!(!review::same_path(&real, &root.0.join("other")));
}
