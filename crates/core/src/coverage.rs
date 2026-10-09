//! The diff model and the guide check around the verified kernel (`guide-check`).
//!
//! The kernel decides. This module only feeds it and explains its answer:
//! - `build` reads the PR from git: the changed files, the changed lines, and
//!   both versions of every file. The masks come from `git diff`, but nothing
//!   here trusts them: `rebuild_ok` runs the verified rebuild check on them.
//! - `check_guide` turns the guide into spans and runs the verified `check`.
//! - `explain` says why a guide failed, for the agent to fix it. It does not
//!   decide anything, so it needs no proof.
//! - `complete` adds the step "Not in the guide" with every missed change.

use crate::repos::git;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};

/// The most lines a span may reach past the nearest changed line.
pub const PAD: usize = 20;

/// The title of the step the app adds for missed changes.
pub const MISSED: &str = "Not in the guide";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChangedFile {
    /// The path at the base, or `None` for a new file.
    pub old_path: Option<String>,
    /// The path at the head, or `None` for a deleted file.
    pub new_path: Option<String>,
    pub binary: bool,
    #[serde(skip)]
    pub old: Vec<Vec<u8>>,
    #[serde(skip)]
    pub new: Vec<Vec<u8>>,
    #[serde(skip)]
    pub removed: Vec<bool>,
    #[serde(skip)]
    pub added: Vec<bool>,
    /// The counts from `git diff --numstat`.
    pub additions: usize,
    pub deletions: usize,
}

impl ChangedFile {
    /// The path to show: the head path, or the base path for a deleted file.
    pub fn path(&self) -> &str {
        self.new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .unwrap_or("")
    }
    pub fn changed_lines(&self) -> usize {
        self.removed.iter().filter(|m| **m).count() + self.added.iter().filter(|m| **m).count()
    }
}

#[derive(Debug, Clone, Default)]
pub struct DiffModel {
    pub base: String,
    pub head: String,
    pub files: Vec<ChangedFile>,
}

/// Splits file content into lines, each with its newline.
pub fn split_lines(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut out = vec![];
    let mut start = 0;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            out.push(bytes[start..=i].to_vec());
            start = i + 1;
        }
    }
    if start < bytes.len() {
        out.push(bytes[start..].to_vec());
    }
    out
}

fn git_bytes(dir: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    Ok(out.stdout)
}

/// One entry of `git diff --numstat -z -M`.
#[derive(Debug, PartialEq)]
pub struct NumStat {
    pub additions: Option<usize>,
    pub deletions: Option<usize>,
    pub old_path: String,
    pub new_path: String,
}

/// Parses `git diff --numstat -z -M`. A rename has an empty path field, then both paths.
pub fn parse_numstat(raw: &[u8]) -> Vec<NumStat> {
    let parts: Vec<&[u8]> = raw.split(|b| *b == 0).collect();
    let mut out = vec![];
    let mut i = 0;
    while i < parts.len() {
        let head = String::from_utf8_lossy(parts[i]).into_owned();
        i += 1;
        if head.is_empty() {
            continue;
        }
        let mut f = head.splitn(3, '\t');
        let (a, d, p) = (
            f.next().unwrap_or(""),
            f.next().unwrap_or(""),
            f.next().unwrap_or(""),
        );
        let (old_path, new_path) = if p.is_empty() {
            let o = parts
                .get(i)
                .map(|s| String::from_utf8_lossy(s).into_owned())
                .unwrap_or_default();
            let n = parts
                .get(i + 1)
                .map(|s| String::from_utf8_lossy(s).into_owned())
                .unwrap_or_default();
            i += 2;
            (o, n)
        } else {
            (p.to_owned(), p.to_owned())
        };
        out.push(NumStat {
            additions: a.parse().ok(),
            deletions: d.parse().ok(),
            old_path,
            new_path,
        });
    }
    out
}

/// The changed old and new line numbers (1-based) from the hunk headers of a `-U0` diff.
pub fn parse_hunks(diff: &str) -> (Vec<usize>, Vec<usize>) {
    let mut removed = vec![];
    let mut added = vec![];
    for line in diff.lines() {
        let Some(rest) = line.strip_prefix("@@ ") else {
            continue;
        };
        let mut it = rest.split(' ');
        let range = |s: Option<&str>, sign: char| -> (usize, usize) {
            let s = s.and_then(|s| s.strip_prefix(sign)).unwrap_or("0");
            let mut p = s.split(',');
            let start: usize = p.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            let count: usize = p.next().map(|n| n.parse().unwrap_or(0)).unwrap_or(1);
            (start, count)
        };
        let (os, oc) = range(it.next(), '-');
        let (ns, nc) = range(it.next(), '+');
        removed.extend(os..os + oc);
        added.extend(ns..ns + nc);
    }
    (removed, added)
}

