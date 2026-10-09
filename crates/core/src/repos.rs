//! The repo list for the launcher (SPEC 9.1), and worktrees.

use serde::Serialize;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Repo {
    pub name: String,
    pub path: String,
    pub branch: String,
    pub dirty: bool,
    pub worktrees: Vec<Worktree>,
    /// Seconds since the epoch of the last Claude session here. 0 when none.
    pub last_used: u64,
    pub rust: bool,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Worktree {
    pub path: String,
    pub branch: String,
}

pub fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let t0 = std::time::Instant::now();
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    let ms = t0.elapsed().as_millis() as u64;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        tracing::debug!(target: "sb::git", ms, dir = %dir.display(), ?args, %err, "git failed");
        return Err(err);
    }
    if ms > 1000 {
        tracing::warn!(target: "sb::git", ms, dir = %dir.display(), ?args, "slow git");
    } else {
        tracing::trace!(target: "sb::git", ms, dir = %dir.display(), ?args, "git");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_owned())
}

/// The main checkout of the repo that holds `dir`. A worktree maps to its main checkout.
pub fn main_checkout(dir: &Path) -> Option<PathBuf> {
    let common = git(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .ok()?;
    let common = PathBuf::from(common);
    if common.file_name()? == ".git" {
        return common.parent().map(Path::to_path_buf);
    }
    // A bare repo, or a plain .git dir somewhere else.
    git(dir, &["rev-parse", "--show-toplevel"])
        .ok()
        .map(PathBuf::from)
}

/// Every `cwd` in the Claude session files, with the newest file time for each.
pub fn session_dirs(projects: &Path) -> BTreeMap<PathBuf, u64> {
    let mut out = BTreeMap::new();
    let Ok(dirs) = std::fs::read_dir(projects) else {
        return out;
    };
    for d in dirs.flatten() {
        let Ok(files) = std::fs::read_dir(d.path()) else {
            continue;
        };
        let mut newest: Option<(u64, PathBuf)> = None;
        for f in files.flatten() {
            let p = f.path();
            if p.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let t = f
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if newest.as_ref().is_none_or(|(n, _)| t > *n) {
                newest = Some((t, p));
            }
        }
        let Some((t, file)) = newest else { continue };
        if let Some(cwd) = first_cwd(&file) {
            let e = out.entry(cwd).or_insert(0);
            *e = (*e).max(t);
        }
    }
    out
}

fn first_cwd(file: &Path) -> Option<PathBuf> {
    let f = std::fs::File::open(file).ok()?;
    for line in BufReader::new(f).lines().take(50).map_while(Result::ok) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
            if let Some(c) = v.get("cwd").and_then(|c| c.as_str()) {
                return Some(c.into());
            }
        }
    }
    None
}

/// Folders with a `.git` under `root`, at most `depth` levels down.
pub fn scan(root: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = vec![];
    if root.join(".git").exists() {
        out.push(root.to_path_buf());
        return out;
    }
    if depth == 0 {
        return out;
    }
    let Ok(rd) = std::fs::read_dir(root) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name();
        let hidden = name.to_string_lossy().starts_with('.');
        if p.is_dir() && !hidden && name != "target" && name != "node_modules" {
            out.extend(scan(&p, depth - 1));
        }
    }
    out
}

pub fn info(path: &Path, last_used: u64) -> Option<Repo> {
    let branch = git(path, &["branch", "--show-current"]).ok()?;
    let dirty = git(path, &["status", "--porcelain", "--untracked-files=no"])
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    let worktrees = git(path, &["worktree", "list", "--porcelain"])
        .map(|s| parse_worktrees(&s, path))
        .unwrap_or_default();
    Some(Repo {
        name: path.file_name()?.to_string_lossy().into_owned(),
        path: path.to_string_lossy().into_owned(),
        branch: if branch.is_empty() {
            "detached".into()
        } else {
            branch
        },
        dirty,
        worktrees,
        last_used,
        rust: path.join("Cargo.toml").exists(),
    })
}

/// Parses `git worktree list --porcelain`, without the main checkout. git prints
/// real paths, so `main` is compared as a real path: through a symlink (macOS
/// `/var` is `/private/var`) the two texts differ.
pub fn parse_worktrees(text: &str, main: &Path) -> Vec<Worktree> {
    let real = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let main = real(main);
    let mut out = vec![];
    for block in text.split("\n\n") {
        let mut path = None;
        let mut branch = "detached".to_owned();
        for l in block.lines() {
            if let Some(p) = l.strip_prefix("worktree ") {
                path = Some(p.to_owned());
            } else if let Some(b) = l.strip_prefix("branch ") {
                branch = b.trim_start_matches("refs/heads/").to_owned();
            }
        }
        if let Some(p) = path {
            if real(Path::new(&p)) != main {
                out.push(Worktree { path: p, branch });
            }
        }
    }
    out
}

