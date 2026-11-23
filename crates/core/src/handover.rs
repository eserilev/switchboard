//! Handover (SPEC 38): an agent on the server continues on this computer.
//!
//! The server hub takes a snapshot of the agent's worktree: its commits and the
//! work that is not committed, as one commit under `refs/switchboard/handover/`.
//! This computer fetches that ref over ssh, makes a worktree at the agent's head,
//! puts the work that is not committed back as changes, and starts Claude Code with
//! `--resume` on a copy of the session file. Nothing goes to GitHub.

use crate::config::Config;
use crate::paths::Paths;
use crate::{expand, repos};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Where the snapshots live on the server, and the fetched ref on this computer.
pub const REF_PREFIX: &str = "refs/switchboard/handover/";
/// A snapshot ref on the server stays this long. Then the next handover removes it.
pub const REF_KEEP: u64 = 7 * 24 * 3600;
/// The largest session file a handover carries.
pub const SESSION_MAX: usize = 64 << 20;

/// What the server sends: enough for this computer to rebuild the work.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Plan {
    /// The agent's worktree on the server.
    pub tree: String,
    /// Its branch, or none on a detached head.
    pub branch: Option<String>,
    /// The commit the worktree is on.
    pub head: String,
    /// The snapshot: `head` when nothing is uncommitted, else a commit on top of it.
    pub snap: String,
    /// The ref on the server that holds `snap`.
    #[serde(rename = "ref")]
    pub refname: String,
    /// The fetch URLs of the repo's remotes. They find the clone on this computer.
    pub urls: Vec<String>,
    /// The Claude Code session, and its file. No file: a new session starts.
    pub session: Option<String>,
    pub session_data: Option<String>,
    /// The pane's name on the server.
    pub title: String,
    /// The files the snapshot changes against `head`, as `git diff --name-status`.
    pub changes: Vec<String>,
}

/// A handover that waits for the hub of this computer (SPEC 38.3).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Pending {
    pub repo: String,
    pub tree: String,
    pub branch: String,
    pub session: Option<String>,
    pub title: String,
    pub from: String,
    /// How many files the snapshot changes against the head.
    #[serde(default)]
    pub changes: usize,
}

/// Where the work lands on this computer: the answer of the dry run, which the
/// window shows before you confirm (SPEC 38.2).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Target {
    /// The clone here with a remote of the same repo.
    pub repo: String,
    pub tree: String,
    pub branch: String,
    /// Why the branch has another name than on the server, or none.
    pub note: Option<String>,
}

