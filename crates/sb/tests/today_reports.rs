//! Today for a repo of bug reports (SPEC 39.13) on a real hub with a fake GitHub:
//! a job that reads every open issue, the report sections, Everything that keeps
//! only what involves you, the job saved from the window, and the repo check.
//! Nothing here reaches GitHub.

use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
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

fn iso(days_ago: u64) -> String {
    let t = switchboard_core::now() - days_ago * 86400;
    let d = switchboard_core::hub_today::ymd(t);
    let s = t % 86400;
    format!("{d}T{:02}:{:02}:{:02}Z", s / 3600, s % 3600 / 60, s % 60)
}

fn issue(n: u64, labels: &[&str], assignee: Option<&str>, comments: Value) -> Value {
    json!({
        "number": n, "title": format!("Report {n}"), "url": format!("https://github.com/eb/lh/issues/{n}"),
        "createdAt": iso(5), "updatedAt": iso(1), "author": { "login": "rep" }, "authorAssociation": "NONE",
        "repository": { "nameWithOwner": "eb/lh" },
        "labels": { "nodes": labels.iter().map(|l| json!({ "name": l })).collect::<Vec<_>>() },
        "assignees": { "nodes": assignee.map(|a| vec![json!({ "login": a })]).unwrap_or_default() },
        "comments": comments, "timelineItems": { "nodes": [] }
    })
}

#[derive(Default)]
struct Fake {
    searches: Mutex<Vec<String>>,
}

impl Gh for Fake {
    fn graphql(&self, query: &str, vars: Value) -> Res<Option<Value>> {
        if query.contains("repository(owner") {
            return Ok(Some(if vars["n"] == "lh" {
                json!({ "repository": { "isPrivate": true, "issues": { "totalCount": 3 }, "pullRequests": { "totalCount": 1 },
                    "labels": { "nodes": [ { "name": "High" }, { "name": "Low" }, { "name": "wontfix" } ] } } })
            } else {
                json!({ "repository": null })
            }));
        }
        let q = vars["q"].as_str().unwrap_or("").to_owned();
        self.searches.lock().unwrap().push(q.clone());
        // Assigned to you, and the reporter wrote after you.
        let mine = issue(1, &["High"], Some("me"), json!({ "nodes": [
            { "author": { "login": "me" }, "authorAssociation": "MEMBER", "createdAt": iso(4), "body": "Looking." },
            { "author": { "login": "rep" }, "authorAssociation": "NONE", "createdAt": iso(2), "body": "More." } ] }));
        let nodes = if q.starts_with("is:issue is:open repo:eb/lh") {
            vec![
                mine,
                // New, no severity, no owner.
                issue(2, &[], None, json!({ "nodes": [] })),
                // Hidden by its label.
                issue(3, &["wontfix"], None, json!({ "nodes": [] })),
            ]
        } else if q.starts_with("is:issue is:open archived:false involves:me") {
            vec![mine]
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

fn section(v: &Value, key: &str) -> Vec<u64> {
    v["sections"].as_array().unwrap().iter().filter(|s| s["key"] == key).flat_map(|s| s["items"].as_array().unwrap().iter().map(|i| i["number"].as_u64().unwrap())).collect()
}

#[test]
fn a_reports_job_from_the_window() {
    let root = std::env::temp_dir().join(format!("sb-today-reports-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("config.toml"), "# my config\n[today]\nuser = \"me\"\n").unwrap();
    std::env::set_var("SB_HOME", &root);
    std::env::set_var("SB_TMUX_SOCKET", format!("sb-test-today-reports-{}", std::process::id()));
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

    // The repo check of Add a job.
    let g = hub.today_repo_check("eb/lh").unwrap();
    assert_eq!((g["kind"].as_str(), g["issues"].as_u64()), (Some("reports"), Some(3)));
    assert!(hub.today_repo_check("eb/none").unwrap_err().contains("can't read"));
    assert!(hub.today_repo_check("no slash").unwrap_err().contains("owner/name"));

    // Save the job as the form does: the config gets it, and Today refreshes.
    let form: switchboard_core::config::JobForm = serde_json::from_value(json!({
        "name": "Bug bounty", "repos": ["eb/lh"], "read": ["issues", "prs"], "scope": "all",
        "kind": "reports", "severity": ["High", "Low"], "hide_labels": ["wontfix"], "reply_days": 2 })).unwrap();
    hub.today_job_save(&form).unwrap();
    let text = std::fs::read_to_string(root.join("config.toml")).unwrap();
    assert!(text.starts_with("# my config") && text.contains("[today.jobs.\"Bug bounty\"]"), "{text}");
    wait_for("the refresh", || {
        let v = hub.today(Some("Bug bounty")).unwrap();
        v["loaded"] == true && v["refreshing"] == false
    });
    let searches = fake.searches.lock().unwrap().clone();
    assert!(searches.iter().any(|q| q == "is:issue is:open repo:eb/lh sort:updated-desc"), "{searches:?}");
    assert!(searches.iter().any(|q| q == "is:pr is:open repo:eb/lh sort:updated-desc"));

    let v = hub.today(Some("Bug bounty")).unwrap();
    assert_eq!(section(&v, "assigned"), [1], "{v}");
    assert_eq!(section(&v, "triage"), [2]);
    let one = &v["sections"].as_array().unwrap().iter().find(|s| s["key"] == "assigned").unwrap()["items"][0];
    assert_eq!((one["line"].as_str(), one["severity"].as_str(), one["issue"].as_bool()), (Some("rep replied 2 days ago. Answer."), Some("High"), Some(true)));
    assert!(!v.to_string().contains("Report 3"), "a wontfix report never shows");
    let tab = v["jobs"].as_array().unwrap().iter().find(|j| j["name"] == "Bug bounty").unwrap()["count"].as_u64();
    assert_eq!(tab, Some(2));

    // Everything keeps what involves you: the assigned report, not the new one.
    let e = hub.today(None).unwrap();
    assert_eq!((section(&e, "assigned"), section(&e, "triage")), (vec![1], vec![]));

    // The pane tool and `sb today` show the sections in words.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_sb")).args(["today", "Bug bounty"]).env("SB_HOME", &root).env_remove("SB_SOCK").env_remove("SB_PANE").output().unwrap();
    let out = String::from_utf8_lossy(&out.stdout);
    assert!(out.contains("FOR YOU (1)") && out.contains("[High]") && out.contains("NEEDS TRIAGE (1)"), "{out}");

    // Remove: the job goes, the repo's rules stay.
    hub.today_job_remove("Bug bounty").unwrap();
    let text = std::fs::read_to_string(root.join("config.toml")).unwrap();
    assert!(!text.contains("jobs.\"Bug bounty\"") && text.contains("[today.repos.\"eb/lh\"]"), "{text}");
}
