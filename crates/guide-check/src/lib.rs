//! The verified kernel of the review guide check.
//!
//! The LLM proposes a review guide. This crate decides if the guide is
//! accepted. `proofs/` proves in Lean, with Aeneas, what `check` means:
//!
//! - **Rebuild.** For every file, the old lines that the diff does not remove
//!   are exactly the new lines that the diff does not add. So the diff names
//!   every change: no change can hide outside the changed lines.
//! - **Coverage.** Every changed line is in a span of the guide, with the same
//!   file and side.
//! - **Real spans.** Every span is inside its file and holds a changed line.
//! - **Tight spans.** Every line of a span is at most `pad` lines from a
//!   changed line, so one huge span cannot cover a whole file.
//!
//! The code stays in the Rust subset that Aeneas translates: `while` loops,
//! small functions, no `?`, no closures, no `&&` or `||` in a loop condition.
//! Lines are bytes, with their newline, so a change to a final newline counts.

#![forbid(unsafe_code)]
// Nested `if`s on purpose: the kernel avoids `&&` so the Aeneas translation stays plain.
#![allow(clippy::collapsible_if)]
// A plain `match` on purpose: the kernel avoids macros so the translation stays plain.
#![allow(clippy::match_like_matches_macro)]

/// One changed file. `removed[k]` is true when the diff removes old line `k + 1`;
/// `added[k]` is true when the diff adds new line `k + 1`.
pub struct FileDiff {
    pub old: Vec<Vec<u8>>,
    pub new: Vec<Vec<u8>>,
    pub removed: Vec<bool>,
    pub added: Vec<bool>,
}

/// Lines `from..=to` (1-based) of one side of one file, in one step of the guide.
/// `old` is true for the old side (removed lines), false for the new side.
pub struct Span {
    pub file: usize,
    pub old: bool,
    pub from: usize,
    pub to: usize,
}

pub fn bytes_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// The first index at or after `i` that the mask keeps, or `mask.len()`.
pub fn next_kept(mask: &[bool], i: usize) -> usize {
    let mut j = i;
    while j < mask.len() {
        if !mask[j] {
            return j;
        }
        j += 1;
    }
    j
}

/// True when either index has lines left.
pub fn lines_left(i: usize, n: usize, j: usize, m: usize) -> bool {
    if i < n {
        return true;
    }
    j < m
}

/// Compares the kept old lines from `i0` with the kept new lines from `j0`.
/// The masks must have the lengths of their line lists.
pub fn kept_equal_from(
    old: &[Vec<u8>],
    removed: &[bool],
    new: &[Vec<u8>],
    added: &[bool],
    i0: usize,
    j0: usize,
) -> bool {
    let mut i = i0;
    let mut j = j0;
    while lines_left(i, old.len(), j, new.len()) {
        let a = next_kept(removed, i);
        let b = next_kept(added, j);
        if a >= old.len() {
            return b >= new.len();
        }
        if b >= new.len() {
            return false;
        }
        if !bytes_equal(&old[a], &new[b]) {
            return false;
        }
        i = a + 1;
        j = b + 1;
    }
    true
}

/// True when the span holds line `line` of side `old` of file `file`.
pub fn span_holds(s: &Span, file: usize, old: bool, line: usize) -> bool {
    if s.file != file {
        return false;
    }
    if s.old != old {
        return false;
    }
    if line < s.from {
        return false;
    }
    line <= s.to
}

/// True when some span holds the line.
pub fn covered(spans: &[Span], file: usize, old: bool, line: usize) -> bool {
    let mut i = 0;
    while i < spans.len() {
        if span_holds(&spans[i], file, old, line) {
            return true;
        }
        i += 1;
    }
    false
}

/// True when every changed line of the mask is covered.
pub fn mask_covered(spans: &[Span], file: usize, old: bool, mask: &[bool]) -> bool {
    let mut k = 0;
    while k < mask.len() {
        if mask[k] {
            if !covered(spans, file, old, k + 1) {
                return false;
            }
        }
        k += 1;
    }
    true
}

