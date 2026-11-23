//! The pane tools on a real hub and tmux (SPEC 36): an agent pane starts with the
//! `switchboard` server (not strict), the allow rule and `SB_TOOLS`; `sb mcp --pane`
//! lists the board of the pane's repo only and changes tasks as that pane; and
//! `sb state session` gives the start note. A file of its own: the test sets PATH.

use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use switchboard_core::hub::{Hub, OpenReq, Sink};

struct Quiet;

impl Sink for Quiet {
    fn emit(&self, _event: &str, _payload: Value) {}
    fn output(&self, _pane: &str, _data: &[u8]) {}
}

fn g(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn wait_for(what: &str, f: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out: {what}");
}

/// Runs `sb` with a stdin and returns its stdout.
fn sb(root: &Path, env: &[(&str, &str)], args: &[&str], stdin: &str) -> String {
    let mut c = Command::new(env!("CARGO_BIN_EXE_sb"));
    c.args(args).env("SB_HOME", root).env_remove("SB_SOCK").env_remove("SB_PANE").env_remove("SB_TOOLS");
    for (k, v) in env {
        c.env(k, v);
    }
    let mut c = c.stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    String::from_utf8_lossy(&c.wait_with_output().unwrap().stdout).into_owned()
}

/// One tool call through `sb mcp --pane` as `pane`: (text, is_error).
fn call(root: &Path, pane: &str, name: &str, args: Value) -> (String, bool) {
    let msgs = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":args}}),
    ];
    let input: String = msgs.iter().map(|m| format!("{m}\n")).collect();
    let out = sb(root, &[("SB_PANE", pane)], &["mcp", "--pane"], &input);
    let reply: Value = out.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).find(|v| v["id"] == 2).unwrap_or_else(|| panic!("no reply: {out}"));
    (reply["result"]["content"][0]["text"].as_str().unwrap().to_owned(), reply["result"]["isError"] == true)
}

