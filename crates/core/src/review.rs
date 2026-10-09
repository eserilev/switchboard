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
    let tree = worktree_path(repo_dir, &format!("pr{}", pr.number));
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
pub fn changed_files(tree: &Path, old: &str, new: &str) -> Result<Vec<String>, String> {
    Ok(git(tree, &["diff", "--name-only", old, new])?
        .lines()
        .map(str::to_owned)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

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
