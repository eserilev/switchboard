//! Tasks in the hub (SPEC 27). The rules live in `tasks`. Each change sends the
//! whole list to the window as a `tasks` event.

use crate::hub::{Hub, Start};
use crate::lamp::Lamp;
use crate::now;
use crate::repos;
use crate::tasks::{self, NewSprint, NewTask, Place, Sprint, Task, TaskEvent, TaskFields, TaskMove, TaskOp, TasksView};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

type Res<T> = Result<T, String>;

/// A change from the window. The socket labels its changes `cli` or with a pane id.
const BY: &str = "you";

/// What the launcher shows for Start agent (SPEC 27.6). You can change it there.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct StartPlan {
    pub task: String,
    pub repo: Option<String>,
    pub branch: String,
    pub title: String,
    pub first: String,
}

/// The panes that a `task_resume` works on now.
static RESUMING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

struct Resuming(String);

impl Resuming {
    fn take(pane: &str) -> Res<Resuming> {
        let mut r = RESUMING.lock().unwrap();
        if r.iter().any(|p| p == pane) {
            return Err("The agent is starting already.".into());
        }
        r.push(pane.to_owned());
        Ok(Resuming(pane.to_owned()))
    }
}

impl Drop for Resuming {
    fn drop(&mut self) {
        RESUMING.lock().unwrap().retain(|p| p != &self.0);
    }
}

/// Seconds between two PR polls. `SB_PR_POLL_SECS` changes it, for tests.
fn poll_secs() -> u64 {
    std::env::var("SB_PR_POLL_SECS").ok().and_then(|v| v.parse().ok()).filter(|v| *v > 0).unwrap_or(300)
}

/// Asks GitHub for the state of one PR. It only reads (SPEC 3). A `gh` that does not
/// end in 30 s is killed, so a network stall never stops the polls.
fn pr_state(url: &str) -> Res<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut child = Command::new("gh")
        .args(["pr", "view", url, "--json", "state", "--jq", ".state"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("gh: {e}"))?;
    let end = Instant::now() + Duration::from_secs(30);
    let status = loop {
        match child.try_wait().map_err(|e| format!("gh: {e}"))? {
            Some(st) => break st,
            None if Instant::now() >= end => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("gh did not answer in 30 s".into());
            }
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    };
    let mut out = String::new();
    let mut err = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut out);
    }
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    if !status.success() {
        return Err(err.trim().to_owned());
    }
    Ok(out.trim().to_owned())
}

/// Sets `Hub::polling` back to false when the poll ends, also on a panic.
struct PollDone(std::sync::Weak<Hub>);

