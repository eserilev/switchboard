//! Drafts go to GitHub as a pending review as you write them (SPEC 11.8.1).
//!
//! One sync of a review runs at a time. A change during a sync makes one more sync
//! after it (`again`), never a second at the same time. The store is the copy you
//! can trust: no error deletes a draft.

use crate::gh::{Fate, Gh, Pending};
use crate::hub::Hub;
use crate::now;
use crate::post::{self, GhFile, Inline, SyncOp};
use crate::store::{DraftRow, DraftSync};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

type Res<T> = Result<T, String>;

fn e(x: impl std::fmt::Display) -> String {
    x.to_string()
}

/// The sync of one review, in memory.
#[derive(Default)]
pub(crate) struct SyncRun {
    pub running: bool,
    /// A change came while a sync ran: one more runs after it.
    pub again: bool,
    /// Failed syncs in a row, and when the next try runs.
    pub fails: u32,
    pub next_try: u64,
    pub error: Option<String>,
    /// Why nothing syncs: a pending review from the website on another commit.
    pub blocked: Option<String>,
    /// The drafts that stay in the app, with the reason.
    pub local: HashMap<i64, String>,
    /// The review has a pending review on GitHub, so the poll checks it.
    pub has_pending: bool,
    pub last_check: u64,
    /// GitHub's diff at a head.
    pub diff: Option<(String, Arc<Vec<GhFile>>)>,
    /// A sync ran once, so the states of the drafts are known.
    pub ran: bool,
}

/// The state of one draft on GitHub, for the window.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DraftSyncView {
    pub id: i64,
    /// `synced`, `sending`, `local` (stays in the app) or `error`.
    pub state: String,
    pub note: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SyncView {
    pub running: bool,
    pub error: Option<String>,
    pub blocked: Option<String>,
    /// The comments of the pending review that are drafts here.
    pub pending: usize,
    pub drafts: Vec<DraftSyncView>,
}

/// The wait after `fails` failed syncs: 15 s, doubled each time, at most 10 minutes.
pub(crate) fn backoff(fails: u32) -> u64 {
    (15u64 << fails.saturating_sub(1).min(6)).min(600)
}

/// GitHub's name of a sent review's type, as the app records it.
fn event_of(state: &str) -> &'static str {
    match state {
        "APPROVED" => "APPROVE",
        "CHANGES_REQUESTED" => "REQUEST_CHANGES",
        _ => "COMMENT",
    }
}

/// What the view shows of each draft.
pub(crate) fn view(run: &SyncRun, sync: &[DraftSync]) -> SyncView {
    let drafts = sync
        .iter()
        .map(|d| {
            let (state, note) = if let Some(why) = run.local.get(&d.id) {
                ("local", Some(why.clone()))
            } else if let Some(err) = &d.gh_error {
                ("error", Some(err.clone()))
            } else if d.gh_id.is_some() && d.gh_sig.is_some() && !run.running {
                ("synced", None)
            } else if let Some(err) = run.error.as_ref().or(run.blocked.as_ref()).filter(|_| !run.running) {
                ("error", Some(err.clone()))
            } else {
                ("sending", None)
            };
            DraftSyncView { id: d.id, state: state.into(), note }
        })
        .collect();
    SyncView {
        running: run.running,
        error: run.error.clone(),
        blocked: run.blocked.clone(),
        pending: sync.iter().filter(|d| d.gh_id.is_some()).count(),
        drafts,
    }
}

