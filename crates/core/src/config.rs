//! `~/.config/switchboard/config.toml` (SPEC 14). A missing file gives the defaults.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    pub leader: String,
    pub scan: Vec<String>,
    /// End a session that waits unseen in Your turn for this many minutes. 0 turns it off.
    pub idle_end_minutes: u64,
    pub poll_head_minutes: u64,
    pub connections: BTreeMap<String, ConnectionCfg>,
    pub review: ReviewCfg,
    pub tasks: TasksCfg,
    /// Remote mode (SPEC 29): the ssh host of the hub, for example `me@box`. With
    /// none, the hub runs here.
    pub remote: Option<String>,
    /// The program that links the window to the remote hub. With none,
    /// `ssh -T <remote> sb attach`, with `~/.local/bin` and `~/.cargo/bin` on its PATH.
    pub remote_command: Option<Vec<String>>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConnectionCfg {
    /// `claude` or `endpoint`.
    pub kind: String,
    /// The `CLAUDE_CONFIG_DIR`. With none, the connection uses your own `~/.claude` and its login.
    #[serde(default)]
    pub dir: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// `keyring:<name>` reads the token with `secret-tool`.
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields, default)]
pub struct ReviewCfg {
    pub prompt: Option<String>,
    pub spec_clones: Vec<String>,
}

/// The `[tasks]` table (SPEC 27.11).
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct TasksCfg {
    /// The length of a new sprint in days.
    pub sprint_days: u32,
    /// The start of the branch name for an agent started from a task.
    pub branch_prefix: String,
}

impl Default for TasksCfg {
    fn default() -> Self {
        TasksCfg { sprint_days: 14, branch_prefix: "sb/".into() }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            leader: "ctrl+space".into(),
            scan: vec!["~/Documents/Code".into()],
            idle_end_minutes: 0,
            poll_head_minutes: 2,
            connections: BTreeMap::new(),
            review: ReviewCfg::default(),
            tasks: TasksCfg::default(),
            remote: None,
            remote_command: None,
        }
    }
}

impl Config {
    /// The argv of the remote link, or `None` when the hub runs here. `SB_REMOTE`
    /// sets the host for one run.
    pub fn remote_argv(&self) -> Option<(String, Vec<String>)> {
        let host = std::env::var("SB_REMOTE").ok().filter(|h| !h.is_empty()).or_else(|| self.remote.clone())?;
        let argv = self.remote_command.clone().unwrap_or_else(|| {
            vec![
                "ssh".into(),
                "-T".into(),
                "-o".into(),
                "ServerAliveInterval=15".into(),
                "-o".into(),
                "ServerAliveCountMax=3".into(),
                host.clone(),
                "PATH=\"$HOME/.local/bin:$HOME/.cargo/bin:$PATH\" exec sb attach".into(),
            ]
        });
        Some((host, argv))
    }

    pub fn load(path: &Path) -> Result<Config, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Config::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn parse(text: &str) -> Result<Config, String> {
        let c: Config = toml::from_str(text).map_err(|e| e.message().to_owned())?;
        for (name, conn) in &c.connections {
            match conn.kind.as_str() {
                "claude" => {}
                "endpoint" if conn.url.is_some() => {}
                "endpoint" => return Err(format!("connection {name}: an endpoint needs url")),
                other => return Err(format!("connection {name}: unknown kind {other}")),
            }
        }
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_gives_defaults() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn full_config() {
        let c = Config::parse(
            r#"
            leader = "ctrl+a"
            idle_end_minutes = 30
            [connections.a]
            kind = "claude"
            dir = "~/.claude-a"
            [connections.local]
            kind = "endpoint"
            dir = "~/.claude-local"
            url = "http://127.0.0.1:8080"
            model = "qwen3-coder"
            token = "keyring:local"
            [review]
            spec_clones = ["~/specs"]
            "#,
        )
        .unwrap();
        assert_eq!(c.leader, "ctrl+a");
        assert_eq!(c.connections.len(), 2);
        assert_eq!(c.review.spec_clones, vec!["~/specs"]);
    }

    #[test]
    fn unknown_key_names_the_key() {
        let e = Config::parse("leadr = 'x'").unwrap_err();
        assert!(e.contains("leadr"), "{e}");
    }

    #[test]
    fn endpoint_needs_url() {
        let e = Config::parse("[connections.x]\nkind='endpoint'\ndir='/x'").unwrap_err();
        assert!(e.contains("url"), "{e}");
    }

    #[test]
    fn missing_file_is_default() {
        assert_eq!(
            Config::load(Path::new("/nonexistent/sb.toml")).unwrap(),
            Config::default()
        );
    }
}
