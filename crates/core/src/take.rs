//! Why a step exists, and Claude's take on each step and on the PR (SPEC 11.12).
//! The guide agent writes these fields; `check` refuses a wrong shape and a note on
//! a line that does not exist, with one message that lists every problem.
//!
//! - step `why`: why the step matters. `why_split`: why it is a step of its own.
//! - step `check_answers`: Claude's short answer to each `check` item, in order.
//! - step `take` and guide `take`: `{verdict, checked, summary, notes}`. `verdict` is
//!   `good`, `change` or `question`; `good` needs `checked`, what Claude checked.
//!   Each note is `{severity, text, file, side, line}`: `severity` is `must`,
//!   `should` or `nit`; the line is optional, and when it is there it must exist on
//!   that side of a changed file.

use crate::coverage::DiffModel;
use serde_json::Value;

pub const VERDICTS: [&str; 3] = ["good", "change", "question"];
pub const SEVERITIES: [&str; 3] = ["must", "should", "nit"];

/// Checks the why fields and every take of `guide`. Returns all problems at once.
pub fn check(m: &DiffModel, guide: &Value) -> Result<(), String> {
    let mut errs: Vec<String> = vec![];
    if let Some(t) = guide.get("take") {
        take(m, t, "the PR take", &mut errs);
    }
    for (i, s) in guide.get("steps").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let name = s.get("id").and_then(Value::as_str).map(|id| format!("step {id}")).unwrap_or(format!("step {i}"));
        for key in ["why", "why_split"] {
            if let Some(v) = s.get(key) {
                if !v.is_string() {
                    errs.push(format!("{name}: `{key}` must be a string"));
                }
            }
        }
        if let Some(a) = s.get("check_answers") {
            let checks = s.get("check").and_then(Value::as_array).map(|c| c.len()).unwrap_or(0);
            match a.as_array() {
                Some(list) if list.iter().all(Value::is_string) => {
                    if list.len() > checks {
                        errs.push(format!("{name}: {} check_answers for {checks} check items", list.len()));
                    }
                }
                _ => errs.push(format!("{name}: `check_answers` must be a list of strings")),
            }
        }
        if let Some(t) = s.get("take") {
            take(m, t, &format!("{name} take"), &mut errs);
        }
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(format!("Fix these in the takes and whys:\n- {}\n", errs.join("\n- ")))
    }
}

/// The guide with every part that fails `check` taken out: a bad take, a why that is
/// not a string, bad check answers. The rest stays. For the last try (SPEC 11.12).
pub fn strip_bad(m: &DiffModel, guide: &Value) -> Value {
    let mut g = guide.clone();
    let bad = |t: &Value| {
        let mut e = vec![];
        take(m, t, "", &mut e);
        !e.is_empty()
    };
    if g.get("take").is_some_and(bad) {
        g.as_object_mut().map(|o| o.remove("take"));
    }
    if let Some(steps) = g.get_mut("steps").and_then(Value::as_array_mut) {
        for s in steps {
            let checks = s.get("check").and_then(Value::as_array).map(|c| c.len()).unwrap_or(0);
            let Some(o) = s.as_object_mut() else { continue };
            if o.get("take").is_some_and(bad) {
                o.remove("take");
            }
            for key in ["why", "why_split"] {
                if o.get(key).is_some_and(|v| !v.is_string()) {
                    o.remove(key);
                }
            }
            let answers_ok = o.get("check_answers").is_none_or(|a| {
                a.as_array().is_some_and(|l| l.len() <= checks && l.iter().all(Value::is_string))
            });
            if !answers_ok {
                o.remove("check_answers");
            }
        }
    }
    g
}

fn take(m: &DiffModel, t: &Value, name: &str, errs: &mut Vec<String>) {
    let Some(obj) = t.as_object() else {
        errs.push(format!("{name} must be an object"));
        return;
    };
    let verdict = obj.get("verdict").and_then(Value::as_str);
    match verdict {
        Some(v) if VERDICTS.contains(&v) => {}
        _ => errs.push(format!("{name}: `verdict` must be one of {}", VERDICTS.join(", "))),
    }
    for key in ["checked", "summary"] {
        if obj.get(key).is_some_and(|v| !v.is_string()) {
            errs.push(format!("{name}: `{key}` must be a string"));
        }
    }
    let checked = obj.get("checked").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if verdict == Some("good") && checked.is_empty() {
        errs.push(format!("{name}: a `good` verdict needs `checked`: what you checked"));
    }
    let notes = match obj.get("notes") {
        None => return,
        Some(Value::Array(n)) => n,
        Some(_) => {
            errs.push(format!("{name}: `notes` must be a list"));
            return;
        }
    };
    for (k, n) in notes.iter().enumerate() {
        let at = format!("{name} note {}", k + 1);
        match n.get("severity").and_then(Value::as_str) {
            Some(s) if SEVERITIES.contains(&s) => {}
            _ => errs.push(format!("{at}: `severity` must be one of {}", SEVERITIES.join(", "))),
        }
        if n.get("text").and_then(Value::as_str).is_none_or(|s| s.trim().is_empty()) {
            errs.push(format!("{at}: `text` is missing"));
        }
        if let Some(line) = n.get("line").filter(|l| !l.is_null()) {
            let Some(line) = line.as_u64() else {
                errs.push(format!("{at}: `line` must be a number"));
                continue;
            };
            let side = n.get("side").and_then(Value::as_str).unwrap_or("new");
            if side != "new" && side != "old" {
                errs.push(format!("{at}: `side` must be new or old"));
                continue;
            }
            let Some(file) = n.get("file").and_then(Value::as_str) else {
                errs.push(format!("{at}: a note with a `line` needs its `file`"));
                continue;
            };
            match line_count(m, file, side) {
                None => errs.push(format!("{at}: {file} is not a changed file on the {side} side")),
                Some(count) if line == 0 || line > count as u64 => {
                    errs.push(format!("{at}: line {line} is outside {file} ({count} lines on the {side} side)"))
                }
                Some(_) => {}
            }
        }
    }
}