/// Runs git in `dir` with `env`, and gives its output, trimmed.
fn git_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The snapshot of `tree` (on the server). The work that is not committed, with new
/// files that git does not ignore, goes into a commit on top of HEAD, built in a
/// separate index: the worktree, its index and its branch do not change.
pub fn snapshot(tree: &Path, pane: &str) -> Result<Plan, String> {
    let head = git_env(tree, &["rev-parse", "--verify", "HEAD^{commit}"], &[]).map_err(|_| "the worktree has no commit".to_owned())?;
    let branch = git_env(tree, &["symbolic-ref", "--short", "-q", "HEAD"], &[]).ok().filter(|b| !b.is_empty());
    let index = std::env::temp_dir().join(format!("sb-handover-{}-{}.index", std::process::id(), now_nanos()));
    let idx = index.to_string_lossy().into_owned();
    let env = [("GIT_INDEX_FILE", idx.as_str())];
    let built = git_env(tree, &["read-tree", "HEAD"], &env)
        .and_then(|_| git_env(tree, &["add", "-A"], &env))
        .and_then(|_| git_env(tree, &["write-tree"], &env));
    let _ = std::fs::remove_file(&index);
    let built = built?;
    let head_tree = git_env(tree, &["rev-parse", "HEAD^{tree}"], &[])?;
    let snap = if built == head_tree {
        head.clone()
    } else {
        // A commit only to carry the work: it is never on a branch.
        let who = [
            ("GIT_AUTHOR_NAME", "Switchboard"),
            ("GIT_AUTHOR_EMAIL", "switchboard@localhost"),
            ("GIT_COMMITTER_NAME", "Switchboard"),
            ("GIT_COMMITTER_EMAIL", "switchboard@localhost"),
        ];
        git_env(tree, &["commit-tree", &built, "-p", &head, "-m", "Switchboard handover: the work that is not committed"], &who)?
    };
    let changes = if snap == head {
        vec![]
    } else {
        git_env(tree, &["diff", "--name-status", &head, &snap], &[])?.lines().map(str::to_owned).collect()
    };
    sweep_refs(tree, REF_KEEP);
    let safe: String = pane.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let refname = format!("{REF_PREFIX}{safe}-{}", now());
    git_env(tree, &["update-ref", &refname, &snap], &[])?;
    Ok(Plan {
        tree: tree.to_string_lossy().into_owned(),
        branch,
        head,
        snap,
        refname,
        urls: remote_urls(tree),
        session: None,
        session_data: None,
        title: String::new(),
        changes,
    })
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

/// Removes the snapshot refs older than `keep` seconds. Their name ends in the
/// second they were made.
pub fn sweep_refs(tree: &Path, keep: u64) {
    let Ok(list) = git_env(tree, &["for-each-ref", "--format=%(refname)", REF_PREFIX], &[]) else { return };
    let limit = now().saturating_sub(keep);
    for r in list.lines() {
        let at = r.rsplit('-').next().and_then(|s| s.parse::<u64>().ok());
        if at.is_some_and(|t| t < limit) {
            let _ = git_env(tree, &["update-ref", "-d", r], &[]);
        }
    }
}

/// The fetch URLs of the remotes of the repo at `tree`.
pub fn remote_urls(tree: &Path) -> Vec<String> {
    let mut urls: Vec<String> = git_env(tree, &["remote", "-v"], &[])
        .unwrap_or_default()
        .lines()
        .filter(|l| l.ends_with("(fetch)"))
        .filter_map(|l| l.split_whitespace().nth(1).map(str::to_owned))
        .collect();
    urls.dedup();
    urls
}

/// A remote URL as host and path, so the ssh and https forms of one repo match:
/// `git@github.com:sigp/lighthouse.git` and `https://github.com/sigp/lighthouse`
/// are both `github.com/sigp/lighthouse`.
pub fn url_key(url: &str) -> String {
    let mut u = url.trim();
    for p in ["https://", "http://", "ssh://", "git://", "git+ssh://"] {
        if let Some(rest) = u.strip_prefix(p) {
            u = rest;
            break;
        }
    }
    let u = u.rsplit_once('@').map_or(u, |(_, rest)| rest);
    // The scp form: host:path. A port form (host:22/path) keeps only the path.
    let u = match u.split_once(':') {
        Some((host, path)) if !host.contains('/') => {
            let path = path.trim_start_matches(|c: char| c.is_ascii_digit()).trim_start_matches('/');
            format!("{host}/{path}")
        }
        _ => u.to_owned(),
    };
    let u = u.trim_end_matches('/');
    let u = u.strip_suffix(".git").unwrap_or(u);
    u.to_lowercase()
}

/// The first repo in `repos` with a remote that matches one of `urls`.
pub fn find_clone(repos: &[PathBuf], urls: &[String]) -> Option<PathBuf> {
    let want: Vec<String> = urls.iter().map(|u| url_key(u)).collect();
    repos.iter().find(|r| remote_urls(r).iter().any(|u| want.contains(&url_key(u)))).cloned()
}

/// The branches that a worktree has checked out.
fn checked_out(repo: &Path) -> Vec<String> {
    git_env(repo, &["worktree", "list", "--porcelain"], &[])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.strip_prefix("branch refs/heads/"))
        .map(str::to_owned)
        .collect()
}

