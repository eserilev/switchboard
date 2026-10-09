//! The guided review engine (SPEC 11): PR fetch, the guide session, threads.

use crate::repos::{git, worktree_path};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The tools a review agent may use. Everything else is denied in `-p` mode.
pub const ALLOWED: &str = "Read,Grep,Glob,Bash(git diff:*),Bash(git log:*),Bash(git show:*),Bash(git blame:*),mcp__sb__guide_set_steps,mcp__sb__guide_update_step";
pub const DENIED: &str =
    "Edit,Write,NotebookEdit,MultiEdit,Bash(gh:*),Bash(git push:*),Bash(git commit:*)";

pub const DEFAULT_PROMPT: &str = include_str!("../../../prompts/review.md");

#[derive(Debug, Clone, PartialEq)]
pub struct PrRef {
    /// `owner/name`.
    pub repo: String,
    pub number: u64,
}

pub fn parse_url(url: &str) -> Option<PrRef> {
    let rest = url
        .trim()
        .trim_end_matches('/')
        .split("github.com/")
        .nth(1)?;
    let mut it = rest.split('/');
    let (owner, name, pull, num) = (it.next()?, it.next()?, it.next()?, it.next()?);
    if pull != "pull" || owner.is_empty() || name.is_empty() {
        return None;
    }
    Some(PrRef {
        repo: format!("{owner}/{name}"),
        number: num.split(['#', '?']).next()?.parse().ok()?,
    })
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PrInfo {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub head_ref_oid: String,
    #[serde(default)]
    pub head_ref_name: String,
    pub base_ref_name: String,
    #[serde(default)]
    pub base_ref_oid: String,
    #[serde(default)]
    pub files: Vec<PrFile>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct PrFile {
    pub path: String,
    #[serde(default)]
    pub additions: u64,
    #[serde(default)]
    pub deletions: u64,
}

pub fn gh_view(url: &str) -> Result<PrInfo, String> {
    let out = Command::new("gh")
        .args([
            "pr",
            "view",
            url,
            "--json",
            "number,title,body,headRefOid,headRefName,baseRefName,baseRefOid,files",
        ])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("gh: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        if err.contains("auth login") || err.contains("not logged") {
            return Err("gh is not logged in. Run: gh auth login".into());
        }
        return Err(format!("gh pr view: {err}"));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("gh pr view: {e}"))
}

/// Runs `gh api` with an optional JSON body on stdin. Returns stdout.
fn gh_api(args: &[&str], input: Option<&str>) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let mut child = Command::new("gh")
        .arg("api")
        .args(args)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("gh: {e}"))?;
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("gh: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("gh: {e}"))?;
    if out.status.success() {
        return Ok(out.stdout);
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    if err.contains("auth login") || err.contains("not logged") {
        return Err("gh is not logged in. Run: gh auth login".into());
    }
    // GitHub's reason is in the JSON body on stdout.
    let body: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    let mut why = body["message"].as_str().unwrap_or("").to_owned();
    for e in body["errors"].as_array().into_iter().flatten() {
        why.push_str(&format!(" {}", e.as_str().map(str::to_owned).unwrap_or(e.to_string())));
    }
    Err(format!("gh api: {err} {why}").trim().to_owned())
}

/// GitHub's own diff of the PR.
pub fn gh_pr_diff(repo: &str, number: u64) -> Result<String, String> {
    let out = gh_api(
        &[
            "-H",
            "Accept: application/vnd.github.diff",
            &format!("repos/{repo}/pulls/{number}"),
        ],
        None,
    )?;
    String::from_utf8(out).map_err(|e| format!("GitHub's diff is not UTF-8: {e}"))
}

/// The GitHub login that `gh` posts as.
pub fn gh_login() -> Result<String, String> {
    let out = gh_api(&["user", "--jq", ".login"], None)?;
    Ok(String::from_utf8_lossy(&out).trim().to_owned())
}

/// The reviews of a PR, all pages, as one JSON array.
pub fn gh_reviews(repo: &str, number: u64) -> Result<Value, String> {
    let out = gh_api(
        &["--paginate", "--slurp", &format!("repos/{repo}/pulls/{number}/reviews")],
        None,
    )?;
    let pages: Value = serde_json::from_slice(&out).map_err(|e| format!("gh api: {e}"))?;
    Ok(Value::Array(
        pages.as_array().into_iter().flatten().flat_map(|p| p.as_array().cloned().unwrap_or_default()).collect(),
    ))
}

/// The line comments of one review, as one JSON array. They come from the PR-wide
/// list: the list of one review has no `line` and no `side`.
pub fn gh_review_comments(repo: &str, number: u64, review: u64) -> Result<Value, String> {
    let out = gh_api(
        &["--paginate", "--slurp", &format!("repos/{repo}/pulls/{number}/comments")],
        None,
    )?;
    let pages: Value = serde_json::from_slice(&out).map_err(|e| format!("gh api: {e}"))?;
    Ok(Value::Array(
        pages
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|p| p.as_array().cloned().unwrap_or_default())
            .filter(|c| c["pull_request_review_id"].as_u64() == Some(review))
            .collect(),
    ))
}

