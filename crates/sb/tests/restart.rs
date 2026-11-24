//! Restarts of the hub (SPEC 29.12). Each test runs a real `sb serve`, gets work
//! going, sends `hub_restart`, and checks what comes back. Stand-in `gh` and
//! `claude` scripts on the hub's PATH stand for GitHub and Claude: nothing here
//! reaches the network. Each test has its own home and its own tmux server.

use serde_json::{json, Value};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use switchboard_core::remote::{Client, News};

fn wait_for(what: &str, secs: u64, f: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out: {what}");
}

/// The stand-in `gh`. It logs every call. `gh api graphql` answers a Today
/// search for your PRs with one PR; `gh pr view` answers from `pr.json`.
const GH: &str = r#"#!/bin/sh
D="$(dirname "$0")/.."
if [ "$1 $2" = "api graphql" ]; then
  IN="$(cat)"
  echo graphql >> "$D/gh.log"
  case "$IN" in
    *"author:me"*"is:open"*|*"is:open"*"author:me"*)
      cat "$D/search-mine.json" ;;
    *) echo '{"data":{"search":{"nodes":[]}}}' ;;
  esac
  exit 0
fi
if [ "$1 $2" = "pr view" ]; then
  echo "pr view" >> "$D/gh.log"
  cat "$D/pr.json"
  exit 0
fi
if [ "$1 $2" = "api user" ]; then echo me; exit 0; fi
echo "$*" >> "$D/gh.log"
echo '[]'
"#;

/// The stand-in `claude`. A `-p` run in the Today folder answers Claude's notes.
/// Another `-p` run is a guide: it logs itself and hangs, as a long guide does.
/// An agent pane logs its args and env and keeps what it is sent.
const CLAUDE: &str = r#"#!/bin/sh
D="$(dirname "$0")/.."
if [ "$1" = "-p" ]; then
  case "$PWD" in
    */today)
      echo note >> "$D/claude-notes.log"
      printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"{\"reply\":\"Noted.\",\"items\":[]}"}'
      exit 0 ;;
  esac
  echo guide >> "$D/claude-guide.log"
  exec sleep 600
fi
echo "$*" > "$PWD/.args-$SB_PANE"
stty raw -echo 2>/dev/null
exec cat >> "$PWD/.typed-$SB_PANE"
"#;

/// A home for one hub: its folder, tmux socket, stand-ins and config.
struct Home {
    root: PathBuf,
    socket: String,
}

