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
/// The folders with keys stay closed too, also when `sandbox.write` holds `~`.
pub fn block<'a>(cfg: &'a SandboxCfg, tree: &Path) -> Sandbox<'a> {
    let mut write: Vec<String> = cfg.write.iter().map(|w| crate::expand(w).to_string_lossy().into_owned()).collect();
    let mut deny_write: Vec<String> =
        crate::hooks::SECRET_DIRS.iter().map(|d| crate::expand(d).to_string_lossy().into_owned()).collect();
    if let Some(common) = git_common_dir(tree) {
        if !common.starts_with(tree) {
            write.push(common.to_string_lossy().into_owned());
        }
        deny_write.extend(git_closed(&common).iter().map(|p| p.to_string_lossy().into_owned()));
    }
    Sandbox { domains: &cfg.domains, unsandboxed: &cfg.unsandboxed, write, deny_write }
}

/// The files in the git folder `common` that git runs or reads config from: the
/// hooks and config of the repo, of each submodule (`modules/<name>`, also nested
/// ones) and the `config.worktree` of each worktree. Claude Code skips a glob in a
/// write rule on Linux, so these are the real paths at the start of the pane. A
/// `config.worktree` that is not there yet is listed only when the repo reads it
/// (`extensions.worktreeConfig`). A submodule added later is not covered (SPEC 23).
fn git_closed(common: &Path) -> Vec<PathBuf> {
    let mut out = vec![common.join("hooks"), common.join("config")];
    let worktree_config = crate::repos::git(common, &["config", "--bool", "extensions.worktreeConfig"])
        .is_ok_and(|v| v.trim() == "true");
    let config_worktree =|dir: &Path, out: &mut Vec<PathBuf>| {
        let p = dir.join("config.worktree");
        if worktree_config || p.exists() {
            out.push(p);
        }
    };
    config_worktree(common, &mut out);
    let mut trees: Vec<PathBuf> = std::fs::read_dir(common.join("worktrees"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    trees.sort();
    for t in &trees {
        config_worktree(t, &mut out);
    }
    // The git folders of the submodules. A name can hold `/`, so a folder with no
    // HEAD file is a step of a name: look inside it.
    let mut todo = vec![common.join("modules")];
    while let Some(dir) = todo.pop() {
        let mut subs: Vec<PathBuf> =
            std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        subs.sort();
        for s in subs.into_iter().rev() {
            if s.join("HEAD").is_file() {
                out.push(s.join("hooks"));
                out.push(s.join("config"));
                todo.push(s.join("modules"));
            } else {
                todo.push(s);
            }
        }
    }
    out
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
        let secrets: Vec<String> =
            crate::hooks::SECRET_DIRS.iter().map(|d| crate::expand(d).to_string_lossy().into_owned()).collect();
        assert_eq!(b.deny_write, [secrets.clone(), vec![format!("{common}/hooks"), format!("{common}/config")]].concat());
        assert_eq!(b.domains, &cfg.domains[..]);
        // The main checkout writes its own git folder already: only the deny rules.
        let b = block(&cfg, &main);
        assert_eq!(b.write, vec!["/tmp/build".to_owned()]);
        assert_eq!(b.deny_write.len(), secrets.len() + 2);
        // A folder with no git: only the folders with keys.
        let plain = root.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert_eq!(block(&cfg, &plain).deny_write, secrets);
        // `~` in `sandbox.write` does not open the keys.
        let home = SandboxCfg { write: vec!["~".into()], ..SandboxCfg::default() };
        let b = block(&home, &plain);
        assert!(secrets.iter().all(|s| b.deny_write.contains(s) && s.starts_with(&b.write[0])), "{:?}", b.deny_write);

        // A submodule (a nested one too, and a name with `/`) and worktree config.
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        git(&sub, &["init", "-q"]);
        git(&sub, &["-c", "commit.gpgsign=false", "commit", "-q", "--allow-empty", "-m", "s"]);
        let file = |args: &[&str]| {
            let mut a = vec!["-c", "protocol.file.allow=always"];
            a.extend_from_slice(args);
            git(&main, &a);
        };
        file(&["submodule", "add", "-q", sub.to_str().unwrap(), "libs/one"]);
        file(&["submodule", "add", "-q", "--name", "a/b", sub.to_str().unwrap(), "two"]);
        let nested = main.join(".git/modules/libs/one");
        git(&main.join("libs/one"), &["-c", "protocol.file.allow=always", "submodule", "add", "-q", sub.to_str().unwrap(), "inner"]);
        git(&main, &["config", "extensions.worktreeConfig", "true"]);
        let b = block(&cfg, &tree);
        let want = [
            format!("{common}/hooks"),
            format!("{common}/config"),
            format!("{common}/config.worktree"),
            format!("{common}/worktrees/wt/config.worktree"),
            format!("{common}/modules/a/b/hooks"),
            format!("{common}/modules/a/b/config"),
            format!("{common}/modules/libs/one/hooks"),
            format!("{common}/modules/libs/one/config"),
            format!("{}/modules/inner/hooks", nested.display()),
            format!("{}/modules/inner/config", nested.display()),
        ];
        assert_eq!(b.deny_write, [secrets.clone(), want.to_vec()].concat());
        // Without worktreeConfig, a config.worktree that is not there is left out.
        git(&main, &["config", "--unset", "extensions.worktreeConfig"]);
        std::fs::write(main.join(".git/worktrees/wt/config.worktree"), "").unwrap();
        let b = block(&cfg, &tree);
        assert!(!b.deny_write.contains(&format!("{common}/config.worktree")));
        assert!(b.deny_write.contains(&format!("{common}/worktrees/wt/config.worktree")));

        let f = root.join("p1.json");
        std::fs::write(&f, r#"{"sandbox":{"enabled":true}}"#).unwrap();
        assert!(file_on(&f));
        std::fs::write(&f, r#"{"hooks":{}}"#).unwrap();
        assert!(!file_on(&f) && !file_on(&root.join("none.json")));
        let _ = std::fs::remove_dir_all(&root);
    }
}
