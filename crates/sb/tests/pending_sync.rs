//! The pending review on GitHub (SPEC 11.8.1), on a real hub with a fake GitHub:
//! the drafts go up as you write them, one sync at a time, errors try again, a
//! pending review from the website is taken over, Update guide rebuilds it at the
//! new head, a send on the website shows here, and Finish review submits it.
//! Nothing here reaches GitHub.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use switchboard_core::gh::{Fate, Gh, Pending, PrPending};
use switchboard_core::hub::{Hub, Sink};
use switchboard_core::hub_review::NewComment;
use switchboard_core::post::{GhComment, Inline};
use switchboard_core::review::PrInfo;
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
    let end = Instant::now() + Duration::from_secs(15);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out: {what}");
}

#[derive(Clone, Debug)]
struct C {
    id: String,
    body: String,
    reply: bool,
    at: Inline,
}

#[derive(Clone, Debug)]
struct Pend {
    id: String,
    commit: String,
    comments: Vec<C>,
}

/// GitHub as the fake keeps it.
#[derive(Default)]
struct World {
    head: String,
    diff: HashMap<String, String>,
    pending: Option<Pend>,
    fates: HashMap<String, Fate>,
    /// Sent reviews: review id, type, summary, comments.
    sent: Vec<(String, String, String, Vec<C>)>,
    rest_sent: Vec<Value>,
    deleted: Vec<String>,
    next: u64,
    fail_adds: u32,
    fail_create: bool,
    gate: bool,
    in_add: u32,
    max_in_add: u32,
    pending_calls: u32,
}

#[derive(Default)]
struct Fake(Mutex<World>);

impl Fake {
    fn w(&self) -> std::sync::MutexGuard<'_, World> {
        self.0.lock().unwrap()
    }

    fn id(w: &mut World, p: &str) -> String {
        w.next += 1;
        format!("{p}{}", w.next)
    }

    fn bodies(&self) -> Vec<String> {
        let mut b: Vec<String> = self.w().pending.as_ref().map(|p| p.comments.iter().map(|c| c.body.clone()).collect()).unwrap_or_default();
        b.sort();
        b
    }
}