/// Rebuilds the plan's work in a new worktree of `repo` (on this computer).
/// `source` is where git fetches from: `host:/path` over ssh, or a path in tests.
/// Returns the worktree and its branch.
pub fn bring_in(repo: &Path, source: &str, plan: &Plan) -> Result<(PathBuf, String), String> {
    check(plan)?;
    fetch(repo, source, plan)?;
    let made = make_tree(repo, plan);
    let _ = git_env(repo, &["update-ref", "-d", &plan.refname], &[]);
    made
}

/// Fetches the snapshot into `repo` under the plan's ref, and checks that it is
/// the commit the plan names.
fn fetch(repo: &Path, source: &str, plan: &Plan) -> Result<(), String> {
    // ssh never asks: the window has no terminal (SPEC 29.1).
    let ssh = std::env::var("GIT_SSH_COMMAND").unwrap_or_else(|_| "ssh -o BatchMode=yes".into());
    let spec = format!("+{0}:{0}", plan.refname);
    git_env(repo, &["fetch", "--no-tags", "-q", "--", source, &spec], &[("GIT_SSH_COMMAND", ssh.as_str())])?;
    let got = git_env(repo, &["rev-parse", "--verify", &format!("{}^{{commit}}", plan.refname)], &[])?;
    if got != plan.snap {
        let _ = git_env(repo, &["update-ref", "-d", &plan.refname], &[]);
        return Err(format!("the server sent {got}, not the snapshot {}", plan.snap));
    }
    Ok(())
}

/// The plan comes from the server: each part that reaches git or a file name is
/// checked first. A ref, a branch or a commit that starts with `-` would be an option.
pub fn check(plan: &Plan) -> Result<(), String> {
    let r = &plan.refname;
    if !r.starts_with(REF_PREFIX) || r.contains("..") || !r[REF_PREFIX.len()..].chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(format!("{r} is not a handover ref"));
    }
    for c in [&plan.head, &plan.snap] {
        if c.len() < 7 || c.len() > 64 || !c.chars().all(|x| x.is_ascii_hexdigit()) {
            return Err(format!("{c} is not a commit id"));
        }
    }
    if let Some(b) = &plan.branch {
        if !branch_ok(b) {
            return Err(format!("{b} is not a branch name"));
        }
    }
    if !plan.tree.starts_with('/') || plan.tree.contains(['\n', '\r', '\0']) {
        return Err(format!("{} is not a folder", plan.tree));
    }
    if let Some(s) = &plan.session {
        if !session_ok(s) {
            return Err(format!("{s} is not a session id"));
        }
    }
    Ok(())
}

/// A branch name that git takes and that is no option.
fn branch_ok(b: &str) -> bool {
    !b.is_empty()
        && !b.starts_with('-')
        && b.len() <= 200
        && Command::new("git").args(["check-ref-format", "--branch", b]).stdin(Stdio::null()).output().is_ok_and(|o| o.status.success())
}

/// How the work gets its branch here.
#[derive(Debug, Clone, PartialEq)]
enum Mode {
    /// A new branch at the head.
    New,
    /// The branch here is behind the server's: it moves up, as a pull would.
    Move,
}

