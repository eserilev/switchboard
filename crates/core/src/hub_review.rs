//! Reviews in the hub (SPEC 11).

use crate::diff::Row;
use crate::hub::Hub;
use crate::now;
use crate::repos;
use crate::review::{self, PrInfo, Run, Stream};
use crate::store::{DraftRow, ReviewRow, ThreadRow};
use crate::{connections, coverage, expand};
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
    /// The PR diff that the verified checker works on. Built after the fetch.
    pub model: Option<std::sync::Arc<coverage::DiffModel>>,
    /// Rejected guides in a row, and the last one, for the "Not in the guide" step.
    pub attempts: u32,
    pub proposal: Option<Value>,
    pub coverage: Option<CoverageView>,
    /// The pid of the guide's `claude -p` run while it runs. A close stops it.
    pub run_pid: Option<u32>,
}

impl ReviewState {
    fn new(row: ReviewRow, status: &str, error: Option<String>) -> ReviewState {
        ReviewState {
            row,
            status: status.into(),
            error,
            new_head: None,
            last_poll: now(),
            model: None,
            attempts: 0,
            proposal: None,
            coverage: None,
            run_pid: None,
        }
    }
}

/// The error of a background job whose review closed while it ran.
const CLOSED: &str = "the review was closed";

/// What a close did, for the window.
#[derive(Serialize, Clone, Debug, Default)]
pub struct CloseResult {
    /// False when the worktree has changes and you have not chosen yet.
    pub closed: bool,
    /// The changes that a removal would lose.
    pub changes: Vec<String>,
}

/// What a closed round of a PR offers to a new review of it.
#[derive(Serialize, Clone, Debug)]
pub struct Prior {
    pub id: String,
    pub round: u32,
    pub head: String,
    /// The reviews you sent in that round.
    pub sent: usize,
}

/// Rejected guides before the app completes the guide itself.
const MAX_ATTEMPTS: u32 = 3;

/// What the window shows about coverage.
#[derive(Serialize, Clone, Debug, Default)]
pub struct CoverageView {
    pub files: usize,
    pub changed_lines: usize,
    /// True when the verified checker accepted the guide.
    pub accepted: bool,
    /// Changed lines that the app put into "Not in the guide".
    pub missed_lines: usize,
    pub binary: Vec<String>,
    /// Differences between git's and GitHub's counts.
    pub github: Vec<String>,
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
    /// 1 for a full review; later rounds cover only the changes since `since`.
    pub round: u32,
    pub since: Option<String>,
    pub scope_note: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub new_head: Option<String>,
    pub guide: Option<Value>,
    pub steps: Vec<StepState>,
    pub threads: Vec<ThreadView>,
    pub pins: Vec<crate::store::PinRow>,
    pub drafts: Vec<DraftRow>,
    /// The review summary you write before you send the review.
    pub summary: String,
    /// The reviews sent to GitHub, oldest first.
    pub posted: Vec<crate::store::PostedRow>,
    pub coverage: Option<CoverageView>,
    /// `pr` or `local` (SPEC 11.11).
    pub kind: String,
    /// For `local`: the agent pane.
    pub pane: Option<String>,
    /// The take notes you dismissed, by their key (`take_key` in review.js).
    pub dismissed: Vec<String>,
}

/// What the window shows before a send: the GitHub account, the head, and where
/// each draft goes. `token` names this exact plan.
#[derive(Serialize, Clone, Debug)]
pub struct PostPreview {
    pub login: String,
    pub plan: crate::post::Plan,
    pub token: String,
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
    /// The head of the diff on screen when you clicked the line.
    #[serde(default)]
    pub head: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct DiffView {
    /// The head of the model that drew these rows. The window refetches a view whose
    /// head is not the review's head, and sends it back with each new anchor.
    pub head: String,
    pub sections: Vec<DiffSection>,
}

/// Every row of one file, with the head of the model that drew them.
#[derive(Serialize, Clone, Debug)]
pub struct FileRows {
    pub head: String,
    pub rows: Vec<Row>,
}

/// The diff of one file in a step, with the step's ranges in it.
#[derive(Serialize, Clone, Debug)]
pub struct DiffSection {
    pub path: String,
    pub old_path: Option<String>,
    pub rows: Vec<Row>,
    /// True when the PR does not change the file and the rows are plain context.
    pub context: bool,
    pub ranges: Vec<coverage::Range>,
    /// Why a file has no lines to show: binary, submodule, rename, mode change.
    pub note: Option<String>,
    /// In a step: the changed lines of the file that the step does not show.
    pub other: Option<OtherChanges>,
}

/// Changed lines of a file outside the current step, and the steps that cover them.
#[derive(Serialize, Clone, Debug)]
pub struct OtherChanges {
    pub lines: usize,
    pub steps: Vec<String>,
}

/// The answer when a click comes from the diff of an older head.
const STALE_VIEW: &str = "The diff on screen is from an older commit. It reloads now; click the line again.";

/// Rows of context around a step's ranges. The ranges already hold some context.
const STEP_CONTEXT: usize = 6;

/// A comment that you write on a line or a range of lines (`review_comment`).
#[derive(Deserialize, Debug, Clone)]
pub struct NewComment {
    pub path: String,
    /// `old` or `new`.
    pub side: String,
    /// The last line. With `start_line`, the comment covers a range.
    pub line: u32,
    #[serde(default)]
    pub start_line: Option<u32>,
    pub text: String,
    /// The head of the diff on screen, and the text of `line` there.
    pub head: String,
    pub line_text: String,
    /// The text came from Claude's take: the draft shows as an agent draft.
    #[serde(default)]
    pub agent: bool,
}

/// One changed file in the Files view.
#[derive(Serialize, Clone, Debug)]
pub struct FileEntry {
    pub path: String,
    pub old_path: Option<String>,
    pub added: usize,
    pub removed: usize,
    pub note: Option<String>,
    /// The ids of the guide steps that cover the file.
    pub steps: Vec<String>,
}

/// The worktree of a review round. Each round has its own, so rounds do not share a checkout.
fn worktree_name(number: u64, round: u32) -> String {
    if round > 1 {
        format!("pr{number}-r{round}")
    } else {
        format!("pr{number}")
    }
}

/// The base of the diff of a round and a note. A first round uses the merge base.
/// A later round starts at the head that the round before reviewed (`review::round_base`).
fn scope(f: &review::Fetched, round: u32, since: Option<&str>) -> Res<(String, Option<String>)> {
    match since.filter(|_| round > 1) {
        Some(s) if s == f.head => Err("The PR has no new commits since the round before.".into()),
        Some(s) => review::round_base(&f.tree, s, &f.base, &f.head),
        None => Ok((f.base.clone(), None)),
    }
}

/// Why a file has no lines to show, or `None`.
fn note(f: &coverage::ChangedFile) -> Option<String> {
    if f.binary {
        Some("binary file".to_owned())
    } else if f.submodule {
        Some("submodule".to_owned())
    } else if f.needs_name() {
        Some(match (&f.old_path, &f.new_path) {
            (Some(o), Some(n)) if o != n => format!("renamed from {o}, no line changed"),
            _ => "mode change, no line changed".to_owned(),
        })
    } else {
        None
    }
}

/// The diff section of one file, with no step ranges.
fn section(f: &coverage::ChangedFile) -> DiffSection {
    DiffSection {
        path: f.path().to_owned(),
        old_path: f.old_path.clone(),
        rows: coverage::rows(f, 12),
        context: false,
        ranges: vec![],
        note: note(f),
        other: None,
    }
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
            // A local review has no guide: it is ready once it has its commits.
            let done = if row.is_local() { !row.head.is_empty() } else { row.guide.is_some() };
            let status = if done { "ready" } else { "error" };
            let error = (!done).then(|| {
                if row.is_local() { "The commits did not load. Use Retry." } else { "The guide was not finished. Use Retry." }.to_owned()
            });
            let mut st = ReviewState::new(row, status, error);
            st.last_poll = 0;
            reviews.insert(st.row.id.clone(), st);
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
            round: r.row.round,
            since: r.row.since.clone(),
            scope_note: r.row.scope_note.clone(),
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
            summary: store.summary(id).map_err(e)?,
            posted: store.posted(id).map_err(e)?,
            coverage: r.coverage.clone(),
            kind: r.row.kind.clone(),
            pane: r.row.pane.clone(),
            dismissed: store.dismissed(id).map_err(e)?,
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
    /// An error for a local review: the command needs a PR, a guide or GitHub.
    fn pr_only(&self, id: &str) -> Res<()> {
        let local = self.reviews.lock().unwrap().get(id).map(|r| r.row.is_local()).unwrap_or(false);
        if local {
            return Err("This is a review of local commits: it has no PR and no guide.".into());
        }
        Ok(())
    }

    /// Opens a review of the unpushed commits of an agent pane's worktree (SPEC 11.11).
    /// An open local review of the same folder wins. The commits load in the background.
    pub fn review_open_local(&self, pane: &str) -> Res<String> {
        let (tree, repo) = self
            .panes
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.row.id == pane)
            .map(|p| (p.row.tree.clone(), p.row.repo.clone()))
            .ok_or(format!("no pane {pane}"))?;
        let id = self.store.lock().unwrap().next_id("r").map_err(e)?;
        // The check and the insert in one lock: a double click opens one review.
        let mut reviews = self.reviews.lock().unwrap();
        let existing = reviews
            .values_mut()
            .find(|r| r.row.is_local() && review::same_path(Path::new(&r.row.tree), Path::new(&tree)));
        if let Some(r) = existing {
            // The folder can have a new agent pane since the review opened.
            r.row.pane = Some(pane.to_owned());
            let id = r.row.id.clone();
            drop(reviews);
            self.save_row(&id);
            self.emit_review(&id);
            return Ok(id);
        }
        let row = ReviewRow {
            id: id.clone(),
            url: String::new(),
            repo,
            number: 0,
            title: String::new(),
            head: String::new(),
            base: String::new(),
            tree,
            connection: None,
            guide_session: None,
            guide: None,
            round: 1,
            since: None,
            scope_note: None,
            kind: crate::store::LOCAL.into(),
            pane: Some(pane.to_owned()),
        };
        reviews.insert(id.clone(), ReviewState::new(row, "fetching", None));
        drop(reviews);
        self.save_row(&id);
        self.local_refresh_bg(&id)?;
        Ok(id)
    }

