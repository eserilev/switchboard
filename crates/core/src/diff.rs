//! Unified diff rows for the review diff column.

use serde::Serialize;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Row {
    /// ` ` context, `+` added, `-` removed, `@` hunk header.
    pub kind: char,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: String,
}

/// Parses the diff of one file. Headers before the first hunk are skipped.
///
/// The numbers come from the verified kernel `diffparse` (SPEC 24.2). `header`
/// reads `@@ -a,b +c,d @@` and refuses a header whose end line does not fit in a
/// `u32`. `step` gives each row its numbers and refuses a row past the counts of
/// its hunk. After a refusal the rows up to the next hunk are skipped.
pub fn parse(diff: &str) -> Vec<Row> {
    let mut rows = vec![];
    let mut at: Option<diffparse::Counter> = None;
    for line in diff.lines() {
        if let Some(h) = line.strip_prefix("@@ ") {
            at = diffparse::header(h.as_bytes());
            rows.push(Row {
                kind: '@',
                old: None,
                new: None,
                text: line.to_owned(),
            });
            continue;
        }
        let Some(c) = at.as_ref() else {
            continue;
        };
        let (kind, text) = match line.chars().next() {
            Some('+') => ('+', &line[1..]),
            Some('-') => ('-', &line[1..]),
            Some(' ') => (' ', &line[1..]),
            Some('\\') => continue,
            None => (' ', ""),
            _ => {
                at = None;
                continue;
            }
        };
        let (old_side, new_side) = (kind != '+', kind != '-');
        let Some(next) = diffparse::step(c, old_side, new_side) else {
            at = None;
            continue;
        };
        rows.push(Row {
            kind,
            old: old_side.then_some(c.old),
            new: new_side.then_some(c.new),
            text: text.to_owned(),
        });
        at = Some(next);
    }
    rows
}

/// Rows of a file that the PR does not change: plain context around `lines`.
pub fn context(text: &str, from: u32, to: u32, pad: u32) -> Vec<Row> {
    let start = from.saturating_sub(pad).max(1);
    let end = to.saturating_add(pad);
    text.lines()
        .zip(1..=end)
        .filter(|(_, n)| *n >= start)
        .map(|(l, n)| Row {
            kind: ' ',
            old: Some(n),
            new: Some(n),
            text: l.to_owned(),
        })
        .collect()
}

/// Finds `text` near `line` in `lines` (1-based). Used to re-anchor a thread after new commits.
pub fn find_near(lines: &[&str], text: &str, line: u32) -> Option<u32> {
    let want = text.trim();
    if want.is_empty() {
        return None;
    }
    let hits: Vec<u32> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.trim() == want)
        .map(|(i, _)| i as u32 + 1)
        .collect();
    hits.into_iter().min_by_key(|n| n.abs_diff(line))
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: &str = "diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n@@ -10,4 +10,5 @@ fn x() {\n a\n-b\n+B\n+C\n c\n\\ No newline at end of file\n";

    #[test]
    fn numbers_old_and_new_lines() {
        let r = parse(D);
        assert_eq!(r[0].kind, '@');
        assert_eq!((r[1].kind, r[1].old, r[1].new), (' ', Some(10), Some(10)));
        assert_eq!(
            (r[2].kind, r[2].old, r[2].new, r[2].text.as_str()),
            ('-', Some(11), None, "b")
        );
        assert_eq!((r[3].kind, r[3].new), ('+', Some(11)));
        assert_eq!((r[4].kind, r[4].new), ('+', Some(12)));
        assert_eq!((r[5].kind, r[5].old, r[5].new), (' ', Some(12), Some(13)));
        assert_eq!(r.len(), 6);
    }

    #[test]
    fn hunk_with_no_count() {
        let r = parse("@@ -3 +4 @@\n-x\n+y\n");
        assert_eq!((r[1].old, r[2].new), (Some(3), Some(4)));
    }

    #[test]
    fn a_header_too_big_to_count_from_is_refused() {
        // The old code counted from 4294967295 and overflowed at the first row.
        let r = parse("@@ -1 +4294967295 @@\n+x\n");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].kind, '@');
        // The last line number that fits still counts.
        let r = parse("@@ -1,0 +4294967294 @@\n+x\n");
        assert_eq!((r[1].kind, r[1].new), ('+', Some(u32::MAX - 1)));
    }

    #[test]
    fn rows_past_the_counts_are_not_in_the_hunk() {
        let r = parse("@@ -1,1 +1,1 @@\n-a\n+b\n c\n@@ -9 +9 @@\n x\n");
        let kinds: String = r.iter().map(|r| r.kind).collect();
        assert_eq!(kinds, "@-+@ ");
        assert_eq!((r[4].old, r[4].new), (Some(9), Some(9)));
    }

    #[test]
    fn context_rows() {
        let r = context("a\nb\nc\nd\ne", 3, 3, 1);
        assert_eq!(
            r.iter().map(|r| r.new.unwrap()).collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
    }

    #[test]
    fn context_near_the_last_line_number_does_not_overflow() {
        let r = context("a\nb", 1, u32::MAX, 20);
        assert_eq!(r.len(), 2);
        let r = context("a\nb\nc", u32::MAX, u32::MAX, u32::MAX);
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn find_near_prefers_the_closest_hit() {
        let lines = ["x", "let a = 1;", "y", "let a = 1;", "z"];
        assert_eq!(find_near(&lines, "  let a = 1;", 5), Some(4));
        assert_eq!(find_near(&lines, "let a = 1;", 1), Some(2));
        assert_eq!(find_near(&lines, "gone", 1), None);
    }
}
