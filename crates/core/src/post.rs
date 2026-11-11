//! Send a review to GitHub (SPEC 11.8). Plain code: no agent takes part.
//!
//! The drafts become one GitHub review. Before it goes out, each comment line is
//! checked against two sources: the verified diff model at the reviewed head, and
//! GitHub's own diff of the PR. The text at the line must be the same in both.

use crate::coverage::{ChangedFile, DiffModel};
use crate::diff::{self, Row};
use crate::store::DraftRow;
use serde::Serialize;
use serde_json::{json, Value};

/// One file of GitHub's diff of the PR.
#[derive(Debug, Clone, PartialEq)]
pub struct GhFile {
    /// The head path, or the base path for a deleted file. GitHub comments use it.
    pub path: String,
    pub rows: Vec<Row>,
}

/// Splits GitHub's diff of a PR into files.
pub fn parse_pr_diff(text: &str) -> Vec<GhFile> {
    let mut chunks: Vec<Vec<&str>> = vec![];
    for line in text.lines() {
        if line.starts_with("diff --git ") || chunks.is_empty() {
            chunks.push(vec![]);
        }
        chunks.last_mut().unwrap().push(line);
    }
    chunks
        .into_iter()
        .filter(|c| c.first().is_some_and(|l| l.starts_with("diff --git ")))
        .map(|c| {
            let (mut old, mut new) = (None, None);
            for l in c.iter().take_while(|l| !l.starts_with("@@ ")) {
                if let Some(p) = l.strip_prefix("--- ") {
                    old = header_path(p, "a/");
                } else if let Some(p) = l.strip_prefix("+++ ") {
                    new = header_path(p, "b/");
                } else if let Some(p) = l.strip_prefix("rename to ") {
                    new = Some(unquote(p));
                } else if let Some(p) = l.strip_prefix("rename from ") {
                    old = Some(unquote(p));
                }
            }
            // A file with no `---`/`+++` lines (mode change, pure rename, binary).
            let fallback = || {
                c[0].strip_prefix("diff --git a/")
                    .and_then(|r| r.split_once(" b/"))
                    .map(|(_, b)| b.to_owned())
                    .unwrap_or_default()
            };
            GhFile {
                path: new.or(old).unwrap_or_else(fallback),
                rows: diff::parse(&c.join("\n")),
            }
        })
        .collect()
}

fn header_path(p: &str, prefix: &str) -> Option<String> {
    let p = unquote(p.trim_end_matches('\t'));
    if p == "/dev/null" {
        return None;
    }
    Some(p.strip_prefix(prefix).unwrap_or(&p).to_owned())
}

/// A path as git writes it in a diff header. git puts a path with special bytes in
/// quotes, with C escapes: `"caf\303\251.rs"` is `café.rs`. The verified kernel
/// `diffparse::unquote` decodes the escapes (SPEC 24.2).
fn unquote(p: &str) -> String {
    let Some(inner) = p.strip_prefix('"').and_then(|p| p.strip_suffix('"')) else {
        return p.to_owned();
    };
    String::from_utf8_lossy(&diffparse::unquote(inner.as_bytes())).into_owned()
}

/// A comment that goes on a line of the PR.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Inline {
    pub draft: i64,
    pub path: String,
    /// `RIGHT` for the head side, `LEFT` for the base side.
    pub side: String,
    pub line: u32,
    pub start_line: Option<u32>,
    pub text: String,
}

/// A comment that GitHub cannot take on a line. It goes into the review summary.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Outside {
    pub draft: i64,
    pub at: String,
    pub reason: String,
    pub text: String,
}

/// What the app sends. You see this before anything goes out.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Plan {
    pub head: String,
    pub inline: Vec<Inline>,
    pub outside: Vec<Outside>,
    /// Problems that stop the send.
    pub errors: Vec<String>,
    /// The drafts of `errors`, each with its problem. They stay off GitHub.
    #[serde(skip)]
    pub stuck: Vec<(i64, String)>,
}

/// A problem of one draft: it stops the send, and the draft stays off GitHub.
fn fail(p: &mut Plan, draft: i64, msg: String) {
    p.stuck.push((draft, msg.clone()));
    p.errors.push(msg);
}

impl Plan {
    /// A fingerprint of the plan. The send stops if the plan changed after you saw it.
    pub fn token(&self) -> String {
        let text = json!([self.head, self.inline, self.outside, self.errors]).to_string();
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in text.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        format!("{h:016x}")
    }

    pub fn drafts(&self) -> Vec<i64> {
        let mut ids: Vec<i64> = self.inline.iter().map(|c| c.draft).collect();
        ids.extend(self.outside.iter().map(|c| c.draft));
        ids
    }
}

/// The file of the model that a draft names: by the base path on the old side,
/// by the head path on the new side.
pub fn model_file<'a>(m: &'a DiffModel, path: &str, old: bool) -> Option<&'a ChangedFile> {
    m.files.iter().find(|f| {
        let p = if old { &f.old_path } else { &f.new_path };
        p.as_deref() == Some(path)
    })
}

/// Line `n` (from 1) of one side of a file, as text.
pub fn line_text(f: &ChangedFile, old: bool, n: u32) -> Option<String> {
    let lines = if old { &f.old } else { &f.new };
    let l = lines.get((n as usize).checked_sub(1)?)?;
    let l = l.strip_suffix(b"\n").unwrap_or(l);
    let l = l.strip_suffix(b"\r").unwrap_or(l);
    Some(String::from_utf8_lossy(l).into_owned())
}

/// True when the window's text of a line is `ours`. The window's row text can keep
/// a `\r` (a file with CRLF line ends), and the browser can turn it into `\n`.
pub fn same_line(window: &str, ours: &str) -> bool {
    window.trim_end_matches(['\r', '\n']) == ours.trim_end_matches(['\r', '\n'])
}

/// The text of lines `from` to `to` of one side, joined with "\n".
pub fn block_text(f: &ChangedFile, old: bool, from: u32, to: u32) -> Option<String> {
    let lines: Option<Vec<String>> = (from..=to).map(|n| line_text(f, old, n)).collect();
    lines.map(|l| l.join("\n"))
}

/// Lines of code that two drafts place: the lines, and up to 2 lines around them.
#[derive(Debug, Clone, PartialEq)]
pub struct Anchor {
    pub line: u32,
    pub start_line: Option<u32>,
    pub text: String,
    pub before: String,
    pub after: String,
}

