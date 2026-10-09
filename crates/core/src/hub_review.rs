//! Reviews in the hub (SPEC 11).

use crate::diff::{self, Row};
use crate::hub::Hub;
use crate::now;
use crate::repos::{self, git};
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
}

#[derive(Serialize, Clone, Debug)]
pub struct DiffView {
    pub sections: Vec<DiffSection>,
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
                    model: None,
                    attempts: 0,
                    proposal: None,
                    coverage: None,
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
            summary: store.summary(id).map_err(e)?,
            posted: store.posted(id).map_err(e)?,
            coverage: r.coverage.clone(),
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
                model: None,
                attempts: 0,
                proposal: None,
                coverage: None,
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
        let t0 = std::time::Instant::now();
        let ms = || t0.elapsed().as_millis() as u64;
        let (url, repo) = {
            let r = self.reviews.lock().unwrap();
            let r = r.get(id).ok_or("gone")?;
            (r.row.url.clone(), r.row.repo.clone())
        };
        tracing::info!(target: "sb::review", review = id, %url, "review: gh pr view");
        let pr = review::gh_view(&url)?;
        tracing::info!(target: "sb::review", review = id, ms = ms(), files = pr.files.len(), head = %pr.head_ref_oid, "review: got the PR");
        let (dir, remote) = self.find_clone(&repo)?;
        tracing::info!(target: "sb::review", review = id, ms = ms(), clone = %dir.display(), %remote, "review: found the clone");
        let f = review::fetch(&dir, &remote, &pr)?;
        tracing::info!(target: "sb::review", review = id, ms = ms(), tree = %f.tree.display(), "review: worktree ready");
        if f.head != pr.head_ref_oid {
            return Err(format!(
                "the PR head moved during the fetch (GitHub {}, fetched {}). Use Retry.",
                pr.head_ref_oid, f.head
            ));
        }
        if let Some(r) = self.reviews.lock().unwrap().get_mut(id) {
            r.row.title = pr.title.clone();
            r.row.head = f.head.clone();
            r.row.base = f.base.clone();
            r.row.tree = f.tree.to_string_lossy().into_owned();
            r.row.guide = None;
            r.attempts = 0;
            r.proposal = None;
        }
        self.save_row(id);
        self.load_model(id, Some(&pr))?;
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
                return Ok(true);
            }
        }
        let Some(base) = proposal.or(stored) else {
            return Ok(false);
        };
        let (guide, missed) = coverage::complete(&model, &base)?;
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
        let guide = normalize(guide)?;
        let model = self.model(id)?;
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
        Ok(DiffView { sections })
    }

    /// Every changed file of the PR, in path order, with the steps that cover it.
    pub fn review_files(&self, id: &str) -> Res<Vec<FileEntry>> {
        let model = self.model(id)?;
        let guide: Option<Value> = self
            .reviews
            .lock()
            .unwrap()
            .get(id)
            .and_then(|r| r.row.guide.as_deref().and_then(|g| serde_json::from_str(g).ok()));
        let steps: Vec<Value> = guide
            .and_then(|g| g.get("steps").and_then(Value::as_array).cloned())
            .unwrap_or_default();
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
            .ok_or(format!("The PR does not change {path}."))?;
        Ok(DiffView {
            sections: vec![section(f)],
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
            start_line: None,
            text: line,
            agent: true,
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
    pub fn review_comment(
        &self,
        id: &str,
        path: &str,
        side: &str,
        line: u32,
        start_line: Option<u32>,
        text: &str,
    ) -> Res<()> {
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
            .ok_or(format!("The PR does not change {path}."))?;
        if start == 0 || start > line || crate::post::line_text(f, side == "old", line).is_none() {
            return Err(format!("{path} has no lines {start} to {line}."));
        }
        let d = DraftRow {
            id: 0,
            path: Some(path.to_owned()),
            side: Some(side.to_owned()),
            line: Some(line),
            start_line: (start < line).then_some(start),
            text: text.to_owned(),
            agent: false,
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
        let drafts = self.store.lock().unwrap().drafts(id).map_err(e)?;
        Ok((row, crate::post::plan(&model, &gh, &drafts)))
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
        let (row, plan) = self.post_plan(id)?;
        if plan.token() != token {
            return Err("The review changed after the preview. Check it again.".into());
        }
        let payload = crate::post::payload(&plan, event, summary)?;
        tracing::info!(target: "sb::review", review = id, event, comments = plan.inline.len(), "review: send to GitHub");
        let url = review::gh_submit_review(&row.repo, row.number, &payload)?;
        let posted = crate::store::PostedRow {
            event: event.to_owned(),
            url: url.clone(),
            comments: plan.inline.len() as u32,
            time: now(),
        };
        self.store
            .lock()
            .unwrap()
            .record_posted(id, &posted, &plan.drafts())
            .map_err(e)?;
        self.emit_review(id);
        Ok(url)
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
        if f.head != pr.head_ref_oid {
            return Err(format!(
                "the PR head moved during the fetch (GitHub {}, fetched {}). Use Update again.",
                pr.head_ref_oid, f.head
            ));
        }
        let changed = review::changed_files(&f.tree, &old, &f.head).unwrap_or_default();
        if let Some(r) = self.reviews.lock().unwrap().get_mut(id) {
            r.row.head = f.head.clone();
            r.row.base = f.base.clone();
            r.new_head = None;
            r.attempts = 0;
            r.proposal = None;
        }
        self.save_row(id);
        self.load_model(id, Some(&pr))?;
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
        self.reanchor(id, &f.tree, &f.head)?;
        let prompt = format!(
            "The PR has new commits. The head is now {}. The worktree is checked out at it. Changed files since {old}: {}. \
             Read the new code. Call guide_set_steps with the full guide for the new head. Keep the id of each step \
             that still applies, so the reviewer keeps its notes. Every changed line of the new diff must be in a range. \
             Reply with one line when done.",
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
