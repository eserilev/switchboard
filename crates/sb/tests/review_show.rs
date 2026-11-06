//! Claude points to the code on a real hub and tmux (SPEC 11.15): the Ask pane's
//! start, "Mentioned" from the Stop hook through `sb state turn`, `review_show`
//! through `sb mcp --ask`, and the rows of a file the PR does not change.
//! A file of its own: the test sets PATH.

use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use switchboard_core::hub::{Hub, Sink};
use switchboard_core::store::ReviewRow;

struct Quiet;

impl Sink for Quiet {
    fn emit(&self, _event: &str, _payload: Value) {}
    fn output(&self, _pane: &str, _data: &[u8]) {}
}

fn g(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
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

/// Runs `sb` with a JSON stdin and returns its stdout.
fn sb(root: &Path, env: &[(&str, &str)], args: &[&str], stdin: &str) -> String {
    let mut c = Command::new(env!("CARGO_BIN_EXE_sb"));
    c.args(args).env("SB_HOME", root).env_remove("SB_SOCK").env_remove("SB_PANE");
    for (k, v) in env {
        c.env(k, v);
    }
    let mut c = c.stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    String::from_utf8_lossy(&c.wait_with_output().unwrap().stdout).into_owned()
}

#[test]
fn claude_points_to_the_code() {
    let root = std::env::temp_dir().join(format!("sb-show-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("bin")).unwrap();
    let root = root.canonicalize().unwrap();
    // The review worktree: a base and a head that changes line 5 of src/a.rs.
    let tree = root.join("work/demo-pr7");
    std::fs::create_dir_all(tree.join("src")).unwrap();
    g(&tree, &["init", "-q", "-b", "main"]);
    let body: Vec<String> = (1..=20).map(|n| format!("let x{n} = {n};")).collect();
    std::fs::write(tree.join("src/a.rs"), body.join("\n") + "\n").unwrap();
    std::fs::write(tree.join("README.md"), (1..=40).map(|n| format!("readme {n}\n")).collect::<String>()).unwrap();
    g(&tree, &["add", "-A"]);
    g(&tree, &["commit", "-q", "-m", "base"]);
    let base = g(&tree, &["rev-parse", "HEAD"]);
    let mut head_body = body.clone();
    head_body[4] = "let total = helper_sum(x1, x2);".into();
    std::fs::write(tree.join("src/a.rs"), head_body.join("\n") + "\n").unwrap();
    g(&tree, &["commit", "-q", "-am", "head"]);
    let head = g(&tree, &["rev-parse", "HEAD"]);

    // A stand-in claude that writes its args, one a line, and waits.
    let args_file = root.join("claude-args");
    std::fs::write(root.join("bin/claude"), format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nexec cat >/dev/null\n", args_file.display())).unwrap();
    Command::new("chmod").args(["+x", &root.join("bin/claude").to_string_lossy()]).status().unwrap();
    std::env::set_var("PATH", format!("{}:{}", root.join("bin").display(), std::env::var("PATH").unwrap_or_default()));
    std::env::set_var("SB_HOME", &root);
    std::env::set_var("SB_TMUX_SOCKET", format!("sb-test-show-{}", std::process::id()));
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
    hub.debug_add_review(ReviewRow {
        id: "r1".into(),
        url: "https://github.com/demo/demo/pull/7".into(),
        repo: "demo/demo".into(),
        number: 7,
        title: "PR 7".into(),
        head: head.clone(),
        base,
        tree: tree.to_string_lossy().into(),
        connection: None,
        guide_session: Some("g1".into()),
        guide: Some(json!({"steps": [{"id": "s1", "title": "One", "ranges": [{"file": "src/a.rs", "side": "new", "from": 5, "to": 5}]}]}).to_string()),
        round: 1,
        since: None,
        scope_note: None,
        kind: "pr".into(),
        pane: None,
    });

    // The Ask pane: `sb mcp --ask` with strict MCP, the rules, user settings only,
    // and `review_show` allowed with no prompt.
    let ask = hub.review_term("r1", None, true).unwrap().unwrap();
    wait_for("claude starts", || std::fs::read_to_string(&args_file).is_ok_and(|a| a.contains("--strict-mcp-config")));
    let argv: Vec<String> = std::fs::read_to_string(&args_file).unwrap().lines().map(str::to_owned).collect();
    let after = |flag: &str| argv.iter().position(|a| a == flag).map(|i| argv[i + 1].clone()).unwrap_or_else(|| panic!("no {flag} in {argv:?}"));
    let cfg: Value = serde_json::from_str(&after("--mcp-config")).unwrap();
    assert_eq!(cfg["mcpServers"]["sb"]["args"], json!(["mcp", "--ask"]));
    assert_eq!(cfg["mcpServers"]["sb"]["env"]["SB_REVIEW"], "r1");
    assert_eq!(after("--mcp-config").lines().count(), 1);
    assert!(argv.contains(&"--strict-mcp-config".to_owned()), "{argv:?}");
    assert_eq!(after("--setting-sources"), "user");
    assert!(after("--append-system-prompt").contains("review_show") && !after("--append-system-prompt").contains('\n'));
    let settings: Value = serde_json::from_str(&std::fs::read_to_string(root.join(format!("home/settings/{ask}.json"))).unwrap()).unwrap();
    assert!(settings["permissions"]["allow"].as_array().unwrap().contains(&json!("mcp__sb__review_show")), "{settings}");

    // A turn of the Ask pane: what the answer points to, from `last_assistant_message`.
    let pane = [("SB_PANE", ask.as_str())];
    let answer = format!("Fix {}/src/a.rs:5 first. It calls `helper_sum`. README.md:3 says so. Not /etc/passwd:1 or ../x.rs:2.", tree.display());
    sb(&root, &pane, &["state", "turn"], &json!({ "session_id": "s9", "last_assistant_message": answer }).to_string());
    wait_for("Mentioned", || !hub.review_view("r1").unwrap().mentions.is_empty());
    let m = hub.review_view("r1").unwrap().mentions;
    assert_eq!(m.len(), 1, "{m:?}");
    assert_eq!((m[0].file.as_str(), m[0].line, m[0].changed), ("src/a.rs", Some(5), true));
    // A new answer replaces the list. With no text in the input, the session file has it.
    let transcript = root.join("t.jsonl");
    std::fs::write(&transcript, [
        json!({"type": "user", "message": {"content": "and the rest?"}}).to_string(),
        json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "See a.rs:1 and `helper_sum`."}]}}).to_string(),
    ].join("\n")).unwrap();
    sb(&root, &pane, &["state", "turn"], &json!({ "session_id": "s9", "transcript_path": transcript }).to_string());
    wait_for("the new list", || hub.review_view("r1").unwrap().mentions.first().is_some_and(|x| x.line == Some(1)));
    let m = hub.review_view("r1").unwrap().mentions;
    assert_eq!(m.iter().map(|x| (x.line, x.name.clone(), x.changed)).collect::<Vec<_>>(), vec![(Some(1), None, false), (Some(5), Some("helper_sum".into()), true)]);
    // Another pane's turn changes nothing.
    sb(&root, &[("SB_PANE", "p999")], &["state", "turn"], &json!({ "last_assistant_message": "README.md" }).to_string());

    // `review_show` through `sb mcp --ask`: the places go to the review, and a path
    // that leaves the review is refused.
    let mcp = |points: Value| -> Value {
        let msg = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "review_show", "arguments": {"points": points}}});
        let out = sb(&root, &[("SB_REVIEW", "r1")], &["mcp", "--ask"], &format!("{msg}\n"));
        serde_json::from_str(out.trim()).unwrap()
    };
    let r = mcp(json!([
        {"file": "src/a.rs", "line": 5, "kind": "must", "note": "Use safe_add."},
        {"file": "README.md", "line": 30, "note": "Not changed."},
    ]));
    assert_eq!(r["result"]["isError"], false, "{r}");
    let show = hub.review_view("r1").unwrap().show.unwrap();
    assert_eq!(show.points.iter().map(|p| (p.file.as_str(), p.line, p.changed)).collect::<Vec<_>>(), vec![("src/a.rs", 5, true), ("README.md", 30, false)]);
    for bad in [json!([{"file": "../../etc/passwd", "line": 1}]), json!([{"file": "/etc/passwd", "line": 1}]), json!([{"file": "src/a.rs", "line": 99}]), json!([{"file": "nope.rs", "line": 1}])] {
        let r = mcp(bad.clone());
        assert_eq!(r["result"]["isError"], true, "{bad}: {r}");
    }
    // A refused call keeps the places of the last good one.
    assert_eq!(hub.review_view("r1").unwrap().show.unwrap(), show);
    // Each call is new for the window.
    mcp(json!([{"file": "src/a.rs", "line": 1, "note": "x"}]));
    assert!(hub.review_view("r1").unwrap().show.unwrap().at > show.at);

    // The rows of a file the PR does not change, around a line, at the head.
    let v = hub.review_context("r1", "README.md", 30).unwrap();
    let rows = &v.sections[0].rows;
    assert!(v.sections[0].context && v.head == head);
    assert_eq!((rows[0].new, rows.last().unwrap().new, rows[15].text.as_str()), (Some(15), Some(40), "readme 30"));
    assert!(hub.review_context("r1", "../x", 1).is_err());
    assert!(hub.review_context("r1", "nope.md", 1).is_err());

    // A resume of the ended Ask pane gets the tool and the rules again.
    std::fs::remove_file(&args_file).unwrap();
    hub.resume(&ask).unwrap();
    wait_for("claude starts again", || std::fs::read_to_string(&args_file).is_ok_and(|a| a.contains("--strict-mcp-config")));
    let argv = std::fs::read_to_string(&args_file).unwrap();
    assert!(argv.contains("--append-system-prompt") && argv.contains("\"--ask\""), "{argv}");
    // It runs in the PR's files: only your user settings load, as at the start.
    assert!(argv.contains("--setting-sources\nuser\n"), "{argv}");
    let settings: Value = serde_json::from_str(&std::fs::read_to_string(root.join(format!("home/settings/{ask}.json"))).unwrap()).unwrap();
    assert!(settings["permissions"]["allow"].as_array().unwrap().contains(&json!("mcp__sb__review_show")), "{settings}");
}
