//! Claude points to the code (SPEC 11.15): the places of the Ask pane's last answer
//! ("Mentioned"), and the places of its `review_show` tool. Both only move the view
//! of the review window. Nothing here writes a file or runs the agent.

use crate::coverage::{self, DiffModel};
use crate::diff::Row;
use crate::hub::Hub;
use crate::hub_review::{DiffSection, DiffView};
use crate::mentions::{self, Mention};
use serde::Serialize;
use serde_json::{json, Value};

type Res<T> = Result<T, String>;

/// The most places of one `review_show` call.
pub const MAX_POINTS: usize = 20;
/// The longest note of a place, in chars.
const MAX_NOTE: usize = 2000;
/// The longest range of a place, in lines.
const MAX_SPAN: u32 = 400;
/// Rows of code above and below a line of a file that the PR does not change.
const CONTEXT_ROWS: u32 = 15;

/// The tool of the Ask pane, for its allow rules.
pub const SHOW_TOOL: &str = "mcp__sb__review_show";

/// The rules of the Ask pane, added to Claude Code's system prompt. One line: the
/// command goes through tmux.
pub const ASK_SYSTEM: &str = "You answer questions in a code review inside Switchboard. The reviewer sees your answers next to the diff. \
Cite code as path:line or path:start-end, with the path from the repo root, at the reviewed head. Put names of functions, types and fields in backticks. \
When the reviewer asks where something is, what to fix, or what to look at, call the mcp__sb__review_show tool with those places, the worst first, each with a short note and a kind (must, should, nit or question). \
The tool only moves the review window to the code; it changes nothing. Then answer in text as well, with the same citations.";

/// One place from `review_show`.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Point {
    pub file: String,
    pub line: u32,
    pub end: Option<u32>,
    /// `new` or `old`.
    pub side: String,
    /// `must`, `should`, `nit` or `question`.
    pub kind: Option<String>,
    pub note: String,
    /// The PR changes a line of it.
    pub changed: bool,
}

/// The places of the last `review_show` call. `at` is new for each call, so the
/// window starts each tour once.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Show {
    pub at: u64,
    pub points: Vec<Point>,
}

fn millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The number of lines of a text, as git counts them.
fn count_lines(text: &str) -> u32 {
    coverage::split_lines(text.as_bytes()).len() as u32
}

/// Checks the places from the agent. Every string is checked here, and the window
/// escapes them again. `file_lines` gives the line count of a file the PR does not
/// change, at the head, or None when it is not there.
pub fn check_points(
    points: &Value,
    m: &DiffModel,
    tree: &str,
    file_lines: &dyn Fn(&str) -> Option<u32>,
) -> Res<Vec<Point>> {
    let list = points.as_array().ok_or("points must be a list")?;
    if list.is_empty() {
        return Err("points is empty: give at least one place".into());
    }
    if list.len() > MAX_POINTS {
        return Err(format!("at most {MAX_POINTS} points in one call"));
    }
    let mut out = vec![];
    for (i, p) in list.iter().enumerate() {
        let n = i + 1;
        let bad = |why: String| format!("point {n}: {why}");
        let file = p.get("file").and_then(Value::as_str).ok_or_else(|| bad("file must be a string".into()))?;
        let file = mentions::clean_path(file, tree).map_err(bad)?;
        let side = match p.get("side") {
            None | Some(Value::Null) => "new",
            Some(s) => match s.as_str() {
                Some("new") => "new",
                Some("old") => "old",
                _ => return Err(bad("side must be new or old".into())),
            },
        };
        let line = p.get("line").and_then(Value::as_u64).filter(|l| *l >= 1 && *l < u32::MAX as u64).ok_or_else(|| bad("line must be a number from 1".into()))? as u32;
        let end = match p.get("end") {
            None | Some(Value::Null) => None,
            Some(v) => {
                let e = v.as_u64().filter(|e| *e >= line as u64 && *e - (line as u64) <= MAX_SPAN as u64).ok_or_else(|| bad(format!("end must be a line from {line} to {}", line + MAX_SPAN)))? as u32;
                (e > line).then_some(e)
            }
        };
        let kind = match p.get("kind") {
            None | Some(Value::Null) => None,
            Some(k) => match k.as_str() {
                Some(s @ ("must" | "should" | "nit" | "question")) => Some(s.to_owned()),
                _ => return Err(bad("kind must be must, should, nit or question".into())),
            },
        };
        let note = match p.get("note") {
            None | Some(Value::Null) => String::new(),
            Some(v) => v.as_str().ok_or_else(|| bad("note must be a string".into()))?.to_owned(),
        };
        let note: String = note.chars().filter(|c| *c == '\n' || !c.is_control()).take(MAX_NOTE).collect();
        // A changed file, on its side.
        let changed_file = m.files.iter().find(|f| {
            let path = if side == "old" { f.old_path.as_deref() } else { f.new_path.as_deref() };
            path == Some(file.as_str())
        });
        let (count, changed) = match changed_file {
            Some(f) => {
                let count = if side == "old" { f.old.len() } else { f.new.len() } as u32;
                (count, mentions::changed(f, side, line, end.unwrap_or(line)))
            }
            None if side == "old" => return Err(bad(format!("the PR does not change {file}, so it has no old side"))),
            None => (file_lines(&file).ok_or_else(|| bad(format!("{file} is not in the repo at the reviewed head")))?, false),
        };
        if end.unwrap_or(line) > count {
            return Err(bad(format!("{file} has {count} lines on the {side} side")));
        }
        out.push(Point { file, line, end, side: side.into(), kind, note, changed });
    }
    Ok(out)
}

