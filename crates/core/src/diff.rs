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
pub fn parse(diff: &str) -> Vec<Row> {
    let mut rows = vec![];
    let (mut old, mut new) = (0u32, 0u32);
    let mut in_hunk = false;
    for line in diff.lines() {
        if let Some(h) = line.strip_prefix("@@ ") {
            let (o, n) = hunk_start(h);
            old = o;
            new = n;
            in_hunk = true;
            rows.push(Row {
                kind: '@',
                old: None,
                new: None,
                text: line.to_owned(),
            });
            continue;
        }
        if !in_hunk {
            continue;
        }
        let (kind, text) = match line.chars().next() {
            Some('+') => ('+', &line[1..]),
            Some('-') => ('-', &line[1..]),
            Some(' ') => (' ', &line[1..]),
            Some('\\') => continue,
            None => (' ', ""),
            _ => {
                in_hunk = false;
                continue;
            }
        };
        let row = match kind {
            '+' => {
                new += 1;
                Row {
                    kind,
                    old: None,
                    new: Some(new - 1),
                    text: text.to_owned(),
                }
            }
            '-' => {
                old += 1;
                Row {
                    kind,
                    old: Some(old - 1),
                    new: None,
                    text: text.to_owned(),
                }
            }
            _ => {
                old += 1;
                new += 1;
                Row {
                    kind,
                    old: Some(old - 1),
                    new: Some(new - 1),
                    text: text.to_owned(),
                }
            }
        };
        rows.push(row);
    }
    rows
}

fn hunk_start(h: &str) -> (u32, u32) {
    let mut it = h.split(' ');
    let num = |s: Option<&str>, sign: char| -> u32 {
        s.and_then(|s| s.strip_prefix(sign))
            .and_then(|s| s.split(',').next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(1)
    };
    (num(it.next(), '-'), num(it.next(), '+'))
}

/// Rows of a file that the PR does not change: plain context around `lines`.
pub fn context(text: &str, from: u32, to: u32, pad: u32) -> Vec<Row> {
    let start = from.saturating_sub(pad).max(1);
    let end = to + pad;
    text.lines()
        .enumerate()
        .map(|(i, l)| (i as u32 + 1, l))
        .filter(|(n, _)| *n >= start && *n <= end)
        .map(|(n, l)| Row {
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
        assert_eq!(hunk_start("-3 +4 @@"), (3, 4));
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
    fn find_near_prefers_the_closest_hit() {
        let lines = ["x", "let a = 1;", "y", "let a = 1;", "z"];
        assert_eq!(find_near(&lines, "  let a = 1;", 5), Some(4));
        assert_eq!(find_near(&lines, "let a = 1;", 1), Some(2));
        assert_eq!(find_near(&lines, "gone", 1), None);
    }
}