/// The lines of `file` on `side` in the model, when it is a changed text file there.
fn line_count(m: &DiffModel, file: &str, side: &str) -> Option<usize> {
    m.files.iter().find_map(|f| {
        let (path, lines) = if side == "old" { (&f.old_path, &f.old) } else { (&f.new_path, &f.new) };
        (path.as_deref() == Some(file) && !f.binary && !f.submodule).then_some(lines.len())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage::ChangedFile;
    use serde_json::json;

    fn model() -> DiffModel {
        let f = ChangedFile {
            old_path: Some("a.rs".into()),
            new_path: Some("a.rs".into()),
            old: vec![b"x".to_vec(); 3],
            new: vec![b"x".to_vec(); 5],
            ..ChangedFile::default()
        };
        DiffModel { files: vec![f], ..DiffModel::default() }
    }

    #[test]
    fn a_full_take_passes() {
        let g = json!({
            "take": {"verdict": "change", "summary": "No test for slot 0.", "notes": [{"severity": "should", "text": "No test for slot 0."}]},
            "steps": [{"id": "s1", "title": "t", "why": "w", "why_split": "x", "check": ["a", "b"], "check_answers": ["yes"],
                "take": {"verdict": "change", "notes": [
                    {"severity": "must", "text": "bad", "file": "a.rs", "line": 5},
                    {"severity": "nit", "text": "old", "file": "a.rs", "side": "old", "line": 3}]}},
                {"id": "s2", "title": "t", "take": {"verdict": "good", "checked": "The cache key.", "notes": [{"severity": "nit", "text": "no line", "line": null}]}}]
        });
        assert_eq!(check(&model(), &g), Ok(()));
    }

    #[test]
    fn every_problem_is_listed() {
        let g = json!({"steps": [{"id": "s1", "title": "t", "why": 3, "check": ["a"], "check_answers": ["1", "2"],
            "take": {"verdict": "great", "notes": [
                {"severity": "must", "text": "x", "file": "a.rs", "line": 6},
                {"severity": "huge", "text": ""},
                {"severity": "nit", "text": "x", "file": "b.rs", "line": 1},
                {"severity": "nit", "text": "x", "line": 1}]}},
            {"id": "s2", "title": "t", "take": {"verdict": "good"}}]});
        let err = check(&model(), &g).unwrap_err();
        for want in ["`why` must be a string", "2 check_answers for 1", "`verdict` must be one of", "line 6 is outside a.rs (5 lines",
            "`severity` must be one of", "`text` is missing", "b.rs is not a changed file", "needs its `file`", "needs `checked`"] {
            assert!(err.contains(want), "{want}\n{err}");
        }
    }

    #[test]
    fn strip_bad_keeps_the_good_parts_and_then_passes() {
        let g = json!({"take": {"verdict": "nope"}, "steps": [
            {"id": "s1", "title": "t", "why": "ok", "why_split": 4, "check": ["a"], "check_answers": ["1", "2"],
             "take": {"verdict": "change", "notes": [{"severity": "must", "text": "x", "file": "a.rs", "line": 99}]}},
            {"id": "s2", "title": "t", "take": {"verdict": "good", "checked": "all"}}]});
        let s = strip_bad(&model(), &g);
        assert_eq!(check(&model(), &s), Ok(()));
        assert!(s.get("take").is_none());
        assert_eq!(s["steps"][0]["why"], "ok");
        assert!(s["steps"][0].get("why_split").is_none() && s["steps"][0].get("check_answers").is_none() && s["steps"][0].get("take").is_none());
        assert_eq!(s["steps"][1]["take"]["verdict"], "good");
    }

    /// Random notes: a note passes exactly when its line exists on its side.
    #[test]
    fn note_lines_match_the_model() {
        let m = model();
        let mut seed: u64 = 99;
        for _ in 0..5000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let line = (seed >> 33) % 9;
            let old = (seed >> 20) & 1 == 1;
            let file = if (seed >> 21) & 3 == 0 { "b.rs" } else { "a.rs" };
            let side = if old { "old" } else { "new" };
            let g = json!({"steps": [{"id": "s", "title": "t", "take": {"verdict": "change",
                "notes": [{"severity": "nit", "text": "x", "file": file, "side": side, "line": line}]}}]});
            let count = if old { 3 } else { 5 };
            let ok = file == "a.rs" && line >= 1 && line <= count;
            assert_eq!(check(&m, &g).is_ok(), ok, "{file} {side} {line}");
        }
    }
}
