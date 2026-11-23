//! Repo context (SPEC 37): your own notes for a repo, kept in your Switchboard
//! config and not in the repo, that every agent of that repo gets.
//!
//! The notes of `owner/name` come from, in order:
//! 1. each source in `[repos."owner/name"] context`: a file, or a folder whose
//!    `.md` files are read by name. A shared pack goes here.
//! 2. `context/<owner>/<name>.md` in the config folder, when it exists: your own
//!    notes, last, so they can add to or change what a pack says.
//!
//! A folder belongs to every GitHub repo that one of its git remotes points at,
//! so a fork with an `upstream` remote gets the upstream repo's notes.

use crate::config::Config;
use std::path::{Path, PathBuf};

/// The most text that goes to an agent, in bytes.
pub const MAX: usize = 20_000;

/// One source of notes and what came of reading it.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub repo: String,
    pub path: PathBuf,
    /// The bytes read, or why it was not read.
    pub read: Result<usize, String>,
}

/// The notes for one folder.
#[derive(Debug, Default)]
pub struct Context {
    /// The GitHub repos of the folder, `owner/name`, in lower case.
    pub repos: Vec<String>,
    pub sources: Vec<Source>,
    /// What the agent gets. Empty when there are no notes.
    pub text: String,
}

/// The GitHub repos that the remotes of `dir` point at, in lower case, each once.
pub fn repos_of(dir: &Path) -> Vec<String> {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(["remote", "-v"]).output();
    match out {
        Ok(o) if o.status.success() => repos_in(&String::from_utf8_lossy(&o.stdout)),
        _ => vec![],
    }
}

/// The repos of `git remote -v` output.
pub fn repos_in(remote_v: &str) -> Vec<String> {
    let mut repos: Vec<String> = vec![];
    for l in remote_v.lines() {
        let Some(url) = l.split_whitespace().nth(1) else { continue };
        let Some(r) = crate::base::github_repo(url) else { continue };
        let r = r.to_lowercase();
        if !repos.contains(&r) {
            repos.push(r);
        }
    }
    repos
}

/// The files of notes for `repo`, in order. `config_dir` is the folder of `config.toml`.
pub fn paths_for(config: &Config, config_dir: &Path, repo: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = vec![];
    for (key, cfg) in &config.repos {
        if !key.eq_ignore_ascii_case(repo) {
            continue;
        }
        for s in &cfg.context {
            let p = crate::expand(s);
            out.push(if p.is_absolute() { p } else { config_dir.join(p) });
        }
    }
    let own = config_dir.join("context").join(format!("{repo}.md"));
    if own.exists() {
        out.push(own);
    }
    out
}

/// The notes for the folder `dir`.
pub fn for_dir(config: &Config, config_dir: &Path, dir: &Path) -> Context {
    build(config, config_dir, repos_of(dir))
}

/// The notes for these repos.
pub fn build(config: &Config, config_dir: &Path, repos: Vec<String>) -> Context {
    let mut sources: Vec<Source> = vec![];
    let mut seen: Vec<PathBuf> = vec![];
    let mut text = String::new();
    for repo in &repos {
        let mut body = String::new();
        for p in paths_for(config, config_dir, repo) {
            for f in files(&p) {
                let read = match f {
                    Err(err) => {
                        sources.push(Source { repo: repo.clone(), path: p.clone(), read: Err(err) });
                        continue;
                    }
                    Ok(f) if seen.contains(&f) => continue,
                    Ok(f) => {
                        seen.push(f.clone());
                        let r = std::fs::read_to_string(&f);
                        let read = match r {
                            Ok(t) => {
                                let t = t.trim();
                                if !t.is_empty() {
                                    body.push_str(t);
                                    body.push_str("\n\n");
                                }
                                Ok(t.len())
                            }
                            Err(err) => Err(err.to_string()),
                        };
                        (f, read)
                    }
                };
                sources.push(Source { repo: repo.clone(), path: read.0, read: read.1 });
            }
        }
        if !body.is_empty() {
            text.push_str(&format!(
                "The user's own notes for the repo {repo}, from their Switchboard config. They are not part of the repo, \
and other contributors do not see them. Use them when you work on, review or explain code of this repo.\n\n{body}"
            ));
        }
    }
    Context { repos, sources, text: cut(text.trim_end()) }
}