impl Hub {
    /// The review of a PR Ask pane: its id. Found by the pane's title and worktree.
    fn ask_review(&self, pane: &str) -> Option<String> {
        let (title, tree) = {
            let panes = self.panes.lock().unwrap();
            let p = panes.iter().find(|p| p.row.id == pane && p.row.kind == "claude")?;
            (p.row.title.clone()?, p.row.tree.clone())
        };
        let number: u64 = title.strip_prefix("Ask #")?.parse().ok()?;
        self.reviews
            .lock()
            .unwrap()
            .values()
            .find(|r| !r.row.is_local() && r.row.number == number && r.row.tree == tree)
            .map(|r| r.row.id.clone())
    }

    /// The parts of an Ask pane's start that make it point: the `sb mcp --ask`
    /// config, the rules, and the allow rules. Empty for any other pane.
    pub(crate) fn ask_extras(&self, pane: &str) -> (Option<String>, Option<String>, Vec<String>) {
        let Some(id) = self.ask_review(pane) else { return (None, None, vec![]) };
        let tree = self.reviews.lock().unwrap().get(&id).map(|r| r.row.tree.clone()).unwrap_or_default();
        self.ask_start(&id, &tree)
    }

    /// The `sb mcp --ask` config, the rules and the allow rules of the Ask pane of
    /// review `id`. The server has only `review_show`: no guide tools, no `git`.
    pub(crate) fn ask_start(&self, id: &str, tree: &str) -> (Option<String>, Option<String>, Vec<String>) {
        let cfg = json!({ "mcpServers": { "sb": {
            "command": self.sb.to_string_lossy(), "args": ["mcp", "--ask"],
            "env": { "SB_REVIEW": id, "SB_SOCK": self.paths.sock().to_string_lossy(), "SB_DIR": tree }
        }}});
        let mut allow = crate::review::web_rules(&self.config().review.web_domains);
        allow.push(SHOW_TOOL.into());
        (Some(cfg.to_string()), Some(ASK_SYSTEM.into()), allow)
    }

    /// `review_show` from the Ask pane: checks the places and sends them to the
    /// window, which shows them as a tour. It only changes the view.
    pub fn review_show(&self, id: &str, points: &Value) -> Res<()> {
        let (tree, head) = {
            let r = self.reviews.lock().unwrap();
            let r = r.get(id).ok_or("the review is closed")?;
            (r.row.tree.clone(), r.row.head.clone())
        };
        let model = self.model(id)?;
        let head = if model.head.is_empty() { head } else { model.head.clone() };
        let lines = |file: &str| file_text(&tree, &head, file).map(|t| count_lines(&t));
        let points = check_points(points, &model, &tree, &lines)?;
        tracing::info!(target: "sb::review", review = id, points = points.len(), "review_show");
        {
            let mut reviews = self.reviews.lock().unwrap();
            let r = reviews.get_mut(id).ok_or("the review is closed")?;
            let at = millis().max(r.show.as_ref().map_or(0, |s| s.at + 1));
            r.show = Some(Show { at, points });
        }
        self.emit_review(id);
        Ok(())
    }

