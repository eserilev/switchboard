//! Today on a real hub with a fake GitHub (SPEC 39): the `today` command, the
//! refresh, Done and its undo, Later, a job from the config, `sb today` and the
//! `today_item` pane tool. Nothing here reaches GitHub.

use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use switchboard_core::gh::{Fate, Gh, PrPending};
use switchboard_core::hub::{Hub, Sink};
use switchboard_core::post::Inline;
use switchboard_core::review::PrInfo;

struct Quiet;

impl Sink for Quiet {
    fn emit(&self, _event: &str, _payload: Value) {}
    fn output(&self, _pane: &str, _data: &[u8]) {}
}

type Res<T> = Result<T, String>;

/// GitHub with two open PRs: yours with a thread that waits on you, and one that
/// asks for your review. It counts the searches.
#[derive(Default)]
struct Fake {
    searches: std::sync::Mutex<Vec<String>>,
}

fn node(number: u64, repo: &str, author: &str, extra: Value) -> Value {
    let mut n = json!({
        "number": number, "title": format!("PR {number}"), "url": format!("https://github.com/{repo}/pull/{number}"),
        "isDraft": false, "state": "OPEN", "createdAt": "2026-11-20T10:00:00Z", "updatedAt": "2026-11-22T10:00:00Z",
        "author": { "login": author }, "repository": { "nameWithOwner": repo },
        "labels": { "nodes": [] }, "additions": 5, "deletions": 1, "mergeable": "MERGEABLE", "baseRefName": "main",
        "reviewRequests": { "nodes": [] }, "timelineItems": { "nodes": [] }, "latestReviews": { "nodes": [] },
        "commits": { "nodes": [] }, "comments": { "nodes": [] }, "reviewThreads": { "nodes": [] }, "files": { "nodes": [] }
    });
    if let (Some(o), Some(e)) = (n.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            o.insert(k.clone(), v.clone());
        }
    }
    n
}

impl Gh for Fake {
    fn graphql(&self, _query: &str, vars: Value) -> Res<Option<Value>> {
        let q = vars["q"].as_str().unwrap_or("").to_owned();
        self.searches.lock().unwrap().push(q.clone());
        let nodes = if q.contains("author:me") && q.contains("is:open") {
            vec![node(
                1,
                "o/r",
                "me",
                json!({ "reviewThreads": { "nodes": [ { "isResolved": false, "path": "a.rs",
                    "comments": { "nodes": [ { "author": { "login": "bob" }, "createdAt": "2026-11-22T09:00:00Z" } ] } } ] } }),
            )]
        } else if q.contains("review-requested:me") {
            vec![node(
                2,
                "x/y",
                "ann",
                json!({ "reviewRequests": { "nodes": [ { "requestedReviewer": { "login": "me" } } ] } }),
            )]
        } else {
            vec![]
        };
        Ok(Some(json!({ "search": { "nodes": nodes } })))
    }
    fn view(&self, _url: &str) -> Res<PrInfo> {
        Err("no".into())
    }
    fn pr_diff(&self, _repo: &str, _number: u64) -> Res<String> {
        Err("no".into())
    }
    fn diff_at(&self, _repo: &str, _url: &str, _base: &str, _head: &str) -> Res<String> {
        Err("no".into())
    }
    fn login(&self) -> Res<String> {
        Ok("me".into())
    }
    fn reviews(&self, _repo: &str, _number: u64) -> Res<Value> {
        Err("no".into())
    }
    fn review_comments(&self, _repo: &str, _number: u64, _review: u64) -> Res<Value> {
        Err("no".into())
    }
    fn submit_rest(&self, _repo: &str, _number: u64, _payload: &Value) -> Res<String> {
        Err("no".into())
    }
    fn pending(&self, _repo: &str, _number: u64) -> Res<PrPending> {
        Err("no".into())
    }
    fn fate(&self, _review: &str) -> Res<Fate> {
        Err("no".into())
    }
    fn create_pending(&self, _pr_id: &str, _commit: &str) -> Res<String> {
        Err("no".into())
    }
    fn add_comment(&self, _review: &str, _c: &Inline) -> Res<String> {
        Err("no".into())
    }
    fn edit_comment(&self, _comment: &str, _body: &str) -> Res<()> {
        Err("no".into())
    }
    fn delete_comment(&self, _comment: &str) -> Res<()> {
        Err("no".into())
    }
    fn delete_review(&self, _review: &str) -> Res<()> {
        Err("no".into())
    }
    fn submit(&self, _review: &str, _event: &str, _body: &str) -> Res<String> {
        Err("no".into())
    }
}