impl Drop for PollDone {
    fn drop(&mut self) {
        if let Some(hub) = self.0.upgrade() {
            hub.polling.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

/// How long the first message waits for the agent, in ms. Time in the folder trust
/// dialog does not count. `SB_FIRST_WAIT_MS` changes it, for tests.
fn first_wait() -> u64 {
    std::env::var("SB_FIRST_WAIT_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(20_000)
}

impl Hub {
    pub fn tasks(&self) -> Res<TasksView> {
        let mut v = self.store.lock().unwrap().tasks_view()?;
        let mut missing: Vec<String> = v.tasks.iter().filter_map(|t| t.repo.clone()).filter(|r| !Path::new(r).is_dir()).collect();
        missing.sort();
        missing.dedup();
        v.missing = missing;
        let merged = self.merged.lock().unwrap().clone();
        v.merged = merged.into_iter().filter(|m| v.tasks.iter().any(|t| t.links.iter().any(|l| tasks::pr_url(l).as_deref() == Some(m)))).collect();
        Ok(v)
    }

    /// Sends the list after a change. Takes the store lock, so call it with no lock held.
    fn emit_tasks(&self) {
        match self.tasks() {
            Ok(v) => self.sink.emit("tasks", json!(v)),
            Err(err) => tracing::warn!(target: "sb::tasks", %err, "the task list did not load"),
        }
    }

    /// Runs one change on the store, then sends the list.
    fn task_change<X>(&self, f: impl FnOnce(&crate::store::Store) -> Res<X>) -> Res<X> {
        let out = f(&self.store.lock().unwrap());
        if out.is_ok() {
            self.emit_tasks();
        }
        out
    }

    pub fn task_add(&self, task: NewTask) -> Res<Task> {
        self.task_change(|s| s.task_add(task, BY, now()))
    }

    pub fn task_edit(&self, id: &str, fields: TaskFields) -> Res<Task> {
        self.task_change(|s| s.task_edit(id, fields, BY, now()))
    }

    pub fn task_status(&self, id: &str, status: &str, blocked_by: Option<&str>) -> Res<Task> {
        self.task_change(|s| s.task_status(id, status, blocked_by, BY, now()))
    }

    pub fn task_move(&self, id: &str, m: TaskMove) -> Res<Task> {
        self.task_change(|s| s.task_move(id, &m, BY, now()))
    }

    pub fn task_delete(&self, id: &str) -> Res<()> {
        self.task_change(|s| s.task_delete(id, BY, now()))
    }

    pub fn task_restore(&self, id: &str) -> Res<Task> {
        self.task_change(|s| s.task_restore(id, BY, now()))
    }

    /// At hub start: tasks in the archive for more than 30 days go (SPEC 27.2).
    pub(crate) fn prune_tasks(&self) {
        match self.store.lock().unwrap().task_prune(now().saturating_sub(30 * 86_400)) {
            Ok(0) => {}
            Ok(n) => tracing::info!(target: "sb::tasks", n, "old tasks left the archive"),
            Err(err) => tracing::warn!(target: "sb::tasks", %err, "the archive prune failed"),
        }
    }

    pub fn task_copy(&self, id: &str) -> Res<Task> {
        self.task_change(|s| s.task_copy(id, BY, now()))
    }

    pub fn task_history(&self, id: &str) -> Res<Vec<TaskEvent>> {
        self.store.lock().unwrap().task_history(id)
    }

    pub fn sprint_new(&self, sprint: NewSprint) -> Res<Sprint> {
        self.task_change(|s| s.sprint_new(&sprint))
    }

    pub fn sprint_edit(&self, id: &str, sprint: NewSprint) -> Res<Sprint> {
        self.task_change(|s| s.sprint_edit(id, &sprint))
    }

    pub fn sprint_close(&self, id: &str, places: Vec<(String, Place)>, next: Option<NewSprint>) -> Res<Option<Sprint>> {
        self.task_change(|s| s.sprint_close(id, &places, next.as_ref(), BY, now()))
    }

    // ---------- tasks and agents (SPEC 27.6) ----------

    /// The repo, branch, title and first message for Start agent.
    pub fn task_start_plan(&self, id: &str) -> Res<StartPlan> {
        let t = self.store.lock().unwrap().task(id)?;
        if let Some(r) = t.repo.as_deref().filter(|r| !Path::new(r).is_dir()) {
            return Err(format!("The repo folder of this task is missing: {r}"));
        }
        let prefix = self.config().tasks.branch_prefix.clone();
        let repo = t.repo.clone();
        let taken = |b: &str| match &repo {
            Some(r) => {
                let r = Path::new(r);
                repos::git(r, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{b}")]).is_ok()
                    || repos::worktree_path(r, b).exists()
            }
            None => false,
        };
        let branch = tasks::branch_for(&prefix, &t.title, &t.id, taken);
        let first = if t.notes.trim().is_empty() { t.title.clone() } else { format!("{}\n\n{}", t.title, t.notes.trim()) };
        Ok(StartPlan { task: t.id, repo: t.repo, branch, title: t.title, first })
    }

    /// After Start agent: links the pane, and a task in To do goes to In progress. A
    /// failure here leaves the pane running and tells the window.
    pub(crate) fn task_started(&self, task: &str, pane: &str) {
        let at = now();
        let out = (|| {
            let s = self.store.lock().unwrap();
            s.task_link(task, pane, None, BY, at)?;
            if s.task(task)?.status == "todo" {
                s.task_status(task, "doing", None, BY, at)?;
            }
            Ok::<(), String>(())
        })();
        if let Err(err) = out {
            tracing::warn!(target: "sb::tasks", task, pane, %err, "the new pane is not linked");
            self.sink.emit("error", json!({ "message": format!("The agent started, but it is not linked to the task: {err}") }));
        }
        self.emit_tasks();
    }

    /// Types `text` into a new Claude pane as one paste with no Enter, when the agent
    /// is ready: it has its session id, no dialog shows, and 1.5 s went by. After
    /// `first_wait()` with no agent, the window gets the text for the clipboard.
    pub(crate) fn paste_when_ready(&self, pane: String, text: String) {
        let me = self.me.clone();
        let limit = Duration::from_millis(first_wait());
        let _ = std::thread::Builder::new().name("sb-first".into()).spawn(move || {
            let mut waited = Duration::ZERO;
            let mut ready: Option<Instant> = None;
            let step = Duration::from_millis(100);
            let end = Instant::now() + Duration::from_secs(600);
            while Instant::now() < end {
                std::thread::sleep(step);
                let Some(hub) = me.upgrade() else { return };
                let Some(v) = hub.views().into_iter().find(|v| v.id == pane) else { return };
                if matches!(v.lamp, Lamp::Ended | Lamp::Error) {
                    hub.sink.emit("paste_failed", json!({ "pane": pane, "text": text, "why": "the agent ended before it was ready" }));
                    return;
                }
                if v.trust || v.permit.is_some() || v.lamp == Lamp::Needs {
                    // You answer a dialog: that time does not count.
                    ready = None;
                    continue;
                }
                waited += step;
                if v.session.is_some() {
                    let since = *ready.get_or_insert_with(Instant::now);
                    if since.elapsed() >= Duration::from_millis(1500) {
                        tracing::info!(target: "sb::tasks", pane = %pane, "first message pasted");
                        if let Err(err) = hub.input(&pane, crate::hub::bracketed_paste(&text).as_bytes()) {
                            hub.sink.emit("paste_failed", json!({ "pane": pane, "text": text, "why": err }));
                        }
                        return;
                    }
                }
                if waited >= limit {
                    tracing::warn!(target: "sb::tasks", pane = %pane, "the agent did not start in time");
                    hub.sink.emit("paste_failed", json!({ "pane": pane, "text": text, "why": "the agent did not start in time" }));
                    return;
                }
            }
            // A dialog stayed open for 10 minutes.
            if let Some(hub) = me.upgrade() {
                hub.sink.emit("paste_failed", json!({ "pane": pane, "text": text, "why": "a dialog in the pane stayed open" }));
            }
        });
    }

    pub fn task_link(&self, task: &str, pane: &str) -> Res<()> {
        let session = self.views().into_iter().find(|v| v.id == pane).ok_or(format!("no pane {pane}"))?.session;
        self.task_change(|s| s.task_link(task, pane, session.as_deref(), BY, now()))
    }

    pub fn task_unlink(&self, pane: &str) -> Res<()> {
        self.task_change(|s| s.task_unlink(pane, BY, now()))
    }

    /// Resumes the agent of a linked pane. A pane on the board resumes in place. A
    /// closed pane starts again in its folder with its session, and the link moves
    /// to the new pane. Returns the pane id.
    pub fn task_resume(&self, pane: &str) -> Res<String> {
        // Two quick clicks: the second one waits for nothing and fails, so one session
        // never gets two agents.
        let _guard = Resuming::take(pane)?;
        let on_board = self.panes.lock().unwrap().iter().find(|p| p.row.id == pane).map(|p| (p.lamp(), p.dead));
        if let Some((lamp, dead)) = on_board {
            // A resume restarts the process: never while the agent runs.
            if !(lamp == Lamp::Ended || (lamp == Lamp::Error && dead)) {
                return Err("The agent still runs. Open its pane instead.".into());
            }
            self.resume(pane)?;
            return Ok(pane.to_owned());
        }
        let link = self
            .store
            .lock()
            .unwrap()
            .task_links()?
            .into_iter()
            .find(|l| l.pane == pane)
            .ok_or("this pane is not linked to a task")?;
        if link.kind.as_deref() != Some("claude") {
            return Err("only a Claude pane resumes".into());
        }
        let session = link.session.clone().ok_or("the pane has no session to resume")?;
        let tree = PathBuf::from(link.tree.clone().ok_or("the pane has no folder")?);
        if !tree.is_dir() {
            return Err(format!("the folder of the pane is gone: {}", tree.display()));
        }
        let repo = repos::main_checkout(&tree).unwrap_or(tree.clone());
        let name = repo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let start = Start { resume: Some(session.clone()), connection: link.connection.clone(), ..Start::default() };
        let v = self.spawn("claude", &name, &tree, link.title.clone(), start)?;
        self.task_change(|s| {
            s.task_link(&link.task, &v.id, Some(&session), BY, now())?;
            s.task_unlink(pane, BY, now())
        })?;
        Ok(v.id)
    }

    /// Each second from the ticker. The first PR poll runs 10 s after start, then
    /// one every 5 minutes, in a thread of its own: `gh` can take seconds.
    pub(crate) fn task_tick(&self, n: u64) {
        let secs = poll_secs();
        let first = secs.min(10);
        if n < first || !(n - first).is_multiple_of(secs) {
            return;
        }
        if self.polling.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let me = self.me.clone();
        let spawned = std::thread::Builder::new().name("sb-prpoll".into()).spawn(move || {
            let _done = PollDone(me.clone());
            if let Some(hub) = me.upgrade() {
                hub.poll_prs();
            }
        });
        if spawned.is_err() {
            self.polling.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Reads the state of each PR link of the tasks that are not done, with
    /// `gh pr view --json state`. A link GitHub reports as merged gets the "PR merged"
    /// hint (SPEC 27.6). It never posts.
    pub(crate) fn poll_prs(&self) {
        let view = match self.store.lock().unwrap().tasks_view() {
            Ok(v) => v,
            Err(_) => return,
        };
        let known = self.merged.lock().unwrap().clone();
        let mut urls: Vec<String> =
            view.tasks.iter().filter(|t| t.status != "done").flat_map(|t| t.links.iter().filter_map(|l| tasks::pr_url(l))).collect();
        urls.sort();
        urls.dedup();
        urls.retain(|u| !known.contains(u));
        let mut new = vec![];
        for url in urls {
            match pr_state(&url) {
                Ok(state) if state == "MERGED" => new.push(url),
                Ok(_) => {}
                Err(err) => tracing::info!(target: "sb::tasks", %url, %err, "the PR state did not load"),
            }
        }
        if new.is_empty() {
            return;
        }
        tracing::info!(target: "sb::tasks", n = new.len(), "PRs merged");
        self.merged.lock().unwrap().extend(new);
        self.emit_tasks();
    }

    /// One `sb task` request from the socket. The reply is `{ok, error}` plus the data.
    /// The history names the pane only when it is a pane on the board; else `cli`. So
    /// `SB_PANE=you` does not look like a change from the window.
    pub(crate) fn task_socket(&self, pane: Option<&str>, op: TaskOp) -> Value {
        let pane = pane.filter(|p| self.panes.lock().unwrap().iter().any(|x| x.row.id == *p));
        let by = pane.unwrap_or("cli").to_owned();
        let writes = op.writes();
        let out = self.store.lock().unwrap().task_request(op, pane, &by, now());
        match out {
            Ok(mut v) => {
                if writes {
                    self.emit_tasks();
                }
                v["ok"] = json!(true);
                v
            }
            Err(err) => json!({ "ok": false, "error": err }),
        }
    }
}
