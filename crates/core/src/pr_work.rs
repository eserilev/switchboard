//! Work on a PR (SPEC 11.16): a worktree on the PR's own branch, for a new agent.
//! With a remote of yours that holds the head branch, the worktree's branch tracks
//! it, so the agent can push. With none (a PR from a fork you do not have), it is
//! a local branch `pr-N` at the PR head, which cannot push.

use crate::repos::{git, worktree_path};
use std::path::{Path, PathBuf};

/// Where the PR's head lives.
pub struct Head<'a> {
    pub number: u64,
    /// The head branch, for example `redb-fixes`.
    pub branch: &'a str,
    /// `owner/name` of the head repo, when known.
    pub repo: Option<&'a str>,
    /// The remote of the base repo, for `pull/N/head`.
    pub base_remote: &'a str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    pub tree: PathBuf,
    pub branch: String,
    /// The remote that the branch pushes to. `None`: it cannot push.
    pub push: Option<String>,
    /// True when a worktree had the branch already: it is used as it is.
    pub reused: bool,
    /// Something to tell the user: the branch could not move to the PR head.
    pub note: Option<String>,
}

/// The worktrees of `clone` and their branches.
fn worktrees(clone: &Path) -> Vec<(PathBuf, Option<String>)> {
    let text = git(clone, &["worktree", "list", "--porcelain"]).unwrap_or_default();
    let mut out = vec![];
    let mut path: Option<PathBuf> = None;
    for l in text.lines() {
        if let Some(p) = l.strip_prefix("worktree ") {
            if let Some(prev) = path.take() {
                out.push((prev, None));
            }
            path = Some(PathBuf::from(p));
        } else if let Some(b) = l.strip_prefix("branch refs/heads/") {
            if let Some(p) = path.take() {
                out.push((p, Some(b.to_owned())));
            }
        }
    }
    if let Some(p) = path {
        out.push((p, None));
    }
    out
}

/// Moves the branch of `tree` to `target` when that is a fast-forward. A note
/// when it is not: the local branch has commits the PR does not.
fn fast_forward(tree: &Path, target: &str) -> Option<String> {
    match git(tree, &["merge", "--ff-only", "-q", target]) {
        Ok(_) => None,
        Err(_) => Some("The branch here differs from the PR head, so it stays as it is. Pull or rebase in the pane if you need the PR head.".into()),
    }
}

/// Fetches the PR head and gives a worktree on its branch.
pub fn prepare(clone: &Path, head: &Head) -> Result<Prepared, String> {
    let mine = head.repo.and_then(|r| crate::review::find_remote(clone, r)).filter(|_| !head.branch.is_empty());
    let (target, branch, push) = match &mine {
        Some(remote) => {
            let b = head.branch;
            git(clone, &["fetch", "-q", remote, &format!("+refs/heads/{b}:refs/remotes/{remote}/{b}")])
                .map_err(|e| format!("git fetch {remote} {b}: {e}"))?;
            (format!("refs/remotes/{remote}/{b}"), b.to_owned(), Some(remote.clone()))
        }
        None => {
            let n = head.number;
            let r = head.base_remote;
            git(clone, &["fetch", "-q", r, &format!("+refs/pull/{n}/head:refs/remotes/sb-pr/{n}")])
                .map_err(|e| format!("git fetch {r} pull/{n}/head: {e}"))?;
            (format!("refs/remotes/sb-pr/{n}"), format!("pr-{n}"), None)
        }
    };
    // A worktree with the branch already: use it.
    if let Some((tree, _)) = worktrees(clone).into_iter().find(|(_, b)| b.as_deref() == Some(branch.as_str())) {
        let note = fast_forward(&tree, &target);
        return Ok(Prepared { tree, branch, push, reused: true, note });
    }
    let tree = worktree_path(clone, &branch);
    if tree.exists() {
        return Err(format!("{} exists and is not a worktree of {branch}. Move it, then try again.", tree.display()));
    }
    let p = tree.to_string_lossy().into_owned();
    let exists = git(clone, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).is_ok();
    let mut note = None;
    if exists {
        git(clone, &["worktree", "add", "-q", &p, &branch])?;
        note = fast_forward(&tree, &target);
    } else {
        git(clone, &["worktree", "add", "-q", "-b", &branch, &p, &target])?;
    }
    if let Some(remote) = &push {
        let _ = git(&tree, &["branch", "-q", &format!("--set-upstream-to={remote}/{branch}")]);
    }
    Ok(Prepared { tree, branch, push, reused: false, note })
}