impl Gh for Fake {
    fn view(&self, _url: &str) -> Result<PrInfo, String> {
        let w = self.w();
        Ok(PrInfo {
            number: 7,
            title: "PR 7".into(),
            body: String::new(),
            head_ref_oid: w.head.clone(),
            head_ref_name: "pr".into(),
            base_ref_name: "main".into(),
            base_ref_oid: String::new(),
            files: vec![],
        })
    }
    fn pr_diff(&self, _repo: &str, _number: u64) -> Result<String, String> {
        let w = self.w();
        Ok(w.diff[&w.head].clone())
    }
    fn diff_at(&self, _repo: &str, _url: &str, base: &str, head: &str) -> Result<String, String> {
        assert_eq!(base, "main");
        self.w().diff.get(head).cloned().ok_or("no such commit".into())
    }
    fn login(&self) -> Result<String, String> {
        Ok("me".into())
    }
    fn reviews(&self, _repo: &str, _number: u64) -> Result<Value, String> {
        Ok(json!([]))
    }
    fn review_comments(&self, _repo: &str, _number: u64, _review: u64) -> Result<Value, String> {
        let w = self.w();
        let p = w.pending.as_ref().ok_or("no pending review")?;
        Ok(Value::Array(
            p.comments
                .iter()
                .map(|c| json!({ "node_id": c.id, "path": c.at.path, "line": c.at.line, "start_line": c.at.start_line, "side": c.at.side, "body": c.body, "in_reply_to_id": if c.reply { json!(1) } else { Value::Null } }))
                .collect(),
        ))
    }
    fn submit_rest(&self, _repo: &str, _number: u64, payload: &Value) -> Result<String, String> {
        self.w().rest_sent.push(payload.clone());
        Ok("https://github.com/demo/demo/pull/7#rest".into())
    }
    fn pending(&self, _repo: &str, _number: u64) -> Result<PrPending, String> {
        let mut w = self.w();
        w.pending_calls += 1;
        Ok(PrPending {
            pr_id: "PR7".into(),
            pending: w.pending.as_ref().map(|p| Pending {
                id: p.id.clone(),
                db_id: 1,
                commit: p.commit.clone(),
                comments: p.comments.iter().map(|c| GhComment { id: c.id.clone(), body: c.body.clone(), reply: c.reply }).collect(),
            }),
        })
    }
    fn fate(&self, review: &str) -> Result<Fate, String> {
        let w = self.w();
        if w.pending.as_ref().is_some_and(|p| p.id == review) {
            return Ok(Fate::Pending);
        }
        Ok(w.fates.get(review).cloned().unwrap_or(Fate::Gone))
    }
    fn create_pending(&self, pr_id: &str, commit: &str) -> Result<String, String> {
        assert_eq!(pr_id, "PR7");
        let mut w = self.w();
        if w.fail_create {
            return Err("GitHub: the commit is not part of the pull request".into());
        }
        assert!(w.pending.is_none(), "GitHub allows one pending review for each person");
        let id = Fake::id(&mut w, "PRR");
        w.pending = Some(Pend { id: id.clone(), commit: commit.into(), comments: vec![] });
        Ok(id)
    }
    fn add_comment(&self, review: &str, c: &Inline) -> Result<String, String> {
        {
            let mut w = self.w();
            w.in_add += 1;
            w.max_in_add = w.max_in_add.max(w.in_add);
        }
        while self.w().gate {
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut w = self.w();
        w.in_add -= 1;
        if w.fail_adds > 0 {
            w.fail_adds -= 1;
            return Err("GitHub: something went wrong".into());
        }
        let id = Fake::id(&mut w, "C");
        let p = w.pending.as_mut().filter(|p| p.id == review).ok_or("no such pending review")?;
        p.comments.push(C { id: id.clone(), body: c.text.clone(), reply: false, at: c.clone() });
        Ok(id)
    }
    fn edit_comment(&self, comment: &str, body: &str) -> Result<(), String> {
        let mut w = self.w();
        let c = w.pending.as_mut().and_then(|p| p.comments.iter_mut().find(|c| c.id == comment)).ok_or("no such comment")?;
        c.body = body.into();
        Ok(())
    }
    fn delete_comment(&self, comment: &str) -> Result<(), String> {
        if let Some(p) = self.w().pending.as_mut() {
            p.comments.retain(|c| c.id != comment);
        }
        Ok(())
    }
    fn delete_review(&self, review: &str) -> Result<(), String> {
        let mut w = self.w();
        if w.pending.as_ref().is_some_and(|p| p.id == review) {
            w.pending = None;
        }
        w.deleted.push(review.into());
        Ok(())
    }
    fn submit(&self, review: &str, event: &str, body: &str) -> Result<String, String> {
        let mut w = self.w();
        let p = w.pending.take().filter(|p| p.id == review).ok_or("no such pending review")?;
        let url = format!("https://github.com/demo/demo/pull/7#{}", p.id);
        w.fates.insert(p.id.clone(), Fate::Sent { state: "COMMENTED".into(), url: url.clone() });
        w.sent.push((p.id, event.into(), body.into(), p.comments));
        Ok(url)
    }
}

const URL: &str = "https://github.com/demo/demo/pull/7";

fn row(id: &str, tree: &Path, base: &str, head: &str) -> ReviewRow {
    ReviewRow {
        id: id.into(),
        url: URL.into(),
        repo: "demo/demo".into(),
        number: 7,
        title: "PR 7".into(),
        head: head.into(),
        base: base.into(),
        tree: tree.to_string_lossy().into(),
        connection: None,
        guide_session: None,
        guide: Some(r#"{"steps":[]}"#.into()),
        round: 1,
        since: None,
        scope_note: None,
        kind: "pr".into(),
        pane: None,
    }
}

fn file(lines: &[&str]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

#[test]
fn drafts_sync_to_a_pending_review_and_finish_review_submits_it() {
    let root = std::env::temp_dir().join(format!("sb-pending-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    // The PR: a base, head 1, and head 2 that changes another file only.
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    g(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.rs"), file(&["fn a() {", "    one();", "    two();", "    three();", "}"])).unwrap();
    std::fs::write(repo.join("b.rs"), file(&["fn b() {}"])).unwrap();
    g(&repo, &["add", "-A"]);
    g(&repo, &["commit", "-q", "-m", "base"]);
    let base = g(&repo, &["rev-parse", "HEAD"]);
    std::fs::write(repo.join("a.rs"), file(&["fn a() {", "    one();", "    TWO();", "    three();", "    four();", "}"])).unwrap();
    g(&repo, &["commit", "-q", "-am", "head 1"]);
    let head1 = g(&repo, &["rev-parse", "HEAD"]);
    std::fs::write(repo.join("b.rs"), file(&["fn b() { b(); }"])).unwrap();
    g(&repo, &["commit", "-q", "-am", "head 2"]);
    let head2 = g(&repo, &["rev-parse", "HEAD"]);
    let fake = Arc::new(Fake::default());
    {
        let mut w = fake.w();
        w.head = head1.clone();
        w.diff.insert(head1.clone(), g(&repo, &["diff", &base, &head1]));
        w.diff.insert(head2.clone(), g(&repo, &["diff", &base, &head2]));
    }

    std::env::set_var("SB_HOME", &root);
    std::env::set_var("SB_TMUX_SOCKET", format!("sb-pending-{}", std::process::id()));
    std::env::remove_var("SB_SOCK");
    let hub = Hub::start(switchboard_core::paths::Paths::from_env(), PathBuf::from(env!("CARGO_BIN_EXE_sb")), Arc::new(Quiet)).unwrap();
    struct Kill(String, PathBuf);
    impl Drop for Kill {
        fn drop(&mut self) {
            switchboard_core::tmux::kill_server(&self.0);
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }
    let _kill = Kill(std::env::var("SB_TMUX_SOCKET").unwrap(), root.clone());

    // With SB_HOME set and no fake, nothing syncs.
    hub.debug_add_review(row("r1", &repo, &base, &head1));
    let comment = |line: u32, text: &str, line_text: &str| NewComment {
        path: "a.rs".into(),
        side: "new".into(),
        line,
        start_line: None,
        text: text.into(),
        head: head1.clone(),
        line_text: line_text.into(),
        agent: false,
    };
    hub.review_comment("r1", comment(3, "Why upper case?", "    TWO();")).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(fake.w().pending.is_none() && !hub.debug_syncing("r1"));
    assert!(hub.review_view("r1").unwrap().sync.is_none());

    // 1. A sync puts the draft up: a pending review at the head, with the comment.
    hub.debug_set_gh(fake.clone());
    hub.review_draft_edit("r1", hub.review_view("r1").unwrap().drafts[0].id, Some("Why upper case?".into())).unwrap();
    let synced = |n: usize| {
        let v = hub.review_view("r1").unwrap();
        let s = v.sync.unwrap();
        !s.running && s.drafts.len() == n && s.drafts.iter().all(|d| d.state == "synced")
    };
    wait_for("the first comment on GitHub", || synced(1));
    assert_eq!(fake.w().pending.as_ref().unwrap().commit, head1);
    assert_eq!(fake.bodies(), ["Why upper case?"]);
    let c = fake.w().pending.as_ref().unwrap().comments[0].at.clone();
    assert_eq!((c.path.as_str(), c.side.as_str(), c.line), ("a.rs", "RIGHT", 3));

    // 2. An edit edits the comment; a delete deletes it.
    let d1 = hub.review_view("r1").unwrap().drafts[0].id;
    hub.review_draft_edit("r1", d1, Some("Why upper case here?".into())).unwrap();
    wait_for("the edit on GitHub", || fake.bodies() == ["Why upper case here?"] && synced(1));
    hub.review_draft_edit("r1", d1, None).unwrap();
    wait_for("the delete on GitHub", || fake.bodies().is_empty() && synced(0));

    // 3. One sync at a time: three comments while GitHub is slow make at most one
    // more sync, and never two adds at once.
    fake.w().gate = true;
    let calls = fake.w().pending_calls;
    hub.review_comment("r1", comment(2, "one", "    one();")).unwrap();
    wait_for("an add waits", || fake.w().in_add == 1);
    hub.review_comment("r1", comment(4, "three", "    three();")).unwrap();
    hub.review_comment("r1", comment(5, "four", "    four();")).unwrap();
    fake.w().gate = false;
    wait_for("three comments on GitHub", || synced(3) && fake.bodies().len() == 3);
    assert_eq!(fake.w().max_in_add, 1);
    assert!(fake.w().pending_calls - calls <= 2, "{} syncs", fake.w().pending_calls - calls);
    assert_eq!(fake.bodies(), ["four", "one", "three"]);

    // 4. An error keeps the draft and its error; the retry puts it up.
    fake.w().fail_adds = 1;
    hub.review_comment("r1", comment(1, "five", "fn a() {")).unwrap();
    wait_for("the error shows", || {
        let s = hub.review_view("r1").unwrap().sync.unwrap();
        !s.running && s.error.is_some() && s.drafts.iter().any(|d| d.state == "error")
    });
    assert_eq!(hub.review_view("r1").unwrap().drafts.len(), 4);
    hub.debug_retry_now("r1");
    wait_for("the retry", || synced(4) && fake.bodies().len() == 4);

    // 5. Update guide: the head moves, the drafts move with their code, and the
    // pending review is made again at the new head with the same comments.
    let old = fake.w().pending.as_ref().unwrap().id.clone();
    fake.w().head = head2.clone();
    hub.debug_add_review(row("r1", &repo, &base, &head2));
    // The first use of the new model moves the drafts and starts a sync, as an
    // update's model load does.
    let d = hub.review_view("r1").unwrap().drafts[0].clone();
    hub.review_draft_edit("r1", d.id, Some(d.text)).unwrap();
    wait_for("the pending review at the new head", || fake.w().pending.as_ref().is_some_and(|p| p.commit == head2) && synced(4) && fake.bodies().len() == 4);
    assert!(fake.w().deleted.contains(&old));

    // 6. A comment written on the website becomes a draft, on its line.
    {
        let mut w = fake.w();
        let id = Fake::id(&mut w, "W");
        let at = Inline { draft: 0, path: "a.rs".into(), side: "RIGHT".into(), line: 6, start_line: None, text: String::new() };
        w.pending.as_mut().unwrap().comments.push(C { id, body: "From the website".into(), reply: false, at });
        let id = Fake::id(&mut w, "W");
        let at = Inline { draft: 0, path: "a.rs".into(), side: "RIGHT".into(), line: 2, start_line: None, text: String::new() };
        w.pending.as_mut().unwrap().comments.push(C { id, body: "A reply".into(), reply: true, at });
    }
    hub.debug_retry_now("r1");
    wait_for("the website comment is a draft", || synced(5));
    let v = hub.review_view("r1").unwrap();
    let site = v.drafts.iter().find(|d| d.text == "From the website").unwrap();
    assert_eq!((site.line, site.line_text.as_deref(), site.stale), (Some(6), Some("}"), false));
    assert_eq!(fake.bodies().len(), 6, "the reply stays on GitHub");

    // 7. Finish review submits the pending review, with the summary.
    let p = hub.review_post_preview("r1").unwrap();
    assert_eq!(p.plan.inline.len(), 5);
    assert!(p.newer.is_none());
    let url = hub.review_post("r1", "COMMENT", "Looks good.", &p.token).unwrap();
    let pending_id = {
        let w = fake.w();
        assert!(w.pending.is_none());
        assert_eq!(w.sent.len(), 1);
        assert_eq!((w.sent[0].1.as_str(), w.sent[0].2.as_str(), w.sent[0].3.len()), ("COMMENT", "Looks good.", 6));
        assert!(w.rest_sent.is_empty());
        w.sent[0].0.clone()
    };
    assert!(url.ends_with(&pending_id));
    let v = hub.review_view("r1").unwrap();
    assert!(v.drafts.is_empty());
    assert_eq!(v.posted.len(), 1);
    assert_eq!(v.sync.unwrap().pending, 0);

    // 8. A pending review from the website at the reviewed head is taken over; one
    // at another commit is left alone, and Finish review sends in one REST call.
    {
        let mut w = fake.w();
        let id = Fake::id(&mut w, "PRR");
        let at = Inline { draft: 0, path: "a.rs".into(), side: "RIGHT".into(), line: 3, start_line: None, text: String::new() };
        let cid = Fake::id(&mut w, "W");
        w.pending = Some(Pend { id, commit: head2.clone(), comments: vec![C { id: cid, body: "Started on the site".into(), reply: false, at }] });
    }
    let comment2 = |line: u32, text: &str, line_text: &str| NewComment { head: head2.clone(), ..comment(line, text, line_text) };
    hub.review_comment("r1", comment2(2, "Mine", "    one();")).unwrap();
    wait_for("the take-over", || synced(2) && fake.bodies() == ["Mine", "Started on the site"]);
    // You send it on the website: the app records it, and the drafts in it go.
    {
        let mut w = fake.w();
        let p = w.pending.take().unwrap();
        w.fates.insert(p.id, Fate::Sent { state: "APPROVED".into(), url: "https://github.com/demo/demo/pull/7#site".into() });
    }
    hub.debug_retry_now("r1");
    wait_for("the website send shows", || hub.review_view("r1").unwrap().drafts.is_empty());
    let v = hub.review_view("r1").unwrap();
    assert_eq!(v.posted.last().unwrap().event, "APPROVE");
    assert_eq!(v.posted.last().unwrap().url, "https://github.com/demo/demo/pull/7#site");

    {
        let mut w = fake.w();
        let id = Fake::id(&mut w, "PRR");
        w.pending = Some(Pend { id, commit: head1.clone(), comments: vec![] });
    }
    hub.review_comment("r1", comment2(2, "Blocked", "    one();")).unwrap();
    wait_for("the block shows", || hub.review_view("r1").unwrap().sync.unwrap().blocked.is_some() && !hub.debug_syncing("r1"));
    assert!(fake.w().pending.as_ref().unwrap().comments.is_empty(), "the website's pending review is left alone");
    let p = hub.review_post_preview("r1").unwrap();
    hub.review_post("r1", "COMMENT", "", &p.token).unwrap();
    {
        let w = fake.w();
        assert_eq!(w.rest_sent.len(), 1);
        assert_eq!(w.rest_sent[0]["comments"][0]["body"], "Blocked");
    }

    // 9. GitHub takes no pending review from the app: the comment stays here with
    // the error, and Finish review still sends, in one REST call.
    {
        let mut w = fake.w();
        w.pending = None;
        w.fail_create = true;
    }
    hub.review_comment("r1", comment2(4, "No pending review", "    three();")).unwrap();
    wait_for("the error shows", || {
        let s = hub.review_view("r1").unwrap().sync.unwrap();
        !s.running && s.error.as_deref().is_some_and(|e| e.contains("not part of"))
    });
    let p = hub.review_post_preview("r1").unwrap();
    hub.review_post("r1", "COMMENT", "", &p.token).unwrap();
    let w = fake.w();
    assert_eq!(w.rest_sent.len(), 2);
    assert_eq!(w.rest_sent[1]["comments"][0]["body"], "No pending review");
    assert!(hub.review_view("r1").unwrap().drafts.is_empty());
}