/// The full repo list: Claude sessions first by time, then scanned repos by name.
pub fn discover(projects: &Path, roots: &[PathBuf]) -> Vec<Repo> {
    let mut repos: Vec<Repo> = candidates(projects, roots)
        .into_iter()
        .filter_map(|(p, t)| info(&p, t))
        .collect();
    repos.sort_by(|a, b| b.last_used.cmp(&a.last_used).then(a.name.cmp(&b.name)));
    repos
}

/// The main checkouts, with the last session time, and no `git status`. Fast.
pub fn candidates(projects: &Path, roots: &[PathBuf]) -> BTreeMap<PathBuf, u64> {
    let mut used: BTreeMap<PathBuf, u64> = BTreeMap::new();
    for (cwd, t) in session_dirs(projects) {
        if !cwd.is_dir() {
            continue;
        }
        if let Some(main) = main_checkout(&cwd) {
            let e = used.entry(main).or_insert(0);
            *e = (*e).max(t);
        }
    }
    for root in roots {
        for p in scan(root, 3) {
            if let Some(main) = main_checkout(&p) {
                used.entry(main).or_insert(0);
            }
        }
    }
    used
}

/// The folder for a new worktree: next to the repo, `<repo>-<branch>`.
pub fn worktree_path(repo: &Path, branch: &str) -> PathBuf {
    let name = repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let safe: String = branch
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    repo.with_file_name(format!("{name}-{safe}"))
}

/// Adds a worktree with a new branch, or with the branch when it exists.
pub fn add_worktree(repo: &Path, branch: &str) -> Result<PathBuf, String> {
    let path = worktree_path(repo, branch);
    if path.exists() {
        return Err(format!("{} exists", path.display()));
    }
    let p = path.to_string_lossy().into_owned();
    let exists = git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .is_ok();
    if exists {
        git(repo, &["worktree", "add", &p, branch])?;
    } else {
        git(repo, &["worktree", "add", "-b", branch, &p])?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_repo(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("sb-repos-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("myrepo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]).unwrap();
        git(
            &repo,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "x",
            ],
        )
        .unwrap();
        repo
    }

    #[test]
    fn the_main_checkout_is_left_out_through_a_symlink() {
        let repo = temp_repo("link");
        add_worktree(&repo, "feat/y").unwrap();
        let link = repo.parent().unwrap().join("linked");
        std::os::unix::fs::symlink(&repo, &link).unwrap();
        let r = info(&link, 0).unwrap();
        assert_eq!(r.worktrees.len(), 1, "{:?}", r.worktrees);
        assert_eq!(r.worktrees[0].branch, "feat/y");
        std::fs::remove_dir_all(repo.parent().unwrap()).unwrap();
    }

    #[test]
    fn worktrees_and_main_checkout() {
        let repo = temp_repo("wt");
        let wt = add_worktree(&repo, "feat/x").unwrap();
        assert_eq!(wt.file_name().unwrap(), "myrepo-feat-x");
        assert_eq!(main_checkout(&wt).unwrap(), repo.canonicalize().unwrap());
        let r = info(&repo, 0).unwrap();
        assert_eq!(r.branch, "main");
        assert_eq!(r.worktrees.len(), 1);
        assert_eq!(r.worktrees[0].branch, "feat/x");
        assert!(add_worktree(&repo, "feat/x").is_err());
        std::fs::remove_dir_all(repo.parent().unwrap()).unwrap();
    }

    #[test]
    fn session_files_give_cwds() {
        let repo = temp_repo("sess");
        let projects = repo.parent().unwrap().join("projects");
        std::fs::create_dir_all(projects.join("-x-myrepo")).unwrap();
        std::fs::write(
            projects.join("-x-myrepo/a.jsonl"),
            format!(
                "{{\"type\":\"summary\"}}\n{{\"cwd\":\"{}\"}}\n",
                repo.display()
            ),
        )
        .unwrap();
        let found = discover(&projects, &[]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "myrepo");
        assert!(found[0].last_used > 0);
        std::fs::remove_dir_all(repo.parent().unwrap()).unwrap();
    }

    #[test]
    fn scan_finds_nested_repos_and_skips_target() {
        let repo = temp_repo("scan");
        let root = repo.parent().unwrap();
        std::fs::create_dir_all(root.join("target/deep/.git")).unwrap();
        let found = scan(root, 3);
        assert_eq!(found, vec![repo.clone()]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn porcelain_parse() {
        let text =
            "worktree /a\nHEAD x\nbranch refs/heads/main\n\nworktree /a-pr1\nHEAD y\ndetached\n";
        assert_eq!(
            parse_worktrees(text, Path::new("/a")),
            vec![Worktree {
                path: "/a-pr1".into(),
                branch: "detached".into()
            }]
        );
    }
}
