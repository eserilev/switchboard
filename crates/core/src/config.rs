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
    pub sandbox: SandboxCfg,
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

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct ReviewCfg {
    pub prompt: Option<String>,
    pub spec_clones: Vec<String>,
    /// The sites the review agent can fetch, as host names (SPEC 11.2). Other
    /// sites are refused: the PR body can hold instructions, and a fetch to any
    /// site could send out the code.
    pub web_domains: Vec<String>,
    /// Lets the review agent use WebSearch.
    pub web_search: bool,
}

impl Default for ReviewCfg {
    fn default() -> Self {
        ReviewCfg {
            prompt: None,
            spec_clones: Vec::new(),
            web_domains: [
                "github.com",
                "raw.githubusercontent.com",
                "ethereum.github.io",
                "eips.ethereum.org",
            ]
            .map(String::from)
            .to_vec(),
            web_search: false,
        }
    }
}

/// The `[sandbox]` table (SPEC 31): which agents run Bash in Claude Code's sandbox.
#[derive(Deserialize, serde::Serialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct SandboxCfg {
    /// `off`, `review` (only the review Ask panes) or `all` (every agent).
    pub mode: String,
    /// The hosts that commands in the sandbox can reach. `*.x.org` takes every
    /// host under x.org.
    pub domains: Vec<String>,
    /// More folders that commands in the sandbox can write. The tree of the pane
    /// and its git folder are always writable.
    pub write: Vec<String>,
    /// Commands that run outside the sandbox, as `Bash` rule patterns such as `gh *`.
    pub unsandboxed: Vec<String>,
}

pub const SANDBOX_MODES: [&str; 3] = ["off", "review", "all"];

impl Default for SandboxCfg {
    fn default() -> Self {
        SandboxCfg {
            mode: "review".into(),
            domains: [
                "github.com",
                "api.github.com",
                "*.githubusercontent.com",
                "crates.io",
                "index.crates.io",
                "static.crates.io",
                "registry.npmjs.org",
                "pypi.org",
                "files.pythonhosted.org",
            ]
            .map(String::from)
            .to_vec(),
            write: Vec::new(),
            unsandboxed: ["git push *", "git fetch *", "git pull *", "gh *", "docker *"].map(String::from).to_vec(),
        }
    }
}

impl SandboxCfg {
    /// True when a claude pane runs in the sandbox. `ask` is true for a review Ask pane.
    pub fn covers(&self, ask: bool) -> bool {
        match self.mode.as_str() {
            "all" => true,
            "review" => ask,
            _ => false,
        }
    }

    /// The rules of the Settings window for the sandbox fields.
    fn clean(&self) -> Result<SandboxCfg, String> {
        if !SANDBOX_MODES.contains(&self.mode.as_str()) {
            return Err(format!("Sandbox mode {} is not off, review or all.", self.mode));
        }
        let domains: Vec<String> = tidy(&self.domains).iter().map(|d| d.to_ascii_lowercase()).collect();
        if let Some(bad) = domains.iter().find(|d| !host_ok(d.strip_prefix("*.").unwrap_or(d))) {
            return Err(format!("{bad} is not a host name. Write it like github.com or *.github.com, with no https:// and no path."));
        }
        let write = tidy(&self.write);
        if let Some(bad) = write.iter().find(|w| !crate::expand(w).is_absolute()) {
            return Err(format!("{bad} is not a full path. Start it with / or ~/."));
        }
        Ok(SandboxCfg { mode: self.mode.clone(), domains, write, unsandboxed: tidy(&self.unsandboxed) })
    }
}

/// The fields that the Settings window changes (SPEC 14.1).
#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct SettingsEdit {
    pub scan: Vec<String>,
    pub spec_clones: Vec<String>,
    pub web_domains: Vec<String>,
    pub web_search: bool,
    /// With none, the `[sandbox]` table stays as it is.
    #[serde(default)]
    pub sandbox: Option<SandboxCfg>,
}

/// Trims each item and drops empty items and repeats.
fn tidy(v: &[String]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for s in v.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        if !out.iter().any(|o| o == s) {
            out.push(s.to_owned());
        }
    }
    out
}