/// Builds the comment of a website comment as a draft: on its lines when the model
/// has them, else stale. The comment's REST record has its place.
fn import_draft(model: &crate::coverage::DiffModel, rest: Option<&Value>, body: &str) -> (DraftRow, Option<String>) {
    let text = body.replace("\r\n", "\n").trim().to_owned();
    let mut d = DraftRow {
        id: 0,
        path: None,
        side: None,
        line: None,
        start_line: None,
        text,
        agent: false,
        head: Some(model.head.clone()),
        line_text: None,
        stale: false,
        before: None,
        after: None,
    };
    let Some(c) = rest else {
        return (d, None);
    };
    d.path = c["path"].as_str().map(str::to_owned);
    let (Some(path), Some(line)) = (d.path.clone(), c["line"].as_u64().map(|l| l as u32)) else {
        // A comment on a whole file, or an outdated one: it goes into the summary.
        return (d, None);
    };
    let old = c["side"].as_str() == Some("LEFT");
    let start = c["start_line"].as_u64().map(|l| l as u32).filter(|s| *s < line);
    d.side = Some(if old { "old" } else { "new" }.into());
    d.line = Some(line);
    d.start_line = start;
    match post::model_file(model, &path, old).and_then(|f| post::anchor_at(f, old, start.unwrap_or(line), line)) {
        Some(a) => {
            d.line_text = Some(a.text);
            d.before = Some(a.before);
            d.after = Some(a.after);
            let c = Inline {
                draft: 0,
                path,
                side: if old { "LEFT" } else { "RIGHT" }.into(),
                line,
                start_line: start,
                text: d.text.clone(),
            };
            let sig = post::sig(&c);
            (d, Some(sig))
        }
        None => {
            d.stale = true;
            (d, None)
        }
    }
}

impl Hub {
    pub(crate) fn gh(&self) -> Arc<dyn Gh> {
        self.gh.read().unwrap().clone()
    }

