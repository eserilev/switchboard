//! The guided review engine (SPEC 11): PR fetch, the guide session, threads.

use crate::repos::{git, worktree_path};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The tools a review agent may use. Everything else is denied in `-p` mode.
/// git goes through the `git` tool of `sb mcp` (`review_git`), not Bash: a Bash
/// rule such as `Bash(git diff:*)` also lets `git diff --output=<file>` write.
pub const ALLOWED: &str = "Read,Grep,Glob,mcp__sb__git,mcp__sb__guide_set_steps,mcp__sb__guide_update_step";
pub const DENIED: &str = "Edit,Write,NotebookEdit,MultiEdit,Bash";

/// The setting sources of a review agent. The review worktree holds the PR's own
/// files, and `-p` skips the trust dialog, so its `.claude/settings.json` (hooks,
/// allow rules) and `settings.local.json` must not load.
pub const SETTING_SOURCES: &str = "user";

/// The `WebFetch` rules for the sites of `review.web_domains`. A name that is not a
/// plain host name is left out: a comma or a paren would break the tool list.
pub fn web_rules(domains: &[String]) -> Vec<String> {
    domains
        .iter()
        .map(|d| d.trim())
        .filter(|d| {
            !d.is_empty()
                && d.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        })
        .map(|d| format!("WebFetch(domain:{d})"))
        .collect()
}

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
        (crate::base::github_repo(url)?.to_lowercase() == want).then(|| name.to_owned())
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
    let (head, base) = fetch_refs(repo_dir, remote, pr)?;
    let tree = checkout_tree(repo_dir, name, &head)?;
    Ok(Fetched { head, base, tree })
}

/// Fetches the PR head and the base, and points the branch `sb-pr-<n>` at the head. Returns the head
/// and the merge base. It makes no worktree: the hub does that under its tree lock.
pub fn fetch_refs(repo_dir: &Path, remote: &str, pr: &PrInfo) -> Result<(String, String), String> {
    let branch = format!("sb-pr-{}", pr.number);
    let have = |oid: &str| {
        !oid.is_empty() && git(repo_dir, &["cat-file", "-e", &format!("{oid}^{{commit}}")]).is_ok()
    };
    // Fetch only what is missing. A fetch from a big remote takes long.
    // The head is the fetched commit, never a read of the branch or of FETCH_HEAD: a
    // close of another review of this PR can delete the branch, and another fetch
    // can change FETCH_HEAD (SPEC 11.10). So the fetch goes to a ref of its own.
    let t0 = std::time::Instant::now();
    let head = if have(&pr.head_ref_oid) {
        tracing::info!(target: "sb::review", head = %pr.head_ref_oid, "PR head is already local");
        pr.head_ref_oid.clone()
    } else {
        tracing::info!(target: "sb::review", remote, number = pr.number, "fetching the PR head");
        let oid = fetch_one(repo_dir, remote, &format!("pull/{}/head", pr.number))?;
        tracing::info!(target: "sb::review", ms = t0.elapsed().as_millis() as u64, "fetched the PR head");
        oid
    };
    // The branch is for you, to find the PR in the clone. Nothing here reads it.
    point_pr_branch(repo_dir, &branch, &head);
    let base_tip = if have(&pr.base_ref_oid) {
        tracing::info!(target: "sb::review", base = %pr.base_ref_oid, "PR base is already local");
        pr.base_ref_oid.clone()
    } else {
        let t1 = std::time::Instant::now();
        tracing::info!(target: "sb::review", remote, base = %pr.base_ref_name, "fetching the PR base");
        let oid = fetch_one(repo_dir, remote, &pr.base_ref_name)?;
        tracing::info!(target: "sb::review", ms = t1.elapsed().as_millis() as u64, "fetched the PR base");
        oid
    };
    let base = git(repo_dir, &["merge-base", &base_tip, &head])?;
    Ok((head, base))
}

/// Points the branch `branch` at `head`, but never moves work of yours: it moves only
/// when no worktree has it checked out and its tip is a head that a review pinned
/// (`pin`), so it holds no commit of yours. The move names the old tip, so a commit
/// that lands in between stops it. A branch that stays only logs: nothing reads it.
pub fn point_pr_branch(repo_dir: &Path, branch: &str, head: &str) {
    let full = format!("refs/heads/{branch}");
    let tip = git(repo_dir, &["rev-parse", "--verify", "-q", &format!("{full}^{{commit}}")]).ok();
    let why = match tip.as_deref() {
        Some(t) if t == head => return,
        // An empty old value: the branch must still not exist.
        None => git(repo_dir, &["update-ref", &full, head, ""]).err(),
        Some(t) => {
            let list = git(repo_dir, &["worktree", "list", "--porcelain"]).unwrap_or_default();
            let pinned = git(repo_dir, &["for-each-ref", "--format=%(refname)", &format!("--points-at={t}"), "refs/switchboard/"])
                .unwrap_or_default();
            if list.lines().any(|l| l == format!("branch {full}")) {
                Some("it is checked out".to_owned())
            } else if pinned.trim().is_empty() {
                Some("its tip is not a head that a review pinned".to_owned())
            } else {
                git(repo_dir, &["update-ref", &full, head, t]).err()
            }
        }
    };
    if let Some(why) = why {
        tracing::info!(target: "sb::review", %branch, %head, %why, "the PR branch stays where it is");
    }
}