/// True for a plain host name such as `github.com`.
fn host_ok(d: &str) -> bool {
    !d.is_empty() && d.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

impl SettingsEdit {
    /// Trims each item and drops empty items and repeats. A site must be a plain
    /// host name: anything else would break the tool list of the review agent.
    fn clean(&self) -> Result<SettingsEdit, String> {
        let web_domains: Vec<String> = tidy(&self.web_domains).iter().map(|d| d.to_ascii_lowercase()).collect();
        if let Some(bad) = web_domains.iter().find(|d| !host_ok(d)) {
            return Err(format!("{bad} is not a host name. Write it like github.com, with no https:// and no path."));
        }
        Ok(SettingsEdit {
            scan: tidy(&self.scan),
            spec_clones: tidy(&self.spec_clones),
            web_domains,
            web_search: self.web_search,
            sandbox: self.sandbox.as_ref().map(SandboxCfg::clean).transpose()?,
        })
    }
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
            sandbox: SandboxCfg::default(),
            remote: None,
            remote_command: None,
        }
    }
}

/// Makes `doc[key]` a normal table. A missing key gets a new one, and an inline
/// table (`review = { ... }`) turns into a normal one with the same keys.
fn table_at(doc: &mut toml_edit::Document, key: &str, path: &Path) -> Result<(), String> {
    let Some(item) = doc.get_mut(key) else {
        doc[key] = toml_edit::table();
        return Ok(());
    };
    if item.is_table() {
        return Ok(());
    }
    match std::mem::take(item).into_table() {
        Ok(t) => {
            *item = toml_edit::Item::Table(t);
            Ok(())
        }
        Err(old) => {
            *item = old;
            Err(format!("{}: {key} is not a table.", path.display()))
        }
    }
}