/// How many lines around a draft the anchor keeps.
const AROUND: u32 = 2;

fn lines_of(f: &ChangedFile, old: bool) -> u32 {
    (if old { f.old.len() } else { f.new.len() }) as u32
}

/// The anchor of lines `from` to `to` of one side.
pub fn anchor_at(f: &ChangedFile, old: bool, from: u32, to: u32) -> Option<Anchor> {
    let text = block_text(f, old, from, to)?;
    let lo = from.saturating_sub(AROUND).max(1);
    let hi = (to + AROUND).min(lines_of(f, old));
    let before = if lo < from { block_text(f, old, lo, from - 1)? } else { String::new() };
    let after = if to < hi { block_text(f, old, to + 1, hi)? } else { String::new() };
    Some(Anchor {
        line: to,
        start_line: (from < to).then_some(from),
        text,
        before,
        after,
    })
}

/// Where a block of lines is now, after the code changed. The lines must be the
/// same, in order, with the same spaces. With the code around the block stored:
/// - the code above or the code below must be the same too; a place where both are
///   the same wins over a place where one is; two places with the same best match
///   are a tie, and a tie is stale;
/// - else `None`: the draft is stale. So a common line (`}`, a blank line) never
///   jumps to other code.
///
/// With no code around stored (a draft from before the app kept it), only the one
/// place in the file with these lines counts.
pub fn move_anchor(
    f: &ChangedFile,
    old: bool,
    text: &str,
    before: Option<&str>,
    after: Option<&str>,
) -> Option<Anchor> {
    let size = text.split('\n').count() as u32;
    let count = lines_of(f, old);
    if size == 0 || size > count {
        return None;
    }
    let hits: Vec<Anchor> = (1..=count - size + 1)
        .filter_map(|s| anchor_at(f, old, s, s + size - 1))
        .filter(|a| a.text == text)
        .collect();
    if before.is_none() && after.is_none() {
        return (hits.len() == 1).then(|| hits[0].clone());
    }
    let score = |a: &Anchor| {
        u32::from(before.is_some_and(|b| a.before == b)) + u32::from(after.is_some_and(|x| a.after == x))
    };
    // The best score must be at one place only: a tie in repeated code is stale.
    let best = hits.iter().map(score).max().unwrap_or(0);
    let top: Vec<&Anchor> = hits.iter().filter(|a| score(a) == best).collect();
    (best > 0 && top.len() == 1).then(|| top[0].clone())
}

/// The index of the row that shows line `n` of one side in GitHub's diff.
fn gh_row(rows: &[Row], old: bool, n: u32) -> Option<usize> {
    rows.iter().position(|r| {
        if old {
            matches!(r.kind, '-' | ' ') && r.old == Some(n)
        } else {
            matches!(r.kind, '+' | ' ') && r.new == Some(n)
        }
    })
}

/// Checks every draft and sorts it: on a line, into the summary, or an error.
/// `old_on_github` is false in a later review round: there the old side is the code
/// of the round before, which GitHub's diff does not show, so an old-side comment
/// goes into the summary.
pub fn plan(m: &DiffModel, gh: &[GhFile], drafts: &[DraftRow], old_on_github: bool) -> Plan {
    let mut p = Plan {
        head: m.head.clone(),
        inline: vec![],
        outside: vec![],
        errors: vec![],
        stuck: vec![],
    };
    for d in drafts {
        let text = d.text.trim().to_owned();
        if text.is_empty() {
            fail(&mut p, d.id, format!("Draft {} is empty.", d.id));
            continue;
        }
        let (path, line) = match (&d.path, d.line) {
            (Some(path), Some(line)) => (path, line),
            _ => {
                p.outside.push(Outside {
                    draft: d.id,
                    at: d.path.clone().unwrap_or_default(),
                    reason: "no line".into(),
                    text,
                });
                continue;
            }
        };
        let old = d.side.as_deref() == Some("old");
        let start = d.start_line.unwrap_or(line);
        let at = if start < line {
            format!("{path}:{start}-{line}")
        } else {
            format!("{path}:{line}")
        };
        if d.stale {
            fail(&mut p, d.id, format!(
                "{at}: the code of this comment changed after you wrote it. Place it again or delete it."
            ));
            continue;
        }
        let Some(f) = model_file(m, path, old) else {
            fail(&mut p, d.id, format!("{at}: the PR does not change this file."));
            continue;
        };
        if start == 0 || start > line {
            fail(&mut p, d.id, format!("{at}: the line range is not valid."));
            continue;
        }
        let ours: Option<Vec<String>> = (start..=line).map(|n| line_text(f, old, n)).collect();
        let Some(ours) = ours else {
            fail(&mut p, d.id, format!("{at}: the file has no such line at the reviewed head."));
            continue;
        };
        // The draft must belong to this head: a draft from an older head has an old number.
        if d.head.as_deref() != Some(m.head.as_str()) {
            fail(&mut p, d.id, format!(
                "{at}: this comment belongs to an older commit. Place it again or delete it."
            ));
            continue;
        }
        // The comment was written on some text. That text must still be at its lines.
        // A draft with no text is from before the app kept it: it cannot be checked.
        if d.line_text.as_deref() != Some(ours.join("\n").as_str()) {
            fail(&mut p, d.id, format!(
                "{at}: the code of this comment changed after you wrote it. Place it again or delete it."
            ));
            continue;
        }
        let outside = |reason: &str| Outside {
            draft: d.id,
            at: at.clone(),
            reason: reason.into(),
            text: text.clone(),
        };
        if old && !old_on_github {
            p.outside.push(outside("the old side of this round is not on GitHub"));
            continue;
        }
        let Some(g) = gh.iter().find(|g| g.path == f.path()) else {
            p.outside.push(outside("GitHub's diff does not show this file"));
            continue;
        };
        let rows: Option<Vec<usize>> = (start..=line).map(|n| gh_row(&g.rows, old, n)).collect();
        let Some(rows) = rows else {
            p.outside.push(outside("the line is outside GitHub's diff"));
            continue;
        };
        if g.rows[rows[0]..=rows[rows.len() - 1]]
            .iter()
            .any(|r| r.kind == '@')
        {
            p.outside.push(outside("the range is in two parts of GitHub's diff"));
            continue;
        }
        let differ = rows
            .iter()
            .zip(&ours)
            .zip(start..)
            .find(|((&k, t), _)| g.rows[k].text.trim_end_matches('\r') != t.as_str());
        if let Some((_, n)) = differ {
            fail(&mut p, d.id, format!(
                "{path}:{n}: GitHub shows other text on this line. Nothing is sent."
            ));
            continue;
        }
        p.inline.push(Inline {
            draft: d.id,
            path: f.path().to_owned(),
            side: if old { "LEFT" } else { "RIGHT" }.into(),
            line,
            start_line: (start < line).then_some(start),
            text,
        });
    }
    p
}