/// Fetches `src` of `remote` and returns its commit. The fetch goes to a ref of its
/// own, deleted after, so two fetches at the same time do not mix.
fn fetch_one(repo_dir: &Path, remote: &str, src: &str) -> Result<String, String> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = format!(
        "refs/switchboard/fetch/{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let oid = git(repo_dir, &["fetch", "-q", "--no-tags", remote, &format!("+{src}:{tmp}")])
        .and_then(|_| git(repo_dir, &["rev-parse", &format!("{tmp}^{{commit}}")]));
    let _ = git(repo_dir, &["update-ref", "-d", &tmp]);
    oid
}

/// Puts `head` in the worktree `name` next to `repo_dir`, detached. Makes the
/// worktree when it is missing.
pub fn checkout_tree(repo_dir: &Path, name: &str, head: &str) -> Result<PathBuf, String> {
    checkout_tree_for(repo_dir, name, head, None)
}

/// As `checkout_tree`. `own` is the worktree of the open review that checks out:
/// an Update of it moves its HEAD as before, and git carries your edits along or
/// stops on a conflict. Any other existing folder must be free of work.
pub fn checkout_tree_for(repo_dir: &Path, name: &str, head: &str, own: Option<&Path>) -> Result<PathBuf, String> {
    let tree = worktree_path(repo_dir, name);
    tracing::info!(target: "sb::review", tree = %tree.display(), exists = tree.exists(), "PR worktree");
    if tree.exists() && own.is_some_and(|o| same_path(o, &tree)) {
        git(&tree, &["checkout", "-q", "--detach", head])?;
    } else if tree.exists() {
        // A folder that a closed review kept with your work: a checkout would carry
        // the changes into this review or leave the commits to `git gc`.
        // A folder of the same name that is not a review worktree is yours.
        let st = tree_state(repo_dir, &tree)?;
        if !st.worktree || !st.detached {
            return Err(format!(
                "{} exists and is not a review worktree of this clone. Move it, then use Retry.",
                tree.display()
            ));
        }
        let work = st.changes;
        if !work.is_empty() {
            return Err(format!(
                "{} has work that is not in git ({}). Commit it, move it, or remove the folder, then use Retry.",
                tree.display(),
                work.iter().take(3).cloned().collect::<Vec<_>>().join("; ")
            ));
        }
        git(&tree, &["checkout", "-q", "--detach", head])?;
    } else {
        git(
            repo_dir,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                &tree.to_string_lossy(),
                head,
            ],
        )?;
    }
    Ok(tree)
}

/// Keeps the head and the base of a review alive, with a ref of its own for each:
/// `refs/switchboard/<review>/head` and `.../base`. A later round needs them. Without
/// the refs, a removed worktree and a force-push leave them to `git gc`. The base can
/// be a tree id (`round_base`); a ref keeps a tree too.
pub fn pin(repo_dir: &Path, review: &str, head: &str, base: &str) -> Result<(), String> {
    if review.is_empty() || !review.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("bad review id {review:?}"));
    }
    for (name, oid) in [("head", head), ("base", base)] {
        if !oid.is_empty() {
            git(repo_dir, &["update-ref", &format!("refs/switchboard/{review}/{name}"), oid])?;
        }
    }
    Ok(())
}

/// What a removal of a review worktree would lose, and whether it is one.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TreeState {
    /// The folder exists.
    pub exists: bool,
    /// `git worktree list` of the main checkout has it, and it is not the main checkout.
    pub worktree: bool,
    /// It has no branch checked out. Review worktrees are always detached.
    pub detached: bool,
    /// `git worktree lock` was run on it.
    pub locked: bool,
    /// Changed and untracked files (not ignored ones), and commits made in it.
    pub changes: Vec<String>,
}

/// True when two paths name the same folder, also through a symlink.
pub fn same_path(a: &Path, b: &Path) -> bool {
    let real = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    a == b || real(a) == real(b)
}

/// The state of `tree`, a worktree of the repo at `main`.
pub fn tree_state(main: &Path, tree: &Path) -> Result<TreeState, String> {
    let mut st = TreeState {
        exists: tree.is_dir(),
        ..TreeState::default()
    };
    let list = git(main, &["worktree", "list", "--porcelain"])?;
    if let Some(w) = crate::repos::parse_worktrees(&list, main)
        .into_iter()
        .find(|w| same_path(Path::new(&w.path), tree))
    {
        st.worktree = true;
        st.detached = w.branch == "detached";
        st.locked = list.split("\n\n").any(|b| {
            b.lines().next().and_then(|l| l.strip_prefix("worktree ")).is_some_and(|p| same_path(Path::new(p), tree))
                && b.lines().any(|l| l == "locked" || l.starts_with("locked "))
        });
    }
    if !st.exists || !st.worktree {
        return Ok(st);
    }
    st.changes = local_changes(main, tree)?;
    Ok(st)
}

