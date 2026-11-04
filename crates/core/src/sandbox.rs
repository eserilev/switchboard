//! Sandbox mode (SPEC 31): the agents run Bash in Claude Code's own sandbox.
//! This file finds the folders a pane may write and the tools Linux needs.
//! `hooks::settings` writes the block.

use crate::config::SandboxCfg;
use crate::hooks::Sandbox;
use std::path::{Path, PathBuf};

/// The folders that Bash in the sandbox may write, beside the tree, and the ones
/// under them that it may not. The git folder of a worktree lives in the main
/// checkout: without it, `git commit` in a worktree fails. Its hooks and config
/// stay closed, since a command that writes them runs outside the sandbox later.
pub fn block<'a>(cfg: &'a SandboxCfg, tree: &Path) -> Sandbox<'a> {
    let mut write: Vec<String> = cfg.write.iter().map(|w| crate::expand(w).to_string_lossy().into_owned()).collect();
    let mut deny_write = vec![];
    if let Some(common) = git_common_dir(tree) {
        if !common.starts_with(tree) {
            write.push(common.to_string_lossy().into_owned());
        }
        for d in ["hooks", "config"] {
            deny_write.push(common.join(d).to_string_lossy().into_owned());
        }
    }
    Sandbox { domains: &cfg.domains, unsandboxed: &cfg.unsandboxed, write, deny_write }
}

/// The git folder that all worktrees of the repo share, as a full path.
fn git_common_dir(tree: &Path) -> Option<PathBuf> {
    let out = crate::repos::git(tree, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).ok()?;
    let p = PathBuf::from(out.trim());
    p.is_absolute().then_some(p)
}

/// True when the settings file of a pane turns the sandbox on. A pane that
/// starts again keeps its sandbox, whatever the mode is now.
pub fn file_on(settings: &Path) -> bool {
    std::fs::read_to_string(settings)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .is_some_and(|v| v.pointer("/sandbox/enabled") == Some(&serde_json::Value::Bool(true)))
}

/// The programs that the sandbox needs and that are not on `PATH`. Only Linux
/// needs any: macOS has Seatbelt built in.
pub fn missing_tools() -> Vec<&'static str> {
    if !cfg!(target_os = "linux") {
        return vec![];
    }
    ["bwrap", "socat"].into_iter().filter(|t| !on_path(t)).collect()
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d.join(name).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn a_worktree_can_write_its_git_folder() {
        let root = std::env::temp_dir().join(format!("sb-sandbox-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // macOS puts the temp folder behind a link: git prints the real path.
        let root = root.canonicalize().unwrap();
        let main = root.join("main");
        std::fs::create_dir_all(&main).unwrap();
        git(&main, &["init", "-q"]);
        git(&main, &["-c", "commit.gpgsign=false", "commit", "-q", "--allow-empty", "-m", "first"]);
        let tree = root.join("wt");
        git(&main, &["worktree", "add", "-q", "-b", "wt", tree.to_str().unwrap()]);

        let cfg = SandboxCfg { write: vec!["/tmp/build".into()], ..SandboxCfg::default() };
        let common = main.join(".git").to_string_lossy().into_owned();
        let b = block(&cfg, &tree);
        assert_eq!(b.write, vec!["/tmp/build".to_owned(), common.clone()]);
        assert!(b.deny_write.contains(&format!("{common}/hooks")) && b.deny_write.contains(&format!("{common}/config")), "{:?}", b.deny_write);
        assert_eq!(b.domains, &cfg.domains[..]);
        // The main checkout writes its own git folder already: only the deny rules.
        let b = block(&cfg, &main);
        assert_eq!(b.write, vec!["/tmp/build".to_owned()]);
        assert_eq!(b.deny_write.len(), 2);
        // A folder with no git: only the config folders.
        let plain = root.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert!(block(&cfg, &plain).deny_write.is_empty());

        let f = root.join("p1.json");
        std::fs::write(&f, r#"{"sandbox":{"enabled":true}}"#).unwrap();
        assert!(file_on(&f));
        std::fs::write(&f, r#"{"hooks":{}}"#).unwrap();
        assert!(!file_on(&f) && !file_on(&root.join("none.json")));
        let _ = std::fs::remove_dir_all(&root);
    }
}