#[test]
fn agent_panes_see_switchboard() {
    let root = std::env::temp_dir().join(format!("sb-panetools-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("bin")).unwrap();
    let root = root.canonicalize().unwrap();
    let (a, b) = (root.join("alpha"), root.join("beta"));
    for r in [&a, &b] {
        std::fs::create_dir_all(r).unwrap();
        g(r, &["init", "-q", "-b", "main"]);
        g(r, &["commit", "-q", "--allow-empty", "-m", "a"]);
    }
    // A stand-in claude that writes its args and SB_TOOLS, then waits.
    std::fs::write(
        root.join("bin/claude"),
        format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{0}/args-'\"$SB_PANE\"\necho \"$SB_TOOLS\" > '{0}/tools-'\"$SB_PANE\"\nexec cat >/dev/null\n", root.display()),
    )
    .unwrap();
    Command::new("chmod").args(["+x", &root.join("bin/claude").to_string_lossy()]).status().unwrap();
    std::env::set_var("PATH", format!("{}:{}", root.join("bin").display(), std::env::var("PATH").unwrap_or_default()));
    std::env::set_var("SB_HOME", &root);
    std::env::set_var("SB_TMUX_SOCKET", format!("sb-test-panetools-{}", std::process::id()));
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
    let open = |repo: &Path, place: &str, branch: Option<&str>| {
        hub.open(OpenReq {
            repo: repo.to_string_lossy().into(),
            place: place.into(),
            branch: branch.map(str::to_owned),
            tree: None,
            run: "claude".into(),
            title: None,
            task: None,
            first: None,
        })
        .unwrap()
        .id
    };
    let p1 = open(&a, "main", None);
    let p2 = open(&a, "new", Some("wt"));
    let p3 = open(&b, "main", None);

    // The start: the `switchboard` server for this pane, not strict, and the allow rule.
    let args_of = |p: &str| root.join(format!("args-{p}"));
    wait_for("claude starts", || [&p1, &p2, &p3].iter().all(|p| args_of(p).exists()));
    let argv: Vec<String> = std::fs::read_to_string(args_of(&p1)).unwrap().lines().map(str::to_owned).collect();
    let i = argv.iter().position(|x| x == "--mcp-config").unwrap_or_else(|| panic!("no --mcp-config in {argv:?}"));
    let cfg: Value = serde_json::from_str(&argv[i + 1]).unwrap();
    assert_eq!(cfg["mcpServers"]["switchboard"]["args"], json!(["mcp", "--pane"]));
    assert_eq!(cfg["mcpServers"]["switchboard"]["env"]["SB_PANE"], json!(p1));
    assert!(!argv.contains(&"--strict-mcp-config".to_owned()), "{argv:?}");
    assert_eq!(std::fs::read_to_string(root.join(format!("tools-{p1}"))).unwrap().trim(), "1");
    let settings: Value = serde_json::from_str(&std::fs::read_to_string(root.join(format!("home/settings/{p1}.json"))).unwrap()).unwrap();
    assert!(settings["permissions"]["allow"].as_array().unwrap().contains(&json!("mcp__switchboard")), "{settings}");

    // The start note, only with SB_TOOLS.
    let note = sb(&root, &[("SB_PANE", &p1), ("SB_TOOLS", "1")], &["state", "session"], "{}");
    let note: Value = serde_json::from_str(note.trim()).unwrap_or_else(|_| panic!("not JSON: {note}"));
    assert_eq!(note["hookSpecificOutput"]["hookEventName"], "SessionStart");
    assert!(note["hookSpecificOutput"]["additionalContext"].as_str().unwrap().contains("task_list"));
    assert_eq!(sb(&root, &[("SB_PANE", &p1)], &["state", "session"], "{}").trim(), "");

    // The board: the panes of the caller's repo, worktrees too; not the other repo.
    let (text, err) = call(&root, &p1, "board_list", json!({}));
    assert!(!err, "{text}");
    let board: Value = serde_json::from_str(&text).unwrap();
    let ids: Vec<&str> = board["panes"].as_array().unwrap().iter().map(|p| p["pane"].as_str().unwrap()).collect();
    assert!(ids.contains(&p1.as_str()) && ids.contains(&p2.as_str()) && !ids.contains(&p3.as_str()), "{board}");
    let me = board["panes"].as_array().unwrap().iter().find(|p| p["you"] == true).unwrap();
    assert_eq!(me["pane"], json!(p1));
    assert!(!text.contains("summary"), "no agent text: {text}");
    // Not a pane on the board: refused.
    let (text, err) = call(&root, "nobody", "board_list", json!({}));
    assert!(err && text.contains("not a Switchboard pane"), "{text}");

    // Tasks: add as the pane, list, note, and done stays for the window.
    let (text, err) = call(&root, &p1, "task_add", json!({ "title": "Answer Michael's threads" }));
    assert!(!err, "{text}");
    let id = serde_json::from_str::<Value>(&text).unwrap()["task"]["id"].as_str().unwrap().to_owned();
    let (text, _) = call(&root, &p3, "task_list", json!({}));
    assert!(text.contains("Answer Michael's threads"), "{text}");
    let (_, err) = call(&root, &p1, "task_note", json!({ "id": id, "text": "two threads" }));
    assert!(!err);
    let (text, err) = call(&root, &p1, "task_status", json!({ "id": id, "status": "done" }));
    assert!(err && text.contains("window"), "{text}");
    let (text, err) = call(&root, &p1, "task_show", json!({ "id": id }));
    assert!(!err, "{text}");
    let shown: Value = serde_json::from_str(&text).unwrap();
    assert!(shown["task"]["notes"].as_str().unwrap().contains("two threads"));
    assert!(shown["history"].as_array().unwrap().iter().any(|h| h["by"] == json!(p1)), "{shown}");
}
