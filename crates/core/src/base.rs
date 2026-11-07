//! The base of a review of local work (SPEC 11.11): a ref you chose, the parent of a
//! fork, or the branch that HEAD pushes to or pulls from. `review::local_range` uses
//! these rules; the Base menu of the window lists the refs they find.

use crate::repos::git;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

/// What GitHub says about the repo of HEAD's remote.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum Fork {
    /// No answer: no gh, no network, or not a GitHub remote.
    #[default]
    Unknown,
    /// Not a fork.
    No,
    /// A fork of `repo` (`owner/name`), whose default branch is `branch`.
    Of { repo: String, branch: String },
}

/// How a local review picks its base.
#[derive(Debug, Clone, Default)]
pub struct BaseRule {
    /// The ref you chose. It goes first while it exists.
    pub pick: Option<String>,
    pub fork: Fork,
}

/// A ref that can be the base, and why.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub name: String,
    pub why: String,
}

/// One entry of the Base menu.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct BaseOption {
    #[serde(rename = "ref")]
    pub name: String,
    /// Why it is in the list, for example "sigp/lighthouse, the parent of this fork".
    pub why: String,
}

/// The Base menu lists at most this many refs.
pub const MAX_OPTIONS: usize = 16;

fn ok(tree: &Path, args: &[&str]) -> Option<String> {
    git(tree, args).ok().filter(|s| !s.is_empty())
}

/// True when `name` names a commit in the repo.
pub fn exists(tree: &Path, name: &str) -> bool {
    !name.starts_with('-') && ok(tree, &["rev-parse", "--verify", "--quiet", &format!("{name}^{{commit}}")]).is_some()
}

/// The names of the remotes.
pub fn remotes(tree: &Path) -> Vec<String> {
    ok(tree, &["remote"]).unwrap_or_default().lines().map(str::to_owned).collect()
}

/// The remote of a remote branch name like `origin/unstable`, and the branch: the
/// longest remote name that starts it, so `a/b/x` finds the remote `a/b` before `a`.
pub fn split_remote<'a>(name: &'a str, remotes: &[String]) -> Option<(String, &'a str)> {
    remotes
        .iter()
        .filter(|r| name.len() > r.len() + 1 && name.starts_with(r.as_str()) && name.as_bytes()[r.len()] == b'/')
        .max_by_key(|r| r.len())
        .map(|r| (r.clone(), &name[r.len() + 1..]))
}

/// `owner/name` of a GitHub remote URL: `git@github.com:o/n.git`,
/// `https://github.com/o/n`, `ssh://git@github.com/o/n.git`.
pub fn github_repo(url: &str) -> Option<String> {
    if !url.to_lowercase().contains("github.com") {
        return None;
    }
    let path = url.trim_end_matches('/').trim_end_matches(".git").trim_end_matches('/');
    let tail: Vec<&str> = path.rsplit(['/', ':']).take(2).collect();
    (tail.len() == 2 && !tail[0].is_empty() && !tail[1].is_empty()).then(|| format!("{}/{}", tail[1], tail[0]))
}

/// The remote branches that HEAD pushes to and pulls from (`@{push}`, `@{upstream}`),
/// as `origin/x`, when they exist.
pub fn tracked(tree: &Path) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = vec![];
    for (up, why) in [("@{push}", "where this branch pushes"), ("@{upstream}", "the upstream of this branch")] {
        let Some(name) = ok(tree, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", up]) else { continue };
        if exists(tree, &name) && !out.iter().any(|c| c.name == name) {
            out.push(Candidate { name, why: why.into() });
        }
    }
    out
}

/// The GitHub `owner/name` of the remote that HEAD pushes to or pulls from. The hub
/// asks gh whether it is a fork.
pub fn head_github(tree: &Path) -> Option<String> {
    let remotes = remotes(tree);
    let first = tracked(tree).into_iter().next()?;
    let (remote, _) = split_remote(&first.name, &remotes)?;
    github_repo(&ok(tree, &["remote", "get-url", &remote])?)
}