    /// Reads the range again in the background: a new HEAD, or the first load. The
    /// check that no load runs and the switch to "fetching" are one lock, so two
    /// quick clicks start one load.
    fn local_refresh_bg(&self, id: &str) -> Res<()> {
        {
            let mut reviews = self.reviews.lock().unwrap();
            let r = reviews.get_mut(id).ok_or("gone")?;
            if r.status == "fetching" && !r.row.head.is_empty() {
                return Err("the commits are still loading".into());
            }
            r.status = "fetching".into();
            r.error = None;
        }
        self.emit_review(id);
        let hub = self.arc();
        let rid = id.to_owned();
        std::thread::spawn(move || {
            if let Err(err) = hub.local_refresh(&rid) {
                hub.set_status(&rid, "error", Some(err));
            }
        });
        Ok(())
    }

    /// The commits of a local review: the range rule, then the diff model. The
    /// drafts move to the new head as after a PR update (`load_model`).
    fn local_refresh(&self, id: &str) -> Res<()> {
        let tree = self.reviews.lock().unwrap().get(id).map(|r| r.row.tree.clone()).ok_or("gone")?;
        let range = review::local_range(Path::new(&tree))?;
        let branch = repos::git(Path::new(&tree), &["symbolic-ref", "--short", "-q", "HEAD"]).unwrap_or_default();
        let title = format!(
            "{} commit{} on {}",
            range.count,
            if range.count == 1 { "" } else { "s" },
            if branch.is_empty() { "a detached HEAD".to_owned() } else { branch }
        );
        match self.reviews.lock().unwrap().get_mut(id) {
            Some(r) => {
                r.row.head = range.head;
                r.row.base = range.base;
                r.row.scope_note = Some(range.note);
                r.row.title = title;
                r.new_head = None;
            }
            None => return Err(CLOSED.into()),
        }
        self.save_row(id);
        self.load_model(id, None)?;
        self.set_status(id, "ready", None);
        Ok(())
    }

    /// Puts the comments of a local review into its agent's prompt, as one paste with
    /// no Enter, and keeps them as sent. A stale draft stays: place it again first.
    pub fn review_send_agent(&self, id: &str, note: &str) -> Res<usize> {
        let row = self.reviews.lock().unwrap().get(id).map(|r| r.row.clone()).ok_or("gone")?;
        let pane = row.pane.clone().filter(|_| row.is_local()).ok_or("Send to agent is for a review of local commits.")?;
        let view = self
            .views()
            .into_iter()
            .find(|v| v.id == pane)
            .ok_or("The agent pane is closed. Use Copy all.")?;
        if view.permit.is_some() || view.trust || matches!(view.lamp, crate::lamp::Lamp::Needs | crate::lamp::Lamp::Ended) {
            return Err("The agent is asking you something, or it has ended. Answer it first.".into());
        }
        let drafts: Vec<DraftRow> = self.store.lock().unwrap().drafts(id).map_err(e)?.into_iter().filter(|d| !d.stale).collect();
        let note = note.trim();
        if drafts.is_empty() && note.is_empty() {
            return Err("No comments to send.".into());
        }
        let text = agent_text(&row.head, note, &drafts);
        let paste = format!("\x1b[200~{text}\x1b[201~");
        self.input(&pane, paste.as_bytes())?;
        let ids: Vec<i64> = drafts.iter().map(|d| d.id).collect();
        self.store
            .lock()
            .unwrap()
            .record_posted(
                id,
                &crate::store::PostedRow {
                    event: "AGENT".into(),
                    url: String::new(),
                    comments: drafts.len() as u32,
                    time: now(),
                    head: Some(row.head.clone()),
                    body: (!note.is_empty()).then(|| note.to_owned()),
                    lines: Some(text),
                },
                &ids,
            )
            .map_err(e)?;
        self.emit_review(id);
        Ok(drafts.len())
    }

    /// The last closed round of the PR at `url`, when no review of it is open.
    pub fn review_prior(&self, url: &str) -> Res<Option<Prior>> {
        let pr = review::parse_url(url).ok_or("not a GitHub PR URL")?;
        let open = self
            .reviews
            .lock()
            .unwrap()
            .values()
            .any(|r| r.row.repo == pr.repo && r.row.number == pr.number);
        if open {
            return Ok(None);
        }
        let store = self.store.lock().unwrap();
        let Some(row) = store.last_closed(&pr.repo, pr.number).map_err(e)? else {
            return Ok(None);
        };
        let sent = store.posted(&row.id).map_err(e)?.len();
        Ok(Some(Prior {
            id: row.id,
            round: row.round,
            head: row.head,
            sent,
        }))
    }

    /// Opens a review of the PR at `url`. An open review of it wins. With `round_of`,
    /// a closed round of the PR, it starts the next round from that round's head.
    pub fn review_open(&self, url: &str, round_of: Option<&str>) -> Res<String> {
        let pr = review::parse_url(url).ok_or("not a GitHub PR URL")?;
        let existing = self
            .reviews
            .lock()
            .unwrap()
            .values()
            .filter(|r| !r.row.is_local() && r.row.repo == pr.repo && r.row.number == pr.number)
            .max_by_key(|r| r.row.round)
            .map(|r| r.row.id.clone());
        if let Some(id) = existing {
            return Ok(id);
        }
        if let Some(prev) = round_of {
            let prev = self
                .store
                .lock()
                .unwrap()
                .review(prev)
                .map_err(e)?
                .ok_or(format!("no review {prev}"))?;
            if prev.repo != pr.repo || prev.number != pr.number {
                return Err(format!("review {} is not of this PR", prev.id));
            }
            return self.start_round(&prev);
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
            round: 1,
            since: None,
            scope_note: None,
            kind: "pr".into(),
            pane: None,
        };
        self.reviews
            .lock()
            .unwrap()
            .insert(id.clone(), ReviewState::new(row, "fetching", None));
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
        let t0 = std::time::Instant::now();
        let ms = || t0.elapsed().as_millis() as u64;
        let (url, repo, round, since) = {
            let r = self.reviews.lock().unwrap();
            let r = r.get(id).ok_or("gone")?;
            (r.row.url.clone(), r.row.repo.clone(), r.row.round, r.row.since.clone())
        };
        tracing::info!(target: "sb::review", review = id, %url, round, "review: gh pr view");
        let pr = review::gh_view(&url)?;
        tracing::info!(target: "sb::review", review = id, ms = ms(), files = pr.files.len(), head = %pr.head_ref_oid, "review: got the PR");
        let (dir, remote) = self.find_clone(&repo)?;
        tracing::info!(target: "sb::review", review = id, ms = ms(), clone = %dir.display(), %remote, "review: found the clone");
        let f = self.fetch(id, &dir, &remote, &pr, round)?;
        tracing::info!(target: "sb::review", review = id, ms = ms(), tree = %f.tree.display(), "review: worktree ready");
        if f.head != pr.head_ref_oid {
            return Err(format!(
                "the PR head moved during the fetch (GitHub {}, fetched {}). Use Retry.",
                pr.head_ref_oid, f.head
            ));
        }
        let (base, note) = scope(&f, round, since.as_deref())?;
        review::pin(&dir, id, &f.head, &base)?;
        if let Some(r) = self.reviews.lock().unwrap().get_mut(id) {
            r.row.title = pr.title.clone();
            r.row.head = f.head.clone();
            r.row.base = base;
            r.row.scope_note = note;
            r.row.tree = f.tree.to_string_lossy().into_owned();
            r.row.guide = None;
            r.attempts = 0;
            r.proposal = None;
        }
        self.save_row(id);
        // GitHub's counts are for the whole PR, so only a first round compares them.
        let model = self.load_model(id, (round == 1).then_some(&pr))?;
        if model.files.is_empty() {
            return Err("No file changed since the round before.".into());
        }
        tracing::info!(target: "sb::review", review = id, ms = ms(), "review: diff model checked");
        self.run_guide(id, &pr, None)
    }

    /// The local clone of `owner/name`, and the remote that points at it.
    fn find_clone(&self, repo: &str) -> Res<(PathBuf, String)> {
        // Only remotes matter here, so skip `git status`: it is slow in a big repo.
        let roots: Vec<PathBuf> = self.config().scan.iter().map(|s| expand(s)).collect();
        let candidates = repos::candidates(&self.paths.claude.join("projects"), &roots);
        tracing::debug!(target: "sb::review", count = candidates.len(), %repo, "looking for a clone");
        let mut by_time: Vec<(PathBuf, u64)> = candidates.into_iter().collect();
        by_time.sort_by_key(|x| std::cmp::Reverse(x.1));
        for (p, _) in by_time {
            if let Some(remote) = review::find_remote(&p, repo) {
                return Ok((p, remote));
            }
        }
        Err(format!(
            "no local clone has a remote for {repo}. Clone it under a scanned folder."
        ))
    }