    /// After a turn of an Ask pane: the places its answer refers to. A new answer
    /// replaces the list.
    pub(crate) fn turn_mentions(&self, pane: &str, text: &str) {
        let Some(id) = self.ask_review(pane) else { return };
        let tree = self.reviews.lock().unwrap().get(&id).map(|r| r.row.tree.clone()).unwrap_or_default();
        let Ok(model) = self.model(&id) else { return };
        let list: Vec<Mention> = mentions::extract(text, &model, &tree);
        let changed = {
            let mut reviews = self.reviews.lock().unwrap();
            match reviews.get_mut(&id) {
                Some(r) if r.mentions != list => {
                    r.mentions = list;
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.emit_review(&id);
        }
    }

    /// Rows of a file that the PR does not change, around `line`, at the reviewed
    /// head: for a place outside the diff.
    pub fn review_context(&self, id: &str, path: &str, line: u32) -> Res<DiffView> {
        let (tree, head) = {
            let r = self.reviews.lock().unwrap();
            let r = r.get(id).ok_or("gone")?;
            (r.row.tree.clone(), r.row.head.clone())
        };
        let model = self.model(id)?;
        let head = if model.head.is_empty() { head } else { model.head.clone() };
        let path = mentions::clean_path(path, &tree)?;
        let text = file_text(&tree, &head, &path).ok_or(format!("{path} is not in the repo at the reviewed head"))?;
        let all = coverage::split_lines(text.as_bytes());
        let from = line.saturating_sub(CONTEXT_ROWS).max(1);
        let to = line.saturating_add(CONTEXT_ROWS).min(all.len() as u32);
        let rows = (from..=to)
            .map(|n| Row { kind: ' ', old: Some(n), new: Some(n), text: { let l = &all[n as usize - 1]; String::from_utf8_lossy(l.strip_suffix(b"\n").unwrap_or(l)).into_owned() } })
            .collect();
        Ok(DiffView {
            head,
            sections: vec![DiffSection { path, old_path: None, rows, context: true, ranges: vec![], note: None, other: None }],
        })
    }
}

/// A file at `head`, through the read-only git of the `git` tool (`review_git`).
fn file_text(tree: &str, head: &str, path: &str) -> Option<String> {
    crate::review_git::run(std::path::Path::new(tree), "show", &[format!("{head}:{path}")]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage::ChangedFile;

    fn model() -> DiffModel {
        DiffModel {
            base: "b".into(),
            head: "h".into(),
            files: vec![ChangedFile {
                old_path: Some("src/a.rs".into()),
                new_path: Some("src/a.rs".into()),
                new: (0..30).map(|_| b"x".to_vec()).collect(),
                added: (1..=30).map(|n| n == 5).collect(),
                old: (0..10).map(|_| b"y".to_vec()).collect(),
                removed: (1..=10).map(|n| n == 2).collect(),
                ..ChangedFile::default()
            }],
        }
    }

    fn lines(file: &str) -> Option<u32> {
        (file == "README.md").then_some(40)
    }

    #[test]
    fn good_points_pass_with_their_marks() {
        let got = check_points(
            &json!([
                {"file": "src/a.rs", "line": 5, "kind": "must", "note": "Use safe_sub.\u{7}"},
                {"file": "/c/pr1/src/a.rs", "line": 1, "end": 3},
                {"file": "src/a.rs", "line": 2, "side": "old", "kind": "nit", "note": "<b>x</b>"},
                {"file": "README.md", "line": 40, "note": "Not changed."},
            ]),
            &model(),
            "/c/pr1",
            &lines,
        )
        .unwrap();
        assert_eq!(got.len(), 4);
        assert_eq!((got[0].changed, got[0].kind.as_deref(), got[0].note.as_str()), (true, Some("must"), "Use safe_sub."));
        assert_eq!((got[1].file.as_str(), got[1].end, got[1].changed), ("src/a.rs", Some(3), false));
        assert_eq!((got[2].side.as_str(), got[2].changed), ("old", true));
        // The window escapes the note; the hub keeps its text.
        assert_eq!(got[2].note, "<b>x</b>");
        assert!(!got[3].changed);
    }

    #[test]
    fn bad_points_are_refused_with_the_reason() {
        let m = model();
        let bad = |p: Value| check_points(&p, &m, "/c/pr1", &lines).unwrap_err();
        assert!(bad(json!({})).contains("list"));
        assert!(bad(json!([])).contains("empty"));
        assert!(bad(json!(vec![json!({"file": "src/a.rs", "line": 1}); MAX_POINTS + 1])).contains("at most"));
        for file in ["/etc/passwd", "../x.rs", "src/../../x.rs", "~/.ssh/id", "/c/pr1x/a.rs", ""] {
            let e = bad(json!([{"file": file, "line": 1}]));
            assert!(e.starts_with("point 1:"), "{file}: {e}");
        }
        assert!(bad(json!([{"file": "src/a.rs", "line": 31}])).contains("30 lines"));
        assert!(bad(json!([{"file": "src/a.rs", "line": 0}])).contains("line"));
        assert!(bad(json!([{"file": "src/a.rs", "line": 4, "end": 2}])).contains("end"));
        assert!(bad(json!([{"file": "src/a.rs", "line": 1, "side": "left"}])).contains("side"));
        assert!(bad(json!([{"file": "src/a.rs", "line": 1, "kind": "blocker"}])).contains("kind"));
        assert!(bad(json!([{"file": "src/a.rs", "line": 1, "note": 5}])).contains("note"));
        assert!(bad(json!([{"file": "nope.rs", "line": 1}])).contains("not in the repo"));
        assert!(bad(json!([{"file": "README.md", "line": 1, "side": "old"}])).contains("old side"));
        assert!(bad(json!([{"file": "src/a.rs", "line": 1}, {"file": "/x", "line": 1}])).starts_with("point 2:"));
    }

    #[test]
    fn a_long_note_is_cut() {
        let note = "a".repeat(MAX_NOTE * 2);
        let got = check_points(&json!([{"file": "src/a.rs", "line": 1, "note": note}]), &model(), "/c/pr1", &lines).unwrap();
        assert_eq!(got[0].note.len(), MAX_NOTE);
    }
}