pub const EVENTS: [&str; 3] = ["COMMENT", "APPROVE", "REQUEST_CHANGES"];

/// The review summary as sent: your text, then the comments that are not on a line.
pub fn body(p: &Plan, summary: &str) -> String {
    let mut parts: Vec<String> = vec![];
    if !summary.trim().is_empty() {
        parts.push(summary.trim().to_owned());
    }
    for o in &p.outside {
        parts.push(if o.at.is_empty() {
            o.text.clone()
        } else {
            format!("`{}`: {}", o.at, o.text)
        });
    }
    parts.join("\n\n")
}

/// The JSON for `POST /repos/{repo}/pulls/{n}/reviews`.
pub fn payload(p: &Plan, event: &str, summary: &str) -> Result<Value, String> {
    if !p.errors.is_empty() {
        return Err(p.errors.join("\n"));
    }
    if !EVENTS.contains(&event) {
        return Err(format!("Unknown review type: {event}"));
    }
    let body = body(p, summary);
    if event == "REQUEST_CHANGES" && body.is_empty() {
        return Err("Request changes needs a summary.".into());
    }
    if event == "COMMENT" && body.is_empty() && p.inline.is_empty() {
        return Err("There is nothing to send.".into());
    }
    let comments: Vec<Value> = p
        .inline
        .iter()
        .map(|c| {
            let mut v = json!({ "path": c.path, "line": c.line, "side": c.side, "body": c.text });
            if let Some(s) = c.start_line {
                v["start_line"] = json!(s);
                v["start_side"] = json!(c.side);
            }
            v
        })
        .collect();
    Ok(json!({ "commit_id": p.head, "event": event, "body": body, "comments": comments }))
}

/// GitHub's name for the state of a sent review of each type.
fn state_of(event: &str) -> &'static str {
    match event {
        "APPROVE" => "APPROVED",
        "REQUEST_CHANGES" => "CHANGES_REQUESTED",
        _ => "COMMENTED",
    }
}

/// The reviews on GitHub that can be this send already: by `login`, at the same
/// commit, of the same type, with the same summary. Returns their ids and links.
/// The caller then compares the line comments (`same_comments`).
pub fn sent_candidates(reviews: &Value, login: &str, payload: &Value) -> Vec<(u64, String)> {
    let event = payload["event"].as_str().unwrap_or("");
    reviews
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| {
            r["user"]["login"].as_str() == Some(login)
                && r["commit_id"] == payload["commit_id"]
                && r["state"].as_str() == Some(state_of(event))
                && r["body"].as_str().unwrap_or("") == payload["body"].as_str().unwrap_or("")
        })
        .filter_map(|r| Some((r["id"].as_u64()?, r["html_url"].as_str().unwrap_or("").to_owned())))
        .collect()
}

/// True when the line comments of a review on GitHub are the comments of `payload`:
/// the same path, line, side and text, as a set.
pub fn same_comments(comments: &Value, payload: &Value) -> bool {
    let key = |path: &Value, line: &Value, side: &Value, body: &Value| {
        format!("{}|{}|{}|{}", path.as_str().unwrap_or(""), line, side.as_str().unwrap_or("RIGHT"), body.as_str().unwrap_or(""))
    };
    let mut got: Vec<String> = comments
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            // GitHub sets `line` to null on an outdated comment; its first line counts.
            let line = if c["line"].is_null() { &c["original_line"] } else { &c["line"] };
            key(&c["path"], line, &c["side"], &c["body"])
        })
        .collect();
    let mut want: Vec<String> = payload["comments"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| key(&c["path"], &c["line"], &c["side"], &c["body"]))
        .collect();
    got.sort();
    want.sort();
    got == want
}

// ---------- the pending review on GitHub (SPEC 11.8.1) ----------

/// A comment of the pending review on GitHub.
#[derive(Debug, Clone, PartialEq)]
pub struct GhComment {
    pub id: String,
    pub body: String,
    /// A reply to a thread. It stays on GitHub as it is.
    pub reply: bool,
}

/// One change that a sync makes.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncOp {
    /// The draft has no comment on GitHub: add one.
    Add { draft: i64, c: Inline },
    /// The text of the draft changed: edit its comment.
    Edit { draft: i64, gh_id: String, c: Inline },
    /// The draft moved to other lines: delete its comment and add a new one.
    Move { draft: i64, gh_id: String, c: Inline },
    /// You edited the comment on the website, and the draft did not change since
    /// the last sync: the draft takes the new text.
    Pull { draft: i64, body: String, c: Inline },
    /// Delete a comment: its draft left the plan (`draft`), or you deleted the draft.
    Drop { draft: Option<i64>, gh_id: String },
    /// The draft's comment is gone from GitHub: forget its id.
    Forget { draft: i64 },
    /// A deleted draft's comment is gone from GitHub already.
    Unmark { gh_id: String },
    /// A comment that no draft has, written on the website: it becomes a draft.
    Import { gh_id: String },
}

/// The place of a comment, as it went up: path, side, first and last line.
fn place(c: &Inline) -> String {
    format!("{}|{}|{}|{}", c.path, c.side, c.start_line.unwrap_or(c.line), c.line)
}

/// What went up for a comment: its place, then its text.
pub fn sig(c: &Inline) -> String {
    format!("{}\n{}", place(c), c.text)
}

/// The text of a comment as GitHub keeps it, to compare: GitHub can turn line ends
/// into CRLF and keeps no spaces at the ends.
fn norm(body: &str) -> String {
    body.replace("\r\n", "\n").trim().to_owned()
}