    /// Builds the diff model, runs the verified rebuild check, and compares the
    /// counts with GitHub. A failed rebuild stops the review.
    fn load_model(
        &self,
        id: &str,
        pr: Option<&PrInfo>,
    ) -> Res<std::sync::Arc<coverage::DiffModel>> {
        let (tree, head, base) = self.review_refs(id)?;
        let model = coverage::build(&tree, &base, &head)?;
        coverage::rebuild_ok(&model)
            .map_err(|err| format!("The diff check failed, so the review stops: {err}."))?;
        let github = match pr {
            Some(pr) => {
                let counts = pr
                    .files
                    .iter()
                    .map(|f| (f.path.clone(), (f.additions, f.deletions)))
                    .collect();
                let (errors, warnings) = coverage::compare_github(&model, &counts);
                if let Some(err) = errors.first() {
                    return Err(format!(
                        "The file list check failed, so the review stops: {err}."
                    ));
                }
                warnings
            }
            None => vec![],
        };
        for w in &github {
            tracing::warn!(target: "sb::review", review = id, warning = %w, "git and GitHub differ");
        }
        let view = CoverageView {
            files: model.files.len(),
            changed_lines: model
                .files
                .iter()
                .map(coverage::ChangedFile::changed_lines)
                .sum(),
            accepted: false,
            missed_lines: 0,
            binary: model
                .files
                .iter()
                .filter(|f| f.needs_name())
                .map(|f| f.path().to_owned())
                .collect(),
            github,
        };
        let model = std::sync::Arc::new(model);
        if let Some(r) = self.reviews.lock().unwrap().get_mut(id) {
            r.model = Some(model.clone());
            r.coverage = Some(view);
        }
        // The drafts and the threads follow the head in the same step as the model.
        let moved = self.move_drafts(id, &model)? | self.move_threads(id, &model)?;
        if moved {
            self.emit_review(id);
        }
        Ok(model)
    }

    /// The diff model, built when a review loads from the store.
    fn model(&self, id: &str) -> Res<std::sync::Arc<coverage::DiffModel>> {
        let have = self
            .reviews
            .lock()
            .unwrap()
            .get(id)
            .and_then(|r| r.model.clone());
        match have {
            Some(m) => Ok(m),
            None => self.load_model(id, None),
        }
    }

    /// Stores a guide that the verified checker accepted.
    fn accept_guide(&self, id: &str, guide: Value, missed_lines: usize) {
        if let Some(r) = self.reviews.lock().unwrap().get_mut(id) {
            r.row.guide = Some(guide.to_string());
            r.attempts = 0;
            r.proposal = None;
            if let Some(c) = r.coverage.as_mut() {
                c.accepted = true;
                c.missed_lines = missed_lines;
            }
        }
        self.save_row(id);
        self.emit_review(id);
    }