/// The work in a worktree that only the folder holds: changed and untracked files
/// (not ignored ones), and commits that no branch, tag, remote or pin reaches.
pub fn local_changes(main: &Path, tree: &Path) -> Result<Vec<String>, String> {
    // core.quotePath=false: the list shows "café.rs", not "caf\303\251.rs".
    let status = git(tree, &["-c", "core.quotePath=false", "status", "--porcelain", "--untracked-files=all"])?;
    let mut out: Vec<String> = status.lines().map(|l| l.trim_end().to_owned()).filter(|l| !l.is_empty()).collect();
    let head = git(tree, &["rev-parse", "HEAD"])?;
    let own = git(
        main,
        &["rev-list", "--max-count=20", &head, "--not", "--branches", "--tags", "--remotes", "--glob=refs/switchboard/*"],
    )?;
    out.extend(own.lines().filter(|l| !l.is_empty()).map(|c| format!("commit {} made in the worktree", &c[..c.len().min(12)])));
    Ok(out)
}

/// Removes the worktree `tree` of the repo at `main`. With `force`, its changes go too.
pub fn remove_tree(main: &Path, tree: &Path, force: bool) -> Result<(), String> {
    let path = tree.to_string_lossy();
    let mut args = vec!["worktree", "remove"];
    if force {
        // Once: a locked worktree still refuses. A lock is your "do not remove".
        args.push("--force");
    }
    args.push(&path);
    git(main, &args)?;
    let _ = git(main, &["worktree", "prune"]);
    Ok(())
}

/// The prompt for the guide session.
/// `base` is the merge base that the checker uses: the agent diffs against it.
pub fn prompt(template: &str, pr: &PrInfo, head: &str, base: &str, specs: &[String], web: &[String]) -> String {
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
    let web = if web_rules(web).is_empty() {
        "(none)".to_owned()
    } else {
        web.iter()
            .map(|d| format!("- {}", d.trim()))
            .collect::<Vec<_>>()
            .join("\n")
    };
    template
        .replace("{{number}}", &pr.number.to_string())
        .replace("{{title}}", &pr.title)
        .replace("{{head}}", head)
        .replace("{{base}}", base)
        .replace("{{base_ref}}", &pr.base_ref_name)
        .replace("{{files}}", &files.join("\n"))
        .replace("{{body}}", pr.body.trim())
        .replace("{{specs}}", &specs)
        .replace("{{web}}", &web)
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
    /// The process started, in a process group of its own with this id. A close
    /// of the review stops the group (`stop_run`).
    Started {
        pid: u32,
    },
    /// The output ended. Next, `run` reaps the process: its pid is then free for
    /// another process, so nothing may signal it after this event.
    Exited,
    Init {
        session: String,
        /// MCP servers that failed to start.
        failed: Vec<String>,
    },
    Delta(String),
    Result {
        text: String,
        error: bool,
        session: Option<String>,
        /// The tool calls that were refused, one line each: a site that is not
        /// in `review.web_domains`, a `curl`.
        denied: Vec<String>,
    },
}

/// One line for a refused tool call of a `result` event: the tool and its URL,
/// command or path.
fn denial(d: &Value) -> Option<String> {
    let tool = d.get("tool_name")?.as_str()?;
    let input = d.get("tool_input");
    let what = ["url", "query", "command", "file_path"]
        .iter()
        .find_map(|k| input.and_then(|i| i.get(*k)).and_then(Value::as_str));
    Some(match what {
        Some(w) => format!("{tool} {}", crate::proto::first_line(w, 100)),
        None => tool.to_owned(),
    })
}

pub fn parse_stream(line: &str) -> Option<Stream> {
    let v: Value = serde_json::from_str(line).ok()?;
    match v.get("type")?.as_str()? {
        "system" if v.get("subtype").and_then(Value::as_str) == Some("init") => {
            let failed = v
                .get("mcp_servers")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter(|m| m.get("status").and_then(Value::as_str) == Some("failed"))
                        .filter_map(|m| m.get("name").and_then(Value::as_str).map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            Some(Stream::Init {
                session: v.get("session_id")?.as_str()?.to_owned(),
                failed,
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
            denied: v
                .get("permission_denials")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(denial).collect())
                .unwrap_or_default(),
        }),
        _ => None,
    }
}

/// The git empty tree: the base of a root commit.
pub const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// The commits of a local review (SPEC 11.11): `base..head`, where `head` is HEAD.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalRange {
    pub head: String,
    pub base: String,
    /// The commits in the range.
    pub count: u32,
    /// What the base is, for the header: "Commits not on origin/x (...).", or why it is not.
    pub note: String,
    /// False when nothing is unpushed and the range is the last commit.
    pub unpushed: bool,
    /// The ref the base came from, for example `upstream/unstable`. None when the base
    /// is where the history leaves the remotes.
    pub base_ref: Option<String>,
    /// The ref and why it is the base: "upstream/unstable (sigp/lighthouse, the parent of this fork)".
    pub label: Option<String>,
}

/// More commits than this between the base and HEAD, and the header says the base
/// may be wrong.
pub const FAR_BASE: u32 = 500;

type Found = (String, String, Option<(String, String)>);