/// The branch and folder for the plan in `repo`: the server's branch name, or the
/// next free one. With a note on why the name changed.
fn pick(repo: &Path, plan: &Plan) -> Result<(String, PathBuf, Mode, Option<String>), String> {
    let want = plan.branch.clone().unwrap_or_else(|| "handover".into());
    let busy = checked_out(repo);
    let mut why = None;
    for n in 1..100 {
        let name = if n == 1 { want.clone() } else { format!("{want}-{n}") };
        let path = repos::worktree_path(repo, &name);
        let note = |why: &Option<String>| if n == 1 { None } else { why.clone() };
        if busy.contains(&name) {
            why.get_or_insert(format!("{want} is checked out here already"));
            continue;
        }
        if path.exists() {
            why.get_or_insert(format!("{} exists", path.display()));
            continue;
        }
        let exists = git_env(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{name}")], &[]).is_ok();
        if !exists {
            return Ok((name, path, Mode::New, note(&why)));
        }
        // The head is here: the dry run fetches too.
        if git_env(repo, &["merge-base", "--is-ancestor", &name, &plan.head], &[]).is_ok() {
            return Ok((name, path, Mode::Move, note(&why)));
        }
        why.get_or_insert(format!("{want} here has commits that the server does not have"));
    }
    Err(format!("no free branch name for {want}"))
}

/// The worktree at the plan's head, on its branch or the next free name, with the
/// work that is not committed put back.
fn make_tree(repo: &Path, plan: &Plan) -> Result<(PathBuf, String), String> {
    let (name, path, mode, _) = pick(repo, plan)?;
    let p = path.to_string_lossy().into_owned();
    match mode {
        Mode::New => git_env(repo, &["worktree", "add", "-q", "-b", &name, "--", &p, &plan.head], &[])?,
        Mode::Move => {
            git_env(repo, &["branch", "-f", "--", &name, &plan.head], &[])?;
            git_env(repo, &["worktree", "add", "-q", "--", &p, &name], &[])?
        }
    };
    if plan.snap != plan.head {
        if let Err(err) = apply(repo, &path, plan) {
            // A half-made handover leaves nothing behind.
            let _ = git_env(repo, &["worktree", "remove", "--force", "--", &p], &[]);
            if mode == Mode::New {
                let _ = git_env(repo, &["branch", "-D", "--", &name], &[]);
            }
            return Err(err);
        }
    }
    Ok((path, name))
}

/// The clones on this computer: the repos the launcher lists (SPEC 9).
fn clones(paths: &Paths, config: &Config) -> Vec<PathBuf> {
    let roots: Vec<PathBuf> = config.scan.iter().map(|s| expand(s)).collect();
    repos::discover(&paths.claude.join("projects"), &roots).into_iter().map(|r| PathBuf::from(r.path)).collect()
}

/// The clone here for the plan, or why there is none.
fn clone_for(paths: &Paths, config: &Config, plan: &Plan) -> Result<PathBuf, String> {
    find_clone(&clones(paths, config), &plan.urls).ok_or_else(|| match plan.urls.first() {
        Some(u) => format!("No clone of {u} on this computer. Clone it into a scan folder, then try again."),
        None => "The repo on the server has no remote, so no clone here matches it.".into(),
    })
}

/// The dry run: where the work lands (SPEC 38.2). It fetches the snapshot, so the
/// link to the server is tested and the branch choice sees the server's head. It
/// changes no branch, worktree or file.
pub fn target(paths: &Paths, config: &Config, source: &str, plan: &Plan) -> Result<Target, String> {
    check(plan)?;
    let repo = clone_for(paths, config, plan)?;
    fetch(&repo, source, plan)?;
    let picked = pick(&repo, plan);
    let _ = git_env(&repo, &["update-ref", "-d", &plan.refname], &[]);
    let (branch, tree, _, note) = picked?;
    Ok(Target { repo: repo.to_string_lossy().into_owned(), tree: tree.to_string_lossy().into_owned(), branch, note })
}

/// Where git fetches the snapshot: the server's worktree over ssh.
pub fn source(host: &str, tree: &str) -> String {
    if host.starts_with("ssh://") {
        format!("{}{tree}", host.trim_end_matches('/'))
    } else {
        format!("{host}:{tree}")
    }
}

/// The handover on this computer: fetch, worktree, changes, and the pending
/// agent for the hub here (SPEC 38.2). `source` is where git fetches from.
pub fn bring_in_here(paths: &Paths, config: &Config, source: &str, from: &str, plan: &Plan) -> Result<Pending, String> {
    check(plan)?;
    let repo = clone_for(paths, config, plan)?;
    let (tree, branch) = bring_in(&repo, source, plan)?;
    let p = Pending {
        repo: repo.to_string_lossy().into_owned(),
        tree: tree.to_string_lossy().into_owned(),
        branch,
        session: plan.session.clone().filter(|_| plan.session_data.is_some()),
        title: plan.title.clone(),
        from: from.to_owned(),
        changes: plan.changes.len(),
    };
    save_pending(&paths.home, &p, plan.session_data.as_deref())?;
    Ok(p)
}

/// Puts the snapshot's changes into the worktree, not staged.
fn apply(repo: &Path, tree: &Path, plan: &Plan) -> Result<(), String> {
    let diff = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["diff", "--binary", &plan.head, &plan.snap, "--"])
        .output()
        .map_err(|e| format!("git diff: {e}"))?;
    if !diff.status.success() {
        return Err(format!("git diff: {}", String::from_utf8_lossy(&diff.stderr).trim()));
    }
    let mut child = Command::new("git")
        .arg("-C")
        .arg(tree)
        .args(["apply", "--whitespace=nowarn"])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git apply: {e}"))?;
    child.stdin.take().ok_or("no stdin")?.write_all(&diff.stdout).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("git apply: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

/// Claude Code's folder for the sessions of `tree`: each character that is not a
/// letter, a digit or `-` becomes `-`.
pub fn project_dir(claude: &Path, tree: &Path) -> PathBuf {
    let name: String = tree.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect();
    claude.join("projects").join(name)
}

/// True for a session id that is safe as a file name.
pub fn session_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// The folder of the pending handover and its session file, in `home`.
pub fn pending_dir(home: &Path) -> PathBuf {
    home.join("handover")
}

/// Saves the pending handover, with its session file next to it.
pub fn save_pending(home: &Path, p: &Pending, session_data: Option<&str>) -> Result<(), String> {
    let dir = pending_dir(home);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    if let (Some(id), Some(data)) = (p.session.as_deref(), session_data) {
        if !session_ok(id) {
            return Err(format!("{id} is not a session id"));
        }
        std::fs::write(dir.join(format!("{id}.jsonl")), data).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(p).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("pending.json"), json).map_err(|e| e.to_string())
}

/// Takes the pending handover, if any: it is read once. Its session file moves into
/// Claude Code's folder for the worktree, in `claude`. A session with no file there
/// starts as a new one.
pub fn take_pending(home: &Path, claude: &Path) -> Result<Option<Pending>, String> {
    let dir = pending_dir(home);
    let file = dir.join("pending.json");
    let Ok(text) = std::fs::read_to_string(&file) else { return Ok(None) };
    let _ = std::fs::remove_file(&file);
    let mut p: Pending = serde_json::from_str(&text).map_err(|e| format!("the pending handover does not parse: {e}"))?;
    if let Some(id) = p.session.clone() {
        let from = dir.join(format!("{id}.jsonl"));
        let to_dir = project_dir(claude, Path::new(&p.tree));
        let to = to_dir.join(format!("{id}.jsonl"));
        let moved = session_ok(&id)
            && from.is_file()
            && std::fs::create_dir_all(&to_dir).is_ok()
            && (to.exists() || std::fs::copy(&from, &to).is_ok());
        let _ = std::fs::remove_file(&from);
        if !moved {
            p.session = None;
        }
    }
    Ok(Some(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_keys_match_across_forms() {
        let k = "github.com/sigp/lighthouse";
        for u in [
            "git@github.com:sigp/lighthouse.git",
            "https://github.com/sigp/lighthouse",
            "https://github.com/sigp/lighthouse.git/",
            "ssh://git@github.com/sigp/lighthouse.git",
            "ssh://git@github.com:22/sigp/lighthouse",
            "https://user@GitHub.com/Sigp/Lighthouse",
        ] {
            assert_eq!(url_key(u), k, "{u}");
        }
        assert_ne!(url_key("git@github.com:sigp/lighthouse-2.git"), k);
    }

    #[test]
    fn project_dirs_follow_claude_code() {
        let d = project_dir(Path::new("/h/.claude"), Path::new("/home/eitan/.cache/a_b/x"));
        assert_eq!(d, Path::new("/h/.claude/projects/-home-eitan--cache-a-b-x"));
    }

    #[test]
    fn session_ids_are_file_names() {
        assert!(session_ok("89e5cfe3-2fbe-408e-8c4a-5cdac2435be3"));
        assert!(!session_ok("../x") && !session_ok("") && !session_ok("a/b"));
    }
}