/// Sends one review with all its comments. Returns the link to the review.
pub fn gh_submit_review(repo: &str, number: u64, payload: &Value) -> Result<String, String> {
    let out = gh_api(
        &[
            "--method",
            "POST",
            &format!("repos/{repo}/pulls/{number}/reviews"),
            "--input",
            "-",
        ],
        Some(&payload.to_string()),
    )?;
    let v: Value = serde_json::from_slice(&out).map_err(|e| format!("gh api: {e}"))?;
    Ok(v["html_url"].as_str().unwrap_or_default().to_owned())
}

/// The remote of `repo_dir` that points at `owner/name` on GitHub.
pub fn find_remote(repo_dir: &Path, repo: &str) -> Option<String> {
    let text = git(repo_dir, &["remote", "-v"]).ok()?;
    remote_for(&text, repo)
}

pub fn remote_for(remote_v: &str, repo: &str) -> Option<String> {
    let want = repo.to_lowercase();
    remote_v.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let (name, url) = (it.next()?, it.next()?);
        let path = url.to_lowercase();
        let path = path.trim_end_matches(".git").trim_end_matches('/');
        let tail = path.rsplit(['/', ':']).take(2).collect::<Vec<_>>();
        (tail.len() == 2
            && format!("{}/{}", tail[1], tail[0]) == want
            && path.contains("github.com"))
        .then(|| name.to_owned())
    })
}

pub struct Fetched {
    pub head: String,
    pub base: String,
    pub tree: PathBuf,
}

/// Fetches the PR head and base, and puts the head in its own worktree.
pub fn fetch(repo_dir: &Path, remote: &str, pr: &PrInfo) -> Result<Fetched, String> {
    fetch_into(repo_dir, remote, pr, &format!("pr{}", pr.number))
}

/// As `fetch`, with the worktree named `name`. Each review round has its own worktree.
pub fn fetch_into(repo_dir: &Path, remote: &str, pr: &PrInfo, name: &str) -> Result<Fetched, String> {
    let branch = format!("sb-pr-{}", pr.number);
    let have = |oid: &str| {
        !oid.is_empty() && git(repo_dir, &["cat-file", "-e", &format!("{oid}^{{commit}}")]).is_ok()
    };
    // Fetch only what is missing. A fetch from a big remote takes long.
    let t0 = std::time::Instant::now();
    if have(&pr.head_ref_oid) {
        tracing::info!(target: "sb::review", head = %pr.head_ref_oid, "PR head is already local");
        git(
            repo_dir,
            &[
                "update-ref",
                &format!("refs/heads/{branch}"),
                &pr.head_ref_oid,
            ],
        )?;
    } else {
        tracing::info!(target: "sb::review", remote, number = pr.number, "fetching the PR head");
        git(
            repo_dir,
            &[
                "fetch",
                "-q",
                "--no-tags",
                remote,
                &format!("+pull/{}/head:refs/heads/{branch}", pr.number),
            ],
        )?;
        tracing::info!(target: "sb::review", ms = t0.elapsed().as_millis() as u64, "fetched the PR head");
    }
    let base_tip = if have(&pr.base_ref_oid) {
        tracing::info!(target: "sb::review", base = %pr.base_ref_oid, "PR base is already local");
        pr.base_ref_oid.clone()
    } else {
        let t1 = std::time::Instant::now();
        tracing::info!(target: "sb::review", remote, base = %pr.base_ref_name, "fetching the PR base");
        git(
            repo_dir,
            &["fetch", "-q", "--no-tags", remote, &pr.base_ref_name],
        )?;
        tracing::info!(target: "sb::review", ms = t1.elapsed().as_millis() as u64, "fetched the PR base");
        git(repo_dir, &["rev-parse", "FETCH_HEAD"])?
    };
    let head = git(repo_dir, &["rev-parse", &branch])?;
    let base = git(repo_dir, &["merge-base", &base_tip, &head])?;
    let tree = worktree_path(repo_dir, name);
    tracing::info!(target: "sb::review", tree = %tree.display(), exists = tree.exists(), "PR worktree");
    if tree.exists() {
        git(&tree, &["checkout", "-q", "--detach", &head])?;
    } else {
        git(
            repo_dir,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                &tree.to_string_lossy(),
                &head,
            ],
        )?;
    }
    Ok(Fetched { head, base, tree })
}

