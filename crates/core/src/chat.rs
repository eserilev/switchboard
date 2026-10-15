//! The questions and answers of a review's Claude pane, by step (SPEC 11.5). Claude
//! Code keeps each session in a JSON lines file; the pane is a full-screen app, so its
//! history is not in tmux. The window shows these turns for the step you are on.

use serde_json::Value;

/// One question and its answer.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Turn {
    /// The step: from a step line in the question, else the step you viewed then.
    pub step: Option<String>,
    pub question: String,
    pub answer: String,
    /// Seconds since the Unix epoch.
    pub at: u64,
}

/// The first message of a review's Claude pane starts with one of these
/// (`hub_review::review_term`). Everything before it is the guide session's own
/// conversation, which a fork copies in.
pub const STARTS: [&str; 2] = ["I am on step ", "I have questions about this review"];

/// The turns of a session file from its first message that starts with one of
/// `STARTS` on. A turn with only a step line (the first message, or Send step with no
/// question) sets the step and is not shown.
pub fn turns(jsonl: &str, seen: &[(String, u64)]) -> Vec<Turn> {
    let mut out: Vec<Turn> = vec![];
    let mut started = false;
    let mut step: Option<String> = None;
    // When the step came from a step line: a step view after it wins, one before it not.
    let mut marked_at = 0u64;
    let mut open: Option<(Option<String>, String, u64, bool)> = None;
    let mut answer = String::new();
    let close = |open: &mut Option<(Option<String>, String, u64, bool)>, answer: &mut String, out: &mut Vec<Turn>| {
        if let Some((s, q, at, shown)) = open.take() {
            if shown {
                out.push(Turn { step: s, question: q, answer: answer.trim().to_owned(), at });
            }
        }
        answer.clear();
    };
    for line in jsonl.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        match v.get("type").and_then(Value::as_str) {
            Some("user") => {
                let Some(text) = prompt_text(&v) else { continue };
                if !started {
                    if !STARTS.iter().any(|s| text.starts_with(s)) {
                        continue;
                    }
                    started = true;
                }
                close(&mut open, &mut answer, &mut out);
                let at = v.get("timestamp").and_then(Value::as_str).and_then(epoch).unwrap_or(0);
                let (marked, rest) = split_step_line(&text);
                // The latest signal wins: a step line in this question, else the newer
                // of the last step line and the last step you viewed before it.
                match marked {
                    Some(m) => {
                        step = Some(m);
                        marked_at = at;
                    }
                    None => {
                        if let Some((v, t)) = viewed_at(seen, at) {
                            if step.is_none() || t > marked_at {
                                step = Some(v);
                            }
                        }
                    }
                }
                let question = rest.trim();
                let shown = !question.is_empty() && !question.starts_with("Wait for my question.");
                open = Some((step.clone(), question.to_owned(), at, shown));
            }
            Some("assistant") if open.is_some() => {
                for block in v.pointer("/message/content").and_then(Value::as_array).into_iter().flatten() {
                    if block.get("type").and_then(Value::as_str) == Some("text") {
                        if let Some(t) = block.get("text").and_then(Value::as_str) {
                            if !answer.is_empty() {
                                answer.push_str("\n\n");
                            }
                            answer.push_str(t);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    close(&mut open, &mut answer, &mut out);
    out
}

/// The text of a prompt you typed: a user line whose content is text, not a tool
/// result, and not a line that Claude Code adds itself.
fn prompt_text(v: &Value) -> Option<String> {
    // Claude Code's own lines, a compact summary, and a sub-agent's lines.
    for flag in ["isMeta", "isCompactSummary", "isSidechain"] {
        if v.get(flag).and_then(Value::as_bool) == Some(true) {
            return None;
        }
    }
    let c = v.pointer("/message/content")?;
    let text = match c {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => {
            if blocks.iter().any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result")) {
                return None;
            }
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => return None,
    };
    let text = text.trim().to_owned();
    // Claude Code's own lines: command output, interrupts, reminders. A question of
    // yours that starts with "<" stays.
    const OWN: [&str; 8] = ["<command-", "<local-command-", "<system-reminder", "<bash-", "<user-prompt-submit-hook", "<task-notification", "[Request interrupted", "This session is being continued"];
    if text.is_empty() || OWN.iter().any(|p| text.starts_with(p)) {
        return None;
    }
    Some(text)
}

/// Splits `I am on step s2 "Title" (a.rs:3-9 (new)). rest` into (`s2`, `rest`).
/// A text with no step line gives (None, the text).
pub fn split_step_line(text: &str) -> (Option<String>, &str) {
    let Some(after) = text.strip_prefix("I am on step ") else {
        return (None, text);
    };
    let Some((id, rest)) = after.split_once(' ') else {
        return (Some(after.trim_end_matches('.').to_owned()), "");
    };
    // The title in quotes, the ranges in parentheses, then a full stop. A title can
    // hold quotes: its end is the first quote that a space and "(", or ".", follows.
    let mut rest = rest;
    if let Some(r) = rest.strip_prefix('"') {
        let end = r.char_indices().find(|&(i, c)| c == '"' && (r[i + 1..].starts_with(" (") || r[i + 1..].starts_with('.')));
        match end {
            Some((e, _)) => rest = &r[e + 1..],
            None => return (Some(id.to_owned()), ""),
        }
    }
    let rest = rest.trim_start();
    let rest = if rest.starts_with('(') {
        // Ranges hold nested parentheses: "(a.rs:3-9 (new), b.rs:1-2 (old))".
        let mut depth = 0usize;
        let mut cut = rest.len();
        for (i, ch) in rest.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        cut = i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        &rest[cut..]
    } else {
        rest
    };
    (Some(id.to_owned()), rest.strip_prefix('.').unwrap_or(rest))
}

/// The step you viewed at `at`: the last one seen at or before it.
fn viewed_at(seen: &[(String, u64)], at: u64) -> Option<(String, u64)> {
    seen.iter().rev().find(|(_, t)| *t <= at).cloned()
}

/// Seconds since the epoch of an RFC 3339 UTC time, `2026-10-15T04:24:39.155Z`.
pub fn epoch(t: &str) -> Option<u64> {
    let b = t.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| t.get(r)?.parse::<u64>().ok();
    let (y, mo, d, h, mi, s) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 || y < 1970 {
        return None;
    }
    // Days from 1970-01-01 to the date (the civil-from-days rule, in reverse).
    let (y2, m2) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = y2 / 400;
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + h * 3600 + mi * 60 + s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user(text: &str, t: &str) -> String {
        json!({"type": "user", "message": {"role": "user", "content": text}, "timestamp": t}).to_string()
    }
    fn said(text: &str) -> String {
        json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": text}, {"type": "tool_use", "name": "Read"}]}}).to_string()
    }
    fn tool_result() -> String {
        json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "content": "file text"}]}}).to_string()
    }

    #[test]
    fn a_review_chat_by_step() {
        let lines = [
            // The guide session, copied in by the fork: left out.
            user("You are guiding a human through a review", "2026-10-15T04:00:00Z"),
            said("6 steps"),
            user("I am on step s1 \"Read the PR\". Wait for my question. Reply only: Ready.", "2026-10-15T04:10:00Z"),
            said("Ready."),
            user("why slot - 1?", "2026-10-15T04:11:00Z"),
            said("Because the spec says so."),
            tool_result(),
            said("Checked: spec line 40."),
            // Viewed step s2 at 04:12; this question has no step line.
            user("is the cache bounded?", "2026-10-15T04:13:00Z"),
            said("Yes."),
            // Send step for s3, then a question in the same message.
            user("I am on step s3 \"Tests\" (t.rs:1-9 (new), u.rs:2-3 (old)). any test for slot 0?", "2026-10-15T04:14:00Z"),
            said("No."),
            json!({"type": "user", "isMeta": true, "message": {"content": "<command-name>/clear</command-name>"}}).to_string(),
        ]
        .join("\n");
        let seen = vec![("s1".to_owned(), epoch("2026-10-15T04:09:00Z").unwrap()), ("s2".to_owned(), epoch("2026-10-15T04:12:00Z").unwrap())];
        let t = turns(&lines, &seen);
        let got: Vec<(Option<&str>, &str, &str)> = t.iter().map(|x| (x.step.as_deref(), x.question.as_str(), x.answer.as_str())).collect();
        assert_eq!(
            got,
            vec![
                (Some("s1"), "why slot - 1?", "Because the spec says so.\n\nChecked: spec line 40."),
                (Some("s2"), "is the cache bounded?", "Yes."),
                (Some("s3"), "any test for slot 0?", "No."),
            ]
        );
    }

    #[test]
    fn claude_codes_own_lines_go_and_your_lines_stay() {
        let lines = [
            user("I am on step s1 \"A\". Wait for my question. Reply only: Ready.", "2026-10-15T04:10:00Z"),
            said("Ready."),
            json!({"type": "user", "isCompactSummary": true, "message": {"content": "This session is being continued from"}, "timestamp": "2026-10-15T04:11:00Z"}).to_string(),
            json!({"type": "user", "isSidechain": true, "message": {"content": "a sub-agent task"}, "timestamp": "2026-10-15T04:12:00Z"}).to_string(),
            user("<system-reminder>x</system-reminder>", "2026-10-15T04:12:30Z"),
            user("<T> is generic here, why?", "2026-10-15T04:13:00Z"),
            said("Because of the trait."),
        ]
        .join("\n");
        let t = turns(&lines, &[]);
        assert_eq!(t.len(), 1, "{t:?}");
        assert_eq!((t[0].question.as_str(), t[0].answer.as_str()), ("<T> is generic here, why?", "Because of the trait."));
    }

    #[test]
    fn a_session_with_no_review_start_has_no_turns() {
        let lines = [user("hello", "2026-10-15T04:00:00Z"), said("hi")].join("\n");
        assert!(turns(&lines, &[]).is_empty());
    }

    #[test]
    fn step_lines_split() {
        assert_eq!(split_step_line("I am on step s2 \"A (b)\" (a.rs:3-9 (new)). why?"), (Some("s2".into()), " why?"));
        assert_eq!(split_step_line("I am on step s9 \"T\". "), (Some("s9".into()), " "));
        assert_eq!(split_step_line("I am on step s4"), (Some("s4".into()), ""));
        assert_eq!(split_step_line("I am on step s4 \"open"), (Some("s4".into()), ""));
        assert_eq!(split_step_line("plain"), (None, "plain"));
        assert_eq!(split_step_line("I am on step s3 \"Rename \"slot\" field\" (a.rs:1-2 (new)). why?"), (Some("s3".into()), " why?"));
        assert_eq!(split_step_line("I am on step s3 \"Use \"x\" here\". Wait for my question."), (Some("s3".into()), " Wait for my question."));
    }

    #[test]
    fn epoch_matches_known_times() {
        assert_eq!(epoch("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(epoch("2000-03-01T00:00:00.5Z"), Some(951868800));
        assert_eq!(epoch("2026-10-15T04:24:39.155Z"), Some(1792038279));
        assert_eq!(epoch("2026-13-01T00:00:00Z"), None);
        assert_eq!(epoch("nope"), None);
    }

    /// Random chats: every shown question comes out once, in order, with the step of
    /// its step line or else the step viewed at its time, and with its own answer.
    #[test]
    fn random_chats_keep_every_question_in_order() {
        let mut seed: u64 = 7;
        let mut rnd = |n: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        for _ in 0..500 {
            let mut lines = vec![user("I am on step s1 \"A\". Wait for my question. Reply only: Ready.", "2026-10-15T00:00:00Z"), said("Ready.")];
            let mut seen: Vec<(String, u64)> = vec![];
            let mut want: Vec<(String, String, String)> = vec![];
            let mut step = "s1".to_owned();
            let base = epoch("2026-10-15T00:00:00Z").unwrap();
            for i in 0..rnd(12) {
                let at = base + 60 * (i + 1);
                if rnd(3) == 0 {
                    // A step view just before the question: newer than any step line.
                    step = format!("s{}", 1 + rnd(5));
                    seen.push((step.clone(), at - 1));
                }
                let q = format!("question {i}");
                let marked = rnd(4) == 0;
                let text = if marked {
                    step = format!("s{}", 1 + rnd(5));
                    format!("I am on step {step} \"T {i}\" (f.rs:1-2 (new)). {q}")
                } else {
                    q.clone()
                };
                let t = format!("{}Z", stamp(at));
                lines.push(user(&text, &t));
                if rnd(5) == 0 {
                    lines.push(tool_result());
                }
                let a = format!("answer {i}");
                lines.push(said(&a));
                want.push((step.clone(), q, a));
            }
            let got: Vec<(String, String, String)> =
                turns(&lines.join("\n"), &seen).into_iter().map(|t| (t.step.unwrap_or_default(), t.question, t.answer)).collect();
            assert_eq!(got, want);
        }
    }

    /// `epoch` back to text, for the test.
    fn stamp(t: u64) -> String {
        let days = t / 86400;
        let rem = t % 86400;
        let z = days + 719468;
        let era = z / 146097;
        let doe = z - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };
        format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
    }
}