/// The unpushed commits of the checkout at `tree`. The base is, in order:
/// 0. the merge base with the ref you chose (`rule.pick`), while it exists;
/// 1. the merge base with the parent of a fork, then with the push or upstream
///    branch, when HEAD has one (`base::auto_candidates`);
/// 2. else, with a remote, the one commit where the history leaves every remote
///    (a detached agent worktree), or the merge base with the remote's default
///    branch when there are more;
/// 3. else the merge base with a local `main` or `master` that HEAD is not on.
///
/// With no unpushed commit, the range is the last commit, and the note says so.
pub fn local_range(tree: &Path, rule: &crate::base::BaseRule) -> Result<LocalRange, String> {
    let head = git(tree, &["rev-parse", "--verify", "HEAD^{commit}"]).map_err(|_| "this checkout has no commit yet".to_owned())?;
    let ok = |args: &[&str]| git(tree, args).ok().filter(|s| !s.is_empty());
    // The base commit, the note, and the ref with why.
    let from_ref = |name: &str, why: &str| -> Option<Found> {
        let b = ok(&["merge-base", &head, name])?;
        Some((b, format!("Commits not on {name} ({why})."), Some((name.to_owned(), why.to_owned()))))
    };
    let mut found: Option<Found> = None;
    let mut lost = String::new();
    if let Some(p) = rule.pick.as_deref() {
        found = crate::base::exists(tree, p).then(|| from_ref(p, "your choice")).flatten();
        if found.is_none() {
            lost = format!("The base you chose, {p}, is not in this repo any more, so this uses the usual rule. ");
        }
    }
    if found.is_none() {
        found = crate::base::auto_candidates(tree, &rule.fork).iter().find_map(|c| from_ref(&c.name, &c.why));
    }
    let has_remote = ok(&["remote"]).is_some();
    if found.is_none() && has_remote {
        let walk = git(tree, &["rev-list", "--boundary", &head, "--not", "--remotes"])?;
        let ours = walk.lines().filter(|l| !l.starts_with('-')).count();
        let edges: Vec<&str> = walk.lines().filter_map(|l| l.strip_prefix('-')).collect();
        if ours == 0 {
            found = Some((head.clone(), "Every commit is on a remote.".into(), None));
        } else if edges.len() == 1 {
            found = Some((edges[0].to_owned(), "Commits that no remote has.".into(), None));
        } else {
            // The default branch of a remote: `origin` first, then the others.
            let mut remotes: Vec<String> = ok(&["remote"]).unwrap_or_default().lines().map(str::to_owned).collect();
            remotes.sort_by_key(|r| r != "origin");
            let default = remotes.iter().find_map(|r| {
                ok(&["symbolic-ref", "--short", &format!("refs/remotes/{r}/HEAD")]).or_else(|| {
                    ["main", "master"]
                        .into_iter()
                        .map(|b| format!("{r}/{b}"))
                        .find(|b| ok(&["rev-parse", "--verify", "-q", &format!("refs/remotes/{b}")]).is_some())
                })
            });
            if let Some(d) = default {
                found = from_ref(&d, "the default branch of a remote");
            }
        }
    }
    if found.is_none() {
        let branch = ok(&["symbolic-ref", "--short", "-q", "HEAD"]);
        for main in ["main", "master"] {
            if branch.as_deref() != Some(main) && ok(&["rev-parse", "--verify", "-q", &format!("refs/heads/{main}")]).is_some() {
                if let Some(f) = from_ref(main, "a local branch") {
                    found = Some(f);
                    break;
                }
            }
        }
    }
    let (mut base, mut note, named) = found.unwrap_or_else(|| (head.clone(), "No branch to compare with.".into(), None));
    note = lost + &note;
    let unpushed = base != head;
    if !unpushed {
        // Nothing unpushed: show the last commit, so the tab is still useful. Only a
        // true root commit compares with the empty tree: in a shallow clone the parent
        // is missing, and the whole repo would show as new.
        base = match ok(&["rev-parse", "--verify", "-q", &format!("{head}~1")]) {
            Some(p) => p,
            None if ok(&["rev-parse", "--is-shallow-repository"]).as_deref() == Some("true") => {
                return Err("No unpushed commits, and this shallow clone does not have the commit before HEAD.".into());
            }
            None => EMPTY_TREE.to_owned(),
        };
        note = format!("No unpushed commits. {note} This shows the last commit.");
    }
    let count = if base == EMPTY_TREE {
        1
    } else {
        git(tree, &["rev-list", "--count", &format!("{base}..{head}")])?.parse().unwrap_or(0)
    };
    if unpushed && count > FAR_BASE {
        note += &format!(" The merge base is {count} commits back: this ref may be the wrong base.");
    }
    let (base_ref, label) = match named {
        Some((name, why)) => (Some(name.clone()), Some(format!("{name} ({why})"))),
        None => (None, None),
    };
    Ok(LocalRange { head, base, count, note, unpushed, base_ref, label })
}

/// The work tree of a folder as a commit nobody sees (SPEC 11.14): the agent's
/// uncommitted work, new files too, on top of HEAD.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// The snapshot commit, or HEAD when the work tree is clean.
    pub commit: String,
    /// HEAD when the snapshot was made.
    pub head: String,
    /// New files left out because they are bigger than `SNAP_MAX`.
    pub skipped: Vec<String>,
}

/// A new file bigger than this stays out of a snapshot: a snapshot writes into the
/// repo's object store, and a dataset or a build output does not belong there.
pub const SNAP_MAX: u64 = 5 << 20;

