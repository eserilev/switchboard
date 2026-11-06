//! What an answer of the review's Claude pane points to (SPEC 11.15): `path:line`,
//! file names and names in backticks, each found in the changed files of the
//! review at its head. And the check of a path that comes from the agent.
//!
//! It reads only the diff model in memory. Nothing here runs a program or opens a file.

use crate::coverage::{ChangedFile, DiffModel};
use serde::Serialize;

/// The most places one answer gives.
pub const MAX_MENTIONS: usize = 12;
/// The part of an answer that is read. A longer answer is cut.
pub const MAX_TEXT: usize = 64 * 1024;

/// One place an answer refers to.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Mention {
    /// The path in the repo: the head path, or the base path on the old side.
    pub file: String,
    /// None for a file name with no line.
    pub line: Option<u32>,
    pub end: Option<u32>,
    /// `new` or `old`.
    pub side: String,
    /// The name in backticks that found this place, if one did.
    pub name: Option<String>,
    /// True when the PR changes a line of it (or, with no line, the file).
    pub changed: bool,
}

/// A path from the agent, made plain: relative to the repo root, with no `..`, no
/// empty part and no control character. An absolute path inside `tree` loses the
/// tree; any other absolute path, or a `~` path, is refused.
pub fn clean_path(path: &str, tree: &str) -> Result<String, String> {
    let mut p = path.trim();
    if p.is_empty() {
        return Err("the path is empty".into());
    }
    if p.len() > 1024 || p.chars().any(|c| c.is_control() || c == '\\') {
        return Err(format!("{p:?} is not a path in the repo"));
    }
    let tree = tree.trim_end_matches('/');
    if p.starts_with('/') {
        match p.strip_prefix(tree).filter(|_| !tree.is_empty()).and_then(|r| r.strip_prefix('/')) {
            Some(rest) => p = rest,
            None => return Err(format!("{p} is outside the review")),
        }
    }
    if p.starts_with('~') {
        return Err(format!("{p} is outside the review"));
    }
    while let Some(rest) = p.strip_prefix("./") {
        p = rest;
    }
    let mut parts = vec![];
    for part in p.split('/') {
        if part == ".." {
            return Err(format!("{path} leaves the review"));
        }
        if part.is_empty() || part == "." {
            return Err(format!("{path} is not a plain path"));
        }
        parts.push(part);
    }
    Ok(parts.join("/"))
}

/// The lines of a file on one side.
fn lines<'a>(f: &'a ChangedFile, side: &str) -> &'a [Vec<u8>] {
    if side == "old" {
        &f.old
    } else {
        &f.new
    }
}

/// True when the PR changes a line in `from..=to` on that side.
pub fn changed(f: &ChangedFile, side: &str, from: u32, to: u32) -> bool {
    let marks = if side == "old" { &f.removed } else { &f.added };
    (from..=to).any(|n| n >= 1 && marks.get(n as usize - 1).copied().unwrap_or(false))
}