/// The files of one source: the file, or the `.md` files of a folder by name.
fn files(p: &Path) -> Vec<Result<PathBuf, String>> {
    if p.is_dir() {
        let Ok(rd) = std::fs::read_dir(p) else { return vec![Err("cannot read the folder".into())] };
        let mut fs: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|f| f.is_file() && f.extension().is_some_and(|x| x == "md"))
            .collect();
        fs.sort();
        fs.into_iter().map(Ok).collect()
    } else if p.exists() {
        vec![Ok(p.to_path_buf())]
    } else {
        vec![Err("not found".into())]
    }
}

/// Keeps the text within `MAX` bytes, and says so when it cuts.
fn cut(text: &str) -> String {
    if text.len() <= MAX {
        return text.to_owned();
    }
    let mut end = MAX;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n\n(The rest of the notes is cut: they are over {MAX} bytes.)", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sb-ctx-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn remotes_give_each_repo_once() {
        let v = "origin\tgit@github.com:me/lighthouse.git (fetch)\norigin\tgit@github.com:me/lighthouse.git (push)\n\
upstream\thttps://github.com/Sigp/Lighthouse (fetch)\nother\t/tmp/x (fetch)\n";
        assert_eq!(repos_in(v), vec!["me/lighthouse", "sigp/lighthouse"]);
    }

    #[test]
    fn pack_then_own_notes() {
        let d = tmp("order");
        std::fs::create_dir_all(d.join("pack/lh")).unwrap();
        std::fs::write(d.join("pack/lh/b.md"), "B pack").unwrap();
        std::fs::write(d.join("pack/lh/a.md"), "A pack").unwrap();
        std::fs::write(d.join("pack/lh/skip.txt"), "SKIPME").unwrap();
        std::fs::create_dir_all(d.join("context/sigp")).unwrap();
        std::fs::write(d.join("context/sigp/lighthouse.md"), "Mine").unwrap();
        let config = Config::parse("[repos.\"Sigp/Lighthouse\"]\ncontext = [\"pack/lh\", \"gone.md\", \"context/sigp/lighthouse.md\"]\n").unwrap();
        let c = build(&config, &d, vec!["sigp/lighthouse".into(), "me/lighthouse".into()]);
        let (a, b, m) = (c.text.find("A pack").unwrap(), c.text.find("B pack").unwrap(), c.text.find("Mine").unwrap());
        assert!(a < b && b < m, "{}", c.text);
        assert_eq!(c.text.matches("Mine").count(), 1, "a file listed twice comes once");
        assert!(!c.text.contains("SKIPME"));
        assert!(c.text.starts_with("The user's own notes for the repo sigp/lighthouse"));
        assert!(!c.text.contains("me/lighthouse"), "a repo with no notes says nothing");
        assert!(c.sources.iter().any(|s| s.path.ends_with("gone.md") && s.read.is_err()));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn no_notes_is_empty_and_long_notes_are_cut() {
        let d = tmp("cut");
        assert_eq!(build(&Config::default(), &d, vec!["a/b".into()]).text, "");
        std::fs::create_dir_all(d.join("context/a")).unwrap();
        std::fs::write(d.join("context/a/b.md"), "é".repeat(MAX)).unwrap();
        let t = build(&Config::default(), &d, vec!["a/b".into()]).text;
        assert!(t.len() < MAX + 200 && t.ends_with("bytes.)"), "{}", t.len());
        let _ = std::fs::remove_dir_all(&d);
    }
}