/// The message of a snapshot commit.
pub const SNAP_SUBJECT: &str = "Switchboard snapshot of the work tree";

/// Makes a snapshot of `tree`. It never changes the files, the index or HEAD:
/// git works on a copy of the worktree's own index (`GIT_INDEX_FILE`), so it also
/// never takes `index.lock`, and an agent's git command runs as usual. The copy
/// keeps the stat cache and the sparse checkout bits, so `add -A` reads only the
/// files that changed, and a file outside a sparse checkout is not "deleted".
pub fn snapshot(tree: &Path) -> Result<Snapshot, String> {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let head = git(tree, &["rev-parse", "--verify", "HEAD"])?;
    let index = PathBuf::from(git(tree, &["rev-parse", "--path-format=absolute", "--git-path", "index"])?);
    let tmp = std::env::temp_dir().join(format!(
        "sb-snap-{}-{}.index",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    struct Gone(PathBuf);
    impl Drop for Gone {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
            let _ = std::fs::remove_file(self.0.with_extension("index.lock"));
        }
    }
    let _gone = Gone(tmp.clone());
    let run = |args: &[&str]| -> Result<String, String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(tree)
            .args(args)
            .env("GIT_INDEX_FILE", &tmp)
            // commit-tree needs a name; the user may have none set.
            .env("GIT_AUTHOR_NAME", "Switchboard")
            .env("GIT_AUTHOR_EMAIL", "switchboard@localhost")
            .env("GIT_COMMITTER_NAME", "Switchboard")
            .env("GIT_COMMITTER_EMAIL", "switchboard@localhost")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("git: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_owned())
    };
    // Git swaps a new index in with a rename, so a copy is never half written.
    match std::fs::copy(&index, &tmp) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            run(&["read-tree", "HEAD"])?;
        }
        Err(e) => return Err(format!("{}: {e}", index.display())),
    }
    let mut skipped = vec![];
    for f in run(&["ls-files", "-z", "--others", "--exclude-standard"])?.split('\0').filter(|f| !f.is_empty()) {
        if std::fs::metadata(tree.join(f)).map(|m| m.len() > SNAP_MAX).unwrap_or(false) {
            skipped.push(f.to_owned());
        }
    }
    let excludes: Vec<String> = skipped.iter().map(|f| format!(":(exclude,literal){f}")).collect();
    let mut add = vec!["add", "-A", "--", "."];
    add.extend(excludes.iter().map(String::as_str));
    run(&add)?;
    let work = run(&["write-tree"])?;
    let commit = if work == git(tree, &["rev-parse", "HEAD^{tree}"])? {
        head.clone()
    } else {
        // Never signed: a signature could ask for a passphrase.
        run(&["commit-tree", "--no-gpg-sign", &work, "-p", &head, "-m", SNAP_SUBJECT])?
    };
    Ok(Snapshot { commit, head, skipped })
}

/// The ref that keeps a snapshot of a local review from `git gc`: `head` (the
/// view's head) or `reviewed` (Mark reviewed). Not under `refs/switchboard`:
/// `local_changes` leaves out commits that refs there reach, and a snapshot's parent
/// is the agent's HEAD, so its unpushed commits would stop counting as work.
pub fn snap_ref(review: &str, name: &str) -> String {
    format!("refs/sb/review/{review}/{name}")
}

/// Deletes every snapshot ref of a review. Worktrees share one ref store, so this
/// works from any folder of the repo.
/// A failure only logs: with the folder gone there is no repo to clean.
pub fn drop_snap_refs(tree: &Path, review: &str) {
    let refs = match git(tree, &["for-each-ref", "--format=%(refname)", &format!("refs/sb/review/{review}/")]) {
        Ok(r) => r,
        Err(err) => {
            tracing::warn!(target: "sb::review", review, tree = %tree.display(), %err, "the snapshot refs stay: git cannot read the folder");
            return;
        }
    };
    for r in refs.lines().filter(|r| !r.is_empty()) {
        if let Err(err) = git(tree, &["update-ref", "-d", r]) {
            tracing::warn!(target: "sb::review", review, %r, %err, "a snapshot ref stays");
        }
    }
}

/// Stops a `claude -p` run and every process of its group: its tools and its MCP
/// server. `pid` is the group id from `Stream::Started`.
pub fn stop_run(pid: u32) {
    signal_group(pid, libc::SIGTERM);
}

