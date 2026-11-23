//! Messages between agents (SPEC 40), end to end: a "server" hub with the mailbox
//! and a "Mac" hub, each with its own tmux, linked by `sb msg watch`. `env` runs
//! the other side in place of ssh. A stand-in `claude` writes what it gets to a
//! file in its folder. The test sets PATH and other process-wide variables: it is
//! its own binary.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use switchboard_core::hub::{Hub, OpenReq, Sink};
use switchboard_core::paths::Paths;

struct Quiet;

impl Sink for Quiet {
    fn emit(&self, _event: &str, _payload: Value) {}
    fn output(&self, _pane: &str, _data: &[u8]) {}
}

fn wait_for(what: &str, secs: u64, f: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while !f() {
        assert!(Instant::now() < end, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

struct Clean(Vec<String>, PathBuf);
impl Drop for Clean {
    fn drop(&mut self) {
        for s in &self.0 {
            switchboard_core::tmux::kill_server(s);
        }
        let _ = std::fs::remove_dir_all(&self.1);
    }
}

fn repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    for args in [&["init", "-q", "-b", "main"][..], &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"][..]] {
        assert!(Command::new("git").arg("-C").arg(dir).args(args).status().unwrap().success());
    }
}

fn start(root: &Path, socket: &str, config: &str) -> Arc<Hub> {
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(root.join("config.toml"), config).unwrap();
    std::env::set_var("SB_HOME", root);
    std::env::set_var("SB_TMUX_SOCKET", socket);
    Hub::start(Paths::from_env(), PathBuf::from(env!("CARGO_BIN_EXE_sb")), Arc::new(Quiet)).unwrap()
}

fn open(hub: &Hub, dir: &Path) -> String {
    hub.open(OpenReq { repo: dir.to_string_lossy().into(), place: "main".into(), branch: None, tree: None, run: "claude".into(), title: None, task: None, first: None }).unwrap().id
}

/// `sb` as a hook or a tool of `pane` on the hub at `home`, with `input` on stdin.
fn sb(home: &Path, pane: &str, dir: &Path, args: &[&str], input: &str) -> (i32, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_sb"))
        .args(args)
        .current_dir(dir)
        .env("SB_HOME", home)
        .env("SB_PANE", pane)
        .env_remove("SB_SOCK")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut c.stdin.take().unwrap(), input.as_bytes()).unwrap();
    let out = c.wait_with_output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn agents_on_two_machines_send_messages_that_land() {
    let pid = std::process::id();
    let root = std::env::temp_dir().join(format!("sb-mail-{pid}"));
    let _ = std::fs::remove_dir_all(&root);
    let (sa, sb_) = (format!("sb-test-mail-a-{pid}"), format!("sb-test-mail-b-{pid}"));
    let _clean = Clean(vec![sa.clone(), sb_.clone()], root.clone());
    std::env::remove_var("SB_SOCK");
    std::env::remove_var("SB_REMOTE");

    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("claude"), "#!/bin/sh\nstty raw -echo 2>/dev/null\nexec cat >> \"$PWD/.typed-$SB_PANE\"\n").unwrap();
    Command::new("chmod").args(["+x", bin.join("claude").to_str().unwrap()]).status().unwrap();
    std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));

    let (ra, rb) = (root.join("box/demo"), root.join("mac/demo"));
    repo(&ra);
    repo(&rb);
    let (ha, hb) = (root.join("a"), root.join("b"));
    // The server keeps the mailbox: no `remote` in its config.
    let hub_a = start(&ha, &sa, "machine = \"box\"\n");
    let a1 = open(&hub_a, &ra);
    // The Mac reaches the server's `sb msg` through `env` in place of ssh.
    let link = serde_json::json!(["env", format!("SB_HOME={}", ha.display()), env!("CARGO_BIN_EXE_sb"), "msg"]).to_string();
    std::env::set_var("SB_MAIL_LINK", &link);
    let hub_b = start(&hb, &sb_, "machine = \"mac\"\nremote = \"box\"\n");
    let b1 = open(&hub_b, &rb);
    let (to_a, to_b) = (format!("{a1}@box"), format!("{b1}@mac"));

    // The link brings the Mac's agents to the server's list.
    wait_for("the Mac's agent on the server", 15, || hub_a.mail_agents().iter().any(|a| a.address == to_b));
    let (code, out) = sb(&hb, &b1, &rb, &["msg", "agents"], "");
    let v: Value = serde_json::from_str(out.trim()).unwrap();
    let list = v["agents"].as_array().unwrap();
    assert_eq!(code, 0, "{out}");
    assert!(list.iter().any(|a| a["address"] == to_a.as_str() && a["you"] == false), "{out}");
    assert!(list.iter().any(|a| a["address"] == to_b.as_str() && a["you"] == true), "{out}");

    // Refusals: to itself, a bad address, an agent that does not run.
    for (to, why) in [(to_a.as_str(), "itself"), ("nobody", "not an agent address"), ("p99@box", "No agent")] {
        let (code, out) = sb(&ha, &a1, &ra, &["msg", "send", to], "hi");
        assert!(code == 1 && out.contains(why), "{to}: {out}");
    }

    // Server to Mac, to an idle agent: typed in with Enter, after the agent settles.
    let (code, out) = sb(&ha, &a1, &ra, &["msg", "send", &to_b], "Run the tests on your side, please.");
    assert_eq!(code, 0, "{out}");
    let typed = |dir: &Path, pane: &str| std::fs::read_to_string(dir.join(format!(".typed-{pane}"))).unwrap_or_default();
    wait_for("the message in the Mac agent", 15, || typed(&rb, &b1).contains("\r"));
    let got = typed(&rb, &b1);
    assert!(got.contains(&format!("[Message from agent {to_a} (demo · main)]")) && got.contains("Run the tests on your side, please."), "{got}");
    assert!(got.contains("not from the user"), "{got}");
    // The server knows it was handed.
    wait_for("handed on the server", 10, || {
        hub_a.mail_list(&a1).unwrap()["messages"].as_array().unwrap().iter().any(|m| m["to"] == to_b.as_str() && m["state"] == "handed")
    });

    // Mac to server, to a working agent: nothing is typed; the end of its turn hands it over.
    sb(&ha, &a1, &ra, &["state", "working"], "{}");
    wait_for("a1 working", 5, || hub_a.views().iter().any(|v| v.id == a1 && v.lamp.as_str() == "working"));
    let (code, out) = sb(&hb, &b1, &rb, &["msg", "send", &to_a], "Tests pass here. 48 of 48.");
    assert_eq!(code, 0, "{out}");
    std::thread::sleep(Duration::from_secs(5));
    assert!(!typed(&ra, &a1).contains("48 of 48"), "a working agent gets nothing typed");
    let (code, out) = sb(&ha, &a1, &ra, &["state", "turn"], "{}");
    assert_eq!(code, 0);
    let v: Value = serde_json::from_str(out.trim()).unwrap_or(Value::Null);
    assert_eq!(v["decision"], "block", "{out}");
    let reason = v["reason"].as_str().unwrap();
    assert!(reason.contains(&format!("[Message from agent {to_b} (demo · main · {b1})]")) && reason.contains("48 of 48"), "{reason}");
    wait_for("a1 works on", 5, || hub_a.views().iter().any(|v| v.id == a1 && v.lamp.as_str() == "working"));
    // Handed once: the next end of a turn has nothing.
    let (_, out) = sb(&ha, &a1, &ra, &["state", "turn"], "{}");
    assert!(!out.contains("block"), "{out}");
    assert!(hub_a.mail_list(&a1).unwrap()["messages"].as_array().unwrap().iter().all(|m| m["state"] == "handed"));

    // A sender has a limit.
    let mut refused = false;
    for i in 0..25 {
        let (code, out) = sb(&ha, &a1, &ra, &["msg", "send", &to_b], &format!("ping {i}"));
        if code == 1 {
            assert!(out.contains("Wait, or ask the user"), "{out}");
            refused = true;
            break;
        }
    }
    assert!(refused, "the send limit holds");
}