/// The changes that make the pending review match the plan. `sync` is the GitHub side
/// of each draft, `comments` the pending review on GitHub, `gone` the comments of
/// deleted drafts. Pure: the hub runs the changes in this order.
pub fn sync_ops(p: &Plan, sync: &[crate::store::DraftSync], comments: &[GhComment], gone: &[String]) -> Vec<SyncOp> {
    use std::collections::{HashMap, HashSet};
    let live: HashMap<&str, &str> = comments
        .iter()
        .filter(|c| !c.reply)
        .map(|c| (c.id.as_str(), c.body.as_str()))
        .collect();
    let inline: HashMap<i64, &Inline> = p.inline.iter().map(|c| (c.draft, c)).collect();
    let (mut drops, mut moves, mut edits, mut adds, mut rest) = (vec![], vec![], vec![], vec![], vec![]);
    let mut claimed: HashSet<&str> = HashSet::new();
    for d in sync {
        // Two drafts with one comment id cannot be: the second has none.
        let gh = d.gh_id.as_deref().filter(|g| claimed.insert(g));
        let on = gh.filter(|g| live.contains_key(g));
        match (inline.get(&d.id), on) {
            (Some(c), Some(g)) => {
                let (was_place, was_text) = d.gh_sig.as_deref().and_then(|x| x.split_once('\n')).unwrap_or(("", ""));
                let body = live[g];
                if was_place != place(c) {
                    moves.push(SyncOp::Move { draft: d.id, gh_id: g.to_owned(), c: (*c).clone() });
                } else if norm(body) == norm(&c.text) {
                    if d.gh_sig.as_deref() != Some(sig(c).as_str()) {
                        // The same text, and an old record of it: only the record changes.
                        edits.push(SyncOp::Pull { draft: d.id, body: c.text.clone(), c: (*c).clone() });
                    }
                } else if norm(was_text) == norm(&c.text) {
                    edits.push(SyncOp::Pull { draft: d.id, body: norm(body), c: (*c).clone() });
                } else {
                    edits.push(SyncOp::Edit { draft: d.id, gh_id: g.to_owned(), c: (*c).clone() });
                }
            }
            (Some(c), None) => adds.push(SyncOp::Add { draft: d.id, c: (*c).clone() }),
            (None, Some(g)) => drops.push(SyncOp::Drop { draft: Some(d.id), gh_id: g.to_owned() }),
            (None, None) => {
                if d.gh_id.is_some() {
                    rest.push(SyncOp::Forget { draft: d.id });
                }
            }
        }
    }
    let gone: HashSet<&str> = gone.iter().map(String::as_str).collect();
    for c in comments.iter().filter(|c| !c.reply && !claimed.contains(c.id.as_str())) {
        if gone.contains(c.id.as_str()) {
            drops.push(SyncOp::Drop { draft: None, gh_id: c.id.clone() });
        } else {
            rest.push(SyncOp::Import { gh_id: c.id.clone() });
        }
    }
    for g in gone.iter().filter(|g| !live.contains_key(*g)) {
        rest.push(SyncOp::Unmark { gh_id: (*g).to_owned() });
    }
    let mut ops = drops;
    ops.extend(moves);
    ops.extend(edits);
    ops.extend(adds);
    ops.extend(rest);
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    const GH: &str = "diff --git a/src/a.rs b/src/a.rs
index 111..222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,4 +1,4 @@
 one
-two
+TWO
 three
 four
@@ -20,2 +20,3 @@ fn x() {
 twenty
+new
 twentyone
diff --git a/old.rs b/moved.rs
similarity index 90%
rename from old.rs
rename to moved.rs
--- a/old.rs
+++ b/moved.rs
@@ -1,1 +1,1 @@
-x
+y
diff --git a/gone.rs b/gone.rs
deleted file mode 100644
--- a/gone.rs
+++ /dev/null
@@ -1 +0,0 @@
-bye
diff --git a/img.png b/img.png
new file mode 100644
Binary files /dev/null and b/img.png differ
";

    fn lines(s: &[&str]) -> Vec<Vec<u8>> {
        s.iter().map(|l| format!("{l}\n").into_bytes()).collect()
    }

    fn model() -> DiffModel {
        let mut a_old: Vec<&str> = vec!["one", "two", "three", "four"];
        let mut a_new: Vec<&str> = vec!["one", "TWO", "three", "four"];
        for i in 5..20 {
            let s: &'static str = Box::leak(format!("l{i}").into_boxed_str());
            a_old.push(s);
            a_new.push(s);
        }
        a_old.extend(["twenty", "twentyone"]);
        a_new.extend(["twenty", "new", "twentyone"]);
        let mut removed = vec![false; a_old.len()];
        removed[1] = true;
        let mut added = vec![false; a_new.len()];
        added[1] = true;
        added[20] = true;
        let file = |old: Option<&str>, new: Option<&str>, o: &[&str], n: &[&str]| ChangedFile {
            old_path: old.map(str::to_owned),
            new_path: new.map(str::to_owned),
            binary: false,
            submodule: false,
            old: lines(o),
            new: lines(n),
            removed: vec![true; o.len()],
            added: vec![true; n.len()],
            additions: n.len(),
            deletions: o.len(),
        };
        let mut a = file(Some("src/a.rs"), Some("src/a.rs"), &a_old, &a_new);
        a.removed = removed;
        a.added = added;
        DiffModel {
            base: "base".into(),
            head: "head1".into(),
            files: vec![
                a,
                file(Some("old.rs"), Some("moved.rs"), &["x"], &["y"]),
                file(Some("gone.rs"), None, &["bye"], &[]),
            ],
        }
    }

    /// A draft with the text of its lines in the test model, as the app stores it.
    fn draft(id: i64, path: &str, side: &str, start: Option<u32>, line: u32) -> DraftRow {
        let old = side == "old";
        let text = model_file(&model(), path, old).and_then(|f| block_text(f, old, start.unwrap_or(line), line));
        DraftRow {
            head: Some("head1".into()),
            line_text: text,
            id,
            path: Some(path.into()),
            side: Some(side.into()),
            line: Some(line),
            start_line: start,
            text: format!("comment {id}"),
            agent: false,
            stale: false,
            before: None,
            after: None,
        }
    }

    #[test]
    fn the_github_diff_splits_into_files() {
        let files = parse_pr_diff(GH);
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["src/a.rs", "moved.rs", "gone.rs", "img.png"]);
        assert!(files[3].rows.is_empty());
    }

    #[test]
    fn quoted_paths_are_decoded() {
        assert_eq!(unquote("\"caf\\303\\251.rs\""), "café.rs");
        assert_eq!(unquote("\"a\\\"b\\\\c\\td.rs\""), "a\"b\\c\td.rs");
        assert_eq!(unquote("plain/path.rs"), "plain/path.rs");
        // git never writes `\4xx`. The old code computed 4 * 64 in a u8 here and panicked.
        assert_eq!(unquote("\"\\400.rs\""), "\\400.rs");
        assert_eq!(unquote("\"\\777\""), "\\777");
        let diff = "diff --git \"a/caf\\303\\251.rs\" \"b/caf\\303\\251.rs\"\n--- \"a/caf\\303\\251.rs\"\n+++ \"b/caf\\303\\251.rs\"\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/my file.rs b/my file.rs\n--- a/my file.rs\t\n+++ b/my file.rs\t\n@@ -1 +1 @@\n-a\n+b\n";
        let paths: Vec<String> = parse_pr_diff(diff).into_iter().map(|f| f.path).collect();
        assert_eq!(paths, ["café.rs", "my file.rs"]);
    }

    #[test]
    fn lines_in_the_github_diff_go_on_the_line() {
        let gh = parse_pr_diff(GH);
        let p = plan(
            &model(),
            &gh,
            &[
                draft(1, "src/a.rs", "new", None, 2),
                draft(2, "src/a.rs", "old", None, 2),
                draft(3, "src/a.rs", "new", Some(20), 22),
                draft(4, "old.rs", "old", None, 1),
                draft(5, "gone.rs", "old", None, 1),
                draft(6, "src/a.rs", "new", None, 4),
            ],
            true,
        );
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        assert!(p.outside.is_empty(), "{:?}", p.outside);
        let got: Vec<(&str, &str, u32, Option<u32>)> = p
            .inline
            .iter()
            .map(|c| (c.path.as_str(), c.side.as_str(), c.line, c.start_line))
            .collect();
        assert_eq!(
            got,
            [
                ("src/a.rs", "RIGHT", 2, None),
                ("src/a.rs", "LEFT", 2, None),
                ("src/a.rs", "RIGHT", 22, Some(20)),
                ("moved.rs", "LEFT", 1, None),
                ("gone.rs", "LEFT", 1, None),
                ("src/a.rs", "RIGHT", 4, None),
            ]
        );
    }

    #[test]
    fn lines_outside_the_github_diff_go_into_the_summary() {
        let gh = parse_pr_diff(GH);
        let mut general = draft(4, "", "new", None, 1);
        general.path = None;
        general.line = None;
        let p = plan(
            &model(),
            &gh,
            &[
                draft(1, "src/a.rs", "new", None, 10),
                draft(2, "src/a.rs", "new", Some(3), 21),
                draft(3, "src/a.rs", "new", Some(4), 6),
                general,
            ],
            true,
        );
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        assert!(p.inline.is_empty());
        let reasons: Vec<&str> = p.outside.iter().map(|o| o.reason.as_str()).collect();
        assert_eq!(
            reasons,
            [
                "the line is outside GitHub's diff",
                "the line is outside GitHub's diff",
                "the line is outside GitHub's diff",
                "no line",
            ]
        );
        let v = payload(&p, "COMMENT", "Looks good.").unwrap();
        let body = v["body"].as_str().unwrap();
        assert!(body.starts_with("Looks good.\n\n`src/a.rs:10`: comment 1"), "{body}");
        assert!(body.ends_with("\n\ncomment 4"), "{body}");
        assert_eq!(v["comments"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn a_range_over_two_hunks_goes_into_the_summary() {
        // Two hunks that touch: every line is shown, but in two parts.
        let gh = parse_pr_diff(&GH.replace(" three\n four\n", " three\n@@ -4,1 +4,1 @@\n four\n"));
        let p = plan(&model(), &gh, &[draft(1, "src/a.rs", "new", Some(2), 4)], true);
        assert_eq!(p.outside[0].reason, "the range is in two parts of GitHub's diff");
    }

    #[test]
    fn a_later_round_puts_old_side_comments_into_the_summary() {
        let gh = parse_pr_diff(GH);
        let p = plan(&model(), &gh, &[draft(1, "src/a.rs", "old", None, 2), draft(2, "src/a.rs", "new", None, 2)], false);
        assert_eq!(p.outside.len(), 1);
        assert_eq!(p.outside[0].reason, "the old side of this round is not on GitHub");
        assert_eq!(p.inline.len(), 1);
    }

    #[test]
    fn other_text_on_github_stops_the_send() {
        let gh = parse_pr_diff(&GH.replace("+TWO", "+TW0"));
        let p = plan(&model(), &gh, &[draft(1, "src/a.rs", "new", None, 2)], true);
        assert_eq!(p.errors.len(), 1, "{:?}", p.errors);
        assert!(payload(&p, "COMMENT", "x").is_err());
    }

    #[test]
    fn stale_drafts_stop_the_send() {
        let gh = parse_pr_diff(GH);
        let p = plan(
            &model(),
            &gh,
            &[
                draft(1, "nope.rs", "new", None, 1),
                draft(2, "src/a.rs", "new", None, 99),
                draft(3, "src/a.rs", "new", Some(5), 4),
            ],
            true,
        );
        assert_eq!(p.errors.len(), 3, "{:?}", p.errors);
        assert!(p.inline.is_empty() && p.outside.is_empty());
    }

    #[test]
    fn the_payload_is_one_review_at_the_head() {
        let gh = parse_pr_diff(GH);
        let p = plan(&model(), &gh, &[draft(1, "src/a.rs", "new", Some(20), 22)], true);
        let v = payload(&p, "REQUEST_CHANGES", "Please fix.").unwrap();
        assert_eq!(
            v,
            json!({
                "commit_id": "head1",
                "event": "REQUEST_CHANGES",
                "body": "Please fix.",
                "comments": [{ "path": "src/a.rs", "line": 22, "side": "RIGHT", "body": "comment 1",
                               "start_line": 20, "start_side": "RIGHT" }],
            })
        );
        assert!(payload(&p, "REQUEST_CHANGES", "").is_err());
        assert!(payload(&p, "APPROVE", "").is_ok());
        assert!(payload(&p, "MERGE", "").is_err());
        let empty = plan(&model(), &gh, &[], true);
        assert!(payload(&empty, "COMMENT", " ").is_err());
        assert!(payload(&empty, "APPROVE", "").is_ok());
    }

    #[test]
    fn a_review_that_github_already_has_is_found() {
        let payload = json!({ "commit_id": "head1", "event": "REQUEST_CHANGES", "body": "Please fix.",
            "comments": [{ "path": "a.rs", "line": 3, "side": "RIGHT", "body": "x" }, { "path": "a.rs", "line": 1, "side": "LEFT", "body": "y" }] });
        let review = |id: u64, login: &str, commit: &str, state: &str, body: &str| json!({
            "id": id, "user": { "login": login }, "commit_id": commit, "state": state, "body": body,
            "html_url": format!("https://github.com/o/r/pull/1#pullrequestreview-{id}") });
        let reviews = json!([
            review(1, "me", "head1", "CHANGES_REQUESTED", "Please fix."),
            review(2, "other", "head1", "CHANGES_REQUESTED", "Please fix."),
            review(3, "me", "head0", "CHANGES_REQUESTED", "Please fix."),
            review(4, "me", "head1", "COMMENTED", "Please fix."),
            review(5, "me", "head1", "CHANGES_REQUESTED", "Other text."),
        ]);
        let found = sent_candidates(&reviews, "me", &payload);
        assert_eq!(found, [(1, "https://github.com/o/r/pull/1#pullrequestreview-1".to_owned())]);
        // The same comments in another order: the same review.
        let same = json!([{ "path": "a.rs", "line": 1, "side": "LEFT", "body": "y" }, { "path": "a.rs", "line": 3, "side": "RIGHT", "body": "x" }]);
        assert!(same_comments(&same, &payload));
        // An outdated comment has no line; its original line counts.
        let outdated = json!([{ "path": "a.rs", "line": null, "original_line": 1, "side": "LEFT", "body": "y" }, { "path": "a.rs", "line": 3, "side": "RIGHT", "body": "x" }]);
        assert!(same_comments(&outdated, &payload));
        // One comment more or less: another review, so the send goes on.
        assert!(!same_comments(&json!([{ "path": "a.rs", "line": 3, "side": "RIGHT", "body": "x" }]), &payload));
        assert!(!same_comments(&json!([]), &payload));
    }

    #[test]
    fn a_comment_on_changed_code_stops_the_send() {
        let gh = parse_pr_diff(GH);
        let m = model();
        let f = &m.files[0];
        // Written on line 2, and line 2 still has that text: it goes.
        let mut d = draft(1, "src/a.rs", "new", None, 2);
        d.line_text = block_text(f, false, 2, 2);
        assert_eq!(plan(&m, &gh, std::slice::from_ref(&d), true).inline.len(), 1);
        // The text at line 2 is not the text it was written on: it stops.
        d.line_text = Some("something else".into());
        let p = plan(&m, &gh, &[d.clone()], true);
        assert!(p.errors[0].contains("changed after you wrote it"), "{:?}", p.errors);
        // A draft from an older head stops.
        d.line_text = block_text(f, false, 2, 2);
        d.head = Some("head0".into());
        assert!(plan(&m, &gh, &[d.clone()], true).errors[0].contains("older commit"));
        d.head = Some("head1".into());
        // A stale draft stops too.
        d.line_text = None;
        d.stale = true;
        assert_eq!(plan(&m, &gh, &[d], true).errors.len(), 1);
    }

    #[test]
    fn a_crlf_line_from_the_window_is_the_same_line() {
        assert!(same_line("let a = 1;\r", "let a = 1;"));
        assert!(same_line("let a = 1;\n", "let a = 1;"));
        assert!(!same_line("let a = 2;", "let a = 1;"));
        assert!(!same_line(" let a = 1;", "let a = 1;"));
    }

    #[test]
    fn an_anchor_moves_only_to_the_same_place() {
        let m = model();
        let f = &m.files[0];
        // "twenty" / "new" is unique: it moves from a far guess.
        let a = anchor_at(f, false, 20, 21).unwrap();
        assert_eq!(a.before, "l18\nl19");
        assert_eq!(move_anchor(f, false, &a.text, Some(&a.before), Some(&a.after)).map(|x| x.line), Some(21));
        // The lines are gone: stale.
        assert_eq!(move_anchor(f, false, "twenty\nnot here", None, None), None);
        // Spaces count.
        assert_eq!(move_anchor(f, false, " l7", None, None), None);
    }

    #[test]
    fn a_common_line_does_not_jump_to_another_copy() {
        // A file with "}" on lines 3 and 6.
        let lines = |s: &[&str]| s.iter().map(|l| format!("{l}\n").into_bytes()).collect::<Vec<_>>();
        let mut f = model().files[0].clone();
        f.new = lines(&["fn a() {", "  x();", "}", "fn b() {", "  y();", "}"]);
        f.added = vec![false; 6];
        let a = anchor_at(&f, false, 3, 3).unwrap();
        assert_eq!((a.text.as_str(), a.before.as_str(), a.after.as_str()), ("}", "fn a() {\n  x();", "fn b() {\n  y();"));
        // Same code: it stays.
        assert_eq!(move_anchor(&f, false, &a.text, Some(&a.before), Some(&a.after)).map(|x| x.line), Some(3));
        // fn a is gone. The only "}" left is the one of fn b: other code, so stale.
        f.new = lines(&["fn b() {", "  y();", "}"]);
        assert_eq!(move_anchor(&f, false, &a.text, Some(&a.before), Some(&a.after)), None);
        // With no stored context (an old draft), a unique copy is enough.
        assert_eq!(move_anchor(&f, false, "  y();", None, None).map(|x| x.line), Some(2));
        f.new = lines(&["}", "}"]);
        assert_eq!(move_anchor(&f, false, "}", None, None), None);
        // Two copies of the same code: a tie, so stale.
        f.new = lines(&["fn a() {", "  x();", "}", "fn b() {", "  y();", "}", "fn a() {", "  x();", "}", "fn b() {", "  y();", "}"]);
        assert_eq!(move_anchor(&f, false, "}", Some("fn a() {\n  x();"), Some("fn b() {\n  y();")), None);
        // A line above fn a's "}" changed, but the code below is the same: it moves.
        f.new = lines(&["// new", "fn a() {", "  x2();", "}", "fn b() {", "  y();", "}"]);
        assert_eq!(move_anchor(&f, false, "}", Some("fn a() {\n  x();"), Some("fn b() {\n  y();")).map(|x| x.line), Some(4));
    }

    #[test]
    fn the_token_changes_with_the_plan() {
        let gh = parse_pr_diff(GH);
        let a = plan(&model(), &gh, &[draft(1, "src/a.rs", "new", None, 2)], true);
        let mut d = draft(1, "src/a.rs", "new", None, 2);
        d.text = "edited".into();
        let b = plan(&model(), &gh, &[d], true);
        assert_eq!(a.token(), a.clone().token());
        assert_ne!(a.token(), b.token());
    }

    // ---------- the sync planner ----------

    use crate::store::DraftSync;
    use std::collections::BTreeMap;

    fn inl(draft: i64, path: &str, line: u32, text: &str) -> Inline {
        Inline { draft, path: path.into(), side: "RIGHT".into(), line, start_line: None, text: text.into() }
    }

    fn plan_of(inline: Vec<Inline>) -> Plan {
        Plan { head: "h".into(), inline, outside: vec![], errors: vec![], stuck: vec![] }
    }

    fn ds(id: i64, gh: Option<&str>, sig: Option<String>) -> DraftSync {
        DraftSync { id, gh_id: gh.map(str::to_owned), gh_sig: sig, gh_error: None }
    }

    fn com(id: &str, body: &str) -> GhComment {
        GhComment { id: id.into(), body: body.into(), reply: false }
    }

    #[test]
    fn a_new_draft_adds_a_comment_and_a_synced_one_does_nothing() {
        let a = inl(1, "a.rs", 3, "nit");
        let b = inl(2, "a.rs", 9, "why?");
        let p = plan_of(vec![a.clone(), b.clone()]);
        let ops = sync_ops(&p, &[ds(1, Some("C1"), Some(sig(&a))), ds(2, None, None)], &[com("C1", "nit")], &[]);
        assert_eq!(ops, vec![SyncOp::Add { draft: 2, c: b }]);
    }

    #[test]
    fn an_edit_a_move_and_a_draft_that_left_the_plan() {
        let a = inl(1, "a.rs", 3, "nit: new text");
        let b = inl(2, "a.rs", 12, "moved");
        let p = plan_of(vec![a.clone(), b.clone()]);
        let sync = [
            ds(1, Some("C1"), Some(sig(&inl(1, "a.rs", 3, "nit")))),
            ds(2, Some("C2"), Some(sig(&inl(2, "a.rs", 9, "moved")))),
            ds(3, Some("C3"), Some("x\ny".into())),
        ];
        let ops = sync_ops(&p, &sync, &[com("C1", "nit"), com("C2", "moved"), com("C3", "stale one")], &[]);
        assert_eq!(
            ops,
            vec![
                SyncOp::Drop { draft: Some(3), gh_id: "C3".into() },
                SyncOp::Move { draft: 2, gh_id: "C2".into(), c: b },
                SyncOp::Edit { draft: 1, gh_id: "C1".into(), c: a },
            ]
        );
    }

    #[test]
    fn a_website_edit_comes_back_and_a_website_comment_becomes_a_draft() {
        let a = inl(1, "a.rs", 3, "nit");
        let p = plan_of(vec![a.clone()]);
        let sync = [ds(1, Some("C1"), Some(sig(&a)))];
        let comments = [com("C1", "nit, and more\r\n"), com("C9", "from the site"), GhComment { reply: true, ..com("C8", "a reply") }];
        let ops = sync_ops(&p, &sync, &comments, &["C7".into()]);
        assert_eq!(
            ops,
            vec![
                SyncOp::Pull { draft: 1, body: "nit, and more".into(), c: a },
                SyncOp::Import { gh_id: "C9".into() },
                SyncOp::Unmark { gh_id: "C7".into() },
            ]
        );
    }

    #[test]
    fn a_deleted_draft_deletes_its_comment_and_a_lost_comment_goes_up_again() {
        let a = inl(1, "a.rs", 3, "nit");
        let p = plan_of(vec![a.clone()]);
        let ops = sync_ops(&p, &[ds(1, Some("C1"), Some(sig(&a))), ds(2, Some("C5"), None)], &[com("C2", "deleted draft")], &["C2".into()]);
        assert_eq!(
            ops,
            vec![
                SyncOp::Drop { draft: None, gh_id: "C2".into() },
                SyncOp::Add { draft: 1, c: a },
                SyncOp::Forget { draft: 2 },
            ]
        );
    }

    /// A small random number source, the same for each seed.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// GitHub in the random test: a comment's body, its place, and whether it replies.
    #[derive(Clone)]
    struct Gh {
        body: String,
        at: Inline,
        reply: bool,
    }

    #[derive(Clone)]
    struct Local {
        id: i64,
        /// The place when the plan puts it on a line.
        at: Option<Inline>,
        text: String,
        gh: Option<String>,
        sig: Option<String>,
    }

    struct World {
        drafts: Vec<Local>,
        gh: BTreeMap<String, Gh>,
        gone: Vec<String>,
        next: u64,
    }

    impl World {
        fn add(&mut self, c: &Inline) -> String {
            self.next += 1;
            let id = format!("N{}", self.next);
            self.gh.insert(id.clone(), Gh { body: c.text.clone(), at: c.clone(), reply: false });
            id
        }

        fn draft(&mut self, id: i64) -> &mut Local {
            self.drafts.iter_mut().find(|d| d.id == id).unwrap()
        }

        /// Runs the changes as the hub does.
        fn run(&mut self, ops: &[SyncOp]) {
            for op in ops {
                match op {
                    SyncOp::Add { draft, c } => {
                        let id = self.add(c);
                        let d = self.draft(*draft);
                        (d.gh, d.sig) = (Some(id), Some(sig(c)));
                    }
                    SyncOp::Edit { draft, gh_id, c } => {
                        self.gh.get_mut(gh_id).unwrap().body = c.text.clone();
                        self.draft(*draft).sig = Some(sig(c));
                    }
                    SyncOp::Move { draft, gh_id, c } => {
                        assert!(!self.gh.remove(gh_id).unwrap().reply);
                        let id = self.add(c);
                        let d = self.draft(*draft);
                        (d.gh, d.sig) = (Some(id), Some(sig(c)));
                    }
                    SyncOp::Pull { draft, body, c } => {
                        let d = self.draft(*draft);
                        d.text = body.clone();
                        d.sig = Some(sig(&Inline { text: body.clone(), ..c.clone() }));
                    }
                    SyncOp::Drop { draft, gh_id } => {
                        assert!(!self.gh.remove(gh_id).unwrap().reply, "a reply was deleted");
                        match draft {
                            Some(id) => {
                                let d = self.draft(*id);
                                assert!(d.at.is_none(), "a comment of the plan was deleted");
                                (d.gh, d.sig) = (None, None);
                            }
                            None => {
                                assert!(self.gone.contains(gh_id), "a comment from the website was deleted");
                                self.gone.retain(|g| g != gh_id);
                            }
                        }
                    }
                    SyncOp::Forget { draft } => {
                        let d = self.draft(*draft);
                        (d.gh, d.sig) = (None, None);
                    }
                    SyncOp::Unmark { gh_id } => self.gone.retain(|g| g != gh_id),
                    SyncOp::Import { gh_id } => {
                        let g = self.gh[gh_id].clone();
                        let id = self.drafts.iter().map(|d| d.id).max().unwrap_or(0) + 1;
                        let at = Inline { draft: id, text: norm(&g.body), ..g.at };
                        self.drafts.push(Local { id, sig: Some(sig(&at)), text: at.text.clone(), at: Some(at), gh: Some(gh_id.clone()) });
                    }
                }
            }
        }

        fn state(&self) -> (Plan, Vec<DraftSync>, Vec<GhComment>) {
            let p = plan_of(
                self.drafts
                    .iter()
                    .filter_map(|d| d.at.clone().map(|a| Inline { draft: d.id, text: d.text.trim().to_owned(), ..a }))
                    .collect(),
            );
            let sync = self.drafts.iter().map(|d| DraftSync { id: d.id, gh_id: d.gh.clone(), gh_sig: d.sig.clone(), gh_error: None }).collect();
            let comments = self.gh.iter().map(|(id, g)| GhComment { id: id.clone(), body: g.body.clone(), reply: g.reply }).collect();
            (p, sync, comments)
        }

        fn ops(&self) -> Vec<SyncOp> {
            let (p, sync, comments) = self.state();
            sync_ops(&p, &sync, &comments, &self.gone)
        }
    }

    fn place_at(r: &mut Rng, id: i64, text: &str) -> Inline {
        let line = 1 + r.below(6) as u32;
        let range = r.below(3) == 0 && line > 1;
        Inline {
            draft: id,
            path: ["a.rs", "b.rs"][r.below(2) as usize].into(),
            side: ["RIGHT", "LEFT"][r.below(2) as usize].into(),
            line,
            start_line: range.then(|| line - 1),
            text: text.into(),
        }
    }

    /// Random drafts, comments and deleted drafts. One sync makes GitHub match the plan
    /// and loses nothing: a second sync has nothing to do, every draft of the plan has
    /// its comment with its text and place, no reply and no comment from the website
    /// is deleted, and the text of each draft is kept (or is the website's edit).
    #[test]
    fn random_syncs_match_the_plan_and_lose_nothing() {
        let texts = ["nit", "why?", "use safe_sub", "ok", " spaced "];
        for seed in 1..3000u64 {
            let mut r = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
            let mut w = World { drafts: vec![], gh: BTreeMap::new(), gone: vec![], next: 0 };
            for id in 1..=(r.below(6) as i64) {
                let text = texts[r.below(texts.len() as u64) as usize].to_owned();
                let at = (r.below(4) != 0).then(|| place_at(&mut r, id, &text));
                let mut d = Local { id, at: at.clone(), text: text.clone(), gh: None, sig: None };
                match r.below(4) {
                    // On GitHub as it went up, or with an older text or place there.
                    0 | 1 => {
                        let k = r.below(5) as usize;
                        let was = match (r.below(3), &at) {
                            (0, _) | (_, None) => place_at(&mut r, id, texts[k]),
                            (_, Some(a)) => a.clone(),
                        };
                        let gid = format!("C{id}");
                        let body = if r.below(4) == 0 { "edited on the site".to_owned() } else { was.text.clone() };
                        w.gh.insert(gid.clone(), Gh { body, at: was.clone(), reply: false });
                        (d.gh, d.sig) = (Some(gid), Some(sig(&was)));
                    }
                    // A comment id that GitHub lost.
                    2 => (d.gh, d.sig) = (Some(format!("L{id}")), Some("x\ny".into())),
                    _ => {}
                }
                w.drafts.push(d);
            }
            for k in 0..r.below(3) {
                let at = place_at(&mut r, 0, "site");
                let reply = r.below(2) == 0;
                w.gh.insert(format!("W{k}"), Gh { body: "from the site".into(), at, reply });
            }
            for k in 0..r.below(3) {
                let id = format!("G{k}");
                if r.below(2) == 0 {
                    let at = place_at(&mut r, 0, "gone");
                    w.gh.insert(id.clone(), Gh { body: "deleted draft".into(), at, reply: false });
                }
                w.gone.push(id);
            }
            let kept: Vec<String> = w.gh.keys().filter(|k| k.starts_with('W')).cloned().collect();
            let before: Vec<(i64, String, Option<String>)> = w
                .drafts
                .iter()
                .map(|d| (d.id, d.text.clone(), d.gh.as_ref().and_then(|g| w.gh.get(g)).map(|g| norm(&g.body))))
                .collect();
            let ops = w.ops();
            w.run(&ops);
            assert_eq!(w.ops(), vec![], "seed {seed}: a second sync has work");
            let (p, _, _) = w.state();
            for c in &p.inline {
                let d = w.drafts.iter().find(|d| d.id == c.draft).unwrap();
                let g = &w.gh[d.gh.as_ref().unwrap()];
                assert_eq!((norm(&g.body), place(&g.at)), (norm(&c.text), place(c)), "seed {seed}");
            }
            for k in &kept {
                assert!(w.gh.contains_key(k), "seed {seed}: {k} was deleted");
            }
            for (id, text, body) in before {
                let d = w.drafts.iter().find(|d| d.id == id).unwrap();
                assert!(d.text == text || Some(&d.text) == body.as_ref(), "seed {seed}: the text of draft {id} was lost");
            }
            assert!(w.gone.is_empty(), "seed {seed}");
        }
    }
}