fn signal_group(pid: u32, sig: libc::c_int) {
    // A pid of 0 or 1 as a group would reach far more than the run.
    if pid > 1 {
        // SAFETY: kill(2) takes plain integers and reads no memory.
        unsafe {
            libc::kill(-(pid as libc::pid_t), sig);
        }
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
    /// Folders outside `cwd` that the agent can read: the spec clones.
    pub dirs: &'a [String],
    /// The sites the agent can fetch (`review.web_domains`).
    pub web: &'a [String],
    /// WebSearch on. Off, it is denied: Claude Code has it on by default.
    pub search: bool,
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
            "manual",
            "--setting-sources",
            SETTING_SOURCES,
        ]
        .map(String::from),
    );
    if let Some(r) = run.resume {
        a.extend(["--resume".into(), r.into()]);
        if run.fork {
            a.push("--fork-session".into());
        }
    }
    let mut allowed = vec![ALLOWED.to_owned()];
    allowed.extend(web_rules(run.web));
    let mut denied = DENIED.to_owned();
    if run.search {
        allowed.push("WebSearch".into());
    } else {
        denied += ",WebSearch";
    }
    a.extend([
        "--allowedTools".into(),
        allowed.join(","),
        "--disallowedTools".into(),
        denied,
    ]);
    for d in run.dirs {
        a.extend(["--add-dir".into(), d.clone()]);
    }
    if let Some((sb, review, sock)) = run.mcp {
        let cfg = json!({ "mcpServers": { "sb": {
            "command": sb.to_string_lossy(), "args": ["mcp"],
            "env": {
                "SB_REVIEW": review,
                "SB_SOCK": sock.to_string_lossy(),
                // The `git` tool runs here.
                "SB_DIR": run.cwd.to_string_lossy(),
            }
        }}});
        a.extend(["--mcp-config".into(), cfg.to_string()]);
    }
    // Always: the PR's own `.mcp.json` never starts a server.
    a.push("--strict-mcp-config".into());
    a
}

/// Runs `claude -p` and calls `on` for each stream event. Returns the final result.
/// The longest one `claude -p` run may take. A stuck run is killed, so the
/// review shows an error and Retry, not "writing" for good.
pub const RUN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

pub fn run(run: &Run, mut on: impl FnMut(&Stream)) -> Result<Stream, String> {
    if let Some((sb, _, _)) = run.mcp {
        if !exe_found(sb) {
            return Err(format!(
                "The sb helper is missing at {}. Rebuild Switchboard or run the install again.",
                sb.display()
            ));
        }
    }
    use std::os::unix::process::CommandExt;
    let mut child = Command::new("claude")
        .args(args(run))
        // Its own process group, so a stop reaches its tools and its MCP server too.
        .process_group(0)
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
    on(&Stream::Started { pid });
    let t0 = std::time::Instant::now();
    tracing::info!(target: "sb::claude", pid, cwd = %run.cwd.display(), resume = ?run.resume, fork = run.fork, prompt = %crate::proto::first_line(run.prompt, 100), "claude -p started");
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let watchdog = std::thread::spawn(move || {
        if done_rx.recv_timeout(RUN_TIMEOUT).is_err() {
            signal_group(pid, libc::SIGKILL);
            return true;
        }
        false
    });
    let mut last = None;
    let mut mcp_failed = None;
    for line in BufReader::new(out).lines().map_while(Result::ok) {
        if let Some(ev) = parse_stream(&line) {
            match &ev {
                Stream::Init { session, failed } => {
                    tracing::info!(target: "sb::claude", pid, %session, ?failed, ms = t0.elapsed().as_millis() as u64, "claude -p session");
                    // Without the sb tools the agent cannot write the guide: stop now.
                    if !failed.is_empty() {
                        let _ = child.kill();
                        mcp_failed = Some(failed.join(", "));
                        break;
                    }
                }
                Stream::Result { error, denied, .. } => {
                    tracing::info!(target: "sb::claude", pid, error, ?denied, ms = t0.elapsed().as_millis() as u64, "claude -p result")
                }
                Stream::Delta(_) | Stream::Started { .. } | Stream::Exited => {}
            }
            on(&ev);
            if matches!(ev, Stream::Result { .. }) {
                last = Some(ev);
            }
        }
    }
    on(&Stream::Exited);
    let status = child.wait().map_err(|e| e.to_string())?;
    let _ = done_tx.send(());
    let killed = watchdog.join().unwrap_or(false);
    let err = err_text.join().unwrap_or_default();
    tracing::info!(target: "sb::claude", pid, ms = t0.elapsed().as_millis() as u64, %status, killed, got_result = last.is_some(), "claude -p ended");
    if !err.trim().is_empty() {
        tracing::debug!(target: "sb::claude", pid, stderr = %err.trim(), "claude -p stderr");
    }
    if let Some(names) = mcp_failed {
        let sb = run.mcp.map(|(sb, _, _)| sb.display().to_string()).unwrap_or_default();
        return Err(format!(
            "The MCP server {names} failed to start ({sb}). Rebuild Switchboard or run the install again."
        ));
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

/// True when `p` is a file, or a bare name found on `PATH`.
fn exe_found(p: &Path) -> bool {
    if p.components().count() > 1 {
        return p.is_file();
    }
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|d| d.join(p).is_file()))
        .unwrap_or(false)
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
        // -z: names come raw, each ending with NUL. Without it git quotes a name
        // ("caf\303\251.rs"), and the quoted name matches no file.
        .args(["merge-tree", "-z", "--write-tree", "--no-messages", "--name-only", since, new_base])
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git merge-tree: {e}"))?;
    // Exit 0: a clean merge. Exit 1: conflicts, and the tree has conflict markers.
    // The output: the tree, then each conflicted name, then an empty field before the
    // messages (which --no-messages leaves out). Each field ends with NUL.
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut lines = text.split('\0');
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
            // --literal-pathspecs: a name such as "a*.rs" matches only itself.
            let entry = git(dir, &["--literal-pathspecs", "ls-tree", "-z", since, "--", path])?;
            match entry.split_once('\t') {
                // "<mode> blob <oid>\t<path>" becomes "<mode> <oid>\t<path>".
                Some((meta, _)) => {
                    let parts: Vec<&str> = meta.split_whitespace().collect();
                    info.push_str(&format!("{} {}\t{path}\0", parts[0], parts[2]));
                }
                // No such file at `since`: mode 0 removes it. The zero id is as long
                // as the repo's ids (SHA-1 or SHA-256).
                None => info.push_str(&format!("0 {}\t{path}\0", "0".repeat(tree.len()))),
            }
        }
        // -z: each entry ends with NUL, and git does not unquote the name.
        run(&["update-index", "-z", "--index-info"], Some(&info))?;
        run(&["write-tree"], None)
    })();
    let _ = std::fs::remove_file(&index);
    Ok((result?, conflicts))
}