    /// When the agent is done: the stored guide must pass the checker. If it
    /// does not, or the agent gave up, the app completes the last guide.
    fn finalize_guide(&self, id: &str) -> Res<bool> {
        let model = self.model(id)?;
        let (stored, proposal) = {
            let r = self.reviews.lock().unwrap();
            let r = r.get(id).ok_or("gone")?;
            (
                r.row
                    .guide
                    .as_deref()
                    .and_then(|g| serde_json::from_str::<Value>(g).ok()),
                r.proposal.clone(),
            )
        };
        if let Some(g) = &stored {
            if coverage::check_guide(&model, g).is_ok() {
                // After an update with no new guide, an old note can name a line that
                // is gone: that note goes, the rest stays.
                if crate::take::check(&model, g).is_err() {
                    let missed = r_missed(self, id);
                    self.accept_guide(id, crate::take::strip_bad(&model, g), missed);
                }
                return Ok(true);
            }
        }
        let Some(base) = proposal.or(stored) else {
            return Ok(false);
        };
        let (guide, missed) = coverage::complete(&model, &crate::take::strip_bad(&model, &base))?;
        tracing::warn!(target: "sb::review", review = id, missed, "the app completed the guide");
        self.accept_guide(id, guide, missed);
        Ok(true)
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
        let specs = spec_dirs(&config);
        let prompt = match update.clone() {
            Some(u) => u,
            None => {
                let mut p = review::prompt(&template, pr, &head, &specs);
                p += &self.round_prompt(id)?;
                p
            }
        };
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
            dirs: &specs,
        };
        let result = review::run(&run, |ev| match ev {
            Stream::Started { pid } => match self.reviews.lock().unwrap().get_mut(id) {
                Some(r) => r.run_pid = Some(*pid),
                // The review closed before the run started: stop it now.
                None => review::stop_run(*pid),
            },
            // Past here `run` reaps the process, and its pid can go to another one.
            Stream::Exited => {
                if let Some(r) = self.reviews.lock().unwrap().get_mut(id) {
                    r.run_pid = None;
                }
            }
            Stream::Init { session, .. } => {
                if let Some(r) = self.reviews.lock().unwrap().get_mut(id) {
                    r.row.guide_session = Some(session.clone());
                }
                self.save_row(id);
            }
            _ => {}
        });
        if let Some(r) = self.reviews.lock().unwrap().get_mut(id) {
            r.run_pid = None;
        }
        let result = result?;
        let has_guide = self.finalize_guide(id)?;
        match result {
            Stream::Result { .. } if has_guide => self.set_status(id, "ready", None),
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

    /// Starts the next round of a review: a new review of the same PR that covers
    /// only the changes since the head that this review covered. The verified checker
    /// then proves that the new guide covers every line changed since then.
    pub fn review_round(&self, id: &str) -> Res<String> {
        self.pr_only(id)?;
        let prev = self
            .reviews
            .lock()
            .unwrap()
            .get(id)
            .map(|r| r.row.clone())
            .ok_or(format!("no review {id}"))?;
        self.start_round(&prev)
    }

    /// A new round after `prev`, an open or a closed round of the same PR.
    fn start_round(&self, prev: &ReviewRow) -> Res<String> {
        if prev.is_local() {
            return Err("A review of local commits has no rounds. Use Refresh.".into());
        }
        if prev.head.is_empty() || prev.guide.is_none() {
            return Err("Finish this round first: it has no guide yet.".into());
        }
        let later = self
            .reviews
            .lock()
            .unwrap()
            .values()
            .find(|r| r.row.repo == prev.repo && r.row.number == prev.number && r.row.round > prev.round)
            .map(|r| r.row.id.clone());
        if let Some(later) = later {
            return Ok(later);
        }
        let nid = self.store.lock().unwrap().next_id("r").map_err(e)?;
        let row = ReviewRow {
            id: nid.clone(),
            url: prev.url.clone(),
            repo: prev.repo.clone(),
            number: prev.number,
            title: prev.title.clone(),
            head: String::new(),
            base: String::new(),
            tree: String::new(),
            connection: Some(self.active_connection()),
            guide_session: None,
            guide: None,
            round: prev.round + 1,
            since: Some(prev.head.clone()),
            scope_note: None,
            kind: "pr".into(),
            pane: None,
        };
        self.reviews
            .lock()
            .unwrap()
            .insert(nid.clone(), ReviewState::new(row, "fetching", None));
        self.save_row(&nid);
        self.emit_review(&nid);
        let hub = self.arc();
        let rid = nid.clone();
        std::thread::spawn(move || {
            if let Err(err) = hub.review_prepare(&rid) {
                hub.set_status(&rid, "error", Some(err));
            }
        });
        Ok(nid)
    }

    /// The part of the guide prompt for a later round: the scope, and the comments
    /// that the round before sent. Empty for a first round.
    fn round_prompt(&self, id: &str) -> Res<String> {
        let row = self
            .reviews
            .lock()
            .unwrap()
            .get(id)
            .map(|r| r.row.clone())
            .ok_or("gone")?;
        let Some(since) = row.since.clone().filter(|_| row.round > 1) else {
            return Ok(String::new());
        };
        let model = self.model(id)?;
        let files: Vec<String> = model
            .files
            .iter()
            .map(|f| {
                let add = f.added.iter().filter(|m| **m).count();
                let del = f.removed.iter().filter(|m| **m).count();
                format!("- {} (+{add} -{del})", f.path())
            })
            .collect();
        let sent = self
            .store
            .lock()
            .unwrap()
            .posted_for(&row.repo, row.number, row.round - 1)
            .map_err(e)?;
        let mut comments: Vec<String> = vec![];
        for p in &sent {
            if let Some(b) = p.body.as_deref().filter(|b| !b.trim().is_empty()) {
                comments.push(format!("- Summary: {}", b.replace('\n', " ")));
            }
            for l in p.lines.as_deref().unwrap_or("").lines() {
                comments.push(format!("- {l}"));
            }
        }
        let prev = row.round - 1;
        let mut p = format!(
            "\n\nThis is round {} of the review. Round {prev} reviewed commit {since}. This round covers only the changes since then. \
             The rules above about the merge base and the changed files do not apply. \
             The diff of this round is `git diff -U0 {} {}`. Old line numbers are in {}, new line numbers are at the head. \
             Every changed line of this diff must be in a range, and no other line.\n",
            row.round, row.base, row.head, row.base
        );
        if let Some(n) = &row.scope_note {
            p += &format!("{n}\n");
        }
        p += &format!("Files changed in this round:\n{}\n", files.join("\n"));
        if comments.is_empty() {
            p += &format!("The reviewer sent no comments in round {prev}.\n");
        } else {
            p += &format!(
                "Comments the reviewer sent in round {prev}, at commit {since}:\n{}\n\
                 For each comment, say in the step that changes its code whether the change addresses it. \
                 Put the comments that no change touches in one step with no code, titled \"Round {prev} comments not touched\".\n",
                comments.join("\n")
            );
        }
        Ok(p)
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
        if self.pr_only(id).is_err() {
            return self.local_refresh_bg(id);
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
        let guide = normalize(guide)?;
        let model = self.model(id)?;
        // The takes and whys first. A wrong one counts as a try; on the last try the
        // bad parts go and the rest of the guide stays.
        let guide = match crate::take::check(&model, &guide) {
            Ok(()) => guide,
            Err(msg) => {
                // The good parts are the proposal: if the agent stops now, the app
                // completes that, as for a coverage refusal.
                let stripped = crate::take::strip_bad(&model, &guide);
                let attempts = {
                    let mut r = self.reviews.lock().unwrap();
                    let r = r.get_mut(id).ok_or("gone")?;
                    r.attempts += 1;
                    r.proposal = Some(stripped.clone());
                    r.attempts
                };
                if attempts < MAX_ATTEMPTS {
                    // Every problem in one answer: the coverage ones too.
                    let cov = coverage::check_guide(&model, &stripped).err().map(|r| r.text()).unwrap_or_default();
                    return Err(format!("{msg}{cov}This was try {attempts} of {MAX_ATTEMPTS}."));
                }
                tracing::info!(target: "sb::review", review = id, "bad takes dropped on the last try");
                crate::take::strip_bad(&model, &guide)
            }
        };
        match coverage::check_guide(&model, &guide) {
            Ok(()) => {
                tracing::info!(target: "sb::review", review = id, "guide accepted by the checker");
                self.accept_guide(id, guide, 0);
                Ok(())
            }
            Err(report) => {
                let attempts = {
                    let mut r = self.reviews.lock().unwrap();
                    let r = r.get_mut(id).ok_or("gone")?;
                    r.attempts += 1;
                    r.proposal = Some(guide.clone());
                    r.attempts
                };
                tracing::info!(target: "sb::review", review = id, attempts, uncovered = report.uncovered.len(), bad = report.bad_ranges.len(), "guide refused by the checker");
                if attempts >= MAX_ATTEMPTS {
                    // Enough tries: keep what is good, and add every missed change.
                    let (done, missed) = coverage::complete(&model, &guide)?;
                    self.accept_guide(id, done, missed);
                    return Ok(());
                }
                Err(format!(
                    "{}This was try {attempts} of {MAX_ATTEMPTS}.",
                    report.text()
                ))
            }
        }
    }

    pub(crate) fn guide_update(&self, id: &str, step: &str, fields: Value) -> Res<()> {
        let mut guide: Value = {
            let r = self.reviews.lock().unwrap();
            let g = r
                .get(id)
                .and_then(|r| r.row.guide.clone())
                .ok_or("no guide yet: call guide_set_steps first")?;
            serde_json::from_str(&g).map_err(e)?
        };
        {
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
        }
        let guide = normalize(guide)?;
        let model = self.model(id)?;
        // Only the changed step must pass; a bad take elsewhere (from a guide that an
        // update completed) goes, so it never blocks this step.
        let changed: Value = json!({ "steps": guide["steps"].as_array().map(|a| a.iter().filter(|s| s["id"] == step).cloned().collect::<Vec<_>>()).unwrap_or_default() });
        crate::take::check(&model, &changed)?;
        let guide = crate::take::strip_bad(&model, &guide);
        coverage::check_guide(&model, &guide).map_err(|r| r.text())?;
        let _ = self
            .store
            .lock()
            .unwrap()
            .set_step(id, step, None, Some(false));
        self.accept_guide(id, guide, 0);
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

    /// The diff of a step, drawn from the verified model: the same lines and masks
    /// that the checker accepted. There is no second copy from another `git diff`.
    pub fn review_diff(&self, id: &str, step: &str) -> Res<DiffView> {
        let s = self.step(id, step)?;
        let mut ranges = coverage::step_ranges(&s);
        // Files with no lines (binary, renames, mode changes) show as sections too.
        let named: Vec<String> = s
            .get("files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        let model = self.model(id)?;
        let mut sections: Vec<DiffSection> = vec![];
        let add = |fi: usize, range: Option<coverage::Range>, sections: &mut Vec<DiffSection>| {
            let f = &model.files[fi];
            if let Some(sec) = sections.iter_mut().find(|sec| sec.path == f.path()) {
                sec.ranges.extend(range);
                return;
            }
            let mut sec = section(f);
            sec.ranges.extend(range);
            sections.push(sec);
        };
        for r in ranges.drain(..) {
            let fi = model.files.iter().position(|f| {
                let p = if r.side == "old" {
                    f.old_path.as_deref()
                } else {
                    f.new_path.as_deref()
                };
                p == Some(r.file.as_str())
            });
            if let Some(fi) = fi {
                add(fi, Some(r), &mut sections);
            }
        }
        for n in &named {
            if let Some(fi) = model.files.iter().position(|f| f.path() == n) {
                add(fi, None, &mut sections);
            }
        }
        // A step shows only its own part of each file. The bar names the rest.
        let all_steps = self.guide_steps(id);
        for sec in sections.iter_mut().filter(|sec| !sec.ranges.is_empty()) {
            let Some(f) = model.files.iter().find(|f| f.path() == sec.path) else {
                continue;
            };
            let (rows, hidden) = coverage::step_rows(f, &sec.ranges, STEP_CONTEXT);
            sec.rows = rows;
            if hidden.is_empty() {
                continue;
            }
            let steps = all_steps
                .iter()
                .filter(|st| st.get("id").and_then(Value::as_str) != Some(step))
                .filter(|st| {
                    let rs = coverage::step_ranges(st);
                    hidden.iter().any(|&(old, n)| {
                        let path = if old { &f.old_path } else { &f.new_path };
                        rs.iter().any(|g| {
                            (g.side == "old") == old
                                && path.as_deref() == Some(g.file.as_str())
                                && g.from <= n
                                && n <= g.to
                        })
                    })
                })
                .filter_map(|st| st.get("id").and_then(Value::as_str).map(str::to_owned))
                .collect();
            sec.other = Some(OtherChanges {
                lines: hidden.len(),
                steps,
            });
        }
        Ok(DiffView {
            head: model.head.clone(),
            sections,
        })
    }

    /// Every changed file of the PR, in path order, with the steps that cover it.
    /// The steps of the current guide, or none.
    fn guide_steps(&self, id: &str) -> Vec<Value> {
        let guide: Option<Value> = self
            .reviews
            .lock()
            .unwrap()
            .get(id)
            .and_then(|r| r.row.guide.as_deref().and_then(|g| serde_json::from_str(g).ok()));
        guide
            .and_then(|g| g.get("steps").and_then(Value::as_array).cloned())
            .unwrap_or_default()
    }

    pub fn review_files(&self, id: &str) -> Res<Vec<FileEntry>> {
        let model = self.model(id)?;
        let steps = self.guide_steps(id);
        let mut files: Vec<FileEntry> = model
            .files
            .iter()
            .map(|f| {
                let covers = |s: &Value| {
                    let by_range = coverage::step_ranges(s).iter().any(|r| {
                        let p = if r.side == "old" { &f.old_path } else { &f.new_path };
                        p.as_deref() == Some(r.file.as_str())
                    });
                    let named = s
                        .get("files")
                        .and_then(Value::as_array)
                        .is_some_and(|a| a.iter().any(|n| n.as_str() == Some(f.path())));
                    by_range || named
                };
                FileEntry {
                    path: f.path().to_owned(),
                    old_path: f.old_path.clone(),
                    added: f.added.iter().filter(|m| **m).count(),
                    removed: f.removed.iter().filter(|m| **m).count(),
                    note: note(f),
                    steps: steps
                        .iter()
                        .filter(|s| covers(s))
                        .filter_map(|s| s.get("id").and_then(Value::as_str).map(str::to_owned))
                        .collect(),
                }
            })
            .collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    /// The diff of one whole file, from the verified model.
    pub fn review_file_diff(&self, id: &str, path: &str) -> Res<DiffView> {
        let model = self.model(id)?;
        let f = model
            .files
            .iter()
            .find(|f| f.path() == path)
            .ok_or(format!("This review does not change {path}."))?;
        Ok(DiffView {
            head: model.head.clone(),
            sections: vec![section(f)],
        })
    }

    /// Every row of one file, for the expand buttons of the diff.
    pub fn review_file_rows(&self, id: &str, path: &str) -> Res<FileRows> {
        let model = self.model(id)?;
        let f = model
            .files
            .iter()
            .find(|f| f.path() == path)
            .ok_or(format!("This review does not change {path}."))?;
        Ok(FileRows {
            head: model.head.clone(),
            rows: coverage::all_rows(f),
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
        self.pr_only(id)?;
        let guide_session = self
            .reviews
            .lock()
            .unwrap()
            .get(id)
            .and_then(|r| r.row.guide_session.clone())
            .ok_or("the guide session has not started")?;
        // A new thread on a line keeps the head and the code around the line, so it
        // moves with the code as drafts do. The model comes first: no lock is held.
        let at = match &anchor {
            Some(a) if thread.is_none() => {
                let m = self.model(id)?;
                let old = a.side == "old";
                let x = crate::post::model_file(&m, &a.path, old)
                    .and_then(|f| crate::post::anchor_at(f, old, a.line, a.line));
                // The line must be from the diff of this head, with this text.
                if a.head != m.head || !x.as_ref().is_some_and(|x| crate::post::same_line(&a.text, &x.text)) {
                    return Err(STALE_VIEW.into());
                }
                x.map(|x| (m.head.clone(), x))
            }
            _ => None,
        };
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
                    line_text: at.as_ref().map(|(_, x)| x.text.clone()).or_else(|| anchor.as_ref().map(|a| a.text.clone())),
                    fork_session: None,
                    removed: false,
                    head: at.as_ref().map(|(h, _)| h.clone()),
                    before: at.as_ref().map(|(_, x)| x.before.clone()),
                    after: at.as_ref().map(|(_, x)| x.after.clone()),
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
            Some(_) => format!("Respond tersely. Follow-up question: {text}"),
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
        let config = self.config();
        let env = connections::env(&config, &conn)?;
        let specs = spec_dirs(&config);
        let sock = self.paths.sock();
        let run = Run {
            cwd: &tree,
            prompt,
            resume: Some(resume),
            fork,
            env: &env,
            mcp: Some((&self.sb, id, &sock)),
            dirs: &specs,
        };
        let result = review::run(&run, |ev| match ev {
            Stream::Init { session, .. } if fork => {
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
        let config = self.config();
        let env = connections::env(&config, &self.active_connection())?;
        let specs = spec_dirs(&config);
        let run = Run {
            cwd: &tree,
            prompt: ask,
            resume: Some(&fork),
            fork: true,
            env: &env,
            mcp: None,
            dirs: &specs,
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
        self.pr_only(id)?;
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
        self.pr_only(id)?;
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
            start_line: None,
            text: line,
            agent: true,
            head: None,
            line_text: None,
            stale: false,
            before: None,
            after: None,
        };
        // The draft goes on the thread's line only when that line still has the text
        // that the question was about. Else it is stale, and you place it yourself.
        let d = match (&d.path, d.line, self.model(id)) {
            (Some(path), Some(n), Ok(m)) => {
                let old = d.side.as_deref() == Some("old");
                let at = crate::post::model_file(&m, path, old).and_then(|f| crate::post::anchor_at(f, old, n, n));
                // The thread must be at this head, on a line it did not lose, with the
                // text that the question was about.
                let good = !t.removed
                    && t.head.as_deref() == Some(m.head.as_str())
                    && at.as_ref().is_some_and(|a| t.line_text.as_deref() == Some(a.text.as_str()));
                match at.filter(|_| good) {
                    Some(a) => DraftRow { head: Some(m.head.clone()), line_text: Some(a.text), before: Some(a.before), after: Some(a.after), ..d },
                    None => DraftRow { head: Some(m.head.clone()), stale: true, ..d },
                }
            }
            (Some(_), Some(_), Err(_)) => DraftRow { stale: true, ..d },
            _ => d,
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

    /// A comment that you write on a line or a range of lines. The line must exist
    /// in the diff model at the reviewed head.
    pub fn review_comment(&self, id: &str, c: NewComment) -> Res<()> {
        let NewComment {
            path,
            side,
            line,
            start_line,
            text,
            head,
            line_text,
            agent,
        } = c;
        let (path, side, head, line_text) = (path.as_str(), side.as_str(), head.as_str(), line_text.as_str());
        let text = text.trim();
        if text.is_empty() {
            return Err("The comment is empty.".into());
        }
        if side != "old" && side != "new" {
            return Err(format!("Unknown side: {side}"));
        }
        let start = start_line.unwrap_or(line);
        let model = self.model(id)?;
        let f = crate::post::model_file(&model, path, side == "old")
            .ok_or(format!("This review does not change {path}."))?;
        if start == 0 || start > line || crate::post::line_text(f, side == "old", line).is_none() {
            return Err(format!("{path} has no lines {start} to {line}."));
        }
        // The line must be from the diff of this head, with the text you saw.
        if head != model.head
            || !crate::post::line_text(f, side == "old", line).is_some_and(|t| crate::post::same_line(line_text, &t))
        {
            return Err(STALE_VIEW.into());
        }
        let d = DraftRow {
            id: 0,
            path: Some(path.to_owned()),
            side: Some(side.to_owned()),
            line: Some(line),
            start_line: (start < line).then_some(start),
            text: text.to_owned(),
            agent,
            head: Some(model.head.clone()),
            line_text: None,
            stale: false,
            before: None,
            after: None,
        };
        let d = match crate::post::anchor_at(f, side == "old", start, line) {
            Some(a) => DraftRow { line_text: Some(a.text), before: Some(a.before), after: Some(a.after), ..d },
            None => d,
        };
        self.store.lock().unwrap().add_draft(id, &d).map_err(e)?;
        self.emit_review(id);
        Ok(())
    }

    pub fn review_summary(&self, id: &str, text: &str) -> Res<()> {
        self.store
            .lock()
            .unwrap()
            .set_summary(id, text)
            .map_err(e)?;
        Ok(())
    }

    /// Builds the plan for a send. The PR head on GitHub must still be the head
    /// that you reviewed, and every line must match GitHub's diff.
    fn post_plan(&self, id: &str) -> Res<(ReviewRow, crate::post::Plan)> {
        self.pr_only(id)?;
        let row = self
            .reviews
            .lock()
            .unwrap()
            .get(id)
            .map(|r| r.row.clone())
            .ok_or(format!("no review {id}"))?;
        let model = self.model(id)?;
        if model.head != row.head {
            return Err("The diff is not at the reviewed head. Reopen the review.".into());
        }
        let pr = review::gh_view(&row.url)?;
        if pr.head_ref_oid != row.head {
            return Err(format!(
                "The PR has new commits ({} on GitHub). Update the guide, then send the review.",
                &pr.head_ref_oid[..pr.head_ref_oid.len().min(9)]
            ));
        }
        let gh = crate::post::parse_pr_diff(&review::gh_pr_diff(&row.repo, row.number)?);
        // A draft added while a new model loaded can still be at the old head: move
        // the drafts once more before the plan.
        if self.move_drafts(id, &model)? {
            self.emit_review(id);
        }
        let drafts = self.store.lock().unwrap().drafts(id).map_err(e)?;
        // In a later round the old side is the code of the round before, not GitHub's base.
        Ok((row.clone(), crate::post::plan(&model, &gh, &drafts, row.round == 1)))
    }

    pub fn review_post_preview(&self, id: &str) -> Res<PostPreview> {
        let (_, plan) = self.post_plan(id)?;
        Ok(PostPreview {
            login: review::gh_login()?,
            token: plan.token(),
            plan,
        })
    }

    /// Sends the review. `token` is the plan that you saw; the app builds the plan
    /// again and sends nothing if it differs.
    pub fn review_post(&self, id: &str, event: &str, summary: &str, token: &str) -> Res<String> {
        // One send of a review at a time: a second send waits for nothing and stops.
        let _sending = Sending::take(id)?;
        let (row, plan) = self.post_plan(id)?;
        if plan.token() != token {
            return Err("The review changed after the preview. Check it again.".into());
        }
        let payload = crate::post::payload(&plan, event, summary)?;
        // A send that reached GitHub but was not recorded here (the app stopped) is
        // on GitHub already. Find it, record it, and do not send it again.
        let url = match self.already_sent(&row, &payload)? {
            Some(url) => {
                tracing::warn!(target: "sb::review", review = id, %url, "review: GitHub has this review already; not sent again");
                url
            }
            None => {
                tracing::info!(target: "sb::review", review = id, event, comments = plan.inline.len(), "review: send to GitHub");
                review::gh_submit_review(&row.repo, row.number, &payload)?
            }
        };
        let posted = crate::store::PostedRow {
            event: event.to_owned(),
            url: url.clone(),
            comments: plan.inline.len() as u32,
            time: now(),
            head: Some(plan.head.clone()),
            body: payload["body"].as_str().map(str::to_owned),
            lines: Some(
                plan.inline
                    .iter()
                    .map(|c| format!("{}:{}{}: {}", c.path, c.line, if c.side == "LEFT" { " (old)" } else { "" }, c.text))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        };
        self.store
            .lock()
            .unwrap()
            .record_posted(id, &posted, &plan.drafts())
            .map_err(e)?;
        self.emit_review(id);
        Ok(url)
    }

    /// The link of a review on GitHub that is this send: same login, commit, type,
    /// summary and line comments. Reads only.
    fn already_sent(&self, row: &ReviewRow, payload: &Value) -> Res<Option<String>> {
        let login = review::gh_login()?;
        let reviews = review::gh_reviews(&row.repo, row.number)?;
        for (rid, url) in crate::post::sent_candidates(&reviews, &login, payload) {
            let comments = review::gh_review_comments(&row.repo, row.number, rid)?;
            if crate::post::same_comments(&comments, payload) {
                return Ok(Some(url));
            }
        }
        Ok(None)
    }

    /// Fetches the PR and puts its head in the review's worktree. The checkout runs
    /// under the tree lock, and only while the review is open: a review closed during
    /// the network fetch gets no worktree. The review row takes the path in the same
    /// lock, so a close always sees the worktree it has to remove.
    fn fetch(&self, id: &str, dir: &Path, remote: &str, pr: &PrInfo, round: u32) -> Res<review::Fetched> {
        // The head this review has now: pinned before the fetch moves `sb-pr-<n>`, so
        // it never looks like work of yours (a review from before the pins has none).
        let (old_head, old_base) = self
            .reviews
            .lock()
            .unwrap()
            .get(id)
            .map(|r| (r.row.head.clone(), r.row.base.clone()))
            .unwrap_or_default();
        if !old_head.is_empty() {
            review::pin(dir, id, &old_head, &old_base)?;
        }
        let (head, base) = review::fetch_refs(dir, remote, pr)?;
        let _trees = self.trees.lock().unwrap();
        let own = match self.reviews.lock().unwrap().get(id) {
            None => return Err(CLOSED.into()),
            Some(r) => (!r.row.tree.is_empty()).then(|| PathBuf::from(&r.row.tree)),
        };
        let tree = review::checkout_tree_for(dir, &worktree_name(pr.number, round), &head, own.as_deref())?;
        if let Some(r) = self.reviews.lock().unwrap().get_mut(id) {
            r.row.tree = tree.to_string_lossy().into_owned();
        }
        Ok(review::Fetched { head, base, tree })
    }

    /// Closes a review and cleans up after it (SPEC 11.10).
    /// - No choice (`keep` false, `discard` None): when the worktree has changes,
    ///   nothing happens, and the result lists them, so you can choose.
    /// - `discard` Some(list): the changes you saw go with the worktree. When it now
    ///   has a change that is not on the list, nothing happens, and the result lists
    ///   the changes again.
    /// - `keep`: the worktree stays when it has changes.
    ///
    /// The close stops the guide run, closes the review's Claude pane and its nvim
    /// panes with no unsaved files, and then removes the worktree in the background.
    pub fn review_close(&self, id: &str, keep: bool, discard: Option<Vec<String>>) -> Res<CloseResult> {
        let row = self
            .reviews
            .lock()
            .unwrap()
            .get(id)
            .map(|r| r.row.clone())
            .ok_or(format!("no review {id}"))?;
        let tree = PathBuf::from(&row.tree);
        // A local review reads the agent's own worktree: its changes, its panes and the
        // folder are not the review's, so the close leaves them all.
        let main = (!row.tree.is_empty() && !row.is_local()).then(|| repos::main_checkout(&tree)).flatten();
        if !keep {
            if let Some(main) = &main {
                let st = review::tree_state(main, &tree)?;
                let seen = discard.as_deref().unwrap_or(&[]);
                if st.exists && st.worktree && st.changes.iter().any(|c| !seen.contains(c)) {
                    return Ok(CloseResult {
                        closed: false,
                        changes: st.changes,
                    });
                }
            }
        }
        // Take the review out under the tree lock: a checkout of it is then either
        // done, with the path in `row`, or never happens.
        let (row, pid) = {
            let _trees = self.trees.lock().unwrap();
            let st = self.reviews.lock().unwrap().remove(id).ok_or("gone")?;
            (st.row, st.run_pid)
        };
        tracing::info!(target: "sb::review", review = id, tree = %row.tree, ?pid, keep, ?discard, "review close");
        {
            // The last row, with its worktree path, so the sweep at start finds it.
            let store = self.store.lock().unwrap();
            store.save_review(&row, now()).map_err(e)?;
            store.close_review(id, now()).map_err(e)?;
        }
        if let Some(pid) = pid {
            review::stop_run(pid);
        }
        if !row.tree.is_empty() && !row.is_local() {
            for pane in self.review_panes(&row) {
                let _ = self.close(&pane);
            }
        }
        self.sink.emit("review_closed", json!(id));
        let hub = self.arc();
        std::thread::spawn(move || {
            let note = hub.clean_after(&row, discard.unwrap_or_default());
            if let Some(note) = note {
                hub.sink.emit("review_cleanup", json!({ "review": row.id, "note": note }));
            }
        });
        Ok(CloseResult {
            closed: true,
            changes: vec![],
        })
    }

    /// The panes that a close of `row` stops: its Claude pane, and nvim panes in its
    /// worktree with no unsaved files.
    fn review_panes(&self, row: &ReviewRow) -> Vec<String> {
        let title = format!("Ask #{}", row.number);
        let nvim_dir = self.paths.nvim_dir();
        let panes: Vec<(String, String, Option<String>)> = self
            .panes
            .lock()
            .unwrap()
            .iter()
            .filter(|p| review::same_path(Path::new(&p.row.tree), Path::new(&row.tree)))
            .map(|p| (p.row.id.clone(), p.row.kind.clone(), p.row.title.clone()))
            .collect();
        panes
            .into_iter()
            .filter(|(_, kind, t)| match kind.as_str() {
                "claude" => t.as_deref() == Some(title.as_str()),
                "nvim" => nvim_saved(&crate::nvim::socket(&nvim_dir, Path::new(&row.tree))),
                _ => false,
            })
            .map(|(id, _, _)| id)
            .collect()
    }

    /// Removes the worktree of a closed review when the rule allows it, and the PR
    /// branch when no open review of the PR is left. Pins the head first. Returns a
    /// note for the window when the worktree stays.
    fn clean_after(&self, row: &ReviewRow, discard: Vec<String>) -> Option<String> {
        // Never for a local review: its tree is the agent's. A pin there would also
        // hide the agent's unpushed commits from `local_changes`.
        if row.tree.is_empty() || row.is_local() {
            return None;
        }
        let tree = PathBuf::from(&row.tree);
        let main = repos::main_checkout(&tree)
            .or_else(|| self.find_clone(&row.repo).ok().map(|(d, _)| d))?;
        if let Err(err) = review::pin(&main, &row.id, &row.head, &row.base) {
            tracing::warn!(target: "sb::review", review = %row.id, %err, "pin failed: the worktree stays");
            return Some(format!("{} stays: {err}", row.tree));
        }
        let _trees = self.trees.lock().unwrap();
        let (open_review, pr_open) = {
            let reviews = self.reviews.lock().unwrap();
            (
                reviews.values().any(|r| !r.row.tree.is_empty() && review::same_path(Path::new(&r.row.tree), &tree)),
                reviews.values().any(|r| r.row.repo == row.repo && r.row.number == row.number),
            )
        };
        let panes: Vec<String> = self
            .panes
            .lock()
            .unwrap()
            .iter()
            .filter(|p| review::same_path(Path::new(&p.row.tree), &tree))
            .map(|p| p.row.title.clone().unwrap_or_else(|| format!("{} pane {}", p.row.kind, p.row.id)))
            .collect();
        let state = match review::tree_state(&main, &tree) {
            Ok(s) => s,
            Err(err) => return Some(format!("{} stays: {err}", row.tree)),
        };
        let facts = crate::cleanup::Facts {
            tree: state,
            in_store: !row.is_local(),
            open_review,
            panes,
            discard,
        };
        let verdict = crate::cleanup::verdict(&facts);
        tracing::info!(target: "sb::review", review = %row.id, tree = %row.tree, ?verdict, "review cleanup");
        let note = match verdict {
            crate::cleanup::Verdict::Remove { force } => {
                review::remove_tree(&main, &tree, force).err().map(|err| format!("{} stays: {err}", row.tree))
            }
            crate::cleanup::Verdict::Gone => None,
            crate::cleanup::Verdict::Keep(why) => Some(format!("{} stays: {why}.", row.tree)),
            crate::cleanup::Verdict::Ask(c) => Some(format!("{} stays: it has {} change(s).", row.tree, c.len())),
        };
        // The branch goes only while it still points at this review's head: a commit
        // of yours on it stays.
        if !pr_open && !row.head.is_empty() {
            let _ = repos::git(&main, &["update-ref", "-d", &format!("refs/heads/sb-pr-{}", row.number), &row.head]);
        }
        let _ = repos::git(&main, &["worktree", "prune"]);
        note
    }

    /// At start: cleans up after reviews that closed before this version, and after
    /// cleanups that a quit cut short. Same rule as a close, with no discard.
    pub(crate) fn sweep_closed(&self) {
        let rows = self.store.lock().unwrap().closed_reviews().unwrap_or_default();
        let mut done = std::collections::HashSet::new();
        // The newest row of a folder decides: rows are oldest first.
        for row in rows.iter().rev() {
            if row.tree.is_empty() || !done.insert(row.tree.clone()) || !Path::new(&row.tree).exists() {
                continue;
            }
            if let Some(note) = self.clean_after(row, vec![]) {
                tracing::info!(target: "sb::review", review = %row.id, %note, "sweep");
            }
        }
    }

    // ---------- new commits (SPEC 11.7) ----------

    pub(crate) fn review_tick(&self, n: u64) {
        let every = self.config().poll_head_minutes.max(1) * 60;
        if !n.is_multiple_of(30) {
            return;
        }
        // A local review: a new HEAD in its tree is a new head (SPEC 11.11). One
        // `rev-parse` for each local review every 30 s.
        let locals: Vec<(String, String, String)> = self
            .reviews
            .lock()
            .unwrap()
            .values()
            .filter(|r| r.row.is_local() && r.status == "ready")
            .map(|r| (r.row.id.clone(), r.row.tree.clone(), r.row.head.clone()))
            .collect();
        for (id, tree, _) in locals {
            if let Ok(now_head) = repos::git(Path::new(&tree), &["rev-parse", "HEAD"]) {
                let changed = {
                    let mut reviews = self.reviews.lock().unwrap();
                    // Compare with the row as it is now: a Refresh can have run since.
                    match reviews.get_mut(&id) {
                        Some(r) if r.status == "ready" && now_head != r.row.head && r.new_head.as_deref() != Some(now_head.as_str()) => {
                            r.new_head = Some(now_head);
                            true
                        }
                        _ => false,
                    }
                };
                if changed {
                    self.emit_review(&id);
                }
            }
        }
        let due: Vec<(String, String, String)> = self
            .reviews
            .lock()
            .unwrap()
            .values()
            .filter(|r| !r.row.is_local() && r.status == "ready" && now().saturating_sub(r.last_poll) >= every)
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
        if self.pr_only(id).is_err() {
            return self.local_refresh_bg(id);
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
        let (url, repo, old, round, since) = {
            let r = self.reviews.lock().unwrap();
            let r = r.get(id).ok_or("gone")?;
            (r.row.url.clone(), r.row.repo.clone(), r.row.head.clone(), r.row.round, r.row.since.clone())
        };
        self.set_status(id, "updating", None);
        let pr = review::gh_view(&url)?;
        let (dir, remote) = self.find_clone(&repo)?;
        let f = self.fetch(id, &dir, &remote, &pr, round)?;
        if f.head != pr.head_ref_oid {
            return Err(format!(
                "the PR head moved during the fetch (GitHub {}, fetched {}). Use Update again.",
                pr.head_ref_oid, f.head
            ));
        }
        let changed = review::changed_files(&f.tree, &old, &f.head).unwrap_or_default();
        let (base, note) = scope(&f, round, since.as_deref())?;
        review::pin(&dir, id, &f.head, &base)?;
        if let Some(r) = self.reviews.lock().unwrap().get_mut(id) {
            r.row.head = f.head.clone();
            r.row.base = base;
            r.row.scope_note = note;
            r.new_head = None;
            r.attempts = 0;
            r.proposal = None;
        }
        self.save_row(id);
        self.load_model(id, (round == 1).then_some(&pr))?;
        let guide: Option<Value> = self.reviews.lock().unwrap().get(id).and_then(|r| {
            r.row
                .guide
                .as_deref()
                .and_then(|g| serde_json::from_str(g).ok())
        });
        let mut stale = vec![];
        if let Some(g) = &guide {
            for s in g["steps"].as_array().into_iter().flatten() {
                let files: Vec<String> = coverage::step_ranges(s)
                    .into_iter()
                    .map(|r| r.file)
                    .collect();
                if let Some(sid) = s["id"].as_str() {
                    if files.iter().any(|f| changed.contains(f)) {
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
        let prompt = format!(
            "The PR has new commits. The head is now {}. The worktree is checked out at it. Changed files since {old}: {}. \
             Read the new code. Call guide_set_steps with the full guide for the new head, with the whys, the check answers \
             and the takes for the new code. Keep the id of each step \
             that still applies, so the reviewer keeps its notes. Every changed line of the new diff must be in a range. \
             Reply with one line when done.",
            f.head,
            if changed.is_empty() { "(none)".to_owned() } else { changed.join(", ") }
        );
        self.run_guide(id, &pr, Some(prompt))?;
        let _ = stale;
        Ok(())
    }

    /// Puts each draft on its code at the model's head. Runs on every model load, so
    /// every path that changes the head (update, retry, restart) moves the drafts.
    /// - At the draft's own head: a draft with no stored text gets it.
    /// - At a new head: the draft moves to the same code (`post::move_anchor`), or
    ///   becomes stale. A draft with no stored text cannot move, so it is stale.
    fn move_drafts(&self, id: &str, model: &coverage::DiffModel) -> Res<bool> {
        let drafts = self.store.lock().unwrap().drafts(id).map_err(e)?;
        let mut changed = false;
        for d in drafts {
            let (Some(path), Some(line)) = (&d.path, d.line) else {
                continue;
            };
            let old = d.side.as_deref() == Some("old");
            let start = d.start_line.unwrap_or(line);
            let f = crate::post::model_file(model, path, old);
            let same_head = d.head.as_deref() == Some(model.head.as_str());
            let at = match (&d.line_text, same_head) {
                (Some(_), true) => continue,
                (None, true) if !d.stale => f.and_then(|f| crate::post::anchor_at(f, old, start, line)),
                (None, _) => None,
                (Some(text), false) => f.and_then(|f| {
                    crate::post::move_anchor(f, old, text, d.before.as_deref(), d.after.as_deref())
                }),
            };
            if d.stale && same_head {
                continue;
            }
            self.store
                .lock()
                .unwrap()
                .place_draft(d.id, &model.head, at.as_ref())
                .map_err(e)?;
            changed = true;
        }
        Ok(changed)
    }

    /// Moves each thread on a line to its code at the model's head, with the rule of
    /// the drafts (`post::move_anchor`). A thread whose code is gone keeps its line
    /// and shows "line removed"; a draft from it is stale.
    fn move_threads(&self, id: &str, model: &coverage::DiffModel) -> Res<bool> {
        let threads = self.store.lock().unwrap().threads(id).map_err(e)?;
        let mut changed = false;
        for mut t in threads {
            let (Some(path), Some(_), Some(text)) = (t.path.clone(), t.line, t.line_text.clone()) else {
                continue;
            };
            if t.head.as_deref() == Some(model.head.as_str()) {
                continue;
            }
            let old = t.side.as_deref() == Some("old");
            let at = crate::post::model_file(model, &path, old).and_then(|f| {
                crate::post::move_anchor(f, old, &text, t.before.as_deref(), t.after.as_deref())
            });
            match at {
                Some(a) => {
                    t.line = Some(a.line);
                    t.line_text = Some(a.text);
                    t.before = Some(a.before);
                    t.after = Some(a.after);
                    t.removed = false;
                }
                None => t.removed = true,
            }
            t.head = Some(model.head.clone());
            self.store.lock().unwrap().save_thread(&t).map_err(e)?;
            changed = true;
        }
        Ok(changed)
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

    /// The Claude Code pane of a review. It forks the guide session in the review
    /// worktree, so it knows the review. With no pane and `start`, it starts one that
    /// opens on `step`. Returns the pane id.
    pub fn review_term(&self, id: &str, step: Option<&str>, start: bool) -> Res<Option<String>> {
        // A local review shows its agent pane itself, while that pane is open.
        let local = self.reviews.lock().unwrap().get(id).filter(|r| r.row.is_local()).map(|r| r.row.pane.clone());
        if let Some(pane) = local {
            let open = pane.filter(|p| self.panes.lock().unwrap().iter().any(|x| &x.row.id == p));
            return Ok(open);
        }
        let (tree, number, repo, session, connection) = {
            let r = self.reviews.lock().unwrap();
            let r = r.get(id).ok_or("gone")?;
            (
                r.row.tree.clone(),
                r.row.number,
                r.row.repo.clone(),
                r.row.guide_session.clone(),
                r.row.connection.clone(),
            )
        };
        let title = format!("Ask #{number}");
        let found = self
            .panes
            .lock()
            .unwrap()
            .iter()
            .find(|p| {
                p.row.kind == "claude"
                    && p.row.tree == tree
                    && p.row.title.as_deref() == Some(title.as_str())
                    // An ended fork with no session id cannot resume: start a new one.
                    && (p.lamp() != crate::lamp::Lamp::Ended || p.row.session.is_some())
            })
            .map(|p| p.row.id.clone());
        if found.is_some() || !start {
            return Ok(found);
        }
        let session = session.ok_or("the guide session has not started")?;
        let first = match step {
            Some(s) => format!(
                "{} Wait for my question. Reply only: Ready.",
                step_note(&self.step(id, s)?)
            ),
            None => "I have questions about this review. Wait for my question. Reply only: Ready.".into(),
        };
        let name = repo.rsplit('/').next().unwrap_or(&repo).to_owned();
        let view = self.spawn(
            "claude",
            &name,
            Path::new(&tree),
            Some(title),
            crate::hub::Start {
                resume: Some(session),
                fork: true,
                dirs: spec_dirs(&self.config()),
                first: Some(first),
                // The guide session file is in the config dir of its connection.
                connection,
                // It answers questions; rust-analyzer would take 1 to 4 GB for each review.
                rust: Some(false),
            },
        )?;
        Ok(Some(view.id))
    }

    /// One line that tells the review's Claude Code pane the current step.
    pub fn review_step_note(&self, id: &str, step: &str) -> Res<String> {
        self.pr_only(id)?;
        Ok(step_note(&self.step(id, step)?))
    }

    /// Stops every running guide run. The app calls it at exit.
    pub fn stop_runs(&self) {
        for pid in self.reviews.lock().unwrap().values().filter_map(|r| r.run_pid) {
            review::stop_run(pid);
        }
    }

    /// The Claude pane of a PR review (`review_term`): its id, its session id once the
    /// hooks report it, and its connection.
    fn ask_pane(&self, tree: &str, number: u64) -> Option<(String, Option<String>, Option<String>)> {
        let title = format!("Ask #{number}");
        self.panes
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.row.kind == "claude" && p.row.tree == tree && p.row.title.as_deref() == Some(title.as_str()))
            .max_by_key(|p| p.row.session.is_some())
            .map(|p| (p.row.id.clone(), p.row.session.clone(), p.row.connection.clone()))
    }

    /// The questions and answers in the review's Claude pane, each with its step
    /// (SPEC 11.5). Read from Claude's session file, so it works for a full-screen
    /// Claude and after a restart. Empty when the pane or its file is not there yet.
    pub fn review_chat(&self, id: &str) -> Res<Vec<crate::chat::Turn>> {
        let (tree, number) = {
            let r = self.reviews.lock().unwrap();
            let r = r.get(id).ok_or("gone")?;
            if r.row.is_local() {
                return Ok(vec![]);
            }
            (r.row.tree.clone(), r.row.number)
        };
        let Some((_, Some(session), connection)) = self.ask_pane(&tree, number) else {
            return Ok(vec![]);
        };
        let config = self.config();
        let mut dirs = vec![];
        if let Some(dir) = connection.as_deref().and_then(|c| config.connections.get(c)).and_then(|c| c.dir.as_deref()) {
            dirs.push(expand(dir));
        }
        dirs.push(self.paths.claude.clone());
        let file = dirs.iter().find_map(|d| session_file(&d.join("projects"), &session));
        let Some(file) = file else { return Ok(vec![]) };
        let text = std::fs::read_to_string(&file).map_err(e)?;
        let seen = self.store.lock().unwrap().steps_seen(id).map_err(e)?;
        Ok(crate::chat::turns(&text, &seen))
    }

    /// You view `step` now: questions you ask from now on belong to it.
    pub fn review_step_seen(&self, id: &str, step: &str) -> Res<()> {
        self.store.lock().unwrap().step_seen(id, step, now()).map_err(e)
    }

    /// Dismisses a take note, or brings it back (`on` false). `note` is its key.
    pub fn review_dismiss(&self, id: &str, note: &str, on: bool) -> Res<()> {
        self.pr_only(id)?;
        self.store.lock().unwrap().dismiss(id, note, on).map_err(e)?;
        self.emit_review(id);
        Ok(())
    }

    /// Writes Claude's take again: for one step, or for the PR when `step` is None.
    /// The guide session does it, with the notes you dismissed, so it does not repeat
    /// them (SPEC 11.12).
    pub fn review_retake(&self, id: &str, step: Option<&str>) -> Res<()> {
        self.pr_only(id)?;
        if let Some(s) = step {
            self.step(id, s)?;
        }
        // The check and the switch to "updating" in one lock: a double click starts
        // one run.
        let row = {
            let mut reviews = self.reviews.lock().unwrap();
            let r = reviews.get_mut(id).ok_or("gone")?;
            if matches!(r.status.as_str(), "fetching" | "writing" | "updating") {
                return Err("the review is still running".into());
            }
            if r.row.guide_session.is_none() {
                return Err("the guide session has not started".into());
            }
            r.status = "updating".into();
            r.error = None;
            r.row.clone()
        };
        self.emit_review(id);
        let dismissed = self.store.lock().unwrap().dismissed(id).map_err(e)?;
        let mine: Vec<&String> = dismissed.iter().filter(|k| step.is_none_or(|s| k.starts_with(&format!("{s}|")))).collect();
        let what = match step {
            Some(s) => format!("Write your take on step {s} again. Read its code at {} first. Call guide_update_step with id {s} and fields {{\"take\": ...}}.", row.head),
            None => format!("Write your take on the whole PR again. Read the code at {} first. Call guide_set_steps with the full guide, with the new PR take and every step as it is.", row.head),
        };
        let prompt = if mine.is_empty() {
            format!("{what} Reply with one line when done.")
        } else {
            let list: Vec<String> = mine.iter().map(|k| format!("- {}", k.split_once('|').map(|x| x.1).unwrap_or(k))).collect();
            format!("{what} The reviewer dismissed these notes; do not write them again:\n{}\nReply with one line when done.", list.join("\n"))
        };
        let pr = PrInfo {
            number: row.number,
            title: row.title.clone(),
            body: String::new(),
            head_ref_oid: row.head.clone(),
            head_ref_name: String::new(),
            base_ref_name: String::new(),
            base_ref_oid: String::new(),
            files: vec![],
        };
        let hub = self.arc();
        let rid = id.to_owned();
        std::thread::spawn(move || {
            if let Err(err) = hub.run_guide(&rid, &pr, Some(prompt)) {
                hub.set_status(&rid, "error", Some(err));
            }
        });
        Ok(())
    }

    /// Tests only: adds a review row as a review with a guide, with no gh and no agent.
    #[doc(hidden)]
    pub fn debug_add_review(&self, row: ReviewRow) {
        let id = row.id.clone();
        self.reviews
            .lock()
            .unwrap()
            .insert(id.clone(), ReviewState::new(row, "ready", None));
        self.save_row(&id);
    }

    /// Tests only: the fetch and the gated checkout of a review, with no gh.
    #[doc(hidden)]
    pub fn debug_fetch(&self, id: &str, dir: &Path, remote: &str, pr: &PrInfo, round: u32) -> Res<PathBuf> {
        self.fetch(id, dir, remote, pr, round).map(|f| f.tree)
    }

    /// Tests only: the cleanup that runs at start.
    #[doc(hidden)]
    pub fn debug_sweep(&self) {
        self.sweep_closed()
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

/// Checks the shape of a guide, and gives every step a `ranges` list.
fn normalize(mut guide: Value) -> Res<Value> {
    let steps = guide
        .get_mut("steps")
        .and_then(Value::as_array_mut)
        .ok_or("the guide needs a steps array")?;
    if steps.is_empty() {
        return Err("the guide has no steps".into());
    }
    let mut ids = std::collections::HashSet::new();
    for (i, s) in steps.iter_mut().enumerate() {
        let id = s
            .get("id")
            .and_then(Value::as_str)
            .ok_or(format!("step {i} has no id"))?
            .to_owned();
        if !ids.insert(id.clone()) {
            return Err(format!("step id {id} is used twice"));
        }
        s.get("title")
            .and_then(Value::as_str)
            .ok_or(format!("step {id} has no title"))?;
        let ranges = coverage::step_ranges(s);
        s["ranges"] = serde_json::to_value(ranges).map_err(e)?;
    }
    Ok(guide)
}

/// Threads that wait for an answer.
static BUSY: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// The reviews with a send to GitHub in progress.
static SENDING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// A send in progress. It ends when the value drops, also on an error.
struct Sending(String);

impl Sending {
    fn take(id: &str) -> Res<Sending> {
        let mut s = SENDING.lock().unwrap();
        if s.iter().any(|x| x == id) {
            return Err("This review is being sent already.".into());
        }
        s.push(id.to_owned());
        Ok(Sending(id.to_owned()))
    }
}

impl Drop for Sending {
    fn drop(&mut self) {
        SENDING.lock().unwrap().retain(|x| x != &self.0);
    }
}

/// The paste that sends a local review's comments to its agent. Line numbers are at
/// `head`; the agent can have moved on since then.
fn agent_text(head: &str, note: &str, drafts: &[DraftRow]) -> String {
    let mut t = format!(
        "Review comments on commit {} (line numbers are at that commit):\n",
        &head[..head.len().min(12)]
    );
    if !note.is_empty() {
        t += &format!("{note}\n");
    }
    for d in drafts {
        let at = match (d.path.as_deref(), d.line) {
            (Some(p), Some(l)) => {
                let range = match d.start_line {
                    Some(s) if s != l => format!("{s}-{l}"),
                    _ => l.to_string(),
                };
                let old = if d.side.as_deref() == Some("old") { " (old side)" } else { "" };
                format!("{p}:{range}{old}")
            }
            (Some(p), None) => p.to_owned(),
            _ => "general".into(),
        };
        t += &format!("- {at}: {}\n", d.text.trim());
    }
    // One paste: no control character but the line breaks.
    t.trim_end()
        .chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .collect()
}

/// The lines of the review's last coverage that the app put into "Not in the guide".
fn r_missed(hub: &Hub, id: &str) -> usize {
    hub.reviews.lock().unwrap().get(id).and_then(|r| r.coverage.as_ref().map(|c| c.missed_lines)).unwrap_or(0)
}

/// `<projects>/<any folder>/<session>.jsonl`, the file of a Claude session.
fn session_file(projects: &Path, session: &str) -> Option<PathBuf> {
    if session.is_empty() || !session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    let name = format!("{session}.jsonl");
    std::fs::read_dir(projects).ok()?.flatten().map(|d| d.path().join(&name)).find(|p| p.is_file())
}

/// True when the nvim at `sock` has no unsaved file. An nvim that does not answer
/// counts as unsaved, so its pane stays.
fn nvim_saved(sock: &Path) -> bool {
    crate::nvim::expr(sock, "len(getbufinfo({'bufmodified': 1}))")
        .map(|n| n.trim() == "0")
        .unwrap_or(false)
}

/// "I am on step s2 "Title" (a.rs:3-9 (new))." on one line. The guide agent wrote
/// these fields, so control characters become spaces: the line goes into a tmux
/// command and into the pane as keys.
fn step_note(step: &Value) -> String {
    let mut p = format!(
        "I am on step {} \"{}\"",
        step["id"].as_str().unwrap_or("?"),
        step["title"].as_str().unwrap_or("")
    );
    let ranges = coverage::step_ranges(step);
    if !ranges.is_empty() {
        let list: Vec<String> = ranges
            .iter()
            .map(|r| format!("{}:{}-{} ({})", r.file, r.from, r.to, r.side))
            .collect();
        p += &format!(" ({})", list.join(", "));
    }
    p.push('.');
    p.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

fn thread_prompt(step: &Value, t: &ThreadRow, pins: &[String], question: &str) -> String {
    let mut p = format!(
        "The reviewer has a question about step {} \"{}\"",
        step["id"].as_str().unwrap_or("?"),
        step["title"].as_str().unwrap_or("")
    );
    let ranges = coverage::step_ranges(step);
    if !ranges.is_empty() {
        let list: Vec<String> = ranges
            .iter()
            .map(|r| format!("{}:{}-{} ({})", r.file, r.from, r.to, r.side))
            .collect();
        p += &format!(" ({})", list.join(", "));
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
    p += "\nDo not call the guide tools. Respond tersely.\n\nQuestion: ";
    p += question;
    p
}

/// The spec clones of the config, with `~` expanded.
fn spec_dirs(config: &crate::config::Config) -> Vec<String> {
    config
        .review
        .spec_clones
        .iter()
        .map(|s| expand(s).to_string_lossy().into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_send_of_a_review_runs_at_a_time() {
        let first = Sending::take("rs1").unwrap();
        assert!(Sending::take("rs1").is_err());
        // Another review can send at the same time.
        let other = Sending::take("rs2").unwrap();
        drop(first);
        let again = Sending::take("rs1").unwrap();
        drop(again);
        drop(other);
        // From many threads at once, exactly one wins.
        let wins: usize = (0..16)
            .map(|_| std::thread::spawn(|| Sending::take("rs3").map(std::mem::forget).is_ok()))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap() as usize)
            .sum();
        assert_eq!(wins, 1);
        SENDING.lock().unwrap().retain(|x| x != "rs3");
    }

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
            head: None,
            before: None,
            after: None,
        };
        let p = thread_prompt(&step, &t, &["Use safe_sub.".into()], "saturating or safe?");
        assert!(p.contains("gloas.rs:1379"));
        assert!(p.contains("`builder_params.slot - 1,`"));
        assert!(p.contains("- Use safe_sub."));
        assert!(p.ends_with("saturating or safe?"));
    }

    #[test]
    fn step_note_is_one_line() {
        let step = json!({"id":"s2","title":"Two\nlines","ranges":[{"file":"a.rs","side":"new","from":3,"to":9}]});
        assert_eq!(step_note(&step), "I am on step s2 \"Two lines\" (a.rs:3-9 (new)).");
        // A line break in any field would start a second tmux command.
        let step = json!({"id":"s1\nrun-shell 'x' #","title":"t\r\u{1b}[2J","ranges":[{"file":"a\n.rs","side":"new","from":1,"to":1}]});
        let note = step_note(&step);
        assert!(!note.chars().any(char::is_control), "{note:?}");
    }
}