    pub(crate) fn sync_on(&self) -> bool {
        self.gh_sync.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Starts a sync of the review, or asks the running one for one more.
    pub(crate) fn kick_sync(&self, id: &str) {
        if !self.sync_on() {
            return;
        }
        let start = {
            let mut reviews = self.reviews.lock().unwrap();
            let Some(r) = reviews.get_mut(id) else {
                return;
            };
            if r.row.is_local() || r.row.head.is_empty() {
                return;
            }
            if r.sync.running {
                r.sync.again = true;
                false
            } else {
                r.sync.running = true;
                true
            }
        };
        if !start {
            return;
        }
        self.emit_review(id);
        let hub = self.arc();
        let rid = id.to_owned();
        std::thread::spawn(move || hub.sync_loop(&rid));
    }

    /// A sync of a review with drafts or a pending review. One with neither has
    /// nothing to send, so it asks GitHub nothing.
    pub(crate) fn kick_sync_if_drafts(&self, id: &str) {
        if !self.sync_on() {
            return;
        }
        let wanted = {
            let store = self.store.lock().unwrap();
            store.drafts(id).is_ok_and(|d| !d.is_empty()) || store.gh_pending(id).is_ok_and(|p| p.0.is_some())
        };
        if wanted {
            self.kick_sync(id);
        }
    }

    /// Runs syncs while changes come. Called with `running` set.
    fn sync_loop(&self, id: &str) {
        loop {
            let res = self.sync_once(id);
            self.sync_done(id, res);
            if !self.sync_next(id) {
                self.emit_review(id);
                return;
            }
        }
    }

    /// True when one more sync must run; else the sync ends.
    fn sync_next(&self, id: &str) -> bool {
        let mut reviews = self.reviews.lock().unwrap();
        match reviews.get_mut(id) {
            Some(r) if r.sync.again => {
                r.sync.again = false;
                true
            }
            Some(r) => {
                r.sync.running = false;
                false
            }
            None => false,
        }
    }

    fn sync_done(&self, id: &str, res: Res<()>) {
        let mut reviews = self.reviews.lock().unwrap();
        let Some(r) = reviews.get_mut(id) else {
            return;
        };
        r.sync.ran = true;
        r.sync.last_check = now();
        match res {
            Ok(()) => {
                r.sync.fails = 0;
                r.sync.error = None;
                r.sync.next_try = 0;
            }
            Err(err) => {
                r.sync.fails += 1;
                r.sync.next_try = now() + backoff(r.sync.fails);
                tracing::warn!(target: "sb::review", review = id, error = %err, fails = r.sync.fails, "pending review sync failed");
                r.sync.error = Some(err);
            }
        }
    }

    /// Takes the sync of a review for a send: waits for a running sync to end.
    pub(crate) fn claim_sync(&self, id: &str) -> Res<()> {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            {
                let mut reviews = self.reviews.lock().unwrap();
                let r = reviews.get_mut(id).ok_or("gone")?;
                if !r.sync.running {
                    r.sync.running = true;
                    return Ok(());
                }
            }
            if std::time::Instant::now() > end {
                return Err("The comments are still going to GitHub. Try again.".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    /// Gives the sync back after a send. A change meanwhile starts a sync.
    pub(crate) fn release_sync(&self, id: &str) {
        let again = {
            let mut reviews = self.reviews.lock().unwrap();
            let Some(r) = reviews.get_mut(id) else {
                return;
            };
            r.sync.running = false;
            std::mem::take(&mut r.sync.again)
        };
        if again {
            self.kick_sync(id);
        } else {
            self.emit_review(id);
        }
    }

    /// The sync, run by the holder of `running`, for a send. Records the result.
    pub(crate) fn sync_held(&self, id: &str) -> Res<()> {
        if !self.sync_on() {
            return Ok(());
        }
        let res = self.sync_once(id);
        self.sync_done(id, res.clone());
        res
    }

    /// GitHub's diff at the reviewed head, once for each head.
    fn sync_diff(&self, id: &str, gh: &dyn Gh) -> Res<Arc<Vec<GhFile>>> {
        let (row, base_ref, cached) = {
            let r = self.reviews.lock().unwrap();
            let r = r.get(id).ok_or("gone")?;
            (r.row.clone(), r.base_ref.clone(), r.sync.diff.clone())
        };
        if let Some((head, diff)) = cached.filter(|(h, _)| *h == row.head) {
            let _ = head;
            return Ok(diff);
        }
        let base_ref = match base_ref {
            Some(b) => b,
            None => gh.view(&row.url)?.base_ref_name,
        };
        let diff = Arc::new(post::parse_pr_diff(&gh.diff_at(&row.repo, &row.url, &base_ref, &row.head)?));
        if let Some(r) = self.reviews.lock().unwrap().get_mut(id) {
            r.sync.diff = Some((row.head.clone(), diff.clone()));
        }
        Ok(diff)
    }

    /// One sync: makes the pending review on GitHub match the plan.
    fn sync_once(&self, id: &str) -> Res<()> {
        let gh = self.gh();
        let (row, status) = {
            let r = self.reviews.lock().unwrap();
            let r = r.get(id).ok_or("gone")?;
            (r.row.clone(), r.status.clone())
        };
        if row.is_local() || row.head.is_empty() || matches!(status.as_str(), "fetching" | "updating") {
            return Ok(());
        }
        let model = self.model(id)?;
        if model.head != row.head {
            return Ok(());
        }
        let diff = self.sync_diff(id, gh.as_ref())?;
        let drafts = self.store.lock().unwrap().drafts(id).map_err(e)?;
        let plan = post::plan(&model, &diff, &drafts, row.round == 1);
        let mut local: HashMap<i64, String> = plan.outside.iter().map(|o| (o.draft, format!("here only: {}", o.reason))).collect();
        local.extend(plan.stuck.iter().map(|(d, why)| (*d, format!("here only: {why}"))));
        let pr = gh.pending(&row.repo, row.number)?;
        let (ours, _) = self.store.lock().unwrap().gh_pending(id).map_err(e)?;
        let mut blocked = None;
        let pending: Option<Pending> = match pr.pending {
            Some(p) if Some(&p.id) == ours.as_ref() => {
                if p.commit == row.head {
                    Some(p)
                } else {
                    // Update guide moved the head: GitHub ties a pending review to one
                    // commit. Its comments with no draft become drafts first.
                    self.rescue(id, &model, gh.as_ref(), &row, &p)?;
                    tracing::info!(target: "sb::review", review = id, from = %p.commit, to = %row.head, "pending review: rebuild at the new head");
                    gh.delete_review(&p.id)?;
                    self.store.lock().unwrap().set_gh_pending(id, None).map_err(e)?;
                    None
                }
            }
            other => {
                if let Some(old) = &ours {
                    self.settle(id, gh.as_ref(), &row, old)?;
                }
                match other {
                    Some(p) if p.commit == row.head => {
                        tracing::info!(target: "sb::review", review = id, pending = %p.id, "pending review: take over the one from the website");
                        self.store.lock().unwrap().set_gh_pending(id, Some((&p.id, &row.head))).map_err(e)?;
                        Some(p)
                    }
                    Some(p) => {
                        blocked = Some(format!(
                            "You have a pending review on GitHub at commit {}, from the website. Submit or delete it there; until then your comments stay here.",
                            &p.commit[..p.commit.len().min(9)]
                        ));
                        None
                    }
                    None => None,
                }
            }
        };
        let blocked_now = blocked.is_some();
        if let Some(r) = self.reviews.lock().unwrap().get_mut(id) {
            r.sync.local = local;
            r.sync.blocked = blocked;
            r.sync.has_pending = pending.is_some();
        }
        if blocked_now {
            return Ok(());
        }
        let pending = match pending {
            Some(p) => p,
            None if plan.inline.is_empty() => return Ok(()),
            None => {
                let pid = gh.create_pending(&pr.pr_id, &row.head)?;
                tracing::info!(target: "sb::review", review = id, pending = %pid, head = %row.head, "pending review: made");
                self.store.lock().unwrap().set_gh_pending(id, Some((&pid, &row.head))).map_err(e)?;
                if let Some(r) = self.reviews.lock().unwrap().get_mut(id) {
                    r.sync.has_pending = true;
                }
                Pending { id: pid, db_id: 0, commit: row.head.clone(), comments: vec![] }
            }
        };
        let (sync, gone) = {
            let store = self.store.lock().unwrap();
            (store.draft_sync(id).map_err(e)?, store.gh_gone(id).map_err(e)?)
        };
        let ops = post::sync_ops(&plan, &sync, &pending.comments, &gone);
        if !ops.is_empty() {
            tracing::info!(target: "sb::review", review = id, ops = ops.len(), "pending review: sync");
        }
        let mut first_err: Option<String> = None;
        let mut rest: Option<Value> = None;
        let mut changed = false;
        for op in ops {
            let res = self.run_op(id, gh.as_ref(), &row, &model, &pending, &sync, &mut rest, &op);
            changed = true;
            if let Err(err) = res {
                let draft = match &op {
                    SyncOp::Add { draft, .. } | SyncOp::Edit { draft, .. } | SyncOp::Move { draft, .. } | SyncOp::Pull { draft, .. } => Some(*draft),
                    SyncOp::Drop { draft, .. } => *draft,
                    _ => None,
                };
                if let Some(d) = draft {
                    let _ = self.store.lock().unwrap().set_draft_error(d, Some(&err));
                }
                first_err.get_or_insert(err);
            }
        }
        if changed {
            self.emit_review(id);
        }
        match first_err {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn run_op(
        &self,
        id: &str,
        gh: &dyn Gh,
        row: &crate::store::ReviewRow,
        model: &crate::coverage::DiffModel,
        pending: &Pending,
        sync: &[DraftSync],
        rest: &mut Option<Value>,
        op: &SyncOp,
    ) -> Res<()> {
        match op {
            SyncOp::Add { draft, c } => self.add_one(gh, &pending.id, *draft, c),
            SyncOp::Edit { draft, gh_id, c } => {
                gh.edit_comment(gh_id, &c.text)?;
                self.store.lock().unwrap().set_draft_gh(*draft, Some(gh_id), Some(&post::sig(c))).map_err(e)
            }
            SyncOp::Move { draft, gh_id, c } => {
                gh.delete_comment(gh_id)?;
                self.store.lock().unwrap().set_draft_gh(*draft, None, None).map_err(e)?;
                self.add_one(gh, &pending.id, *draft, c)
            }
            SyncOp::Pull { draft, body, c } => {
                let gh_id = sync.iter().find(|d| d.id == *draft).and_then(|d| d.gh_id.clone());
                let store = self.store.lock().unwrap();
                store.edit_draft(*draft, body).map_err(e)?;
                let c = Inline { text: body.clone(), ..c.clone() };
                store.set_draft_gh(*draft, gh_id.as_deref(), Some(&post::sig(&c))).map_err(e)
            }
            SyncOp::Drop { draft, gh_id } => {
                gh.delete_comment(gh_id)?;
                let store = self.store.lock().unwrap();
                match draft {
                    Some(d) => store.set_draft_gh(*d, None, None).map_err(e),
                    None => store.drop_gh_gone(id, gh_id).map_err(e),
                }
            }
            SyncOp::Forget { draft } => self.store.lock().unwrap().set_draft_gh(*draft, None, None).map_err(e),
            SyncOp::Unmark { gh_id } => self.store.lock().unwrap().drop_gh_gone(id, gh_id).map_err(e),
            SyncOp::Import { gh_id } => {
                if rest.is_none() {
                    *rest = Some(gh.review_comments(&row.repo, row.number, pending.db_id)?);
                }
                let place = rest
                    .as_ref()
                    .and_then(|v| v.as_array())
                    .and_then(|a| a.iter().find(|c| c["node_id"].as_str() == Some(gh_id.as_str())));
                let body = pending.comments.iter().find(|c| c.id == *gh_id).map(|c| c.body.clone()).unwrap_or_default();
                let (d, sig) = import_draft(model, place, &body);
                tracing::info!(target: "sb::review", review = id, comment = %gh_id, placed = sig.is_some(), "pending review: a comment from the website becomes a draft");
                let store = self.store.lock().unwrap();
                let nid = store.add_draft(id, &d).map_err(e)?;
                store.set_draft_gh(nid, Some(gh_id), sig.as_deref()).map_err(e)
            }
        }
    }

    /// Adds one comment and records it. A draft deleted meanwhile takes its new
    /// comment with it.
    fn add_one(&self, gh: &dyn Gh, pending: &str, draft: i64, c: &Inline) -> Res<()> {
        let cid = gh.add_comment(pending, c)?;
        let kept = {
            let store = self.store.lock().unwrap();
            store.set_draft_gh(draft, Some(&cid), Some(&post::sig(c))).map_err(e)?;
            store.draft_exists(draft).map_err(e)?
        };
        if !kept {
            gh.delete_comment(&cid)?;
        }
        Ok(())
    }

    /// Before the app deletes its pending review: its comments that no draft has
    /// become drafts. They are on another commit, so they are stale (SPEC 11.7).
    fn rescue(&self, id: &str, model: &crate::coverage::DiffModel, gh: &dyn Gh, row: &crate::store::ReviewRow, p: &Pending) -> Res<()> {
        let (sync, gone) = {
            let store = self.store.lock().unwrap();
            (store.draft_sync(id).map_err(e)?, store.gh_gone(id).map_err(e)?)
        };
        let claimed: Vec<&str> = sync.iter().filter_map(|d| d.gh_id.as_deref()).chain(gone.iter().map(String::as_str)).collect();
        let lost: Vec<&post::GhComment> = p.comments.iter().filter(|c| !claimed.contains(&c.id.as_str())).collect();
        if lost.is_empty() {
            return Ok(());
        }
        let rest = gh.review_comments(&row.repo, row.number, p.db_id)?;
        for c in lost {
            let place = rest.as_array().and_then(|a| a.iter().find(|x| x["node_id"].as_str() == Some(c.id.as_str())));
            let (mut d, _) = import_draft(model, None, &c.body);
            if let Some(x) = place {
                d.path = x["path"].as_str().map(str::to_owned);
                d.line = x["line"].as_u64().or(x["original_line"].as_u64()).map(|l| l as u32);
                d.side = Some(if x["side"].as_str() == Some("LEFT") { "old" } else { "new" }.into());
                d.head = Some(p.commit.clone());
                d.stale = d.line.is_some();
            }
            if c.reply {
                d.text = format!("(A reply on GitHub) {}", d.text);
            }
            self.store.lock().unwrap().add_draft(id, &d).map_err(e)?;
        }
        Ok(())
    }

    /// The app's pending review is not on GitHub as pending: you sent it on the
    /// website (record it, and the drafts in it go), or deleted it (the drafts stay).
    fn settle(&self, id: &str, gh: &dyn Gh, row: &crate::store::ReviewRow, old: &str) -> Res<()> {
        match gh.fate(old)? {
            Fate::Pending => return Err("GitHub did not list the pending review. Trying again.".into()),
            Fate::Gone => {
                tracing::info!(target: "sb::review", review = id, pending = %old, "pending review: deleted on GitHub");
            }
            Fate::Sent { state, url } => {
                let drafts = self.store.lock().unwrap().drafts(id).map_err(e)?;
                let sync = self.store.lock().unwrap().draft_sync(id).map_err(e)?;
                let sent: Vec<&DraftRow> = drafts.iter().filter(|d| sync.iter().any(|s| s.id == d.id && s.gh_id.is_some())).collect();
                tracing::info!(target: "sb::review", review = id, %url, comments = sent.len(), "pending review: sent on GitHub");
                let posted = crate::store::PostedRow {
                    event: event_of(&state).into(),
                    url,
                    comments: sent.len() as u32,
                    time: now(),
                    head: Some(row.head.clone()),
                    body: None,
                    lines: Some(
                        sent.iter()
                            .map(|d| format!("{}:{}: {}", d.path.as_deref().unwrap_or(""), d.line.unwrap_or(0), d.text))
                            .collect::<Vec<_>>()
                            .join("\n"),
                    ),
                };
                let ids: Vec<i64> = sent.iter().map(|d| d.id).collect();
                self.store.lock().unwrap().record_posted(id, &posted, &ids).map_err(e)?;
            }
        }
        self.store.lock().unwrap().set_gh_pending(id, None).map_err(e)
    }

    /// The ticker: a failed sync tries again after its wait, and a review with a
    /// pending review checks it every 2 minutes, so a send on the website shows.
    pub(crate) fn sync_tick(&self) {
        if !self.sync_on() {
            return;
        }
        let t = now();
        let due: Vec<String> = self
            .reviews
            .lock()
            .unwrap()
            .values()
            .filter(|r| !r.row.is_local() && !r.sync.running && r.status == "ready")
            .filter(|r| (r.sync.error.is_some() && t >= r.sync.next_try) || (r.sync.has_pending && t.saturating_sub(r.sync.last_check) >= 120))
            .map(|r| r.row.id.clone())
            .collect();
        for id in due {
            self.kick_sync(&id);
        }
    }

    /// Tests only: GitHub for the send and the sync. The sync runs from now on.
    #[doc(hidden)]
    pub fn debug_set_gh(&self, gh: Arc<dyn Gh>) {
        *self.gh.write().unwrap() = gh;
        self.gh_sync.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Tests only: true while a sync of the review runs.
    #[doc(hidden)]
    pub fn debug_syncing(&self, id: &str) -> bool {
        self.reviews.lock().unwrap().get(id).is_some_and(|r| r.sync.running)
    }

    /// Tests only: the next retry is due now.
    #[doc(hidden)]
    pub fn debug_retry_now(&self, id: &str) {
        if let Some(r) = self.reviews.lock().unwrap().get_mut(id) {
            r.sync.next_try = 0;
            r.sync.last_check = 0;
        }
        self.sync_tick();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_doubles_up_to_ten_minutes() {
        let waits: Vec<u64> = (1..=8).map(backoff).collect();
        assert_eq!(waits, [15, 30, 60, 120, 240, 480, 600, 600]);
    }
}