/// The files that differ between `old` and `new`. -z keeps each name raw: without it
/// git quotes a name with a non-ASCII byte, a tab or a quote.
pub fn changed_files(tree: &Path, old: &str, new: &str) -> Result<Vec<String>, String> {
    Ok(git(tree, &["diff", "-z", "--name-only", "--no-ext-diff", old, new])?
        .split('\0')
        .filter(|f| !f.is_empty())
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

    /// git quotes these names without -z: "caf\303\251.rs", "a\tb.rs".
    const ODD_NAMES: [&str; 4] = ["café.rs", "a b.rs", "a\tb.rs", "q\"uote.rs"];

    /// A conflict on a file with an odd name keeps the file at its round-1 version.
    /// Before -z, the quoted name matched nothing and the file left the round base.
    #[test]
    fn a_conflict_on_an_odd_name_keeps_the_file() {
        let dir = std::env::temp_dir().join(format!("sb-round-{}-odd", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = |args: &[&str]| git(&dir, args).unwrap();
        let put = |text: &str| {
            for n in ODD_NAMES {
                std::fs::write(dir.join(n), format!("1\n{text}\n3\n")).unwrap();
            }
        };
        g(&["init", "-q", "-b", "main"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "t"]);
        put("2");
        g(&["add", "."]);
        g(&["commit", "-qm", "base"]);
        g(&["checkout", "-qb", "pr"]);
        put("TWO");
        g(&["commit", "-qam", "round 1"]);
        let since = g(&["rev-parse", "HEAD"]);
        g(&["checkout", "-q", "main"]);
        put("zwei");
        g(&["commit", "-qam", "main"]);
        let new_base = g(&["rev-parse", "HEAD"]);
        let (tree, mut conflicts) = replay(&dir, &since, &new_base).unwrap();
        let mut want: Vec<String> = ODD_NAMES.iter().map(|n| n.to_string()).collect();
        want.sort();
        conflicts.sort();
        assert_eq!(conflicts, want);
        for n in ODD_NAMES {
            let spec = format!("{tree}:{n}");
            assert_eq!(git(&dir, &["show", &spec]).unwrap(), "1\nTWO\n3", "{n:?}");
        }
        // changed_files gives the raw names too.
        let mut changed = changed_files(&dir, &since, &new_base).unwrap();
        changed.sort();
        assert_eq!(changed, want);
        let _ = std::fs::remove_dir_all(&dir);
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

    /// A stop reaches the whole group: a tool that the run started stops too.
    #[test]
    fn stop_run_stops_the_group() {
        use std::os::unix::process::CommandExt;
        let mut child = Command::new("sh")
            .args(["-c", "sleep 30 & echo $!; wait"])
            .process_group(0)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        let tool: i32 = line.trim().parse().unwrap();
        // SAFETY: kill with signal 0 only checks that the process exists.
        let alive = |pid: i32| unsafe { libc::kill(pid, 0) } == 0;
        assert!(alive(tool));
        stop_run(child.id());
        child.wait().unwrap();
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while alive(tool) && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!alive(tool), "the tool of the run still runs");
        // Pids 0 and 1 as a group are never signalled.
        stop_run(0);
        stop_run(1);
    }

    #[test]
    fn stream_lines() {
        assert_eq!(
            parse_stream(r#"{"type":"system","subtype":"init","session_id":"s1"}"#),
            Some(Stream::Init {
                session: "s1".into(),
                failed: vec![]
            })
        );
        assert_eq!(
            parse_stream(r#"{"type":"system","subtype":"init","session_id":"s1","mcp_servers":[{"name":"sb","status":"failed","source":"dynamic"},{"name":"x","status":"connected"}]}"#),
            Some(Stream::Init {
                session: "s1".into(),
                failed: vec!["sb".into()]
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
                session: Some("s1".into()),
                denied: vec![],
            })
        );
        // The refused calls come from `permission_denials`, with the URL or command.
        let deny = r#"{"type":"result","subtype":"success","result":"x","permission_denials":[{"tool_name":"WebFetch","tool_input":{"url":"https://example.com","prompt":"p"}},{"tool_name":"Bash","tool_input":{"command":"curl x\ny"}},{"tool_name":"Odd"}]}"#;
        match parse_stream(deny) {
            Some(Stream::Result { denied, .. }) => {
                assert_eq!(denied, ["WebFetch https://example.com", "Bash curl x", "Odd"])
            }
            other => panic!("{other:?}"),
        }
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
            dirs: &[],
            web: &[],
            search: false,
        });
        let s = a.join(" ");
        assert!(s.contains("--resume g1 --fork-session"));
        assert!(s.contains("--disallowedTools Edit,Write"));
        // Search off is a deny: Claude Code has WebSearch on by default.
        let i = a.iter().position(|x| x == "--disallowedTools").unwrap();
        assert_eq!(a[i + 1], format!("{DENIED},WebSearch"));
        assert!(!s.contains("WebFetch"));
        assert!(s.contains("--verbose"));
        assert_eq!(&a[..2], ["-p", "q"]);
        // `default` is a hidden old name; Claude Code lists `manual`.
        assert!(s.contains("--permission-mode manual"));
        // The PR's own settings and MCP servers never load.
        assert!(s.contains("--setting-sources user "));
        assert_eq!(a.last().unwrap(), "--strict-mcp-config");
        // git is a tool of `sb mcp`; Bash is denied, so `git diff --output=` cannot run.
        assert!(!ALLOWED.contains("Bash") && DENIED.split(',').any(|d| d == "Bash"));
        assert!(ALLOWED.split(',').any(|t| t == "mcp__sb__git"));
        let b = args(&Run {
            cwd: Path::new("/"),
            prompt: "q",
            resume: Some("t1"),
            fork: false,
            env: &[],
            mcp: Some((Path::new("/sb"), "r1", Path::new("/s"))),
            dirs: &["/specs a".into()],
            web: &["github.com".into(), " eips.ethereum.org ".into(), "x,Bash".into(), "".into()],
            search: true,
        });
        let i = b.iter().position(|x| x == "--allowedTools").unwrap();
        assert_eq!(
            b[i + 1],
            format!("{ALLOWED},WebFetch(domain:github.com),WebFetch(domain:eips.ethereum.org),WebSearch")
        );
        assert_eq!(b[i + 3], DENIED);
        assert!(!b.contains(&"--fork-session".to_owned()));
        let i = b.iter().position(|x| x == "--add-dir").unwrap();
        assert_eq!(b[i + 1], "/specs a");
        let i = b.iter().position(|x| x == "--mcp-config").unwrap();
        let cfg: Value = serde_json::from_str(&b[i + 1]).unwrap();
        assert_eq!(cfg["mcpServers"]["sb"]["env"]["SB_REVIEW"], "r1");
        assert_eq!(cfg["mcpServers"]["sb"]["env"]["SB_DIR"], "/");
        assert_eq!(b.iter().filter(|x| *x == "--strict-mcp-config").count(), 1);
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
        let p = prompt(DEFAULT_PROMPT, &pr, "abc123", "def456", &["~/specs".into()], &["eips.ethereum.org".into()]);
        assert!(!p.contains("{{"), "unfilled field in:\n{p}");
        assert!(p.contains("- eips.ethereum.org"));
        // The agent diffs against the base the checker uses, not a branch that can be old.
        assert!(p.contains(r#"["-U0", "def456", "HEAD"]"#), "{p}");
        assert!(!p.contains("merge-base HEAD"), "{p}");
        let none = prompt(DEFAULT_PROMPT, &pr, "abc123", "def456", &[], &[]);
        assert!(none.contains("Sites you can fetch:\n(none)"));
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

    #[test]
    fn the_pr_branch_never_moves_your_work() {
        let dir = std::env::temp_dir().join(format!("sb-prbranch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let commit = |d: &Path, m: &str| {
            git(d, &["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "commit", "-q", "--allow-empty", "-m", m]).unwrap();
            git(d, &["rev-parse", "HEAD"]).unwrap()
        };
        git(&dir, &["init", "-q", "-b", "main"]).unwrap();
        let h1 = commit(&dir, "h1");
        let h2 = commit(&dir, "h2");
        let h3 = commit(&dir, "h3");
        let tip = || git(&dir, &["rev-parse", "refs/heads/sb-pr-7"]).unwrap();
        // No branch: it is made.
        point_pr_branch(&dir, "sb-pr-7", &h1);
        assert_eq!(tip(), h1);
        // A tip that no review pinned can be a commit of yours: it stays.
        point_pr_branch(&dir, "sb-pr-7", &h2);
        assert_eq!(tip(), h1);
        // A pinned tip moves to the new head.
        pin(&dir, "r1", &h1, "").unwrap();
        point_pr_branch(&dir, "sb-pr-7", &h2);
        assert_eq!(tip(), h2);
        // You check the branch out and commit on it: it stays, even at a pinned tip.
        pin(&dir, "r2", &h2, "").unwrap();
        let wt = dir.join("wt");
        git(&dir, &["worktree", "add", "-q", &wt.to_string_lossy(), "sb-pr-7"]).unwrap();
        point_pr_branch(&dir, "sb-pr-7", &h3);
        assert_eq!(tip(), h2, "a checked-out branch stays");
        let mine = commit(&wt, "mine");
        git(&wt, &["checkout", "-q", "--detach"]).unwrap();
        point_pr_branch(&dir, "sb-pr-7", &h3);
        assert_eq!(tip(), mine, "your commit stays");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
