//! The verified byte parts of the diff parser (SPEC 24.2).
//!
//! Two small kernels that read GitHub's diff of a PR, on bytes:
//!
//! - `unquote` decodes a path that git wrote in quotes with C escapes, as in
//!   `"caf\303\251.rs"`. The caller strips the quotes and decodes UTF-8.
//! - `header` reads the numbers of a hunk header `@@ -a,b +c,d @@`, and `step`
//!   counts the line numbers of the rows of the hunk.
//!
//! `proofs/DiffParse` proves in Lean, with Aeneas, that they never panic or
//! overflow, that `unquote` undoes git's quoting, that a header whose end line does
//! not fit in a `u32` is refused, and that the counters of a well-formed hunk end
//! at `a + b` and `c + d`.
//!
//! The code stays in the Rust subset that Aeneas translates: `while` loops,
//! small functions, no `?`, no closures, no `&&` or `||`.

#![forbid(unsafe_code)]
// Nested `if`s on purpose: the kernel avoids `&&` so the Aeneas translation stays plain.
#![allow(clippy::collapsible_if)]
#![allow(clippy::needless_bool)]
#![allow(clippy::manual_range_contains)]
// A plain `match` on purpose: `Option::map` takes a closure, which Aeneas does not translate.
#![allow(clippy::manual_map)]
#![allow(clippy::single_match)]

#[cfg(test)]
mod tests;

/// The largest line number: `u32::MAX`.
pub const MAX: u32 = 4294967295;

/// True for an octal digit `0` to `7`.
pub fn is_octal(x: u8) -> bool {
    if x < b'0' {
        return false;
    }
    x <= b'7'
}

/// True for a decimal digit `0` to `9`.
pub fn is_digit(x: u8) -> bool {
    if x < b'0' {
        return false;
    }
    x <= b'9'
}

/// The byte of a one-letter escape that git writes, as `n` in `\n`, or `None`.
pub fn letter(c: u8) -> Option<u8> {
    if c == b'"' {
        return Some(c);
    }
    if c == b'\\' {
        return Some(c);
    }
    if c == b'a' {
        return Some(7);
    }
    if c == b'b' {
        return Some(8);
    }
    if c == b't' {
        return Some(9);
    }
    if c == b'n' {
        return Some(10);
    }
    if c == b'v' {
        return Some(11);
    }
    if c == b'f' {
        return Some(12);
    }
    if c == b'r' {
        return Some(13);
    }
    None
}

/// True when `c d e` is an octal byte that git writes: `000` to `377`.
pub fn octal_byte(c: u8, d: u8, e: u8) -> bool {
    if c < b'0' {
        return false;
    }
    if c > b'3' {
        return false;
    }
    if !is_octal(d) {
        return false;
    }
    is_octal(e)
}

/// The byte that starts at `b[k]`, and how many bytes of `b` it takes. Needs
/// `k < b.len()`. git writes `\ooo` only for a byte, so up to `\377`. A backslash
/// that starts no escape that git writes, as in `\400` or `\q`, is a plain byte.
pub fn decode_at(b: &[u8], k: usize) -> (u8, usize) {
    let x = b[k];
    if x != b'\\' {
        return (x, 1);
    }
    let left = b.len() - k;
    if left >= 4 {
        let c = b[k + 1];
        let d = b[k + 2];
        let e = b[k + 3];
        if octal_byte(c, d, e) {
            return ((c - b'0') * 64 + (d - b'0') * 8 + (e - b'0'), 4);
        }
    }
    if left >= 2 {
        match letter(b[k + 1]) {
            Some(y) => return (y, 2),
            None => {}
        }
    }
    (x, 1)
}

/// The bytes of a path that git quoted, without the quotes: `caf\303\251.rs`
/// gives the bytes of `café.rs`.
pub fn unquote(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut k = 0;
    while k < b.len() {
        let (y, n) = decode_at(b, k);
        out.push(y);
        k += n;
    }
    out
}