/// The changed file that a path or a short name means, and its side. A full path
/// wins; else the one file whose path ends with `/name`. Two such files: none.
pub fn find_file<'a>(m: &'a DiffModel, name: &str) -> Option<(&'a ChangedFile, &'static str)> {
    let exact = m.files.iter().find_map(|f| {
        if f.new_path.as_deref() == Some(name) {
            Some((f, "new"))
        } else if f.new_path.is_none() && f.old_path.as_deref() == Some(name) {
            Some((f, "old"))
        } else {
            None
        }
    });
    if exact.is_some() {
        return exact;
    }
    let tail = format!("/{name}");
    let hits: Vec<(&ChangedFile, &'static str)> = m
        .files
        .iter()
        .filter_map(|f| {
            if f.new_path.as_deref().is_some_and(|p| p.ends_with(&tail)) {
                Some((f, "new"))
            } else if f.new_path.is_none() && f.old_path.as_deref().is_some_and(|p| p.ends_with(&tail)) {
                Some((f, "old"))
            } else {
                None
            }
        })
        .collect();
    if hits.len() == 1 {
        hits.into_iter().next()
    } else {
        None
    }
}

const PATH_CHARS: &str = "_./-";

fn is_path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || PATH_CHARS.contains(c)
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// A number after `at` in `s`: the digits and their end.
fn number(s: &str, at: usize) -> Option<(u32, usize)> {
    let digits: String = s[at..].chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || digits.len() > 7 {
        return None;
    }
    Some((digits.parse().ok()?, at + digits.len()))
}

/// The `path`, `path:line` and `path:from-to` in a text, in order.
fn paths(text: &str) -> Vec<(String, Option<u32>, Option<u32>, usize)> {
    let mut out = vec![];
    let mut i = 0;
    let b = text.as_bytes();
    while i < text.len() {
        let c = text[i..].chars().next().unwrap_or(' ');
        if !is_path_char(c) {
            i += c.len_utf8();
            continue;
        }
        let start = i;
        while i < text.len() && is_path_char(b[i] as char) {
            i += 1;
        }
        // An absolute path keeps its `/`; a sentence end does not count.
        let token = text[start..i].trim_end_matches('.');
        let end_tok = start + token.len();
        let name = token.rsplit('/').next().unwrap_or("");
        let ext = name.rsplit_once('.').map(|x| x.1).unwrap_or("");
        let file_like = name.contains('.') && !name.starts_with('.') && !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()) && !ext.chars().all(|c| c.is_ascii_digit());
        if !file_like {
            continue;
        }
        let (mut line, mut end) = (None, None);
        if text[end_tok..].starts_with(':') {
            if let Some((n, after)) = number(text, end_tok + 1) {
                line = Some(n);
                i = after;
                if text[after..].starts_with('-') {
                    if let Some((m, after2)) = number(text, after + 1) {
                        end = Some(m);
                        i = after2;
                    }
                }
            }
        }
        out.push((token.to_owned(), line, end, start));
    }
    out
}

/// Words that are not worth a search: keywords and very common names.
const SKIP: &[&str] = &[
    "self", "Self", "None", "Some", "Ok", "Err", "true", "false", "mut", "let", "fn", "pub", "impl", "return", "match", "if",
    "else", "for", "while", "loop", "use", "mod", "crate", "super", "struct", "enum", "trait", "type", "const", "static",
    "async", "await", "move", "ref", "where", "dyn", "unwrap", "clone", "new", "into", "from", "len", "map", "Vec",
    "String", "Option", "Result", "bool", "u8", "u16", "u32", "u64", "u128", "usize", "i32", "i64", "str", "null",
];

/// The names in single backticks: `foo`, `Foo::bar()`, `self.get`. Each gives its last
/// part. A span with a space or a path is not a name.
fn names(text: &str) -> Vec<(String, usize)> {
    let mut out = vec![];
    let mut rest = text;
    let mut base = 0;
    while let Some(a) = rest.find('`') {
        // A code fence is not a name.
        if rest[a..].starts_with("``") {
            let skip = rest[a..].chars().take_while(|c| *c == '`').count();
            base += a + skip;
            rest = &rest[a + skip..];
            continue;
        }
        let Some(z) = rest[a + 1..].find('`') else { break };
        let span = &rest[a + 1..a + 1 + z];
        let at = base + a;
        base += a + z + 2;
        rest = &rest[a + z + 2..];
        let s = span.trim().trim_end_matches("()").trim_end_matches('!');
        if s.is_empty() || s.len() > 120 || s.contains(char::is_whitespace) || s.contains('/') {
            continue;
        }
        let last = s.rsplit("::").next().unwrap_or(s).rsplit('.').next().unwrap_or(s);
        let ok = last.len() >= 3
            && last.bytes().all(is_word)
            && !last.as_bytes()[0].is_ascii_digit()
            && !SKIP.contains(&last);
        if ok {
            out.push((last.to_owned(), at));
        }
    }
    out
}