fn wait_for(what: &str, f: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(15);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out: {what}");
}

/// Runs `sb` with a stdin and returns (stdout, stderr).
fn sb(root: &Path, env: &[(&str, &str)], args: &[&str], stdin: &str) -> (String, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_sb"));
    c.args(args).env("SB_HOME", root).env_remove("SB_SOCK").env_remove("SB_PANE").env_remove("SB_TOOLS");
    for (k, v) in env {
        c.env(k, v);
    }
    let mut c = c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let out = c.wait_with_output().unwrap();
    (String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

fn rows(v: &Value, section: &str) -> Vec<u64> {
    v["sections"].as_array().unwrap().iter().filter(|s| s["key"] == section).flat_map(|s| s["items"].as_array().unwrap().iter().map(|i| i["number"].as_u64().unwrap())).collect()
}

#[test]
fn today_lists_ranks_and_hides() {
    let root = std::env::temp_dir().join(format!("sb-today-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    std::fs::write(root.join("config.toml"), "[today]\nuser = \"me\"\n\n[today.jobs.Work]\nrepos = [\"o/r\"]\n").unwrap();
    std::env::set_var("SB_HOME", &root);
    std::env::set_var("SB_TMUX_SOCKET", format!("sb-test-today-{}", std::process::id()));
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
    let fake = Arc::new(Fake::default());
    hub.debug_set_gh(fake.clone());

    // The first call starts a refresh; the list comes when it ends.
    let first = hub.today(None).unwrap();
    assert_eq!(first["job"], "Everything");
    wait_for("the first refresh", || hub.today(None).unwrap()["loaded"] == true && hub.today(None).unwrap()["refreshing"] == false);
    let v = hub.today(None).unwrap();
    assert_eq!(rows(&v, "mine"), [1], "{v}");
    assert_eq!(rows(&v, "reviews"), [2]);
    assert_eq!(v["sections"][0]["items"][0]["line"], "Reply to bob's 1 open thread.");
    assert_eq!(v["jobs"], json!([{ "name": "Everything", "count": 2 }, { "name": "Work", "count": 1 }]));
    assert_eq!(v["login"], "me");
    let n = fake.searches.lock().unwrap().len();
    assert!(fake.searches.lock().unwrap().iter().any(|q| q == "is:pr is:open archived:false author:me"));
    // A fresh list is not fetched again.
    hub.today(None).unwrap();
    assert_eq!(fake.searches.lock().unwrap().len(), n);

    // A job is a filter on the same list.
    let w = hub.today(Some("Work")).unwrap();
    assert_eq!((rows(&w, "mine"), rows(&w, "reviews")), (vec![1], vec![]));

    // Done hides it, the undo brings it back; Later hides it until its time.
    hub.today_done("O/R", 1, false, true).unwrap();
    let v = hub.today(None).unwrap();
    assert!(rows(&v, "mine").is_empty());
    assert_eq!(v["done"][0]["number"], 1);
    hub.today_done("o/r", 1, false, false).unwrap();
    assert_eq!(rows(&hub.today(None).unwrap(), "mine"), [1]);
    let later = switchboard_core::now() + 3600;
    hub.today_later("x/y", 2, false, Some(later)).unwrap();
    let v = hub.today(None).unwrap();
    assert!(rows(&v, "reviews").is_empty());
    assert_eq!(v["later"][0]["later_until"], later);
    hub.today_later("x/y", 2, false, None).unwrap();

    // Tell Claude (39.12): a stand-in `claude` hides #2 and lowers #1, and counts its runs.
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let answer = json!({ "reply": "Hid #2.", "items": [
        { "pr": "x/y#2", "act": "hide", "why": "FOCIL is on hold" },
        { "pr": "o/r#1", "act": "down", "why": "not urgent" } ] });
    let line = json!({ "type": "result", "subtype": "success", "is_error": false, "result": answer.to_string() });
    std::fs::write(bin.join("claude"), format!("#!/bin/sh\nprintf '%s' \"$2\" > \"$PWD/prompt.txt\"\necho run >> \"$PWD/runs\"\ncat <<'EOF'\n{line}\nEOF\n")).unwrap();
    Command::new("chmod").args(["+x", bin.join("claude").to_str().unwrap()]).status().unwrap();
    std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
    assert!(hub.today_tell("   ", "Everything").is_err());
    hub.today_tell("FOCIL work does not matter now", "Everything").unwrap();
    wait_for("Claude read the note", || {
        let v = hub.today(None).unwrap();
        v["judging"] == false && v["hidden"].as_array().is_some_and(|h| h.len() == 1)
    });
    let v = hub.today(None).unwrap();
    assert!(rows(&v, "reviews").is_empty(), "{v}");
    assert_eq!(v["hidden"][0]["number"], 2);
    assert_eq!(v["hidden"][0]["claude"], json!({ "act": "hide", "why": "FOCIL is on hold" }));
    assert_eq!(v["reply"], "Hid #2.");
    assert_eq!(v["notes"][0]["text"], "FOCIL work does not matter now");
    assert!(v["sections"][0]["items"][0]["why"].as_str().unwrap().ends_with("Claude moved it down, from your notes: not urgent."), "{v}");
    let prompt = std::fs::read_to_string(root.join("home/today/prompt.txt")).unwrap();
    assert!(prompt.contains("1. FOCIL work does not matter now") && prompt.contains("x/y#2 [Reviews]") && prompt.contains("<github-data>PR 2</github-data>"), "{prompt}");
    // A note belongs to the tab it was told in: the Work job has neither the note,
    // nor Claude's reply, nor its verdicts.
    let w = hub.today(Some("Work")).unwrap();
    assert_eq!((w["notes"].as_array().unwrap().len(), w["reply"].is_null()), (0, true), "{w}");
    assert!(!w["sections"][0]["items"][0]["why"].as_str().unwrap().contains("Claude moved"), "{w}");
    // The list is saved, so a restart of the hub shows it at once.
    assert!(root.join("home/today/list.json").is_file());
    // The same notes and PRs: a refresh does not ask Claude again.
    hub.today_refresh_now().unwrap();
    wait_for("the second refresh", || hub.today(None).unwrap()["refreshing"] == false);
    std::thread::sleep(Duration::from_millis(300));
    let runs = || std::fs::read_to_string(root.join("home/today/runs")).unwrap_or_default().lines().count();
    assert_eq!(runs(), 1);
    let (out, _) = sb(&root, &[], &["today"], "");
    assert!(out.contains("Hidden by the user's notes to Claude: #2.") && !out.contains("REVIEWS"), "{out}");
    // Forget the note: the list is as before, and Claude does not run.
    let id = v["notes"][0]["id"].as_u64().unwrap();
    hub.today_forget(id).unwrap();
    let v = hub.today(None).unwrap();
    assert_eq!((rows(&v, "reviews"), v["hidden"].as_array().unwrap().len(), v["notes"].as_array().unwrap().len()), (vec![2], 0, 0), "{v}");
    assert!(hub.today_forget(id).is_err());
    assert_eq!(runs(), 1);

    // `sb today` and the pane tool read the same list.
    let (out, err) = sb(&root, &[], &["today"], "");
    assert!(out.contains("YOUR PRS (1)") && out.contains("Reply to bob's 1 open thread.") && out.contains("REVIEWS (1)"), "{out}{err}");
    let (out, _) = sb(&root, &[], &["today", "Work"], "");
    assert!(out.starts_with("Today · Work") && !out.contains("REVIEWS"), "{out}");
    let msgs = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"today_item","arguments":{"repo":"x/y","number":2}}}),
    ];
    let input: String = msgs.iter().map(|m| format!("{m}\n")).collect();
    let (out, _) = sb(&root, &[("SB_PANE", "p-none")], &["mcp", "--pane"], &input);
    let reply: Value = out.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).find(|v| v["id"] == 2).unwrap_or_else(|| panic!("no reply: {out}"));
    let text = reply["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Review asked of you") && text.contains("<github-data>PR 2</github-data>"), "{text}");
}