impl Home {
    fn new(name: &str, config: &str) -> Home {
        let root = std::env::temp_dir().join(format!("sb-restart-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::create_dir_all(root.join("work/demo")).unwrap();
        git(&root.join("work/demo"), &["init", "-q", "-b", "main"]);
        git(&root.join("work/demo"), &["commit", "-q", "--allow-empty", "-m", "base"]);
        for (f, text) in [("gh", GH), ("claude", CLAUDE)] {
            let p = root.join("bin").join(f);
            std::fs::write(&p, text).unwrap();
            Command::new("chmod").args(["+x", p.to_str().unwrap()]).status().unwrap();
        }
        std::fs::write(root.join("search-mine.json"), json!({ "data": { "search": { "nodes": [] } } }).to_string()).unwrap();
        std::fs::write(root.join("config.toml"), config).unwrap();
        Home { root, socket: format!("sb-test-restart-{name}-{}", std::process::id()) }
    }

    fn path_var(&self) -> String {
        format!("{}:{}", self.root.join("bin").display(), std::env::var("PATH").unwrap_or_default())
    }

    /// `sb attach` with this home's env, as ssh would run it on the server.
    fn argv(&self) -> Vec<String> {
        vec![
            "env".into(),
            "-u".into(),
            "SB_SOCK".into(),
            "-u".into(),
            "SB_PANE".into(),
            format!("SB_HOME={}", self.root.display()),
            format!("SB_TMUX_SOCKET={}", self.socket),
            format!("PATH={}", self.path_var()),
            "SHELL=/bin/sh".into(),
            env!("CARGO_BIN_EXE_sb").into(),
            "attach".into(),
        ]
    }

    /// `sb` as a pane's hook or tool, with `input` on stdin.
    fn sb(&self, pane: &str, args: &[&str], input: &str) -> String {
        use std::io::Write;
        let mut c = Command::new(env!("CARGO_BIN_EXE_sb"))
            .args(args)
            .current_dir(self.root.join("work/demo"))
            .env("SB_HOME", &self.root)
            .env("SB_TMUX_SOCKET", &self.socket)
            .env("SB_PANE", pane)
            .env("PATH", self.path_var())
            .env_remove("SB_SOCK")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        c.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        String::from_utf8_lossy(&c.wait_with_output().unwrap().stdout).into_owned()
    }

    fn serve_pid(&self) -> Option<i32> {
        std::fs::read_to_string(self.root.join("run/serve.pid")).ok()?.trim().parse().ok()
    }

    fn lines(&self, file: &str) -> usize {
        std::fs::read_to_string(self.root.join(file)).unwrap_or_default().lines().count()
    }

    /// The window's link: `sb attach` starts `sb serve` when none runs.
    fn connect(&self) -> (Arc<Client>, Arc<Mutex<Option<bool>>>) {
        let down = Arc::new(Mutex::new(None));
        let d = Arc::clone(&down);
        let c = Client::connect(&self.argv(), move |n| {
            if let News::Down { taken, .. } = n {
                *d.lock().unwrap() = Some(taken);
            }
        })
        .expect("connect");
        (c, down)
    }

    /// `hub_restart`, then a new link to the hub that came back: the same process.
    fn restart(&self, c: &Client, down: &Mutex<Option<bool>>) -> Arc<Client> {
        let pid = self.serve_pid().unwrap();
        c.call("hub_restart", json!({})).unwrap();
        wait_for("the link goes down", 10, || down.lock().unwrap().is_some());
        assert_eq!(*down.lock().unwrap(), Some(false), "a restart is no takeover");
        wait_for("sb serve listens again", 15, || UnixStream::connect(self.root.join("run/window.sock")).is_ok() && self.serve_pid().is_some());
        assert_eq!(self.serve_pid(), Some(pid), "the same process, started again");
        self.connect().0
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        if let Some(pid) = self.serve_pid() {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
        std::thread::sleep(Duration::from_millis(300));
        let _ = Command::new("tmux").args(["-L", &self.socket, "kill-server"]).status();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=t", "-c", "user.email=t@t"]).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn open_claude(c: &Client, home: &Home) -> String {
    let v = c.call("pane_open", json!({ "req": { "repo": home.root.join("work/demo"), "place": "main", "run": "claude" } })).unwrap();
    v["id"].as_str().unwrap().to_owned()
}

/// Test 1: a guide that a restart cuts off starts again on its own (SPEC 11.2).
#[test]
fn a_cut_off_guide_starts_again() {
    let home = Home::new("guide", "");
    // "GitHub": a bare repo with the PR's head at `refs/pull/7/head`, and a clone
    // whose remote names github.com, sent to the bare repo with `insteadOf`.
    let up = home.root.join("up.git");
    git(&home.root, &["init", "-q", "--bare", "-b", "main", up.to_str().unwrap()]);
    let seed = home.root.join("seed");
    git(&home.root, &["clone", "-q", up.to_str().unwrap(), seed.to_str().unwrap()]);
    std::fs::write(seed.join("a.txt"), "one\n").unwrap();
    git(&seed, &["add", "a.txt"]);
    git(&seed, &["commit", "-q", "-m", "base"]);
    git(&seed, &["push", "-q", "origin", "HEAD:main"]);
    let base = git(&seed, &["rev-parse", "HEAD"]);
    std::fs::write(seed.join("a.txt"), "one\ntwo\n").unwrap();
    git(&seed, &["commit", "-q", "-am", "the change"]);
    git(&seed, &["push", "-q", "origin", "HEAD:refs/pull/7/head", "HEAD:refs/heads/feat"]);
    let head = git(&seed, &["rev-parse", "HEAD"]);
    let clone = home.root.join("work/lh");
    git(&home.root, &["clone", "-q", up.to_str().unwrap(), clone.to_str().unwrap()]);
    git(&clone, &["remote", "set-url", "origin", "https://github.com/o/lh.git"]);
    git(&clone, &["config", &format!("url.{}.insteadOf", up.display()), "https://github.com/o/lh.git"]);
    std::fs::write(
        home.root.join("pr.json"),
        json!({ "number": 7, "title": "The change", "body": "", "headRefOid": head, "headRefName": "feat", "baseRefName": "main", "baseRefOid": base,
            "files": [ { "path": "a.txt", "additions": 1, "deletions": 0 } ] }).to_string(),
    )
    .unwrap();
    std::fs::write(home.root.join("config.toml"), format!("scan = [\"{}\"]\n", home.root.join("work").display())).unwrap();

    let (c, down) = home.connect();
    let id = c.call("review_open", json!({ "url": "https://github.com/o/lh/pull/7", "roundOf": null })).unwrap();
    let id = id.as_str().unwrap().to_owned();
    wait_for("the guide run starts", 30, || home.lines("claude-guide.log") == 1);
    let c = home.restart(&c, &down);
    // The new start finds the review with no guide and runs it again.
    wait_for("the guide starts again", 30, || home.lines("claude-guide.log") == 2);
    let reviews = c.call("reviews", json!({})).unwrap();
    let r = reviews.as_array().unwrap().iter().find(|r| r["id"] == id.as_str()).unwrap_or_else(|| panic!("{reviews}"));
    assert_eq!(r["status"], "writing", "{r}");
    c.close();
}

/// Tests 2 and 3: the Today list comes back from disk with no fetch, and Claude's
/// notes come back with no new Claude run.
#[test]
fn today_and_its_notes_come_back_with_no_new_fetch() {
    let home = Home::new("today", "[today]\nuser = \"me\"\nrefresh_minutes = 10\n");
    let pr = json!({
        "number": 5, "title": "My PR", "url": "https://github.com/o/r/pull/5", "isDraft": false, "state": "OPEN",
        "createdAt": "2026-11-20T10:00:00Z", "updatedAt": "2026-11-22T10:00:00Z", "author": { "login": "me" },
        "repository": { "nameWithOwner": "o/r" }, "labels": { "nodes": [] }, "additions": 5, "deletions": 1,
        "mergeable": "MERGEABLE", "baseRefName": "main", "reviewRequests": { "nodes": [] }, "timelineItems": { "nodes": [] },
        "latestReviews": { "nodes": [] }, "commits": { "nodes": [] }, "comments": { "nodes": [] },
        "reviewThreads": { "nodes": [ { "isResolved": false, "path": "a.rs",
            "comments": { "nodes": [ { "author": { "login": "bob" }, "createdAt": "2026-11-22T09:00:00Z" } ] } } ] },
        "files": { "nodes": [] }
    });
    std::fs::write(home.root.join("search-mine.json"), json!({ "data": { "search": { "nodes": [pr] } } }).to_string()).unwrap();

    let (c, down) = home.connect();
    let rows = |v: &Value| -> usize { v["sections"].as_array().map(|s| s.iter().map(|x| x["items"].as_array().map_or(0, |i| i.len())).sum()).unwrap_or(0) };
    c.call("today", json!({ "job": null })).unwrap();
    wait_for("the first refresh", 30, || {
        let v = c.call("today", json!({ "job": null })).unwrap();
        v["loaded"] == true && v["refreshing"] == false && rows(&v) == 1
    });
    c.call("today_tell", json!({ "text": "My PRs come first.", "job": "Everything" })).unwrap();
    wait_for("Claude read the note", 30, || {
        let v = c.call("today", json!({ "job": null })).unwrap();
        v["judging"] == false && v["reply"] == "Noted."
    });
    let (fetches, notes) = (home.lines("gh.log"), home.lines("claude-notes.log"));
    assert_eq!(notes, 1);

    let c = home.restart(&c, &down);
    // At once: the saved list, its time, the note and Claude's reply.
    let v = c.call("today", json!({ "job": null })).unwrap();
    assert_eq!((v["loaded"].as_bool(), rows(&v)), (Some(true), 1), "{v}");
    assert!(v["updated"].as_u64().is_some());
    assert_eq!((v["notes"][0]["text"].as_str(), v["reply"].as_str()), (Some("My PRs come first."), Some("Noted.")));
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(home.lines("gh.log"), fetches, "a fresh saved list is not fetched again");
    assert_eq!(home.lines("claude-notes.log"), notes, "Claude does not read the same notes again");
    c.close();
}

/// Test 4: a message that waits for an agent arrives once after a restart.
#[test]
fn a_waiting_message_arrives_once_after_a_restart() {
    let home = Home::new("mail", "machine = \"box\"\n");
    let (c, down) = home.connect();
    let (a, b) = (open_claude(&c, &home), open_claude(&c, &home));
    let typed = |p: &str| std::fs::read_to_string(home.root.join(format!("work/demo/.typed-{p}"))).unwrap_or_default();
    // b works, so the message waits for the end of its turn.
    home.sb(&b, &["state", "working"], "{}");
    let out = home.sb(&a, &["msg", "send", &format!("{b}@box")], "Run the tests, please.");
    assert!(out.contains("\"ok\":true"), "{out}");
    std::thread::sleep(Duration::from_secs(1));
    assert!(!typed(&b).contains("Run the tests"), "a working agent gets nothing typed");

    let c = home.restart(&c, &down);
    let turn = home.sb(&b, &["state", "turn"], "{}");
    // It arrives once: by the end of the turn, or typed in after the restart.
    wait_for("the message arrives", 20, || turn.contains("Run the tests") || typed(&b).contains("Run the tests"));
    std::thread::sleep(Duration::from_secs(4));
    let again = home.sb(&b, &["state", "turn"], "{}");
    let count = turn.matches("Run the tests").count() + typed(&b).matches("Run the tests").count() + again.matches("Run the tests").count();
    assert_eq!(count, 1, "turn: {turn} typed: {} again: {again}", typed(&b));
    c.close();
}

/// Test 5: an agent keeps running through a restart and keeps its session, and
/// `sb resume` starts it again with that session.
#[test]
fn an_agent_keeps_its_session_through_a_restart() {
    let home = Home::new("agent", "");
    let (c, down) = home.connect();
    let p = open_claude(&c, &home);
    home.sb(&p, &["state", "session"], r#"{"session_id":"s-9"}"#);
    let tree = home.root.join("work/demo");
    let proj = switchboard_core::handover::project_dir(&home.root.join("claude"), &tree);
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(proj.join("s-9.jsonl"), "{}\n").unwrap();
    let pane = |c: &Client| c.call("panes", json!({})).unwrap().as_array().unwrap().iter().find(|x| x["id"] == p.as_str()).cloned();
    wait_for("the session is known", 10, || pane(&c).is_some_and(|x| x["session"] == "s-9"));

    let c = home.restart(&c, &down);
    let after = pane(&c).expect("the pane is still on the board");
    assert_eq!(after["session"], "s-9", "{after}");
    assert_ne!(after["lamp"], "ended", "its process still runs: {after}");
    let args = || std::fs::read_to_string(tree.join(format!(".args-{p}"))).unwrap_or_default();
    std::fs::remove_file(tree.join(format!(".args-{p}"))).unwrap();
    let out = home.sb("", &["resume", &p], "");
    assert!(out.contains("started again"), "{out}");
    wait_for("resumed with its session", 15, || args().contains("--resume s-9"));
    c.close();
}
