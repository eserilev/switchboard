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

fn unquote(p: &str) -> String {
    p.strip_prefix('"')
        .and_then(|p| p.strip_suffix('"'))
        .unwrap_or(p)
        .to_owned()
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

    fn draft(id: i64, path: &str, side: &str, start: Option<u32>, line: u32) -> DraftRow {
        DraftRow {
            id,
            path: Some(path.into()),
            side: Some(side.into()),
            line: Some(line),
            start_line: start,
            text: format!("comment {id}"),
            agent: false,
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
