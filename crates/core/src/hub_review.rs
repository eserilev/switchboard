//! Reviews in the hub (SPEC 11).

use crate::diff::{self, Row};
use crate::hub::Hub;
use crate::now;
use crate::repos::{self, git};
use crate::review::{self, PrInfo, Run, Stream};
use crate::store::{DraftRow, ReviewRow, ThreadRow};
use crate::{connections, expand};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub(crate) struct ReviewState {
    pub row: ReviewRow,
    /// `fetching`, `writing`, `ready`, `updating` or `error`.
    pub status: String,
    pub error: Option<String>,
    pub new_head: Option<String>,
    pub last_poll: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct ReviewView {
    pub id: String,
    pub url: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub head: String,
    pub base: String,
    pub tree: String,
    pub status: String,
    pub error: Option<String>,
    pub new_head: Option<String>,
    pub guide: Option<Value>,
    pub steps: Vec<StepState>,
    pub threads: Vec<ThreadView>,
    pub pins: Vec<crate::store::PinRow>,
    pub drafts: Vec<DraftRow>,
}

#[derive(Serialize, Clone, Debug)]
pub struct StepState {
    pub id: String,
    pub checked: bool,
    pub stale: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct ThreadView {
    pub id: String,
    pub step: String,
    pub path: Option<String>,
    pub side: Option<String>,
    pub line: Option<u32>,
    pub removed: bool,
    pub busy: bool,
    pub messages: Vec<MessageView>,
}

#[derive(Serialize, Clone, Debug)]
pub struct MessageView {
    pub id: i64,
    pub me: bool,
    pub text: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Anchor {
    pub path: String,
    pub side: String,
    pub line: u32,
    #[serde(default)]
    pub text: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct DiffView {
    pub path: Option<String>,
    pub rows: Vec<Row>,
    /// True when the PR does not change the file and the rows are plain context.
    pub context: bool,
}

type Res<T> = Result<T, String>;

fn e(x: impl std::fmt::Display) -> String {
    x.to_string()
}

impl Hub {
    pub(crate) fn load_reviews(&self) {
        let rows = self
            .store
            .lock()
            .unwrap()
            .open_reviews()
            .unwrap_or_default();
        let mut reviews = self.reviews.lock().unwrap();
        for row in rows {
            let status = if row.guide.is_some() {
                "ready"
            } else {
                "error"
            };
            let error = row
                .guide
                .is_none()
                .then(|| "The guide was not finished. Use Retry.".to_owned());
            reviews.insert(
                row.id.clone(),
                ReviewState {
                    row,
                    status: status.into(),
                    error,
                    new_head: None,
                    last_poll: 0,
                },
            );
        }
    }

    pub fn reviews(&self) -> Vec<ReviewView> {
        let ids: Vec<String> = self.reviews.lock().unwrap().keys().cloned().collect();
        let mut v: Vec<ReviewView> = ids
            .iter()
            .filter_map(|id| self.review_view(id).ok())
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    pub fn review_view(&self, id: &str) -> Res<ReviewView> {
        let reviews = self.reviews.lock().unwrap();
        let r = reviews.get(id).ok_or(format!("no review {id}"))?;
        let store = self.store.lock().unwrap();
        let steps = store
            .steps(id)
            .map_err(e)?
            .into_iter()
            .map(|(id, checked, stale)| StepState { id, checked, stale })
            .collect();
        let messages = store.messages(id).map_err(e)?;
        let busy = self.busy_threads();
        let threads = store
            .threads(id)
            .map_err(e)?
            .into_iter()
            .map(|t| ThreadView {
                messages: messages
                    .iter()
                    .filter(|m| m.thread == t.id)
                    .map(|m| MessageView {
                        id: m.id,
                        me: m.me,
                        text: m.text.clone(),
                    })
                    .collect(),
                busy: busy.contains(&t.id),
                id: t.id,
                step: t.step,
                path: t.path,
                side: t.side,
                line: t.line,
                removed: t.removed,
            })
            .collect();
        Ok(ReviewView {
            id: r.row.id.clone(),
            url: r.row.url.clone(),
            repo: r.row.repo.clone(),
            number: r.row.number,
            title: r.row.title.clone(),
            head: r.row.head.clone(),
            base: r.row.base.clone(),
            tree: r.row.tree.clone(),
            status: r.status.clone(),
            error: r.error.clone(),
            new_head: r.new_head.clone(),
            guide: r
                .row
                .guide
                .as_deref()
                .and_then(|g| serde_json::from_str(g).ok()),
            steps,
            threads,
            pins: store.pins(id).map_err(e)?,
            drafts: store.drafts(id).map_err(e)?,
        })
    }

    fn busy_threads(&self) -> Vec<String> {
        BUSY.lock().unwrap().clone()
    }

    fn emit_review(&self, id: &str) {
        if let Ok(v) = self.review_view(id) {
            self.sink.emit("review", json!(v));
        }
    }

    fn set_status(&self, id: &str, status: &str, error: Option<String>) {
        if let Some(r) = self.reviews.lock().unwrap().get_mut(id) {
            r.status = status.into();
            r.error = error;
        }
        self.emit_review(id);
    }

    fn save_row(&self, id: &str) {
        let row = self.reviews.lock().unwrap().get(id).map(|r| r.row.clone());
        if let Some(row) = row {
            let _ = self.store.lock().unwrap().save_review(&row, now());
        }
    }

    /// Opens a review from a PR URL. The work runs in the background; `review` events report it.
    pub fn review_open(&self, url: &str) -> Res<String> {
        let pr = review::parse_url(url).ok_or("not a GitHub PR URL")?;
        let existing = self
            .reviews
            .lock()
            .unwrap()
            .values()
            .find(|r| r.row.repo == pr.repo && r.row.number == pr.number)
            .map(|r| r.row.id.clone());
        if let Some(id) = existing {
            return Ok(id);
        }
        let id = self.store.lock().unwrap().next_id("r").map_err(e)?;
        let row = ReviewRow {
            id: id.clone(),
            url: url.trim().into(),
            repo: pr.repo.clone(),
            number: pr.number,
            title: format!("#{}", pr.number),
            head: String::new(),
            base: String::new(),
            tree: String::new(),
            connection: Some(self.active_connection()),
            guide_session: None,
            guide: None,
        };
        self.reviews.lock().unwrap().insert(
            id.clone(),
            ReviewState {
                row,
                status: "fetching".into(),
                error: None,
                new_head: None,
                last_poll: now(),
            },
        );
        self.emit_review(&id);
        let hub = self.arc();
        let rid = id.clone();
        std::thread::spawn(move || {
            if let Err(err) = hub.review_prepare(&rid) {
                hub.set_status(&rid, "error", Some(err));
            }
        });
        Ok(id)
    }

    fn review_prepare(&self, id: &str) -> Res<()> {
        let (url, repo) = {
            let r = self.reviews.lock().unwrap();
            let r = r.get(id).ok_or("gone")?;
            (r.row.url.clone(), r.row.repo.clone())
        };
        let pr = review::gh_view(&url)?;
        let (dir, remote) = self.find_clone(&repo)?;
        let f = review::fetch(&dir, &remote, &pr)?;
        if let Some(r) = self.reviews.lock().unwrap().get_mut(id) {
            r.row.title = pr.title.clone();
            r.row.head = f.head.clone();
            r.row.base = f.base.clone();
            r.row.tree = f.tree.to_string_lossy().into_owned();
        }
        self.save_row(id);
        self.run_guide(id, &pr, None)
    }

    /// The local clone of `owner/name`, and the remote that points at it.
    fn find_clone(&self, repo: &str) -> Res<(PathBuf, String)> {
        for r in self.repos() {
            let p = PathBuf::from(&r.path);
            if let Some(remote) = review::find_remote(&p, repo) {
                return Ok((p, remote));
            }
        }
        Err(format!(
            "no local clone has a remote for {repo}. Clone it under a scanned folder."
        ))
    }

    /// Runs the guide session: a new one, or a resume with an update prompt.
    fn run_guide(&self, id: &str, pr: &PrInfo, update: Option<String>) -> Res<()> {
        let (tree, head, session, connection) = {
            let r = self.reviews.lock().unwrap();
            let r = r.get(id).ok_or("gone")?;
            (
                PathBuf::from(&r.row.tree),
                r.row.head.clone(),
                r.row.guide_session.clone(),
                r.row
                    .connection
                    .clone()
                    .unwrap_or_else(|| self.active_connection()),
            )
        };
        self.set_status(
            id,
            if update.is_some() {
                "updating"
            } else {
                "writing"
            },
            None,
        );
        let config = self.config();
        let template = match &config.review.prompt {
            Some(p) => std::fs::read_to_string(expand(p)).map_err(|err| format!("{p}: {err}"))?,
            None => review::DEFAULT_PROMPT.to_owned(),
        };
        let specs: Vec<String> = config
            .review
            .spec_clones
            .iter()
            .map(|s| expand(s).to_string_lossy().into_owned())
            .collect();
        let prompt = update
            .clone()
            .unwrap_or_else(|| review::prompt(&template, pr, &head, &specs));
        let env = connections::env(&config, &connection)?;
        let sock = self.paths.sock();
        let resume = update.as_ref().and(session.as_deref());
        let run = Run {
            cwd: &tree,
            prompt: &prompt,
            resume,
            fork: false,
            env: &env,
            mcp: Some((&self.sb, id, &sock)),
        };
        let result = review::run(&run, |ev| {
            if let Stream::Init { session } = ev {
                if let Some(r) = self.reviews.lock().unwrap().get_mut(id) {
                    r.row.guide_session = Some(session.clone());
                }
                self.save_row(id);
            }
        })?;
        let has_guide = self
            .reviews
            .lock()
            .unwrap()
            .get(id)
            .map(|r| r.row.guide.is_some())
            .unwrap_or(false);
        match result {
            Stream::Result { error: false, .. } if has_guide => self.set_status(id, "ready", None),
            Stream::Result { text, .. } => self.set_status(
                id,
                "error",
                Some(format!(
                    "The agent ended with no guide. {}",
                    crate::proto::first_line(&text, 300)
                )),
            ),
            _ => {}
        }
        Ok(())
    }

    pub fn review_retry(&self, id: &str) -> Res<()> {
        let running = self
            .reviews
            .lock()
            .unwrap()
            .get(id)
            .map(|r| matches!(r.status.as_str(), "fetching" | "writing" | "updating"))
            .unwrap_or(false);
        if running {
            return Err("the review is still running".into());
        }
        self.set_status(id, "fetching", None);
        let hub = self.arc();
        let rid = id.to_owned();
        std::thread::spawn(move || {
            if let Err(err) = hub.review_prepare(&rid) {
                hub.set_status(&rid, "error", Some(err));
            }
        });
        Ok(())
    }

    // ---------- the MCP tools ----------

    pub(crate) fn guide_set(&self, id: &str, guide: Value) -> Res<()> {
        let (tree, head, base) = self.review_refs(id)?;
        review::validate(&guide, &tree, &head, &base)?;
        if let Some(r) = self.reviews.lock().unwrap().get_mut(id) {
            r.row.guide = Some(guide.to_string());
        }
        self.save_row(id);
        self.emit_review(id);
        Ok(())
    }

    pub(crate) fn guide_update(&self, id: &str, step: &str, fields: Value) -> Res<()> {
        let (tree, head, base) = self.review_refs(id)?;
        let mut guide: Value = {
            let r = self.reviews.lock().unwrap();
            let g = r
                .get(id)
                .and_then(|r| r.row.guide.clone())
                .ok_or("no guide yet: call guide_set_steps first")?;
            serde_json::from_str(&g).map_err(e)?
        };
        let steps = guide
            .get_mut("steps")
            .and_then(Value::as_array_mut)
            .ok_or("the guide has no steps")?;
        let s = steps
            .iter_mut()
            .find(|s| s.get("id").and_then(Value::as_str) == Some(step))
            .ok_or(format!("no step {step}"))?;
        let obj = fields.as_object().ok_or("fields must be an object")?;
        for (k, v) in obj {
            if k != "id" {
                s[k] = v.clone();
            }
        }
        review::validate_step(s, step, &tree, &head, &base)?;
        let _ = self
            .store
            .lock()
            .unwrap()
            .set_step(id, step, None, Some(false));
        if let Some(r) = self.reviews.lock().unwrap().get_mut(id) {
            r.row.guide = Some(guide.to_string());
        }
        self.save_row(id);
        self.emit_review(id);
        Ok(())
    }

    fn review_refs(&self, id: &str) -> Res<(PathBuf, String, String)> {
        let r = self.reviews.lock().unwrap();
        let r = r.get(id).ok_or(format!("no review {id}"))?;
        Ok((
            PathBuf::from(&r.row.tree),
            r.row.head.clone(),
            r.row.base.clone(),
        ))
    }

    fn step(&self, id: &str, step: &str) -> Res<Value> {
        let r = self.reviews.lock().unwrap();
        let g = r
            .get(id)
            .and_then(|r| r.row.guide.clone())
            .ok_or("no guide")?;
        let g: Value = serde_json::from_str(&g).map_err(e)?;
        g["steps"]
            .as_array()
            .and_then(|s| s.iter().find(|s| s["id"] == step).cloned())
            .ok_or(format!("no step {step}"))
    }

    // ---------- the diff column ----------

    pub fn review_diff(&self, id: &str, step: &str) -> Res<DiffView> {
        let s = self.step(id, step)?;
        let Some(path) = s.get("file").and_then(Value::as_str) else {
            return Ok(DiffView {
                path: None,
                rows: vec![],
                context: false,
            });
        };
        let (tree, head, base) = self.review_refs(id)?;
        let text = git(
            &tree,
            &["diff", "-U12", "--no-color", &base, &head, "--", path],
        )?;
        let rows = diff::parse(&text);
        if !rows.is_empty() {
            return Ok(DiffView {
                path: Some(path.into()),
                rows,
                context: false,
            });
        }
        let lines: Vec<u32> = s["lines"]
            .as_array()
            .map(|l| {
                l.iter()
                    .filter_map(|n| n.as_u64().map(|n| n as u32))
                    .collect()
            })
            .unwrap_or_default();
        let (from, to) = (
            lines.first().copied().unwrap_or(1),
            lines.last().copied().unwrap_or(1),
        );
        let content = git(&tree, &["show", &format!("{head}:{path}")])?;
        Ok(DiffView {
            path: Some(path.into()),
            rows: diff::context(&content, from, to, 12),
            context: true,
        })
    }

    pub fn review_mark(&self, id: &str, step: &str, checked: bool) -> Res<()> {
        self.store
            .lock()
            .unwrap()
            .set_step(id, step, Some(checked), None)
            .map_err(e)?;
        self.emit_review(id);
        Ok(())
    }

    // ---------- threads (SPEC 11.5) ----------

    /// Asks a question. A new thread forks the guide session; a follow-up resumes the thread.
    pub fn review_ask(
        &self,
        id: &str,
        step: &str,
        anchor: Option<Anchor>,
        thread: Option<String>,
        text: &str,
    ) -> Res<String> {
        let guide_session = self
            .reviews
            .lock()
            .unwrap()
            .get(id)
            .and_then(|r| r.row.guide_session.clone())
            .ok_or("the guide session has not started")?;
        let store = self.store.lock().unwrap();
        let t = match thread {
            Some(tid) => store
                .threads(id)
                .map_err(e)?
                .into_iter()
                .find(|t| t.id == tid)
                .ok_or(format!("no thread {tid}"))?,
            None => {
                let t = ThreadRow {
                    id: store.next_id("t").map_err(e)?,
                    review: id.into(),
                    step: step.into(),
                    path: anchor.as_ref().map(|a| a.path.clone()),
                    side: anchor.as_ref().map(|a| a.side.clone()),
                    line: anchor.as_ref().map(|a| a.line),
                    line_text: anchor.as_ref().map(|a| a.text.clone()),
                    fork_session: None,
                    removed: false,
                };
                store.save_thread(&t).map_err(e)?;
                t
            }
        };
        if BUSY.lock().unwrap().contains(&t.id) {
            return Err("this thread is still answering; wait for it".into());
        }
        store.add_message(&t.id, true, text, now()).map_err(e)?;
        let pins: Vec<String> = store
            .pins(id)
            .map_err(e)?
            .into_iter()
            .filter(|p| p.step == t.step)
            .map(|p| p.text)
            .collect();
        drop(store);

        let prompt = match &t.fork_session {
            // A plain follow-up can start with "-", and claude would read it as a flag.
            Some(_) => format!("Follow-up question: {text}"),
            None => thread_prompt(&self.step(id, &t.step)?, &t, &pins, text),
        };
        let (resume, fork) = match &t.fork_session {
            Some(f) => (f.clone(), false),
            None => (guide_session, true),
        };
        BUSY.lock().unwrap().push(t.id.clone());
        self.emit_review(id);
        let hub = self.arc();
        let (rid, tid) = (id.to_owned(), t.id.clone());
        std::thread::spawn(move || {
            let res = hub.run_thread(&rid, &tid, &prompt, &resume, fork);
            BUSY.lock().unwrap().retain(|b| b != &tid);
            if let Err(err) = res {
                let _ = hub.store.lock().unwrap().add_message(
                    &tid,
                    false,
                    &format!("Error: {err}"),
                    now(),
                );
            }
            hub.emit_review(&rid);
        });
        Ok(t.id)
    }

    fn run_thread(
        &self,
        id: &str,
        thread: &str,
        prompt: &str,
        resume: &str,
        fork: bool,
    ) -> Res<()> {
        let (tree, connection) = {
            let r = self.reviews.lock().unwrap();
            let r = r.get(id).ok_or("gone")?;
            (PathBuf::from(&r.row.tree), r.row.connection.clone())
        };
        // Threads use the active connection (D4).
        let conn = self.active_connection();
        let _ = connection;
        let env = connections::env(&self.config(), &conn)?;
        let sock = self.paths.sock();
        let run = Run {
            cwd: &tree,
            prompt,
            resume: Some(resume),
            fork,
            env: &env,
            mcp: Some((&self.sb, id, &sock)),
        };
        let result = review::run(&run, |ev| match ev {
            Stream::Init { session } if fork => {
                let store = self.store.lock().unwrap();
                if let Ok(Some(mut t)) = store
                    .threads(id)
                    .map(|ts| ts.into_iter().find(|t| t.id == thread))
                {
                    t.fork_session = Some(session.clone());
                    let _ = store.save_thread(&t);
                }
            }
            Stream::Delta(d) => self.sink.emit(
                "review_stream",
                json!({ "review": id, "thread": thread, "delta": d }),
            ),
            _ => {}
        })?;
        if let Stream::Result { text, error, .. } = result {
            let text = if error {
                format!("Error: {text}")
            } else {
                text
            };
            self.store
                .lock()
                .unwrap()
                .add_message(thread, false, &text, now())
                .map_err(e)?;
        }
        Ok(())
    }

    /// Asks the thread's fork for a one-line summary, without changing the fork.
    fn summarize(&self, id: &str, thread: &str, ask: &str) -> Res<(ThreadRow, String)> {
        let t = self
            .store
            .lock()
            .unwrap()
            .threads(id)
            .map_err(e)?
            .into_iter()
            .find(|t| t.id == thread)
            .ok_or(format!("no thread {thread}"))?;
        let fork = t
            .fork_session
            .clone()
            .ok_or("the thread has no answer yet")?;
        let tree = PathBuf::from(
            self.reviews
                .lock()
                .unwrap()
                .get(id)
                .map(|r| r.row.tree.clone())
                .ok_or("gone")?,
        );
        let env = connections::env(&self.config(), &self.active_connection())?;
        let run = Run {
            cwd: &tree,
            prompt: ask,
            resume: Some(&fork),
            fork: true,
            env: &env,
            mcp: None,
        };
        match review::run(&run, |_| {})? {
            Stream::Result {
                text, error: false, ..
            } => Ok((t, crate::proto::first_line(&text, 300))),
            Stream::Result { text, .. } => Err(text),
            _ => Err("no result".into()),
        }
    }

    pub fn review_pin(&self, id: &str, thread: &str) -> Res<()> {
        let (t, line) = self.summarize(id, thread, "Summarize your last answer as one line of at most 20 words, in simple English. Reply with the line only.")?;
        self.store
            .lock()
            .unwrap()
            .add_pin(id, &t.step, &line)
            .map_err(e)?;
        self.emit_review(id);
        Ok(())
    }

    pub fn review_draft(&self, id: &str, thread: &str) -> Res<()> {
        let (t, line) = self.summarize(
            id,
            thread,
            "Write your last answer as one short PR review comment: one or two sentences, simple English, no greeting. Reply with the comment only.",
        )?;
        let d = DraftRow {
            id: 0,
            path: t.path.clone(),
            side: t.side.clone(),
            line: t.line,
            text: line,
        };
        self.store.lock().unwrap().add_draft(id, &d).map_err(e)?;
        self.emit_review(id);
        Ok(())
    }

    pub fn review_pin_edit(&self, id: &str, pin: i64, text: Option<String>) -> Res<()> {
        {
            let store = self.store.lock().unwrap();
            match text.filter(|t| !t.trim().is_empty()) {
                Some(t) => store.edit_pin(pin, &t),
                None => store.delete_pin(pin),
            }
            .map_err(e)?;
        }
        self.emit_review(id);
        Ok(())
    }

    pub fn review_draft_edit(&self, id: &str, draft: i64, text: Option<String>) -> Res<()> {
        {
            let store = self.store.lock().unwrap();
            match text.filter(|t| !t.trim().is_empty()) {
                Some(t) => store.edit_draft(draft, &t),
                None => store.delete_draft(draft),
            }
            .map_err(e)?;
        }
        self.emit_review(id);
        Ok(())
    }

    /// All drafts as `path:line: text`, one for each line.
    pub fn review_drafts_text(&self, id: &str) -> Res<String> {
        let drafts = self.store.lock().unwrap().drafts(id).map_err(e)?;
        Ok(drafts
            .iter()
            .map(|d| match (&d.path, d.line) {
                (Some(p), Some(l)) => format!("{p}:{l}: {}", d.text),
                (Some(p), None) => format!("{p}: {}", d.text),
                _ => d.text.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    pub fn review_close(&self, id: &str) -> Res<()> {
        self.reviews.lock().unwrap().remove(id);
        self.store
            .lock()
            .unwrap()
            .close_review(id, now())
            .map_err(e)?;
        self.sink.emit("review_closed", json!(id));
        Ok(())
    }

    // ---------- new commits (SPEC 11.7) ----------

    pub(crate) fn review_tick(&self, n: u64) {
        let every = self.config().poll_head_minutes.max(1) * 60;
        if !n.is_multiple_of(30) {
            return;
        }
        let due: Vec<(String, String, String)> = self
            .reviews
            .lock()
            .unwrap()
            .values()
            .filter(|r| r.status == "ready" && now().saturating_sub(r.last_poll) >= every)
            .map(|r| (r.row.id.clone(), r.row.url.clone(), r.row.head.clone()))
            .collect();
        for (id, url, head) in due {
            if let Some(r) = self.reviews.lock().unwrap().get_mut(&id) {
                r.last_poll = now();
            }
            let hub = self.arc();
            std::thread::spawn(move || {
                if let Ok(pr) = review::gh_view(&url) {
                    if pr.head_ref_oid != head && !head.is_empty() {
                        if let Some(r) = hub.reviews.lock().unwrap().get_mut(&id) {
                            r.new_head = Some(pr.head_ref_oid.clone());
                        }
                        hub.emit_review(&id);
                    }
                }
            });
        }
    }

    /// Moves the review to the new head: stale steps, re-anchored threads, a guide update.
    pub fn review_update(&self, id: &str) -> Res<()> {
        let running = self
            .reviews
            .lock()
            .unwrap()
            .get(id)
            .map(|r| matches!(r.status.as_str(), "fetching" | "writing" | "updating"))
            .unwrap_or(false);
        if running {
            return Err("the review is still running".into());
        }
        let hub = self.arc();
        let rid = id.to_owned();
        std::thread::spawn(move || {
            if let Err(err) = hub.review_update_now(&rid) {
                hub.set_status(&rid, "error", Some(err));
            }
        });
        Ok(())
    }

    fn review_update_now(&self, id: &str) -> Res<()> {
        let (url, repo, old) = {
            let r = self.reviews.lock().unwrap();
            let r = r.get(id).ok_or("gone")?;
            (r.row.url.clone(), r.row.repo.clone(), r.row.head.clone())
        };
        self.set_status(id, "updating", None);
        let pr = review::gh_view(&url)?;
        let (dir, remote) = self.find_clone(&repo)?;
        let f = review::fetch(&dir, &remote, &pr)?;
        let changed = review::changed_files(&f.tree, &old, &f.head).unwrap_or_default();
        if let Some(r) = self.reviews.lock().unwrap().get_mut(id) {
            r.row.head = f.head.clone();
            r.row.base = f.base.clone();
            r.new_head = None;
        }
        self.save_row(id);
        let guide: Option<Value> = self.reviews.lock().unwrap().get(id).and_then(|r| {
            r.row
                .guide
                .as_deref()
                .and_then(|g| serde_json::from_str(g).ok())
        });
        let mut stale = vec![];
        if let Some(g) = &guide {
            for s in g["steps"].as_array().into_iter().flatten() {
                if let (Some(sid), Some(file)) = (s["id"].as_str(), s["file"].as_str()) {
                    if changed.iter().any(|c| c == file) {
                        stale.push(sid.to_owned());
                        let _ = self
                            .store
                            .lock()
                            .unwrap()
                            .set_step(id, sid, None, Some(true));
                    }
                }
            }
        }
        self.reanchor(id, &f.tree, &f.head)?;
        let prompt = format!(
            "The PR has new commits. The head is now {}. The worktree is checked out at it. Changed files since {old}: {}. \
             Read the new code. Call guide_update_step for each step whose code changed: fix its lines, what and check. \
             Do not call guide_set_steps. Reply with one line when done.",
            f.head,
            if changed.is_empty() { "(none)".to_owned() } else { changed.join(", ") }
        );
        self.run_guide(id, &pr, Some(prompt))?;
        let _ = stale;
        Ok(())
    }

    /// Moves line anchors to the line with the same text at the new head.
    fn reanchor(&self, id: &str, tree: &Path, head: &str) -> Res<()> {
        let store = self.store.lock().unwrap();
        for mut t in store.threads(id).map_err(e)? {
            let (Some(path), Some(line), Some(text)) =
                (t.path.clone(), t.line, t.line_text.clone())
            else {
                continue;
            };
            if t.side.as_deref() == Some("old") {
                continue;
            }
            let content = git(tree, &["show", &format!("{head}:{path}")]).unwrap_or_default();
            let lines: Vec<&str> = content.lines().collect();
            match diff::find_near(&lines, &text, line) {
                Some(n) => {
                    t.line = Some(n);
                    t.removed = false;
                }
                None => t.removed = true,
            }
            store.save_thread(&t).map_err(e)?;
        }
        Ok(())
    }

    /// Opens nvim in the review worktree at a file and line.
    pub fn review_nvim(&self, id: &str, path: &str, line: u32) -> Res<String> {
        let tree = PathBuf::from(
            self.reviews
                .lock()
                .unwrap()
                .get(id)
                .map(|r| r.row.tree.clone())
                .ok_or("gone")?,
        );
        let file = tree.join(path);
        self.nvim_open(&tree, Some((&file.to_string_lossy(), line.max(1))))
    }

    pub fn review_repo_of(&self, id: &str) -> Option<PathBuf> {
        let tree = self
            .reviews
            .lock()
            .unwrap()
            .get(id)
            .map(|r| r.row.tree.clone())?;
        repos::main_checkout(Path::new(&tree))
    }
}

/// Threads that wait for an answer.
static BUSY: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn thread_prompt(step: &Value, t: &ThreadRow, pins: &[String], question: &str) -> String {
    let mut p = format!(
        "The reviewer has a question about step {} \"{}\"",
        step["id"].as_str().unwrap_or("?"),
        step["title"].as_str().unwrap_or("")
    );
    if let Some(f) = step["file"].as_str() {
        p += &format!(" ({f} {})", step["lines"]);
    }
    p += ".\n";
    if let (Some(path), Some(line)) = (&t.path, t.line) {
        p += &format!(
            "The question is about {path}:{line} ({} side): `{}`\n",
            t.side.as_deref().unwrap_or("new"),
            t.line_text.as_deref().unwrap_or("").trim()
        );
    }
    if !pins.is_empty() {
        p += "Notes the reviewer pinned for this step:\n";
        for pin in pins {
            p += &format!("- {pin}\n");
        }
    }
    p += "\nRules: you only read. Do not call the guide tools. Answer short, in simple English. Name file:line for each claim.\n\nQuestion: ";
    p += question;
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_prompt_has_the_anchor_and_pins() {
        let step = json!({"id":"s3","title":"Proposal path","file":"gloas.rs","lines":[1374,1384]});
        let t = ThreadRow {
            id: "t1".into(),
            review: "r1".into(),
            step: "s3".into(),
            path: Some("gloas.rs".into()),
            side: Some("new".into()),
            line: Some(1379),
            line_text: Some("  builder_params.slot - 1,".into()),
            fork_session: None,
            removed: false,
        };
        let p = thread_prompt(&step, &t, &["Use safe_sub.".into()], "saturating or safe?");
        assert!(p.contains("gloas.rs:1379"));
        assert!(p.contains("`builder_params.slot - 1,`"));
        assert!(p.contains("- Use safe_sub."));
        assert!(p.ends_with("saturating or safe?"));
    }
}
