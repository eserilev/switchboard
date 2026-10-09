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

#[cfg(test)]
mod tests;