/// Writes a config file through a new file and a rename, so a crash never leaves
/// half a config. A symlinked config stays a symlink: the new file goes next to
/// the file it points to. The file keeps its mode.
pub fn write_file(path: &Path, text: &str) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let is_link = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
    let target = if is_link {
        match std::fs::canonicalize(path) {
            Ok(t) => t,
            // A link to a file that is not there yet: follow it one step.
            Err(_) => {
                let to = std::fs::read_link(path).map_err(|e| format!("{}: {e}", path.display()))?;
                path.parent().unwrap_or(Path::new("/")).join(to)
            }
        }
    } else {
        path.to_path_buf()
    };
    let dir = target.parent().unwrap_or(Path::new("/"));
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let tmp = dir.join(format!(".{name}.{}.{n}.new", std::process::id()));
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    if let Ok(m) = std::fs::metadata(&target) {
        if let Err(e) = std::fs::set_permissions(&tmp, m.permissions()) {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("{}: {e}", tmp.display()));
        }
    }
    std::fs::rename(&tmp, &target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", target.display())
    })
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

    /// Writes the Settings fields into the config file, and keeps every other key
    /// and the comments. Returns the new config. A file that does not parse is not
    /// changed: the error names the problem.
    pub fn edit(path: &Path, set: &SettingsEdit) -> Result<Config, String> {
        let set = set.clean()?;
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let mut doc: toml_edit::Document = text
            .parse()
            .map_err(|e: toml_edit::TomlError| format!("{}: {}. Fix the file, then save again.", path.display(), e.message()))?;
        let list = |v: &[String]| toml_edit::value(v.iter().map(String::as_str).collect::<toml_edit::Array>());
        doc["scan"] = list(&set.scan);
        table_at(&mut doc, "review", path)?;
        doc["review"]["spec_clones"] = list(&set.spec_clones);
        doc["review"]["web_domains"] = list(&set.web_domains);
        doc["review"]["web_search"] = toml_edit::value(set.web_search);
        if let Some(sb) = &set.sandbox {
            table_at(&mut doc, "sandbox", path)?;
            doc["sandbox"]["mode"] = toml_edit::value(sb.mode.as_str());
            doc["sandbox"]["domains"] = list(&sb.domains);
            doc["sandbox"]["write"] = list(&sb.write);
            doc["sandbox"]["unsandboxed"] = list(&sb.unsandboxed);
        }
        let text = doc.to_string();
        // The app reads the file with the same rules at its next start.
        let config = Config::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        write_file(path, &text)?;
        Ok(config)
    }

    /// Adds a `[connections.<name>]` table, written with `toml_edit`, so a value
    /// with a quote or a backslash stays one TOML string. Keeps every other key
    /// and the comments. Returns false when the name is there already.
    pub fn add_connection(path: &Path, name: &str, conn: &ConnectionCfg) -> Result<bool, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let mut doc: toml_edit::Document = text
            .parse()
            .map_err(|e: toml_edit::TomlError| format!("{}: {}. Fix the file, then try again.", path.display(), e.message()))?;
        let fresh = !doc.contains_key("connections");
        table_at(&mut doc, "connections", path)?;
        let Some(conns) = doc["connections"].as_table_mut() else {
            return Err(format!("{}: connections is not a table.", path.display()));
        };
        if fresh {
            conns.set_implicit(true);
        }
        if conns.contains_key(name) {
            return Ok(false);
        }
        let mut t = toml_edit::Table::new();
        t["kind"] = toml_edit::value(conn.kind.as_str());
        for (k, v) in [("dir", &conn.dir), ("url", &conn.url), ("model", &conn.model), ("token", &conn.token)] {
            if let Some(v) = v {
                t[k] = toml_edit::value(v.as_str());
            }
        }
        conns.insert(name, toml_edit::Item::Table(t));
        let text = doc.to_string();
        Config::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        write_file(path, &text)?;
        Ok(true)
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
        if !SANDBOX_MODES.contains(&c.sandbox.mode.as_str()) {
            return Err(format!("sandbox: mode {} is not off, review or all", c.sandbox.mode));
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
            web_domains = ["example.org"]
            web_search = true
            "#,
        )
        .unwrap();
        assert_eq!(c.leader, "ctrl+a");
        assert_eq!(c.connections.len(), 2);
        assert_eq!(c.review.spec_clones, vec!["~/specs"]);
        assert_eq!(c.review.web_domains, vec!["example.org"]);
        assert!(c.review.web_search);
        // With no `web_domains`, the default sites stay.
        let d = Config::parse("[review]\nspec_clones = []").unwrap();
        assert!(d.review.web_domains.contains(&"raw.githubusercontent.com".to_owned()));
    }

    #[test]
    fn edit_keeps_comments_and_other_keys() {
        let dir = std::env::temp_dir().join(format!("sb-config-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("config.toml");
        // No file yet: the edit writes one.
        let set = SettingsEdit {
            scan: vec![" ~/code ".into(), "".into(), "~/code".into(), "~/notes".into()],
            spec_clones: vec![],
            web_domains: vec!["GitHub.com".into()],
            web_search: true,
            sandbox: None,
        };
        let c = Config::edit(&path, &set).unwrap();
        assert_eq!(c.scan, vec!["~/code", "~/notes"]);
        assert_eq!(c.review.web_domains, vec!["github.com"]);
        assert!(c.review.web_search);
        // A file with comments, another key and a review table.
        std::fs::write(&path, "# my config\nleader = \"ctrl+a\" # the key\nscan = [\"~/x\"]\n\n[review]\nprompt = \"p.md\"\n").unwrap();
        let c = Config::edit(&path, &SettingsEdit { scan: vec!["~/y".into()], ..set.clone() }).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# my config") && text.contains("# the key"), "{text}");
        assert_eq!((c.leader.as_str(), c.scan.clone(), c.review.prompt.as_deref()), ("ctrl+a", vec!["~/y".to_owned()], Some("p.md")));
        assert_eq!(Config::load(&path).unwrap(), c);
        // A bad site, and a file that does not parse, change nothing.
        let bad = SettingsEdit { web_domains: vec!["https://x.org/a".into()], ..set.clone() };
        assert!(Config::edit(&path, &bad).unwrap_err().contains("not a host name"));
        std::fs::write(&path, "scan = [").unwrap();
        assert!(Config::edit(&path, &set).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "scan = [");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sandbox_defaults_parse_and_edit() {
        let d = Config::default().sandbox;
        assert_eq!(d.mode, "review");
        assert!(d.covers(true) && !d.covers(false));
        assert!(d.domains.contains(&"*.githubusercontent.com".to_owned()));
        assert!(d.unsandboxed.contains(&"gh *".to_owned()));
        let c = Config::parse("[sandbox]\nmode = \"all\"\nwrite = [\"~/.kube\"]").unwrap();
        assert!(c.sandbox.covers(false));
        assert_eq!(c.sandbox.write, vec!["~/.kube"]);
        // Fields left out keep their defaults.
        assert_eq!(c.sandbox.domains, d.domains);
        assert!(!Config::parse("[sandbox]\nmode = \"off\"").unwrap().sandbox.covers(true));
        assert!(Config::parse("[sandbox]\nmode = \"some\"").unwrap_err().contains("mode"));

        let dir = std::env::temp_dir().join(format!("sb-config-sandbox-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "# top\n[sandbox]\nmode = \"off\" # mine\n").unwrap();
        let base = SettingsEdit { scan: vec!["~/c".into()], ..SettingsEdit::default() };
        // With no sandbox fields, the table stays as it is.
        assert_eq!(Config::edit(&path, &base).unwrap().sandbox.mode, "off");
        let sb = SandboxCfg {
            mode: "all".into(),
            domains: vec![" *.Example.org ".into(), "x.io".into(), "x.io".into()],
            write: vec!["~/out".into(), "/tmp/b".into()],
            unsandboxed: vec!["make deploy".into(), "".into()],
        };
        let c = Config::edit(&path, &SettingsEdit { sandbox: Some(sb.clone()), ..base.clone() }).unwrap();
        assert_eq!(c.sandbox.mode, "all");
        assert_eq!(c.sandbox.domains, vec!["*.example.org", "x.io"]);
        assert_eq!(c.sandbox.write, vec!["~/out", "/tmp/b"]);
        assert_eq!(c.sandbox.unsandboxed, vec!["make deploy"]);
        assert!(std::fs::read_to_string(&path).unwrap().contains("# top"));
        assert_eq!(Config::load(&path).unwrap(), c);
        // Bad fields change nothing.
        let bad = |s: SandboxCfg| Config::edit(&path, &SettingsEdit { sandbox: Some(s), ..base.clone() }).unwrap_err();
        assert!(bad(SandboxCfg { mode: "most".into(), ..sb.clone() }).contains("mode"));
        assert!(bad(SandboxCfg { domains: vec!["https://x.org".into()], ..sb.clone() }).contains("not a host name"));
        assert!(bad(SandboxCfg { domains: vec!["a.*.org".into()], ..sb.clone() }).contains("not a host name"));
        assert!(bad(SandboxCfg { write: vec!["out".into()], ..sb.clone() }).contains("full path"));
        assert_eq!(Config::load(&path).unwrap(), c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn edit_takes_inline_tables_and_keeps_a_symlink_and_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("sb-config-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("dots")).unwrap();
        let real = dir.join("dots/config.toml");
        std::fs::write(&real, "review = { prompt = \"p.md\" }\nsandbox = { mode = \"off\" }\n").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.join("config.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let set = SettingsEdit {
            scan: vec!["~/c".into()],
            web_search: true,
            sandbox: Some(SandboxCfg { mode: "all".into(), ..SandboxCfg::default() }),
            ..SettingsEdit::default()
        };
        let c = Config::edit(&link, &set).unwrap();
        assert_eq!(c.review.prompt.as_deref(), Some("p.md"));
        assert!(c.review.web_search);
        assert_eq!(c.sandbox.mode, "all");
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(Config::load(&real).unwrap(), c);
        let mode = std::fs::metadata(&real).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        // No temp file is left.
        assert_eq!(std::fs::read_dir(dir.join("dots")).unwrap().count(), 1);
        // A key that is not a table at all is still an error.
        std::fs::write(&real, "review = 3\n").unwrap();
        assert!(Config::edit(&link, &set).unwrap_err().contains("review is not a table"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn add_connection_writes_strings_as_toml() {
        let dir = std::env::temp_dir().join(format!("sb-config-conn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("config.toml");
        let conn = ConnectionCfg {
            kind: "endpoint".into(),
            dir: Some("~/.claude-x".into()),
            url: Some("https://h/\"a\\b\nscan = 1".into()),
            model: Some("m\"1".into()),
            token: None,
        };
        assert!(Config::add_connection(&path, "x", &conn).unwrap());
        let c = Config::load(&path).unwrap();
        assert_eq!(c.connections["x"], conn);
        assert_eq!(c.scan, Config::default().scan);
        assert!(std::fs::read_to_string(&path).unwrap().contains("[connections.x]"));
        // A second add of the same name changes nothing.
        assert!(!Config::add_connection(&path, "x", &conn).unwrap());
        let other = ConnectionCfg { kind: "claude".into(), url: None, model: None, ..conn.clone() };
        assert!(Config::add_connection(&path, "y", &other).unwrap());
        assert_eq!(Config::load(&path).unwrap().connections.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
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
