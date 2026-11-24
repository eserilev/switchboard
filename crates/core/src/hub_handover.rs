//! Handover in the hub (SPEC 38). The git work lives in `handover`.

use crate::expand;
use crate::handover::{self, Plan};
use crate::hub::{Hub, PaneView, Start};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// The agent of a handover, as it opens here, with what came along for the note
/// in the window (SPEC 38.3).
#[derive(Serialize, Debug, Clone)]
pub struct Arrived {
    pub pane: PaneView,
    pub from: String,
    pub branch: String,
    pub changes: usize,
    /// The conversation came along: the agent resumes it.
    pub resumed: bool,
}

type Res<T> = Result<T, String>;

impl Hub {
    /// The Claude folder of a connection: its `dir`, or your own `~/.claude`.
    pub(crate) fn claude_dir(&self, connection: Option<&str>) -> PathBuf {
        let config = self.config();
        connection
            .and_then(|c| config.connections.get(c))
            .and_then(|c| c.dir.as_deref())
            .map(expand)
            .unwrap_or_else(|| self.paths.claude.clone())
    }

    /// On the server: the snapshot of an agent's work, and its session file, for
    /// this computer to continue it (SPEC 38.1). The agent keeps running.
    pub fn handover_out(&self, id: &str) -> Res<Plan> {
        let (kind, lamp, tree, session, connection, title) = {
            let panes = self.panes.lock().unwrap();
            let p = panes.iter().find(|p| p.row.id == id).ok_or(format!("no pane {id}"))?;
            let r = &p.row;
            (r.kind.clone(), r.lamp.clone(), r.tree.clone(), r.session.clone(), r.connection.clone(), r.title.clone())
        };
        if kind != "claude" {
            return Err("only an agent pane can continue on this computer".into());
        }
        // A turn in progress changes the files while git reads them.
        if lamp == "working" {
            return Err("The agent is working. Wait until its turn ends, or press Esc in the pane.".into());
        }
        tracing::info!(target: "sb::hub", pane = id, %tree, "handover out");
        let mut plan = handover::snapshot(Path::new(&tree), id)?;
        // The Work agent's title belongs to the Work tab of each hub (SPEC 30).
        plan.title = title.filter(|t| t != crate::hub::WORK_TITLE).unwrap_or_default();
        if let Some(s) = session.filter(|s| handover::session_ok(s)) {
            let claude = self.claude_dir(connection.as_deref());
            let name = format!("{s}.jsonl");
            // Claude Code's folder for the tree, else any folder with the session.
            let file = Some(handover::project_dir(&claude, Path::new(&tree)).join(&name)).filter(|f| f.is_file()).or_else(|| {
                std::fs::read_dir(claude.join("projects")).ok()?.flatten().map(|d| d.path().join(&name)).find(|f| f.is_file())
            });
            if let Some(meta) = file.as_ref().and_then(|f| std::fs::metadata(f).ok()) {
                let file = file.unwrap();
                if meta.len() as usize > handover::SESSION_MAX {
                    return Err(format!("the session file has {} MB; the limit is {} MB", meta.len() >> 20, handover::SESSION_MAX >> 20));
                }
                plan.session_data = Some(std::fs::read_to_string(&file).map_err(|e| e.to_string())?);
                plan.session = Some(s);
            }
        }
        Ok(plan)
    }

    /// On this computer, at the window's start: opens the agent of a pending
    /// handover, with `--resume` on its session (SPEC 38.3). None when no handover
    /// waits.
    pub fn handover_take(&self) -> Res<Option<Arrived>> {
        let connection = self.active_connection();
        let claude = self.claude_dir(Some(&connection));
        let Some(p) = handover::take_pending(&self.paths.home, &claude)? else { return Ok(None) };
        tracing::info!(target: "sb::hub", tree = %p.tree, session = ?p.session, from = %p.from, "handover in");
        let tree = PathBuf::from(&p.tree);
        if !tree.is_dir() {
            return Err(format!("{} is gone", p.tree));
        }
        let name = Path::new(&p.repo).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let start = Start { resume: p.session.clone(), ..Start::default() };
        let title = Some(p.title.clone()).filter(|t| !t.is_empty());
        let pane = self.spawn("claude", &name, &tree, title, start)?;
        Ok(Some(Arrived { pane, from: p.from, branch: p.branch, changes: p.changes, resumed: p.session.is_some() }))
    }
}
