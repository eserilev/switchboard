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
/// quotes, with C escapes: `"caf\303\251.rs"` is `café.rs`.
fn unquote(p: &str) -> String {
    let Some(inner) = p.strip_prefix('"').and_then(|p| p.strip_suffix('"')) else {
        return p.to_owned();
    };
    let b = inner.as_bytes();
    let mut out: Vec<u8> = vec![];
    let mut k = 0;
    while k < b.len() {
        if b[k] != b'\\' || k + 1 >= b.len() {
            out.push(b[k]);
            k += 1;
            continue;
        }
        let c = b[k + 1];
        let octal = |x: u8| (b'0'..=b'7').contains(&x);
        if octal(c) && k + 3 < b.len() && octal(b[k + 2]) && octal(b[k + 3]) {
            out.push((c - b'0') * 64 + (b[k + 2] - b'0') * 8 + (b[k + 3] - b'0'));
            k += 4;
            continue;
        }
        out.push(match c {
            b'n' => b'\n',
            b't' => b'\t',
            b'a' => 7,
            b'b' => 8,
            b'f' => 12,
            b'r' => b'\r',
            b'v' => 11,
            other => other,
        });
        k += 2;
    }
    String::from_utf8_lossy(&out).into_owned()
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
    };
    for d in drafts {
        let text = d.text.trim().to_owned();
        if text.is_empty() {
            p.errors.push(format!("Draft {} is empty.", d.id));
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
            p.errors.push(format!(
                "{at}: the code of this comment changed after you wrote it. Place it again or delete it."
            ));
            continue;
        }
        let Some(f) = model_file(m, path, old) else {
            p.errors.push(format!("{at}: the PR does not change this file."));
            continue;
        };
        if start == 0 || start > line {
            p.errors.push(format!("{at}: the line range is not valid."));
            continue;
        }
        let ours: Option<Vec<String>> = (start..=line).map(|n| line_text(f, old, n)).collect();
        let Some(ours) = ours else {
            p.errors.push(format!("{at}: the file has no such line at the reviewed head."));
            continue;
        };
        // The draft must belong to this head: a draft from an older head has an old number.
        if d.head.as_deref() != Some(m.head.as_str()) {
            p.errors.push(format!(
                "{at}: this comment belongs to an older commit. Place it again or delete it."
            ));
            continue;
        }
        // The comment was written on some text. That text must still be at its lines.
        // A draft with no text is from before the app kept it: it cannot be checked.
        if d.line_text.as_deref() != Some(ours.join("\n").as_str()) {
            p.errors.push(format!(
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
            p.errors.push(format!(
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
}
