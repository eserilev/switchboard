//! LLM connections (SPEC 8). One is active for the whole app.
//!
//! With no connection in the config, there is one: `default`, which is plain
//! `claude` with your own `~/.claude`.

use crate::config::{Config, ConnectionCfg};
use crate::expand;
use std::path::Path;
use std::process::Command;

pub const DEFAULT: &str = "default";

/// Files in a connection folder that link back to `~/.claude`. The login stays separate.
pub const SHARED: [&str; 5] = [
    "settings.json",
    "CLAUDE.md",
    "projects",
    "skills",
    "plugins",
];

pub fn names(config: &Config) -> Vec<String> {
    if config.connections.is_empty() {
        return vec![DEFAULT.into()];
    }
    config.connections.keys().cloned().collect()
}

/// The environment for `claude` on a connection.
pub fn env(config: &Config, name: &str) -> Result<Vec<(String, String)>, String> {
    let Some(c) = config.connections.get(name) else {
        return if name == DEFAULT {
            Ok(vec![])
        } else {
            Err(format!("no connection named {name}"))
        };
    };
    let mut env = vec![(
        "CLAUDE_CONFIG_DIR".to_owned(),
        expand(&c.dir).to_string_lossy().into_owned(),
    )];
    if c.kind == "endpoint" {
        env.push((
            "ANTHROPIC_BASE_URL".into(),
            c.url.clone().unwrap_or_default(),
        ));
        if let Some(m) = &c.model {
            env.push(("ANTHROPIC_MODEL".into(), m.clone()));
        }
        if let Some(t) = token(c)? {
            env.push(("ANTHROPIC_AUTH_TOKEN".into(), t));
        }
    }
    Ok(env)
}

fn token(c: &ConnectionCfg) -> Result<Option<String>, String> {
    let Some(t) = &c.token else { return Ok(None) };
    let Some(key) = t.strip_prefix("keyring:") else {
        return Err(
            "a token must be keyring:<name>; the config never holds the token itself".into(),
        );
    };
    let out = Command::new("secret-tool")
        .args(["lookup", "service", "switchboard", "name", key])
        .output()
        .map_err(|e| format!("secret-tool: {e}"))?;
    if !out.status.success() {
        return Err(format!("no token in the keyring for {key}"));
    }
    Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_owned()))
}

/// Makes a connection folder with links to the shared files in `claude_dir`.
pub fn make_folder(dir: &Path, claude_dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for name in SHARED {
        let target = claude_dir.join(name);
        let link = dir.join(name);
        if link.symlink_metadata().is_ok() || !target.exists() {
            continue;
        }
        std::os::unix::fs::symlink(&target, &link)?;
    }
    Ok(())
}

/// The next connection after `current` that is not at limit.
pub fn next_free(all: &[String], current: &str, at_limit: impl Fn(&str) -> bool) -> Option<String> {
    let start = all
        .iter()
        .position(|n| n == current)
        .map(|i| i + 1)
        .unwrap_or(0);
    (0..all.len())
        .map(|k| &all[(start + k) % all.len()])
        .find(|n| n.as_str() != current && !at_limit(n))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_connection_has_no_env() {
        let c = Config::default();
        assert_eq!(names(&c), vec!["default"]);
        assert!(env(&c, DEFAULT).unwrap().is_empty());
        assert!(env(&c, "b").is_err());
    }

    #[test]
    fn claude_and_endpoint_env() {
        let c = Config::parse(
            "[connections.a]\nkind='claude'\ndir='/x/a'\n[connections.l]\nkind='endpoint'\ndir='/x/l'\nurl='http://h'\nmodel='m'",
        )
        .unwrap();
        assert_eq!(
            env(&c, "a").unwrap(),
            vec![("CLAUDE_CONFIG_DIR".into(), "/x/a".into())]
        );
        let l = env(&c, "l").unwrap();
        assert!(l.contains(&("ANTHROPIC_BASE_URL".into(), "http://h".into())));
        assert!(l.contains(&("ANTHROPIC_MODEL".into(), "m".into())));
    }

    #[test]
    fn plain_token_is_refused() {
        let c =
            Config::parse("[connections.l]\nkind='endpoint'\ndir='/x'\nurl='u'\ntoken='sk-123'")
                .unwrap();
        assert!(env(&c, "l").unwrap_err().contains("keyring"));
    }

    #[test]
    fn next_free_skips_limits_and_wraps() {
        let all: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        assert_eq!(next_free(&all, "a", |_| false), Some("b".into()));
        assert_eq!(next_free(&all, "a", |n| n == "b"), Some("c".into()));
        assert_eq!(next_free(&all, "c", |_| false), Some("a".into()));
        assert_eq!(next_free(&all, "a", |n| n != "a"), None);
    }

    #[test]
    fn folder_links_only_existing_files() {
        let root = std::env::temp_dir().join(format!("sb-conn-{}", std::process::id()));
        let claude = root.join("claude");
        std::fs::create_dir_all(claude.join("projects")).unwrap();
        std::fs::write(claude.join("settings.json"), "{}").unwrap();
        let dir = root.join("claude-b");
        make_folder(&dir, &claude).unwrap();
        make_folder(&dir, &claude).unwrap();
        assert!(dir
            .join("projects")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(dir.join("settings.json").exists());
        assert!(dir.join("skills").symlink_metadata().is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