/// The first place of a whole word in the changed files: a changed line first, else
/// the first line that has it. New side first, then the removed lines.
fn search(m: &DiffModel, word: &str) -> Option<(String, u32, &'static str, bool)> {
    let w = word.as_bytes();
    let has = |line: &[u8]| {
        let mut k = 0;
        while k + w.len() <= line.len() {
            if &line[k..k + w.len()] == w
                && (k == 0 || !is_word(line[k - 1]))
                && (k + w.len() == line.len() || !is_word(line[k + w.len()]))
            {
                return true;
            }
            k += 1;
        }
        false
    };
    let mut first = None;
    for f in m.files.iter().filter(|f| !f.binary && !f.submodule) {
        let Some(path) = f.new_path.as_deref() else { continue };
        for (i, line) in f.new.iter().enumerate() {
            if has(line) {
                if f.added.get(i).copied().unwrap_or(false) {
                    return Some((path.to_owned(), i as u32 + 1, "new", true));
                }
                if first.is_none() {
                    first = Some((path.to_owned(), i as u32 + 1, "new", false));
                }
            }
        }
    }
    if first.is_some() {
        return first;
    }
    for f in m.files.iter().filter(|f| !f.binary && !f.submodule) {
        let Some(path) = f.old_path.as_deref() else { continue };
        for (i, line) in f.old.iter().enumerate() {
            if f.removed.get(i).copied().unwrap_or(false) && has(line) {
                return Some((path.to_owned(), i as u32 + 1, "old", true));
            }
        }
    }
    None
}