fn mask(len: usize, lines: &[usize]) -> Vec<bool> {
    let mut m = vec![false; len];
    for l in lines {
        if *l >= 1 && *l <= len {
            m[l - 1] = true;
        }
    }
    m
}

/// Reads the PR diff from git: every changed file, its lines, and its masks.
pub fn build(tree: &Path, base: &str, head: &str) -> Result<DiffModel, String> {
    let t0 = std::time::Instant::now();
    let raw = git_bytes(
        tree,
        &["diff", "--numstat", "-z", "-M", "--no-ext-diff", base, head],
    )?;
    let mut files = vec![];
    for n in parse_numstat(&raw) {
        let binary = n.additions.is_none();
        let exists = |rev: &str, path: &str| {
            git(tree, &["cat-file", "-e", &format!("{rev}:{path}")]).is_ok()
        };
        let old_path = exists(base, &n.old_path).then(|| n.old_path.clone());
        let new_path = exists(head, &n.new_path).then(|| n.new_path.clone());
        let mut f = ChangedFile {
            old_path,
            new_path,
            binary,
            old: vec![],
            new: vec![],
            removed: vec![],
            added: vec![],
            additions: n.additions.unwrap_or(0),
            deletions: n.deletions.unwrap_or(0),
        };
        if !binary {
            if let Some(p) = &f.old_path {
                f.old = split_lines(&git_bytes(
                    tree,
                    &["cat-file", "blob", &format!("{base}:{p}")],
                )?);
            }
            if let Some(p) = &f.new_path {
                f.new = split_lines(&git_bytes(
                    tree,
                    &["cat-file", "blob", &format!("{head}:{p}")],
                )?);
            }
            let mut args = vec![
                "diff",
                "-U0",
                "--no-color",
                "--no-ext-diff",
                "-M",
                base,
                head,
                "--",
            ];
            args.push(&n.old_path);
            if n.new_path != n.old_path {
                args.push(&n.new_path);
            }
            let text = String::from_utf8_lossy(&git_bytes(tree, &args)?).into_owned();
            let (r, a) = parse_hunks(&text);
            f.removed = mask(f.old.len(), &r);
            f.added = mask(f.new.len(), &a);
        }
        files.push(f);
    }
    let lines: usize = files.iter().map(ChangedFile::changed_lines).sum();
    tracing::info!(target: "sb::coverage", files = files.len(), lines, ms = t0.elapsed().as_millis() as u64, "diff model built");
    Ok(DiffModel {
        base: base.into(),
        head: head.into(),
        files,
    })
}

fn kernel_files(m: &DiffModel) -> Vec<guide_check::FileDiff> {
    m.files
        .iter()
        .map(|f| guide_check::FileDiff {
            old: f.old.clone(),
            new: f.new.clone(),
            removed: f.removed.clone(),
            added: f.added.clone(),
        })
        .collect()
}

/// The verified rebuild check for every file: the diff names every change.
/// A file that fails means the diff model is wrong, so the review must stop.
pub fn rebuild_ok(m: &DiffModel) -> Result<(), String> {
    for f in &m.files {
        let ok = f.removed.len() == f.old.len()
            && f.added.len() == f.new.len()
            && guide_check::kept_equal_from(&f.old, &f.removed, &f.new, &f.added, 0, 0);
        if !ok {
            return Err(format!(
                "the diff of {} does not rebuild the new file from the old one",
                f.path()
            ));
        }
    }
    Ok(())
}

/// One range of a step: lines `from..=to` of one side of one file.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Range {
    pub file: String,
    /// `new` or `old`.
    pub side: String,
    pub from: usize,
    pub to: usize,
}