/// The index of the first byte at or after `k` that is not a digit, or `h.len()`.
pub fn digits_end(h: &[u8], k: usize) -> usize {
    let mut i = k;
    while i < h.len() {
        if !is_digit(h[i]) {
            return i;
        }
        i += 1;
    }
    i
}

/// The number in the digits `h[from..to]`, or `None` when it is over `MAX`.
/// Needs `from <= to <= h.len()` and only digits there.
pub fn value(h: &[u8], from: usize, to: usize) -> Option<u32> {
    let mut n: u32 = 0;
    let mut big = false;
    let mut i = from;
    while i < to {
        let d = (h[i] - b'0') as u32;
        if n > (MAX - d) / 10 {
            big = true;
            i = to;
        } else {
            n = n * 10 + d;
            i += 1;
        }
    }
    if big {
        return None;
    }
    Some(n)
}

/// The number at `h[k..]` and the index after its digits. `None` when there is no
/// digit, or when the number is over `MAX`.
pub fn number(h: &[u8], k: usize) -> Option<(u32, usize)> {
    let end = digits_end(h, k);
    if end == k {
        return None;
    }
    match value(h, k, end) {
        Some(n) => Some((n, end)),
        None => None,
    }
}

/// A range `start[,count]` at `h[k..]`: the start, the end `start + count`, and the
/// index after it. The count is 1 when it is not there. `None` when the end is over `MAX`.
pub fn range(h: &[u8], k: usize) -> Option<(u32, u32, usize)> {
    match number(h, k) {
        None => None,
        Some((start, next)) => {
            let mut count: u32 = 1;
            let mut after = next;
            if next < h.len() {
                if h[next] == b',' {
                    match number(h, next + 1) {
                        None => return None,
                        Some((c, a)) => {
                            count = c;
                            after = a;
                        }
                    }
                }
            }
            if count > MAX - start {
                return None;
            }
            Some((start, start + count, after))
        }
    }
}

/// The line counters of a hunk: the numbers of the next old and new line, and the
/// numbers after the last line of each side.
pub struct Counter {
    pub old: u32,
    pub new: u32,
    pub old_end: u32,
    pub new_end: u32,
}

/// Reads a hunk header after its `@@ `: `-a[,b] +c[,d]`, then the end or a space.
/// The counters start at `a` and `c`, and end at `a + b` and `c + d`. `None` when
/// the header has another form, or when an end is over `MAX`.
pub fn header(h: &[u8]) -> Option<Counter> {
    if h.is_empty() {
        return None;
    }
    if h[0] != b'-' {
        return None;
    }
    match range(h, 1) {
        None => None,
        Some((a, a_end, k)) => {
            if h.len() - k < 2 {
                return None;
            }
            if h[k] != b' ' {
                return None;
            }
            if h[k + 1] != b'+' {
                return None;
            }
            match range(h, k + 2) {
                None => None,
                Some((c, c_end, j)) => {
                    if j < h.len() {
                        if h[j] != b' ' {
                            return None;
                        }
                    }
                    Some(Counter {
                        old: a,
                        new: c,
                        old_end: a_end,
                        new_end: c_end,
                    })
                }
            }
        }
    }
}

/// The counters after one row of a hunk. A row that shows an old line takes the
/// number `c.old`, and a row that shows a new line takes `c.new`. `None` when the
/// hunk has no line left on a side the row shows: the row is not in the hunk.
pub fn step(c: &Counter, old_side: bool, new_side: bool) -> Option<Counter> {
    let mut old = c.old;
    let mut new = c.new;
    if old_side {
        if old >= c.old_end {
            return None;
        }
        old += 1;
    }
    if new_side {
        if new >= c.new_end {
            return None;
        }
        new += 1;
    }
    Some(Counter {
        old,
        new,
        old_end: c.old_end,
        new_end: c.new_end,
    })
}