/// The first message of the new agent: where it is, and the task.
pub fn first_message(number: u64, title: &str, p: &Prepared, task: &str) -> String {
    let push = match &p.push {
        Some(r) => format!("It tracks {r}/{}: push there when the user asks.", p.branch),
        None => "No remote of the user's holds the PR branch, so this branch cannot push.".to_owned(),
    };
    let mut m = format!("You work on PR #{number} ({}) in this worktree, on branch {}. {push}", title.replace('\n', " "), p.branch);
    let task = task.trim();
    if !task.is_empty() {
        m.push_str(&format!(" The task: {}", task.replace('\n', " ")));
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn run(dir: &Path, args: &[&str]) {
        let ok = Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=t", "-c", "user.email=t@t"]).args(args).status().unwrap().success();
        assert!(ok, "git {args:?}");
    }

    /// A "GitHub" repo with a PR ref and a fork with the head branch, and a clone
    /// with both as remotes.
    fn setup(name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("sb-prwork-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("up")).unwrap();
        // macOS: the temp folder is a link (/var -> /private/var), and git gives the real path.
        let root = root.canonicalize().unwrap();
        let up = root.join("up");
        run(&up, &["init", "-q", "-b", "main"]);
        run(&up, &["commit", "-q", "--allow-empty", "-m", "base"]);
        run(&root, &["clone", "-q", up.to_str().unwrap(), "fork"]);
        let fork = root.join("fork");
        run(&fork, &["checkout", "-q", "-b", "redb-fixes"]);
        run(&fork, &["commit", "-q", "--allow-empty", "-m", "fix"]);
        // The PR ref on the upstream, as GitHub keeps it.
        run(&up, &["fetch", "-q", fork.to_str().unwrap(), "redb-fixes:refs/pull/7/head"]);
        run(&root, &["clone", "-q", up.to_str().unwrap(), "lh"]);
        let clone = root.join("lh");
        // `find_remote` reads GitHub URLs: give the remotes such URLs, and point
        // git at the folders with insteadOf.
        run(&clone, &["remote", "set-url", "origin", "https://github.com/sigp/lighthouse.git"]);
        run(&clone, &["remote", "add", "me", "git@github.com:me/lighthouse.git"]);
        run(&clone, &["config", &format!("url.{}.insteadOf", up.display()), "https://github.com/sigp/lighthouse.git"]);
        run(&clone, &["config", &format!("url.{}.insteadOf", fork.display()), "git@github.com:me/lighthouse.git"]);
        (root, clone)
    }

    fn head_of(tree: &Path) -> String {
        git(tree, &["log", "-1", "--format=%s"]).unwrap().trim().to_owned()
    }

    #[test]
    fn your_own_pr_gets_a_worktree_on_its_branch_that_pushes() {
        let (root, clone) = setup("own");
        let h = Head { number: 7, branch: "redb-fixes", repo: Some("me/lighthouse"), base_remote: "origin" };
        let p = prepare(&clone, &h).unwrap();
        assert_eq!((p.branch.as_str(), p.push.as_deref(), p.reused), ("redb-fixes", Some("me"), false));
        assert_eq!(p.tree, root.join("lh-redb-fixes"));
        assert_eq!(head_of(&p.tree), "fix");
        assert_eq!(git(&p.tree, &["rev-parse", "--abbrev-ref", "@{upstream}"]).unwrap().trim(), "me/redb-fixes");
        // Again: the same worktree, used as it is.
        let again = prepare(&clone, &h).unwrap();
        assert_eq!((again.tree.clone(), again.reused, again.note), (p.tree.clone(), true, None));
        let msg = first_message(7, "Fix redb", &p, "Make compact() commit first.");
        assert!(msg.contains("branch redb-fixes") && msg.contains("me/redb-fixes") && msg.ends_with("The task: Make compact() commit first."), "{msg}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pr_from_a_fork_you_lack_gets_pr_n_at_its_head() {
        let (root, clone) = setup("fork");
        let h = Head { number: 7, branch: "redb-fixes", repo: Some("someone/lighthouse"), base_remote: "origin" };
        let p = prepare(&clone, &h).unwrap();
        assert_eq!((p.branch.as_str(), p.push.clone()), ("pr-7", None));
        assert_eq!(head_of(&p.tree), "fix");
        assert!(first_message(7, "t", &p, "").contains("cannot push"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