/// The ranges of a step. Older guides have `file`, `side` and `lines` in place of `ranges`.
pub fn step_ranges(step: &Value) -> Vec<Range> {
    if let Some(rs) = step.get("ranges").and_then(Value::as_array) {
        return rs
            .iter()
            .filter_map(|r| {
                Some(Range {
                    file: r.get("file")?.as_str()?.to_owned(),
                    side: r
                        .get("side")
                        .and_then(Value::as_str)
                        .unwrap_or("new")
                        .to_owned(),
                    from: r.get("from")?.as_u64()? as usize,
                    to: r
                        .get("to")
                        .and_then(Value::as_u64)
                        .or_else(|| r.get("from")?.as_u64())? as usize,
                })
            })
            .collect();
    }
    let Some(file) = step.get("file").and_then(Value::as_str) else {
        return vec![];
    };
    let lines: Vec<usize> = step
        .get("lines")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(|n| n.as_u64().map(|n| n as usize))
                .collect()
        })
        .unwrap_or_default();
    let (Some(from), Some(to)) = (lines.first().copied(), lines.last().copied()) else {
        return vec![];
    };
    vec![Range {
        file: file.into(),
        side: step
            .get("side")
            .and_then(Value::as_str)
            .unwrap_or("new")
            .into(),
        from,
        to,
    }]
}

/// The file index for a range: the head path for the new side, the base path for the old side.
fn file_index(m: &DiffModel, r: &Range) -> Option<usize> {
    m.files.iter().position(|f| {
        let p = if r.side == "old" {
            f.old_path.as_deref()
        } else {
            f.new_path.as_deref()
        };
        p == Some(r.file.as_str())
    })
}

/// Why a guide fails. Empty means the kernel accepts it.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Report {
    /// Changed lines that no range holds, as `path:from-to (side)`.
    pub uncovered: Vec<String>,
    /// Ranges that the kernel refuses, with the reason.
    pub bad_ranges: Vec<String>,
    /// Binary files that no step names.
    pub binary: Vec<String>,
}

impl Report {
    pub fn is_empty(&self) -> bool {
        self.uncovered.is_empty() && self.bad_ranges.is_empty() && self.binary.is_empty()
    }
    pub fn text(&self) -> String {
        let mut out = String::from(
            "The guide does not cover the PR. Fix it and call guide_set_steps again.\n",
        );
        if !self.uncovered.is_empty() {
            out += "Changed lines that no step range holds:\n";
            for u in &self.uncovered {
                out += &format!("- {u}\n");
            }
        }
        if !self.bad_ranges.is_empty() {
            out += &format!("Ranges that do not fit (each range must hold a changed line, and every line of it must be at most {PAD} lines from a changed line):\n");
            for b in &self.bad_ranges {
                out += &format!("- {b}\n");
            }
        }
        if !self.binary.is_empty() {
            out += "Binary files that no step names (add the path to a step's `files` list):\n";
            for b in &self.binary {
                out += &format!("- {b}\n");
            }
        }
        out
    }
}

/// The spans of a guide, for the kernel. A range with an unknown file goes to the report.
fn spans(m: &DiffModel, guide: &Value, report: &mut Report) -> Vec<guide_check::Span> {
    let mut out = vec![];
    for step in guide
        .get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let sid = step.get("id").and_then(Value::as_str).unwrap_or("?");
        for r in step_ranges(step) {
            match file_index(m, &r) {
                Some(file) => out.push(guide_check::Span {
                    file,
                    old: r.side == "old",
                    from: r.from,
                    to: r.to,
                }),
                None => report.bad_ranges.push(format!(
                    "step {sid}: {} ({}) is not a changed file on that side",
                    r.file, r.side
                )),
            }
        }
    }
    out
}

/// Binary files must be named by a step, in `files`, in a range, or as `file`.
fn binary_missing(m: &DiffModel, guide: &Value) -> Vec<String> {
    let mut named = std::collections::HashSet::new();
    for step in guide
        .get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(f) = step.get("file").and_then(Value::as_str) {
            named.insert(f.to_owned());
        }
        for f in step
            .get("files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            named.insert(f.to_owned());
        }
        for r in step_ranges(step) {
            named.insert(r.file);
        }
    }
    m.files
        .iter()
        .filter(|f| f.binary && !named.contains(f.path()))
        .map(|f| f.path().to_owned())
        .collect()
}

/// Runs the verified check on a guide. `Ok` means the kernel accepts it.
pub fn check_guide(m: &DiffModel, guide: &Value) -> Result<(), Report> {
    let mut report = Report::default();
    let sp = spans(m, guide, &mut report);
    report.binary = binary_missing(m, guide);
    let accepted = guide_check::check(&kernel_files(m), &sp, PAD);
    tracing::info!(target: "sb::coverage", accepted, spans = sp.len(), unknown = report.bad_ranges.len(), binary_missing = report.binary.len(), "guide check");
    if accepted && report.is_empty() {
        return Ok(());
    }
    explain(m, &sp, &mut report);
    if report.is_empty() {
        // The kernel refused, and the explainer found no reason. Never accept then.
        report
            .bad_ranges
            .push("the checker refused the guide".into());
    }
    Err(report)
}