/// The places an answer refers to, in the order of the text, at most `MAX_MENTIONS`.
/// `tree` is the review worktree, so an absolute path in it counts. A path outside
/// the review, a line past the end of its file, and a name that no changed file has,
/// give nothing.
pub fn extract(text: &str, m: &DiffModel, tree: &str) -> Vec<Mention> {
    let mut cut = text.len().min(MAX_TEXT);
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let text = &text[..cut];
    let mut found: Vec<(usize, Mention)> = vec![];
    for (token, line, end, at) in paths(text) {
        let Ok(path) = clean_path(&token, tree) else { continue };
        let Some((f, side)) = find_file(m, &path) else { continue };
        let file = if side == "old" { f.old_path.clone() } else { f.new_path.clone() }.unwrap_or_default();
        let count = lines(f, side).len() as u32;
        let (line, end) = match line {
            Some(l) if l >= 1 && l <= count => (Some(l), end.filter(|e| *e > l && *e <= count)),
            Some(_) => continue,
            None => (None, None),
        };
        let changed = match line {
            Some(l) => changed(f, side, l, end.unwrap_or(l)),
            None => true,
        };
        found.push((at, Mention { file, line, end, side: side.into(), name: None, changed }));
    }
    for (name, at) in names(text) {
        // A name that is a file name was found above.
        if name.contains('.') {
            continue;
        }
        if let Some((file, line, side, changed)) = search(m, &name) {
            found.push((at, Mention { file, line: Some(line), end: None, side: side.into(), name: Some(name), changed }));
        }
    }
    found.sort_by_key(|(at, _)| *at);
    let mut out: Vec<Mention> = vec![];
    for (_, x) in found {
        let dup = out.iter().any(|y| y.file == x.file && y.line == x.line && y.side == x.side);
        // A file name with no line adds nothing to a line of the same file.
        let covered = x.line.is_none() && out.iter().any(|y| y.file == x.file);
        if !dup && !covered {
            out.push(x);
        }
        if out.len() == MAX_MENTIONS {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, new: &[&str], added: &[usize], old: &[&str], removed: &[usize]) -> ChangedFile {
        ChangedFile {
            old_path: Some(path.into()),
            new_path: Some(path.into()),
            new: new.iter().map(|l| l.as_bytes().to_vec()).collect(),
            added: (1..=new.len()).map(|n| added.contains(&n)).collect(),
            old: old.iter().map(|l| l.as_bytes().to_vec()).collect(),
            removed: (1..=old.len()).map(|n| removed.contains(&n)).collect(),
            ..ChangedFile::default()
        }
    }

    fn model() -> DiffModel {
        let body: Vec<String> = (1..=300).map(|n| format!("line {n}")).collect();
        let mut store: Vec<&str> = body.iter().map(String::as_str).collect();
        store[214] = "    let bid = self.put_bid(root, &bid)?;";
        store[9] = "fn put_bid(&self) {}";
        let mut fc = vec!["fn a() {}"; 20];
        fc[11] = "    process_withdrawals(state)?;";
        DiffModel {
            base: "b".into(),
            head: "h".into(),
            files: vec![
                file("beacon_node/store/src/hot_cold_store.rs", &store, &[215, 216], &["old"], &[]),
                file("consensus/fork_choice/src/fork_choice.rs", &fc, &[3], &["x", "    old_weight(bid)", "y"], &[2]),
                file("a/mod.rs", &["x"], &[1], &[], &[]),
                file("b/mod.rs", &["x"], &[1], &[], &[]),
            ],
        }
    }

    const TREE: &str = "/c/lighthouse-pr7";

    #[test]
    fn paths_short_names_and_ranges() {
        let m = model();
        let got = extract(
            "See beacon_node/store/src/hot_cold_store.rs:215-216, then fork_choice.rs:12. Also `fork_choice.rs:3`.",
            &m,
            TREE,
        );
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!((got[0].file.as_str(), got[0].line, got[0].end, got[0].changed), ("beacon_node/store/src/hot_cold_store.rs", Some(215), Some(216), true));
        assert_eq!((got[1].file.as_str(), got[1].line, got[1].changed), ("consensus/fork_choice/src/fork_choice.rs", Some(12), false));
        assert_eq!((got[2].line, got[2].changed), (Some(3), true));
    }

    #[test]
    fn absolute_paths_in_the_tree_count_and_others_do_not() {
        let m = model();
        let got = extract("/c/lighthouse-pr7/consensus/fork_choice/src/fork_choice.rs:3 and /etc/fork_choice.rs:3 and ../fork_choice.rs:4 and ~/fork_choice.rs:5", &m, TREE);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].line, Some(3));
    }

    #[test]
    fn a_line_past_the_end_an_unknown_file_and_two_files_of_one_name_give_nothing() {
        let m = model();
        assert!(extract("fork_choice.rs:21 and nothing.rs:3 and mod.rs:1", &m, TREE).is_empty());
        // A full path picks one of the two.
        assert_eq!(extract("a/mod.rs:1", &m, TREE)[0].file, "a/mod.rs");
    }

    #[test]
    fn a_bare_file_name_is_a_mention_with_no_line() {
        let m = model();
        let got = extract("The change is in hot_cold_store.rs.", &m, TREE);
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].line, got[0].changed), (None, true));
        // With a line of the same file too, the name alone adds nothing.
        assert_eq!(extract("hot_cold_store.rs:215 and hot_cold_store.rs", &m, TREE).len(), 1);
    }

    #[test]
    fn backtick_names_find_a_changed_line_first() {
        let m = model();
        // `put_bid` is on line 10 (not changed) and on 215 (changed).
        let got = extract("It calls `self.put_bid()` and `process_withdrawals` and `old_weight` and `missing_fn`.", &m, TREE);
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!((got[0].name.as_deref(), got[0].line, got[0].changed), (Some("put_bid"), Some(215), true));
        assert_eq!((got[1].name.as_deref(), got[1].line, got[1].changed), (Some("process_withdrawals"), Some(12), false));
        // Only on a removed line: the old side.
        assert_eq!((got[2].side.as_str(), got[2].line), ("old", Some(2)));
    }

    #[test]
    fn names_are_whole_words_and_skip_keywords_and_code() {
        let m = model();
        // `put` is part of `put_bid`; `self`, `Some` and a span with a space are not names.
        assert!(extract("`put` `self` `Some` `let x = 1` ```put_bid```", &m, TREE).is_empty());
    }

    #[test]
    fn the_list_is_capped_and_has_no_duplicates() {
        let m = model();
        let text: String = (1..=40).map(|n| format!("hot_cold_store.rs:{n} hot_cold_store.rs:{n} ")).collect();
        let got = extract(&text, &m, TREE);
        assert_eq!(got.len(), MAX_MENTIONS);
        assert_eq!(got[1].line, Some(2));
        // A long text is cut, on a char boundary.
        let long = "é".repeat(MAX_TEXT);
        assert!(extract(&long, &m, TREE).is_empty());
    }

    #[test]
    fn clean_path_refuses_what_leaves_the_review() {
        assert_eq!(clean_path("./a/b.rs", TREE).unwrap(), "a/b.rs");
        assert_eq!(clean_path("/c/lighthouse-pr7/a/b.rs", TREE).unwrap(), "a/b.rs");
        for bad in ["", "/etc/passwd", "/c/lighthouse-pr7x/a.rs", "../a.rs", "a/../../b.rs", "~/a.rs", "a//b.rs", "a\nb.rs", "a\\b.rs", "a/./b.rs"] {
            assert!(clean_path(bad, TREE).is_err(), "{bad:?}");
        }
    }
}