/// Rebuild and coverage for file number `f`.
pub fn file_ok(d: &FileDiff, f: usize, spans: &[Span]) -> bool {
    if d.removed.len() != d.old.len() {
        return false;
    }
    if d.added.len() != d.new.len() {
        return false;
    }
    if !kept_equal_from(&d.old, &d.removed, &d.new, &d.added, 0, 0) {
        return false;
    }
    if !mask_covered(spans, f, true, &d.removed) {
        return false;
    }
    mask_covered(spans, f, false, &d.added)
}

/// Rebuild and coverage for every file.
pub fn files_ok(files: &[FileDiff], spans: &[Span]) -> bool {
    let mut f = 0;
    while f < files.len() {
        if !file_ok(&files[f], f, spans) {
            return false;
        }
        f += 1;
    }
    true
}

/// True when some line in `from..=to` is changed. Needs `1 <= from` and `to <= mask.len()`.
/// The index `k` is line `k + 1`, so `k < to` never overflows.
pub fn has_change(mask: &[bool], from: usize, to: usize) -> bool {
    let mut k = from - 1;
    while k < to {
        if mask[k] {
            return true;
        }
        k += 1;
    }
    false
}

pub fn dist(a: usize, b: usize) -> usize {
    if a < b {
        return b - a;
    }
    a - b
}

/// True when a changed line is at most `pad` lines from `line`.
pub fn near(mask: &[bool], line: usize, pad: usize) -> bool {
    let mut k = 0;
    while k < mask.len() {
        if mask[k] {
            if dist(k + 1, line) <= pad {
                return true;
            }
        }
        k += 1;
    }
    false
}

/// True when every line in `from..=to` is near a changed line. Needs `1 <= from`.
pub fn all_near(mask: &[bool], from: usize, to: usize, pad: usize) -> bool {
    let mut k = from - 1;
    while k < to {
        if !near(mask, k + 1, pad) {
            return false;
        }
        k += 1;
    }
    true
}

/// A span against the mask of its side: in bounds, real, and tight.
pub fn span_fits(mask: &[bool], from: usize, to: usize, pad: usize) -> bool {
    if from == 0 {
        return false;
    }
    if from > to {
        return false;
    }
    if to > mask.len() {
        return false;
    }
    if !has_change(mask, from, to) {
        return false;
    }
    all_near(mask, from, to, pad)
}

pub fn span_ok(s: &Span, files: &[FileDiff], pad: usize) -> bool {
    if s.file >= files.len() {
        return false;
    }
    if s.old {
        return span_fits(&files[s.file].removed, s.from, s.to, pad);
    }
    span_fits(&files[s.file].added, s.from, s.to, pad)
}

pub fn spans_ok(spans: &[Span], files: &[FileDiff], pad: usize) -> bool {
    let mut i = 0;
    while i < spans.len() {
        if !span_ok(&spans[i], files, pad) {
            return false;
        }
        i += 1;
    }
    true
}

/// The whole check. `true` means the guide is accepted.
pub fn check(files: &[FileDiff], spans: &[Span], pad: usize) -> bool {
    if !files_ok(files, spans) {
        return false;
    }
    spans_ok(spans, files, pad)
}

/// What a diff row shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A line on both sides: not removed, not added.
    Same,
    Removed,
    Added,
    /// The start of a run of rows after a cut.
    Header,
}

/// One row of the diff of a file, with no text. `old` and `new` are 1-based line
/// numbers, and 0 means "no line on this side". The caller gets the text from the
/// line list with the number, so the text and the number cannot disagree.
/// A header carries the numbers of the next old line and the next new line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub kind: Kind,
    pub old: usize,
    pub new: usize,
}