/// The refs the rules try, in order, before the rules of the commit history:
/// 1. when HEAD's remote is a fork, the parent's default branch on the local remote
///    of the parent. With no answer from GitHub, a remote named `upstream`, with the
///    branch of HEAD's upstream, else its default branch;
/// 2. the push branch, then the upstream branch of HEAD.
pub fn auto_candidates(tree: &Path, fork: &Fork) -> Vec<Candidate> {
    let remotes = remotes(tree);
    let tracked = tracked(tree);
    let mut out: Vec<Candidate> = vec![];
    if let Some((remote, branch)) = tracked.first().and_then(|c| split_remote(&c.name, &remotes)) {
        match fork {
            Fork::Of { repo, branch: main } => {
                let text = ok(tree, &["remote", "-v"]).unwrap_or_default();
                if let Some(parent) = crate::review::remote_for(&text, repo).filter(|p| *p != remote) {
                    let name = format!("{parent}/{main}");
                    if exists(tree, &name) {
                        out.push(Candidate { name, why: format!("{repo}, the parent of this fork") });
                    }
                }
            }
            Fork::Unknown if remote != "upstream" && remotes.iter().any(|r| r == "upstream") => {
                let head = ok(tree, &["symbolic-ref", "--short", "-q", "refs/remotes/upstream/HEAD"]);
                let tries = [Some(format!("upstream/{branch}")), head, Some("upstream/main".into()), Some("upstream/master".into())];
                if let Some(name) = tries.into_iter().flatten().find(|n| exists(tree, n)) {
                    out.push(Candidate { name, why: "the remote named upstream, often the parent of a fork".into() });
                }
            }
            _ => {}
        }
    }
    for c in tracked {
        if !out.iter().any(|o| o.name == c.name) {
            out.push(c);
        }
    }
    out
}

/// The refs of the Base menu: your choice, the ref in use, the rules' refs, and for
/// each remote its HEAD, `main`, `master` and `unstable` when they exist. At most
/// `MAX_OPTIONS`.
pub fn options(tree: &Path, rule: &BaseRule, used: Option<&str>) -> Vec<BaseOption> {
    let mut out: Vec<BaseOption> = vec![];
    let mut add = |name: &str, why: &str| {
        if out.len() < MAX_OPTIONS && !out.iter().any(|o| o.name == name) {
            out.push(BaseOption { name: name.to_owned(), why: why.to_owned() });
        }
    };
    if let Some(p) = rule.pick.as_deref().filter(|p| exists(tree, p)) {
        add(p, "your choice");
    }
    if let Some(u) = used {
        add(u, "in use");
    }
    for c in auto_candidates(tree, &rule.fork) {
        add(&c.name, &c.why);
    }
    // One read of the remote refs, with the target of each `<remote>/HEAD`.
    let refs = ok(tree, &["for-each-ref", "--format=%(refname)\t%(symref)", "refs/remotes"]).unwrap_or_default();
    let list: Vec<(&str, &str)> = refs
        .lines()
        .filter_map(|l| {
            let (name, sym) = l.split_once('\t').unwrap_or((l, ""));
            Some((name.strip_prefix("refs/remotes/")?, sym.strip_prefix("refs/remotes/").unwrap_or("")))
        })
        .collect();
    for r in remotes(tree) {
        if let Some((_, target)) = list.iter().find(|(n, t)| *n == format!("{r}/HEAD") && !t.is_empty()) {
            add(target, &format!("the default branch of {r}"));
        }
        for b in ["main", "master", "unstable"] {
            let name = format!("{r}/{b}");
            if list.iter().any(|(n, _)| *n == name) {
                add(&name, &format!("a branch of {r}"));
            }
        }
    }
    out
}

/// When the remote branch `name` was last fetched, in seconds since the epoch: the
/// newer of its last reflog entry (a fetch that moved it) and the time of
/// `FETCH_HEAD` when that names it (a fetch that found it as it was). None for a
/// ref that is not a remote branch.
pub fn fetched_at(tree: &Path, name: &str) -> Option<u64> {
    let full = format!("refs/remotes/{name}");
    if !exists(tree, &full) {
        return None;
    }
    let moved = ok(tree, &["reflog", "show", "-n1", "--date=unix", "--format=%gd", &full]).and_then(|s| {
        let at = s.rsplit_once("@{")?.1.trim_end_matches('}');
        at.parse::<u64>().ok()
    });
    let seen = (|| {
        let (remote, branch) = split_remote(name, &remotes(tree))?;
        let url = ok(tree, &["remote", "get-url", &remote])?;
        let url = url.trim_end_matches('/').trim_end_matches(".git");
        let path = ok(tree, &["rev-parse", "--path-format=absolute", "--git-path", "FETCH_HEAD"])?;
        let text = std::fs::read_to_string(&path).ok()?;
        let named = text.lines().any(|l| l.contains(&format!("branch '{branch}' of ")) && l.contains(url));
        if !named {
            return None;
        }
        let m = std::fs::metadata(&path).ok()?.modified().ok()?;
        Some(m.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs())
    })();
    moved.max(seen)
}

/// The folder that every worktree of the repo shares: the main checkout, or the git
/// folder of a bare repo. A choice of base is remembered for it.
pub fn repo_root(tree: &Path) -> Option<String> {
    let common = ok(tree, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let p = Path::new(&common);
    let root = if p.file_name().is_some_and(|n| n == ".git") { p.parent()? } else { p };
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    Some(root.to_string_lossy().into_owned())
}

/// Checks a ref you typed. It must not look like an option, and it must name a
/// commit here. Returns the commit.
pub fn verify(tree: &Path, name: &str) -> Result<String, String> {
    if name.is_empty() {
        return Err("Name a branch or a commit.".into());
    }
    if name.starts_with('-') {
        return Err(format!("{name} is not a ref: a ref never starts with -"));
    }
    if name.len() > 256 || name.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("A ref has no spaces and no control characters.".into());
    }
    git(tree, &["rev-parse", "--verify", "--quiet", &format!("{name}^{{commit}}")])
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{name} is not a branch or a commit in this repo. Fetch it first."))
}

