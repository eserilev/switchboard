//! Sandbox mode on a real hub and tmux (SPEC 31): which panes get the sandbox
//! block, and the git folder of a worktree in it. A file of its own: the test
//! sets PATH and other process-wide variables.

use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use switchboard_core::hub::{Hub, OpenReq, Sink};

struct Quiet;

impl Sink for Quiet {
    fn emit(&self, _event: &str, _payload: Value) {}
    fn output(&self, _pane: &str, _data: &[u8]) {}
}

fn g(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

#[test]
fn panes_get_the_sandbox_by_mode() {
    let root = std::env::temp_dir().join(format!("sb-sandbox-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("bin")).unwrap();
    // macOS puts the temp folder behind a link: git prints the real path.
    let root = root.canonicalize().unwrap();
    let repo = root.join("demo");
    std::fs::create_dir_all(&repo).unwrap();
    g(&repo, &["init", "-q", "-b", "main"]);
    g(&repo, &["commit", "-q", "--allow-empty", "-m", "a"]);
    // A stand-in claude that waits.
    let claude = root.join("bin/claude");
    std::fs::write(&claude, "#!/bin/sh\nexec cat >/dev/null\n").unwrap();
    std::process::Command::new("chmod").args(["+x", &claude.to_string_lossy()]).status().unwrap();
    std::fs::write(root.join("config.toml"), "[sandbox]\nmode = \"all\"\nwrite = [\"/tmp/sb-build\"]\n").unwrap();
    std::env::set_var("PATH", format!("{}:{}", root.join("bin").display(), std::env::var("PATH").unwrap_or_default()));
    std::env::set_var("SB_HOME", &root);
    std::env::set_var("SB_TMUX_SOCKET", format!("sb-test-sandbox-{}", std::process::id()));
    std::env::remove_var("SB_SOCK");
    struct Kill(String, PathBuf);
    impl Drop for Kill {
        fn drop(&mut self) {
            switchboard_core::tmux::kill_server(&self.0);
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }
    let _kill = Kill(std::env::var("SB_TMUX_SOCKET").unwrap(), root.clone());
    let hub = Hub::start(switchboard_core::paths::Paths::from_env(), PathBuf::from(env!("CARGO_BIN_EXE_sb")), Arc::new(Quiet)).unwrap();
    let open = |place: &str, branch: Option<&str>, run: &str| {
        hub.open(OpenReq {
            repo: repo.to_string_lossy().into(),
            place: place.into(),
            branch: branch.map(str::to_owned),
            tree: None,
            run: run.into(),
            title: None,
            task: None,
            first: None,
        })
        .unwrap()
    };
    let file = |id: &str| -> Value {
        serde_json::from_str(&std::fs::read_to_string(root.join(format!("home/settings/{id}.json"))).unwrap()).unwrap()
    };

    // Mode all: an agent in a new worktree can write the git folder of the main checkout.
    let v = open("new", Some("wt"), "claude");
    assert!(v.sandboxed);
    let s = file(&v.id);
    assert_eq!(s["sandbox"]["enabled"], true);
    let write = s["sandbox"]["filesystem"]["allowWrite"].clone();
    let common = repo.join(".git").to_string_lossy().into_owned();
    assert_eq!(write, serde_json::json!(["/tmp/sb-build", common]), "{s}");
    assert!(s["sandbox"]["filesystem"]["denyWrite"].as_array().unwrap().contains(&Value::String(format!("{common}/hooks"))));
    assert!(s["permissions"]["deny"].as_array().unwrap().contains(&Value::String("Read(~/.ssh/**)".into())));
    // A shell has no settings and no sandbox mark.
    assert!(!open("main", None, "shell").sandboxed);

    // Mode review: a plain agent runs with no sandbox.
    let mut set: switchboard_core::config::SettingsEdit = serde_json::from_value(serde_json::json!({
        "scan": [], "spec_clones": [], "web_domains": [], "web_search": false,
        "sandbox": { "mode": "review", "domains": ["github.com"], "write": [], "unsandboxed": ["gh *"] }
    }))
    .unwrap();
    let view = hub.settings_save(&set).unwrap();
    assert_eq!(view["sandbox"]["mode"], "review");
    assert!(view["sandbox_missing"].is_array());
    let plain = open("main", None, "claude");
    assert!(!plain.sandboxed);
    assert!(file(&plain.id).get("sandbox").is_none());
    // The first pane keeps its mark.
    assert!(hub.views().iter().any(|p| p.id == v.id && p.sandboxed));

    // A bad mode is refused and the config stays.
    set.sandbox.as_mut().unwrap().mode = "most".into();
    assert!(hub.settings_save(&set).is_err());
    assert_eq!(hub.config().sandbox.mode, "review");
}