/// Runs of consecutive line numbers.
fn runs(lines: &[usize]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = vec![];
    for &l in lines {
        match out.last_mut() {
            Some((_, to)) if *to + 1 == l => *to = l,
            _ => out.push((l, l)),
        }
    }
    out
}

fn side_name(old: bool) -> &'static str {
    if old {
        "old"
    } else {
        "new"
    }
}

/// Fills the report with the reasons, in the words of the kernel's spec.
fn explain(m: &DiffModel, sp: &[guide_check::Span], report: &mut Report) {
    for (fi, f) in m.files.iter().enumerate() {
        for (old, mask) in [(true, &f.removed), (false, &f.added)] {
            let missed: Vec<usize> = mask
                .iter()
                .enumerate()
                .filter(|(k, m)| {
                    **m && !sp
                        .iter()
                        .any(|s| s.file == fi && s.old == old && s.from <= k + 1 && *k < s.to)
                })
                .map(|(k, _)| k + 1)
                .collect();
            for (a, b) in runs(&missed) {
                report
                    .uncovered
                    .push(format!("{}:{a}-{b} ({})", f.path(), side_name(old)));
            }
        }
    }
    for s in sp {
        let f = &m.files[s.file];
        let mask = if s.old { &f.removed } else { &f.added };
        let name = format!("{}:{}-{} ({})", f.path(), s.from, s.to, side_name(s.old));
        if s.from == 0 || s.from > s.to || s.to > mask.len() {
            report.bad_ranges.push(format!(
                "{name}: outside the file, which has {} lines on that side",
                mask.len()
            ));
        } else if !(s.from..=s.to).any(|l| mask[l - 1]) {
            report
                .bad_ranges
                .push(format!("{name}: holds no changed line"));
        } else if let Some(far) = (s.from..=s.to).find(|l| {
            !mask
                .iter()
                .enumerate()
                .any(|(k, m)| *m && (k + 1).abs_diff(*l) <= PAD)
        }) {
            report.bad_ranges.push(format!(
                "{name}: line {far} is more than {PAD} lines from a changed line"
            ));
        }
    }
}

/// Makes a guide that the kernel accepts: drops the ranges it refuses, and adds
/// the step "Not in the guide" with every change that is left. Returns the
/// guide and the number of changed lines in the added step.
pub fn complete(m: &DiffModel, guide: &Value) -> Result<(Value, usize), String> {
    let files = kernel_files(m);
    let mut guide = guide.clone();
    if guide.get("steps").and_then(Value::as_array).is_none() {
        guide["steps"] = json!([]);
    }
    // Keep only the ranges the kernel accepts on their own.
    for step in guide["steps"].as_array_mut().unwrap() {
        let kept: Vec<Value> = step_ranges(step)
            .into_iter()
            .filter(|r| {
                file_index(m, r).is_some_and(|file| {
                    guide_check::span_ok(
                        &guide_check::Span {
                            file,
                            old: r.side == "old",
                            from: r.from,
                            to: r.to,
                        },
                        &files,
                        PAD,
                    )
                })
            })
            .map(|r| json!(r))
            .collect();
        if step.get("ranges").is_some() || step.get("file").and_then(Value::as_str).is_some() {
            step["ranges"] = Value::Array(kept);
        }
    }
    // Every change with no range goes into one added step.
    let mut report = Report::default();
    let sp = spans(m, &guide, &mut report);
    let mut missed_ranges = vec![];
    let mut missed_lines = 0;
    for (fi, f) in m.files.iter().enumerate() {
        for (old, mask) in [(true, &f.removed), (false, &f.added)] {
            let path = if old {
                f.old_path.clone()
            } else {
                f.new_path.clone()
            };
            let missed: Vec<usize> = mask
                .iter()
                .enumerate()
                .filter(|(k, m)| {
                    **m && !sp
                        .iter()
                        .any(|s| s.file == fi && s.old == old && s.from <= k + 1 && *k < s.to)
                })
                .map(|(k, _)| k + 1)
                .collect();
            missed_lines += missed.len();
            for (a, b) in runs(&missed) {
                missed_ranges.push(json!({ "file": path.clone().unwrap_or_default(), "side": side_name(old), "from": a, "to": b }));
            }
        }
    }
    let binary = binary_missing(m, &guide);
    if !missed_ranges.is_empty() || !binary.is_empty() {
        let n = guide["steps"].as_array().map(Vec::len).unwrap_or(0) + 1;
        guide["steps"].as_array_mut().unwrap().push(json!({
            "id": format!("s{n}"),
            "title": MISSED,
            "what": "The guide did not cover these changes. The app added them, so you review every line.",
            "check": ["Read each change here like any other step."],
            "ranges": missed_ranges,
            "files": binary,
            "auto": true,
        }));
    }
    check_guide(m, &guide).map_err(|r| format!("the completed guide still fails: {}", r.text()))?;
    Ok((guide, missed_lines))
}