/// Runs a command with no stdin and kills it after `secs`.
fn output_within(mut cmd: Command, secs: u64) -> Result<Output, String> {
    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(Duration::from_secs(secs)) {
        Ok(out) => out.map_err(|e| e.to_string()),
        Err(_) => {
            // The thread still owns the child, so it is not reaped yet: the pid is still ours.
            // SAFETY: kill(2) takes plain integers and reads no memory.
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
            Err(format!("it did not end in {secs} s"))
        }
    }
}

/// Fetches only the branch of the remote ref `name`, for example `upstream/unstable`.
/// It never asks for a login, and stops after 2 minutes.
pub fn fetch_one(tree: &Path, name: &str) -> Result<(), String> {
    let (remote, branch) = split_remote(name, &remotes(tree)).ok_or(format!("{name} is not a branch of a remote."))?;
    if branch.starts_with('-') || remote.starts_with('-') {
        return Err(format!("{name} is not a branch of a remote."));
    }
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(tree).args(["fetch", "--no-tags", "-q", &remote, branch]).env("GIT_TERMINAL_PROMPT", "0");
    let out = output_within(cmd, 120).map_err(|e| format!("git fetch: {e}"))?;
    if !out.status.success() {
        return Err(format!("git fetch: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

/// Asks GitHub whether `repo` (`owner/name`) is a fork, and of what. It only reads,
/// and stops after 20 s.
pub fn gh_fork(repo: &str) -> Result<Fork, String> {
    let plain = |s: &str| s.starts_with(|c: char| c.is_ascii_alphanumeric()) && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    match repo.split_once('/') {
        Some((o, n)) if plain(o) && plain(n) => {}
        _ => return Err(format!("{repo} is not a GitHub owner/name")),
    }
    let mut cmd = Command::new("gh");
    cmd.args(["api", &format!("repos/{repo}")]);
    let out = output_within(cmd, 20).map_err(|e| format!("gh: {e}"))?;
    if !out.status.success() {
        return Err(format!("gh: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("gh: {e}"))?;
    Ok(parse_fork(&v))
}

/// The fork answer from GitHub's repo JSON.
pub fn parse_fork(v: &serde_json::Value) -> Fork {
    let parent = &v["parent"];
    match (v["fork"].as_bool(), parent["full_name"].as_str(), parent["default_branch"].as_str()) {
        (Some(true), Some(repo), Some(branch)) => Fork::Of { repo: repo.into(), branch: branch.into() },
        (Some(_), _, _) => Fork::No,
        _ => Fork::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_urls() {
        assert_eq!(github_repo("git@github.com:eserilev/lighthouse.git").as_deref(), Some("eserilev/lighthouse"));
        assert_eq!(github_repo("https://github.com/sigp/lighthouse").as_deref(), Some("sigp/lighthouse"));
        assert_eq!(github_repo("https://github.com/sigp/lighthouse.git/").as_deref(), Some("sigp/lighthouse"));
        assert_eq!(github_repo("ssh://git@github.com/a/b.git").as_deref(), Some("a/b"));
        assert_eq!(github_repo("/home/me/up"), None);
        assert_eq!(github_repo("https://gitlab.com/a/b"), None);
    }

    #[test]
    fn remote_names() {
        let r = vec!["origin".to_owned(), "a".to_owned(), "a/b".to_owned()];
        assert_eq!(split_remote("origin/unstable", &r), Some(("origin".into(), "unstable")));
        assert_eq!(split_remote("a/b/x", &r), Some(("a/b".into(), "x")));
        assert_eq!(split_remote("a/x/y", &r), Some(("a".into(), "x/y")));
        assert_eq!(split_remote("origin", &r), None);
        assert_eq!(split_remote("originx/y", &r), None);
    }

    #[test]
    fn fork_answers() {
        let v = serde_json::json!({"fork": true, "parent": {"full_name": "sigp/lighthouse", "default_branch": "unstable"}});
        assert_eq!(parse_fork(&v), Fork::Of { repo: "sigp/lighthouse".into(), branch: "unstable".into() });
        assert_eq!(parse_fork(&serde_json::json!({"fork": false})), Fork::No);
        assert_eq!(parse_fork(&serde_json::json!({"message": "Not Found"})), Fork::Unknown);
    }

    #[test]
    fn gh_never_runs_for_a_strange_name() {
        assert!(gh_fork("a/b/../c").is_err());
        assert!(gh_fork("-x/y").is_err());
        assert!(gh_fork("nope").is_err());
    }
}