/// The kind of the next row when the walk is at old index `i` and new index `j`.
/// A removed line comes first, then an added line, then a line on both sides.
/// `Header` means that the walk is stuck: one side has unchanged lines and the
/// other side has none. That cannot happen when the file rebuilds.
pub fn next_kind(removed: &[bool], added: &[bool], i: usize, j: usize) -> Kind {
    if i < removed.len() {
        if removed[i] {
            return Kind::Removed;
        }
    }
    if j < added.len() {
        if added[j] {
            return Kind::Added;
        }
    }
    if i < removed.len() {
        if j < added.len() {
            return Kind::Same;
        }
    }
    Kind::Header
}

/// Every row of the diff of a file, in the order of the rebuild check.
/// The masks give the line counts: `removed` has one bit per old line, `added`
/// one bit per new line.
pub fn number_rows(removed: &[bool], added: &[bool]) -> Vec<Row> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while lines_left(i, removed.len(), j, added.len()) {
        let kind = next_kind(removed, added, i, j);
        match kind {
            Kind::Removed => {
                i += 1;
                out.push(Row { kind, old: i, new: 0 });
            }
            Kind::Added => {
                j += 1;
                out.push(Row { kind, old: 0, new: j });
            }
            Kind::Same => {
                i += 1;
                j += 1;
                out.push(Row { kind, old: i, new: j });
            }
            Kind::Header => {
                return out;
            }
        }
    }
    out
}

pub fn is_change(r: &Row) -> bool {
    match r.kind {
        Kind::Removed => true,
        Kind::Added => true,
        _ => false,
    }
}

/// The first row index that is at most `ctx` rows before row `k`.
pub fn window_start(k: usize, ctx: usize) -> usize {
    if k > ctx {
        return k - ctx;
    }
    0
}

/// One past the last row index that is at most `ctx` rows after row `k`. Needs `k < len`.
pub fn window_end(k: usize, ctx: usize, len: usize) -> usize {
    if ctx < len - k {
        return k + ctx + 1;
    }
    len
}

/// True when a row at most `ctx` rows from row `k` is removed or added. Needs `k < rows.len()`.
pub fn near_change(rows: &[Row], k: usize, ctx: usize) -> bool {
    let mut c = window_start(k, ctx);
    let end = window_end(k, ctx, rows.len());
    while c < end {
        if is_change(&rows[c]) {
            return true;
        }
        c += 1;
    }
    false
}

/// True when the row shows an old line: a removed row or a row on both sides.
pub fn shows_old(r: &Row) -> bool {
    match r.kind {
        Kind::Removed => true,
        Kind::Same => true,
        _ => false,
    }
}

/// True when the row shows a new line: an added row or a row on both sides.
pub fn shows_new(r: &Row) -> bool {
    match r.kind {
        Kind::Added => true,
        Kind::Same => true,
        _ => false,
    }
}

/// `n + 1` when `b` is true, else `n`.
pub fn bump(n: usize, b: bool) -> usize {
    if b {
        return n + 1;
    }
    n
}

/// Puts row `r` in the cut. A header with the numbers `old` and `new` comes first
/// when the row before `r` is not kept.
pub fn keep_row(out: &mut Vec<Row>, kept: bool, r: Row, old: usize, new: usize) {
    if !kept {
        out.push(Row {
            kind: Kind::Header,
            old,
            new,
        });
    }
    out.push(r);
}

/// The rows that are at most `ctx` rows from a removed or added row, with a
/// header before each run of kept rows. A header carries the number of the next
/// old line and of the next new line: one more than the count of old (new) lines
/// in the rows before it. The rows must have no header.
pub fn cut_rows(rows: &[Row], ctx: usize) -> Vec<Row> {
    let mut out = Vec::new();
    let mut k = 0;
    let mut old = 1;
    let mut new = 1;
    // True when row `k - 1` is kept.
    let mut kept = false;
    while k < rows.len() {
        let near = near_change(rows, k, ctx);
        if near {
            keep_row(&mut out, kept, rows[k], old, new);
        }
        kept = near;
        old = bump(old, shows_old(&rows[k]));
        new = bump(new, shows_new(&rows[k]));
        k += 1;
    }
    out
}

#[cfg(test)]
mod tests;