/// The prompt for the guide session.
pub fn prompt(template: &str, pr: &PrInfo, head: &str, specs: &[String]) -> String {
    let files: Vec<String> = pr
        .files
        .iter()
        .map(|f| format!("- {} (+{} -{})", f.path, f.additions, f.deletions))
        .collect();
    let specs = if specs.is_empty() {
        "(none)".to_owned()
    } else {
        specs
            .iter()
            .map(|s| format!("- {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    template
        .replace("{{number}}", &pr.number.to_string())
        .replace("{{title}}", &pr.title)
        .replace("{{head}}", head)
        .replace("{{base_ref}}", &pr.base_ref_name)
        .replace("{{files}}", &files.join("\n"))
        .replace("{{body}}", pr.body.trim())
        .replace("{{specs}}", &specs)
}

/// Checks a guide from `guide_set_steps`. Paths must exist at `head`, lines must be in the file.
pub fn validate(guide: &Value, tree: &Path, head: &str, base: &str) -> Result<(), String> {
    let steps = guide
        .get("steps")
        .and_then(Value::as_array)
        .ok_or("the guide needs a steps array")?;
    if steps.is_empty() {
        return Err("the guide has no steps".into());
    }
    let mut ids = std::collections::HashSet::new();
    for (i, s) in steps.iter().enumerate() {
        let id = s
            .get("id")
            .and_then(Value::as_str)
            .ok_or(format!("step {i} has no id"))?;
        if !ids.insert(id) {
            return Err(format!("step id {id} is used twice"));
        }
        s.get("title")
            .and_then(Value::as_str)
            .ok_or(format!("step {id} has no title"))?;
        validate_step(s, id, tree, head, base)?;
    }
    Ok(())
}

pub fn validate_step(
    s: &Value,
    id: &str,
    tree: &Path,
    head: &str,
    base: &str,
) -> Result<(), String> {
    let Some(file) = s.get("file").and_then(Value::as_str) else {
        return Ok(());
    };
    let rev = if s.get("side").and_then(Value::as_str) == Some("old") {
        base
    } else {
        head
    };
    let text = git(tree, &["show", &format!("{rev}:{file}")])
        .map_err(|_| format!("step {id}: {file} does not exist at {rev}"))?;
    let count = text.lines().count() as u64;
    if let Some(lines) = s.get("lines").and_then(Value::as_array) {
        for l in lines {
            let n = l
                .as_u64()
                .ok_or(format!("step {id}: lines must be numbers"))?;
            if n == 0 || n > count {
                return Err(format!(
                    "step {id}: line {n} is outside {file} ({count} lines)"
                ));
            }
        }
    }
    Ok(())
}

/// One event from `claude -p --output-format stream-json`.
#[derive(Debug, Clone, PartialEq)]
pub enum Stream {
    Init {
        session: String,
    },
    Delta(String),
    Result {
        text: String,
        error: bool,
        session: Option<String>,
    },
}

pub fn parse_stream(line: &str) -> Option<Stream> {
    let v: Value = serde_json::from_str(line).ok()?;
    match v.get("type")?.as_str()? {
        "system" if v.get("subtype").and_then(Value::as_str) == Some("init") => {
            Some(Stream::Init {
                session: v.get("session_id")?.as_str()?.to_owned(),
            })
        }
        "stream_event" => {
            let d = v.pointer("/event/delta")?;
            (d.get("type")?.as_str()? == "text_delta").then(|| {
                Stream::Delta(
                    d.get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                )
            })
        }
        "result" => Some(Stream::Result {
            text: v
                .get("result")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            error: v.get("is_error").and_then(Value::as_bool).unwrap_or(false)
                || v.get("subtype").and_then(Value::as_str) != Some("success"),
            session: v
                .get("session_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
        }),
        _ => None,
    }
}

/// How to start one `claude -p` run.
pub struct Run<'a> {
    pub cwd: &'a Path,
    pub prompt: &'a str,
    pub resume: Option<&'a str>,
    pub fork: bool,
    pub env: &'a [(String, String)],
    /// The `sb` binary and review id, for the MCP server.
    pub mcp: Option<(&'a Path, &'a str, &'a Path)>,
}

pub fn args(run: &Run) -> Vec<String> {
    // The prompt goes first. `--allowedTools`, `--disallowedTools` and `--mcp-config`
    // take lists, so a prompt after them becomes one more list item.
    let mut a: Vec<String> = vec!["-p".into(), run.prompt.to_owned()];
    a.extend(
        [
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--permission-mode",
            "default",
        ]
        .map(String::from),
    );
    if let Some(r) = run.resume {
        a.extend(["--resume".into(), r.into()]);
        if run.fork {
            a.push("--fork-session".into());
        }
    }
    a.extend([
        "--allowedTools".into(),
        ALLOWED.into(),
        "--disallowedTools".into(),
        DENIED.into(),
    ]);
    if let Some((sb, review, sock)) = run.mcp {
        let cfg = json!({ "mcpServers": { "sb": {
            "command": sb.to_string_lossy(), "args": ["mcp"],
            "env": { "SB_REVIEW": review, "SB_SOCK": sock.to_string_lossy() }
        }}});
        a.extend([
            "--mcp-config".into(),
            cfg.to_string(),
            "--strict-mcp-config".into(),
        ]);
    }
    a
}

/// Runs `claude -p` and calls `on` for each stream event. Returns the final result.
/// The longest one `claude -p` run may take. A stuck run is killed, so the
/// review shows an error and Retry, not "writing" for good.
pub const RUN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

pub fn run(run: &Run, mut on: impl FnMut(&Stream)) -> Result<Stream, String> {
    let mut child = Command::new("claude")
        .args(args(run))
        .current_dir(run.cwd)
        .envs(run.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("claude: {e}"))?;
    let out = child.stdout.take().expect("piped");
    // Read stderr on its own thread: a full stderr pipe blocks the child.
    let mut err = child.stderr.take().expect("piped");
    let err_text = std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        let s = String::from_utf8_lossy(&buf).into_owned();
        s.chars()
            .rev()
            .take(2000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>()
    });
    // A watchdog kills a run that takes too long.
    let pid = child.id();
    let t0 = std::time::Instant::now();
    tracing::info!(target: "sb::claude", pid, cwd = %run.cwd.display(), resume = ?run.resume, fork = run.fork, prompt = %crate::proto::first_line(run.prompt, 100), "claude -p started");
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let watchdog = std::thread::spawn(move || {
        if done_rx.recv_timeout(RUN_TIMEOUT).is_err() {
            let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
            return true;
        }
        false
    });
    let mut last = None;
    for line in BufReader::new(out).lines().map_while(Result::ok) {
        if let Some(ev) = parse_stream(&line) {
            match &ev {
                Stream::Init { session } => {
                    tracing::info!(target: "sb::claude", pid, %session, ms = t0.elapsed().as_millis() as u64, "claude -p session")
                }
                Stream::Result { error, .. } => {
                    tracing::info!(target: "sb::claude", pid, error, ms = t0.elapsed().as_millis() as u64, "claude -p result")
                }
                Stream::Delta(_) => {}
            }
            on(&ev);
            if matches!(ev, Stream::Result { .. }) {
                last = Some(ev);
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let _ = done_tx.send(());
    let killed = watchdog.join().unwrap_or(false);
    let err = err_text.join().unwrap_or_default();
    tracing::info!(target: "sb::claude", pid, ms = t0.elapsed().as_millis() as u64, %status, killed, got_result = last.is_some(), "claude -p ended");
    if !err.trim().is_empty() {
        tracing::debug!(target: "sb::claude", pid, stderr = %err.trim(), "claude -p stderr");
    }
    if killed {
        return Err(format!(
            "claude took longer than {} minutes and was stopped.",
            RUN_TIMEOUT.as_secs() / 60
        ));
    }
    match last {
        Some(r) => Ok(r),
        None => Err(format!(
            "claude exited with {status} and no result. {}",
            err.trim()
        )),
    }
}

/// Files changed between two heads, for stale steps.
/// The base of a later round: the code that the round before reviewed, so the round
/// shows only what changed since then.
///
/// - The author only added commits on top of `since`: the base is `since`.
/// - The author rebased, or merged the base branch: the base is `since` with the
///   new base branch merged in (`replay`). The round then leaves out the changes that
///   came from the base branch.
/// - A file of that merge has a conflict: that file takes its version at `since`, so
///   only that file can also show changes from the base branch. The note names it.
///
/// Returns the base (a commit or a tree id) and a note when the base is not `since`.
pub fn round_base(
    dir: &Path,
    since: &str,
    new_base: &str,
    head: &str,
) -> Result<(String, Option<String>), String> {
    let ancestor = |a: &str, b: &str| {
        Command::new("git")
            .args(["merge-base", "--is-ancestor", a, b])
            .current_dir(dir)
            .stdin(Stdio::null())
            .status()
            .map(|s| s.success())
            .map_err(|e| format!("git: {e}"))
    };
    if ancestor(since, head)? && ancestor(new_base, since)? {
        return Ok((since.to_owned(), None));
    }
    let (tree, conflicts) = replay(dir, since, new_base)?;
    let mut note = "The author rebased or merged the base branch. This round leaves out the changes from the base branch".to_owned();
    if conflicts.is_empty() {
        note.push('.');
    } else {
        note.push_str(&format!(
            ", except in {} file{} where git found a merge conflict: {}.",
            conflicts.len(),
            if conflicts.len() == 1 { "" } else { "s" },
            conflicts.join(", ")
        ));
    }
    Ok((tree, Some(note)))
}

/// `since` with `new_base` merged in, as a tree (`git merge-tree --write-tree`). Each
/// file with a conflict takes its version at `since`, or is left out when `since` has
/// no such file. Returns the tree and the files with a conflict, sorted.
pub fn replay(dir: &Path, since: &str, new_base: &str) -> Result<(String, Vec<String>), String> {
    let out = Command::new("git")
        .args(["merge-tree", "--write-tree", "--no-messages", "--name-only", since, new_base])
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git merge-tree: {e}"))?;
    // Exit 0: a clean merge. Exit 1: conflicts, and the tree has conflict markers.
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut lines = text.lines();
    let tree = lines.next().unwrap_or("").trim().to_owned();
    if tree.is_empty() || !matches!(out.status.code(), Some(0 | 1)) {
        return Err(format!(
            "git merge-tree: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let mut conflicts: Vec<String> = lines
        .take_while(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    conflicts.sort();
    conflicts.dedup();
    if conflicts.is_empty() {
        return Ok((tree, conflicts));
    }
    // Put the `since` version of each conflicted file into a copy of the tree, in a
    // private index file, so the repo's own index does not change.
    // A name of its own for each call, also for two calls at the same time.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let index = std::env::temp_dir().join(format!(
        "sb-round-index-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let run = |args: &[&str], input: Option<&str>| -> Result<String, String> {
        use std::io::Write;
        let mut child = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_INDEX_FILE", &index)
            .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("git: {e}"))?;
        if let (Some(t), Some(mut i)) = (input, child.stdin.take()) {
            i.write_all(t.as_bytes()).map_err(|e| format!("git: {e}"))?;
        }
        let out = child.wait_with_output().map_err(|e| format!("git: {e}"))?;
        if !out.status.success() {
            return Err(format!("git {}: {}", args[0], String::from_utf8_lossy(&out.stderr).trim()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };
    let result = (|| {
        run(&["read-tree", &tree], None)?;
        let mut info = String::new();
        for path in &conflicts {
            let entry = git(dir, &["ls-tree", since, "--", path])?;
            match entry.split_once('\t') {
                // "<mode> blob <oid>\t<path>" becomes "<mode> <oid>\t<path>".
                Some((meta, _)) => {
                    let parts: Vec<&str> = meta.split_whitespace().collect();
                    info.push_str(&format!("{} {}\t{path}\n", parts[0], parts[2]));
                }
                // No such file at `since`: mode 0 removes it.
                None => info.push_str(&format!("0 {}\t{path}\n", "0".repeat(40))),
            }
        }
        run(&["update-index", "--index-info"], Some(&info))?;
        run(&["write-tree"], None)
    })();
    let _ = std::fs::remove_file(&index);
    Ok((result?, conflicts))
}

pub fn changed_files(tree: &Path, old: &str, new: &str) -> Result<Vec<String>, String> {
    Ok(git(tree, &["diff", "--name-only", old, new])?
        .lines()
        .map(str::to_owned)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repo with `main` (a.rs, base.rs) and a PR branch that changed a.rs line 2.
    /// Returns the dir, the base commit and the round-1 head.
    fn round_repo(name: &str) -> (PathBuf, String, String) {
        let dir = std::env::temp_dir().join(format!("sb-round-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = |args: &[&str]| git(&dir, args).unwrap();
        g(&["init", "-q", "-b", "main"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "t"]);
        std::fs::write(dir.join("a.rs"), "1\n2\n3\n4\n5\n").unwrap();
        std::fs::write(dir.join("base.rs"), "x\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-qm", "base"]);
        let base = g(&["rev-parse", "HEAD"]);
        g(&["checkout", "-qb", "pr"]);
        std::fs::write(dir.join("a.rs"), "1\nTWO\n3\n4\n5\n").unwrap();
        g(&["commit", "-qam", "round 1"]);
        let since = g(&["rev-parse", "HEAD"]);
        (dir, base, since)
    }

    /// The changed lines of a round, as `path:side:line`.
    fn round_changes(dir: &Path, base: &str, head: &str) -> Vec<String> {
        let m = crate::coverage::build(dir, base, head).unwrap();
        crate::coverage::rebuild_ok(&m).unwrap();
        let mut out = vec![];
        for f in &m.files {
            for (k, c) in f.removed.iter().enumerate() {
                if *c {
                    out.push(format!("{}:old:{}", f.path(), k + 1));
                }
            }
            for (k, c) in f.added.iter().enumerate() {
                if *c {
                    out.push(format!("{}:new:{}", f.path(), k + 1));
                }
            }
        }
        out
    }

    /// The review finding H1: after an update, a draft moves with its code. Without
    /// the move, its comment goes to GitHub on the old number, which is another line.
    #[test]
    fn a_draft_moves_with_its_code_after_an_update() {
        use crate::post::{anchor_at, block_text, model_file, move_anchor};
        let dir = std::env::temp_dir().join(format!("sb-move-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = |args: &[&str]| git(&dir, args).unwrap();
        g(&["init", "-q", "-b", "main"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "t"]);
        let file = |lines: &[String]| std::fs::write(dir.join("a.rs"), lines.join("\n") + "\n").unwrap();
        let mut lines: Vec<String> = (1..=60).map(|i| format!("line {i}")).collect();
        file(&lines);
        g(&["add", "."]);
        g(&["commit", "-qm", "base"]);
        let base = g(&["rev-parse", "HEAD"]);
        lines[39] = "let x = risky();".into();
        file(&lines);
        g(&["commit", "-qam", "A"]);
        let a = g(&["rev-parse", "HEAD"]);
        // B adds 3 lines above line 40 and removes line 50.
        lines.splice(10..10, ["// one".to_owned(), "// two".to_owned(), "// three".to_owned()]);
        lines.remove(52);
        file(&lines);
        g(&["commit", "-qam", "B"]);
        let b = g(&["rev-parse", "HEAD"]);

        let ma = crate::coverage::build(&dir, &base, &a).unwrap();
        let mb = crate::coverage::build(&dir, &base, &b).unwrap();
        let fa = model_file(&ma, "a.rs", false).unwrap();
        let fb = model_file(&mb, "a.rs", false).unwrap();
        let text = block_text(fa, false, 40, 40).unwrap();
        assert_eq!(text, "let x = risky();");
        // Without a move, line 40 at B is another line.
        assert_ne!(block_text(fb, false, 40, 40).unwrap(), text);
        // The move finds it at 43.
        let a = anchor_at(fa, false, 40, 40).unwrap();
        let moved = move_anchor(fb, false, &a.text, Some(&a.before), Some(&a.after)).unwrap();
        assert_eq!(moved.line, 43);
        // A range moves as one block.
        let range = block_text(fa, false, 39, 41).unwrap();
        let r = anchor_at(fa, false, 39, 41).unwrap();
        assert_eq!(r.text, range);
        let moved = move_anchor(fb, false, &r.text, Some(&r.before), Some(&r.after)).unwrap();
        assert_eq!((moved.start_line, moved.line), (Some(42), 44));
        // Line 50 at A is gone at B: the draft is stale.
        let gone = block_text(fa, false, 50, 50).unwrap();
        assert_eq!(gone, "line 50");
        let g = anchor_at(fa, false, 50, 50).unwrap();
        assert_eq!(move_anchor(fb, false, &g.text, Some(&g.before), Some(&g.after)), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_round_after_new_commits_shows_only_them() {
        let (dir, base, since) = round_repo("plain");
        std::fs::write(dir.join("a.rs"), "1\nTWO\n3\n4\nFIVE\n").unwrap();
        git(&dir, &["commit", "-qam", "fix"]).unwrap();
        let head = git(&dir, &["rev-parse", "HEAD"]).unwrap();
        let (b, note) = round_base(&dir, &since, &base, &head).unwrap();
        assert_eq!((b.as_str(), note), (since.as_str(), None));
        assert_eq!(round_changes(&dir, &b, &head), ["a.rs:old:5", "a.rs:new:5"]);
    }

    #[test]
    fn a_round_after_a_base_merge_leaves_out_the_base_changes() {
        let (dir, _, since) = round_repo("merge");
        let g = |args: &[&str]| git(&dir, args).unwrap();
        g(&["checkout", "-q", "main"]);
        std::fs::write(dir.join("base.rs"), "x\ny\n").unwrap();
        g(&["commit", "-qam", "main moves"]);
        g(&["checkout", "-q", "pr"]);
        g(&["merge", "-q", "--no-edit", "main"]);
        std::fs::write(dir.join("a.rs"), "1\nTWO\n3\nFOUR\n5\n").unwrap();
        g(&["commit", "-qam", "fix"]);
        let head = g(&["rev-parse", "HEAD"]);
        let new_base = g(&["merge-base", "main", "pr"]);
        let (b, note) = round_base(&dir, &since, &new_base, &head).unwrap();
        assert!(note.unwrap().contains("leaves out"));
        assert_eq!(round_changes(&dir, &b, &head), ["a.rs:old:4", "a.rs:new:4"]);
        // A plain diff from the reviewed head would also show base.rs.
        assert!(round_changes(&dir, &since, &head).iter().any(|c| c.starts_with("base.rs")));
    }

    #[test]
    fn a_round_after_a_rebase_leaves_out_the_base_changes() {
        let (dir, _, since) = round_repo("rebase");
        let g = |args: &[&str]| git(&dir, args).unwrap();
        g(&["checkout", "-q", "main"]);
        std::fs::write(dir.join("base.rs"), "x\ny\n").unwrap();
        g(&["commit", "-qam", "main moves"]);
        g(&["checkout", "-q", "pr"]);
        g(&["rebase", "-q", "main"]);
        std::fs::write(dir.join("a.rs"), "1\nTWO\nTHREE\n4\n5\n").unwrap();
        g(&["commit", "-qam", "fix"]);
        let head = g(&["rev-parse", "HEAD"]);
        let new_base = g(&["merge-base", "main", "pr"]);
        let (b, note) = round_base(&dir, &since, &new_base, &head).unwrap();
        assert!(note.is_some());
        assert_eq!(round_changes(&dir, &b, &head), ["a.rs:old:3", "a.rs:new:3"]);
    }

    #[test]
    fn a_conflicted_file_takes_its_round_1_version() {
        let (dir, _, since) = round_repo("conflict");
        let g = |args: &[&str]| git(&dir, args).unwrap();
        g(&["checkout", "-q", "main"]);
        std::fs::write(dir.join("a.rs"), "1\nzwei\n3\n4\n5\n").unwrap();
        g(&["commit", "-qam", "main changes line 2"]);
        g(&["checkout", "-q", "pr"]);
        let _ = git(&dir, &["merge", "-q", "--no-edit", "main"]);
        std::fs::write(dir.join("a.rs"), "1\nTWO\n3\n4\n5\n").unwrap();
        g(&["add", "a.rs"]);
        g(&["commit", "-qm", "resolve"]);
        let head = g(&["rev-parse", "HEAD"]);
        let new_base = g(&["merge-base", "main", "pr"]);
        let (b, note) = round_base(&dir, &since, &new_base, &head).unwrap();
        assert!(note.unwrap().contains("1 file where git found a merge conflict: a.rs"));
        // The conflicted file takes its round-1 version, and the clean files merge.
        assert_eq!(git(&dir, &["show", &format!("{b}:a.rs")]).unwrap(), git(&dir, &["show", &format!("{since}:a.rs")]).unwrap());
        assert_eq!(round_changes(&dir, &b, &head), Vec::<String>::new());
        // The repo's own index did not change.
        assert_eq!(git(&dir, &["status", "--porcelain"]).unwrap(), "");
    }

    #[test]
    fn a_conflict_keeps_the_other_files_merged() {
        let (dir, _, since) = round_repo("conflict2");
        let g = |args: &[&str]| git(&dir, args).unwrap();
        g(&["checkout", "-q", "main"]);
        std::fs::write(dir.join("a.rs"), "1\nzwei\n3\n4\n5\n").unwrap();
        std::fs::write(dir.join("base.rs"), "x\ny\n").unwrap();
        g(&["commit", "-qam", "main changes line 2 and base.rs"]);
        g(&["checkout", "-q", "pr"]);
        let _ = git(&dir, &["merge", "-q", "--no-edit", "main"]);
        std::fs::write(dir.join("a.rs"), "1\nTWO\n3\n4\nFIVE\n").unwrap();
        g(&["add", "a.rs"]);
        g(&["commit", "-qm", "resolve and fix"]);
        let head = g(&["rev-parse", "HEAD"]);
        let new_base = g(&["merge-base", "main", "pr"]);
        let (b, _) = round_base(&dir, &since, &new_base, &head).unwrap();
        // base.rs merged cleanly, so it does not show; a.rs shows the author's fix.
        assert_eq!(round_changes(&dir, &b, &head), ["a.rs:old:5", "a.rs:new:5"]);
    }

    #[test]
    fn urls() {
        assert_eq!(
            parse_url("https://github.com/sigp/lighthouse/pull/10071"),
            Some(PrRef {
                repo: "sigp/lighthouse".into(),
                number: 10071
            })
        );
        assert_eq!(
            parse_url("https://github.com/eserilev/lighthouse/pull/121/"),
            Some(PrRef {
                repo: "eserilev/lighthouse".into(),
                number: 121
            })
        );
        assert_eq!(
            parse_url("https://github.com/a/b/pull/7/files#diff"),
            Some(PrRef {
                repo: "a/b".into(),
                number: 7
            })
        );
        assert_eq!(parse_url("https://github.com/a/b/issues/7"), None);
        assert_eq!(parse_url("lighthouse"), None);
    }

    #[test]
    fn remotes() {
        let v = "origin\tgit@github.com:eserilev/lighthouse.git (fetch)\norigin\tgit@github.com:eserilev/lighthouse.git (push)\nupstream\thttps://github.com/sigp/lighthouse (fetch)\n";
        assert_eq!(
            remote_for(v, "sigp/lighthouse").as_deref(),
            Some("upstream")
        );
        assert_eq!(
            remote_for(v, "eserilev/Lighthouse").as_deref(),
            Some("origin")
        );
        assert_eq!(remote_for(v, "sigp/other"), None);
    }

    #[test]
    fn stream_lines() {
        assert_eq!(
            parse_stream(r#"{"type":"system","subtype":"init","session_id":"s1"}"#),
            Some(Stream::Init {
                session: "s1".into()
            })
        );
        assert_eq!(
            parse_stream(
                r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}}"#
            ),
            Some(Stream::Delta("Hi".into()))
        );
        assert_eq!(
            parse_stream(
                r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"{"}}}"#
            ),
            None
        );
        assert_eq!(
            parse_stream(
                r#"{"type":"result","subtype":"success","is_error":false,"result":"done","session_id":"s1"}"#
            ),
            Some(Stream::Result {
                text: "done".into(),
                error: false,
                session: Some("s1".into())
            })
        );
        assert!(matches!(
            parse_stream(r#"{"type":"result","subtype":"error_max_turns"}"#),
            Some(Stream::Result { error: true, .. })
        ));
        assert_eq!(parse_stream("not json"), None);
    }

    #[test]
    fn args_are_read_only_and_fork_when_asked() {
        let a = args(&Run {
            cwd: Path::new("/"),
            prompt: "q",
            resume: Some("g1"),
            fork: true,
            env: &[],
            mcp: None,
        });
        let s = a.join(" ");
        assert!(s.contains("--resume g1 --fork-session"));
        assert!(s.contains("--disallowedTools Edit,Write"));
        assert!(s.contains("--verbose"));
        assert_eq!(&a[..2], ["-p", "q"]);
        let b = args(&Run {
            cwd: Path::new("/"),
            prompt: "q",
            resume: Some("t1"),
            fork: false,
            env: &[],
            mcp: Some((Path::new("/sb"), "r1", Path::new("/s"))),
        });
        assert!(!b.contains(&"--fork-session".to_owned()));
        let i = b.iter().position(|x| x == "--mcp-config").unwrap();
        let cfg: Value = serde_json::from_str(&b[i + 1]).unwrap();
        assert_eq!(cfg["mcpServers"]["sb"]["env"]["SB_REVIEW"], "r1");
    }

    #[test]
    fn prompt_fills_every_field() {
        let pr = PrInfo {
            number: 7,
            title: "T".into(),
            body: "B".into(),
            head_ref_oid: "h".into(),
            head_ref_name: "x".into(),
            base_ref_name: "unstable".into(),
            base_ref_oid: String::new(),
            files: vec![PrFile {
                path: "a.rs".into(),
                additions: 3,
                deletions: 1,
            }],
        };
        let p = prompt(DEFAULT_PROMPT, &pr, "abc123", &["~/specs".into()]);
        assert!(!p.contains("{{"), "unfilled field in:\n{p}");
        assert!(p.contains("a.rs (+3 -1)") && p.contains("abc123") && p.contains("~/specs"));
        assert!(p.contains("guide_set_steps"));
    }

    #[test]
    fn validate_checks_paths_and_lines() {
        let dir = std::env::temp_dir().join(format!("sb-validate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]).unwrap();
        std::fs::write(dir.join("a.rs"), "1\n2\n3\n").unwrap();
        git(&dir, &["add", "."]).unwrap();
        git(
            &dir,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "x",
            ],
        )
        .unwrap();
        let head = git(&dir, &["rev-parse", "HEAD"]).unwrap();
        let ok = json!({"steps":[{"id":"s1","title":"PR"},{"id":"s2","title":"A","file":"a.rs","lines":[1,3]}]});
        assert_eq!(validate(&ok, &dir, &head, &head), Ok(()));
        let bad_line = json!({"steps":[{"id":"s1","title":"A","file":"a.rs","lines":[4]}]});
        assert!(validate(&bad_line, &dir, &head, &head)
            .unwrap_err()
            .contains("outside"));
        let bad_file = json!({"steps":[{"id":"s1","title":"A","file":"b.rs"}]});
        assert!(validate(&bad_file, &dir, &head, &head)
            .unwrap_err()
            .contains("does not exist"));
        let dup = json!({"steps":[{"id":"s1","title":"A"},{"id":"s1","title":"B"}]});
        assert!(validate(&dup, &dir, &head, &head)
            .unwrap_err()
            .contains("twice"));
        assert!(validate(&json!({"steps":[]}), &dir, &head, &head).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