/// Compares the per-file counts with GitHub's counts. Returns one line per difference.
pub fn compare_counts(m: &DiffModel, github: &BTreeMap<String, (u64, u64)>) -> Vec<String> {
    let mut out = vec![];
    for f in &m.files {
        let path = f.path().to_owned();
        match github.get(&path) {
            None => out.push(format!("{path}: git has it, GitHub does not list it")),
            Some((a, d))
                if !f.binary && (*a as usize != f.additions || *d as usize != f.deletions) =>
            {
                out.push(format!(
                    "{path}: git +{} -{}, GitHub +{a} -{d}",
                    f.additions, f.deletions
                ))
            }
            _ => {}
        }
    }
    for path in github.keys() {
        if !m
            .files
            .iter()
            .any(|f| f.path() == path || f.old_path.as_deref() == Some(path.as_str()))
        {
            out.push(format!("{path}: GitHub lists it, git does not"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_keep_their_newline() {
        assert_eq!(split_lines(b"a\nb"), vec![b"a\n".to_vec(), b"b".to_vec()]);
        assert_eq!(split_lines(b"a\n"), vec![b"a\n".to_vec()]);
        assert!(split_lines(b"").is_empty());
    }

    #[test]
    fn numstat_with_a_rename_and_a_binary() {
        let raw = b"3\t1\tsrc/a.rs\x000\t0\t\x00old/b.rs\x00new/b.rs\x00-\t-\timg.png\x00";
        let n = parse_numstat(raw);
        assert_eq!(n.len(), 3);
        assert_eq!(
            (n[0].additions, n[0].old_path.as_str()),
            (Some(3), "src/a.rs")
        );
        assert_eq!(
            (n[1].old_path.as_str(), n[1].new_path.as_str()),
            ("old/b.rs", "new/b.rs")
        );
        assert_eq!(n[2].additions, None);
    }

    #[test]
    fn hunk_headers() {
        let d = "@@ -3 +3,2 @@\n-x\n+y\n+z\n@@ -10,0 +12 @@\n+w\n@@ -20,2 +22,0 @@\n-a\n-b\n";
        let (r, a) = parse_hunks(d);
        assert_eq!(r, vec![3, 20, 21]);
        assert_eq!(a, vec![3, 4, 12]);
    }

    fn repo() -> (std::path::PathBuf, String, String) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("sb-cov-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = |args: &[&str]| git(&dir, args).unwrap();
        g(&["init", "-q"]);
        std::fs::write(
            dir.join("a.txt"),
            (1..=60).map(|i| format!("line {i}\n")).collect::<String>(),
        )
        .unwrap();
        std::fs::write(dir.join("gone.txt"), "bye\n").unwrap();
        std::fs::write(
            dir.join("move.txt"),
            (1..=20).map(|i| format!("m {i}\n")).collect::<String>(),
        )
        .unwrap();
        g(&["add", "."]);
        g(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "base",
        ]);
        let base = g(&["rev-parse", "HEAD"]);
        let mut a: Vec<String> = (1..=60).map(|i| format!("line {i}\n")).collect();
        a[4] = "changed 5\n".into();
        a.insert(40, "new line\n".into());
        a.pop();
        std::fs::write(dir.join("a.txt"), a.concat()).unwrap();
        std::fs::remove_file(dir.join("gone.txt")).unwrap();
        std::fs::write(dir.join("fresh.txt"), "hello\nworld").unwrap();
        g(&["mv", "move.txt", "moved.txt"]);
        std::fs::write(dir.join("bin.dat"), [0u8, 1, 2, 0, 255]).unwrap();
        g(&["add", "-A"]);
        g(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "head",
        ]);
        let head = g(&["rev-parse", "HEAD"]);
        (dir, base, head)
    }

    #[test]
    fn a_real_diff_rebuilds_and_a_full_guide_passes() {
        let (dir, base, head) = repo();
        let m = build(&dir, &base, &head).unwrap();
        assert!(rebuild_ok(&m).is_ok());
        let paths: Vec<&str> = m.files.iter().map(ChangedFile::path).collect();
        assert!(
            paths.contains(&"a.txt")
                && paths.contains(&"fresh.txt")
                && paths.contains(&"gone.txt")
                && paths.contains(&"bin.dat")
        );
        let moved = m.files.iter().find(|f| f.path() == "moved.txt").unwrap();
        assert_eq!(moved.old_path.as_deref(), Some("move.txt"));
        assert_eq!(moved.changed_lines(), 0);

        // An empty guide is refused, with every change named.
        let err = check_guide(&m, &json!({"steps": []})).unwrap_err();
        assert!(
            err.uncovered
                .iter()
                .any(|u| u.starts_with("a.txt:5-5 (old)")),
            "{err:?}"
        );
        assert!(
            err.uncovered
                .iter()
                .any(|u| u.starts_with("gone.txt:1-1 (old)")),
            "{err:?}"
        );
        assert_eq!(err.binary, vec!["bin.dat"]);

        // `complete` makes it pass, and the kernel agrees.
        let (g, missed) = complete(&m, &json!({"steps": [{"id": "s1", "title": "PR"}]})).unwrap();
        assert!(missed >= 6, "{missed}");
        assert!(check_guide(&m, &g).is_ok());
        assert_eq!(
            g["steps"].as_array().unwrap().last().unwrap()["title"],
            MISSED
        );

        // A good guide by hand passes; a too-wide range is refused.
        let good = json!({"steps": [
            {"id": "s1", "title": "a", "ranges": [
                {"file": "a.txt", "side": "old", "from": 5, "to": 5},
                {"file": "a.txt", "side": "old", "from": 60, "to": 60},
                {"file": "a.txt", "side": "new", "from": 3, "to": 7},
                {"file": "a.txt", "side": "new", "from": 41, "to": 41}]},
            {"id": "s2", "title": "files", "files": ["bin.dat"], "ranges": [
                {"file": "gone.txt", "side": "old", "from": 1, "to": 1},
                {"file": "fresh.txt", "side": "new", "from": 1, "to": 2}]}]});
        assert_eq!(check_guide(&m, &good).map_err(|r| r.text()), Ok(()));
        let mut wide = good.clone();
        // Old lines 5 and 60 changed: old line 30 is 25 lines from both.
        wide["steps"][0]["ranges"][0]["to"] = json!(40);
        let err = check_guide(&m, &wide).unwrap_err();
        assert!(
            err.bad_ranges.iter().any(|b| b.contains("more than")),
            "{err:?}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_wrong_mask_fails_the_rebuild() {
        let (dir, base, head) = repo();
        let mut m = build(&dir, &base, &head).unwrap();
        let a = m.files.iter_mut().find(|f| f.path() == "a.txt").unwrap();
        let k = a.added.iter().position(|x| *x).unwrap();
        a.added[k] = false;
        assert!(rebuild_ok(&m).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn counts_compare_with_github() {
        let (dir, base, head) = repo();
        let m = build(&dir, &base, &head).unwrap();
        let mut gh: BTreeMap<String, (u64, u64)> = m
            .files
            .iter()
            .map(|f| {
                (
                    f.path().to_owned(),
                    (f.additions as u64, f.deletions as u64),
                )
            })
            .collect();
        assert!(compare_counts(&m, &gh).is_empty());
        gh.insert("a.txt".into(), (9, 9));
        gh.insert("extra.rs".into(), (1, 0));
        let diff = compare_counts(&m, &gh);
        assert_eq!(diff.len(), 2, "{diff:?}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn old_style_steps_still_give_ranges() {
        let r = step_ranges(&json!({"file": "a.rs", "side": "new", "lines": [3, 9]}));
        assert_eq!(
            r,
            vec![Range {
                file: "a.rs".into(),
                side: "new".into(),
                from: 3,
                to: 9
            }]
        );
        assert!(step_ranges(&json!({"title": "PR"})).is_empty());
    }
}
