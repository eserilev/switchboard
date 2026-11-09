//! Checks of refs, remote branch names and GitHub URLs (SPEC 24.2).
//!
//! `ref_ok` is the pure part of `base::verify`: a ref that may go to git as one
//! arg. `split_remote` splits `origin/main` at the longest remote. `github_repo`
//! finds the owner and the name in a GitHub remote URL.
//! `proofs/GitArgCheck/Refs.lean` and `proofs/GitArgCheck/GitHub.lean` prove what
//! they mean. The rules of the crate hold here too: `while` loops on bytes, no
//! `?`, no closures, no `&&` or `||`.

use crate::{digits_at, equal_at};

/// The longest ref that `ref_ok` takes, in bytes.
pub const REF_MAX: usize = 256;

/// True for a byte that a ref may not hold: a control byte (0 to 31, 127) or a space.
pub fn bad_ref_byte(x: u8) -> bool {
    if x <= b' ' {
        return true;
    }
    x == 127
}

/// True when `r` may go to git as a ref: not empty, no leading `-`, at most
/// `REF_MAX` bytes, and no control byte or space.
pub fn ref_ok(r: &[u8]) -> bool {
    if r.is_empty() {
        return false;
    }
    if r[0] == b'-' {
        return false;
    }
    if r.len() > REF_MAX {
        return false;
    }
    let mut i = 0;
    while i < r.len() {
        if bad_ref_byte(r[i]) {
            return false;
        }
        i += 1;
    }
    true
}

/// True when `name` is `r`, then `/`, then at least one byte.
pub fn fits(name: &[u8], r: &[u8]) -> bool {
    if r.len() >= name.len() {
        return false;
    }
    if r.len() + 1 >= name.len() {
        return false;
    }
    if name[r.len()] != b'/' {
        return false;
    }
    equal_at(name, 0, r.len(), r)
}

/// The length of the longest remote that `name` starts with, then `/` and a
/// branch: `a/b/x` finds the remote `a/b` before `a`. None when no remote fits.
pub fn split_remote(name: &[u8], remotes: &[Vec<u8>]) -> Option<usize> {
    let mut found = false;
    let mut best = 0;
    let mut k = 0;
    while k < remotes.len() {
        if fits(name, &remotes[k]) {
            if found {
                if remotes[k].len() > best {
                    best = remotes[k].len();
                }
            } else {
                found = true;
                best = remotes[k].len();
            }
        }
        k += 1;
    }
    if found {
        return Some(best);
    }
    None
}

/// Where the owner and the name are in a GitHub URL: `u[owner_start..owner_end]`
/// and `u[name_start..name_end]`.
pub struct Repo {
    pub owner_start: usize,
    pub owner_end: usize,
    pub name_start: usize,
    pub name_end: usize,
}

/// The byte in lower case, for ASCII letters.
pub fn lower(x: u8) -> u8 {
    if x >= b'A' {
        if x <= b'Z' {
            return x + 32;
        }
    }
    x
}

/// True when `u[h..h + w.len()]` is `w` with its letters in any case. `w` is in
/// lower case.
pub fn lower_equal_at(u: &[u8], h: usize, w: &[u8]) -> bool {
    if h > u.len() {
        return false;
    }
    if u.len() - h < w.len() {
        return false;
    }
    let mut i = 0;
    while i < w.len() {
        if lower(u[h + i]) != w[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// True when `u[h..h + 10]` is `github.com`, in any case.
pub fn host_at(u: &[u8], h: usize) -> bool {
    lower_equal_at(u, h, b"github.com")
}

/// True when `u` starts with `p`.
pub fn starts(u: &[u8], p: &[u8]) -> bool {
    if p.len() > u.len() {
        return false;
    }
    equal_at(u, 0, p.len(), p)
}

/// The index after `github.com` at `h` and then the byte `sep`, or 0.
pub fn after_host(u: &[u8], h: usize, sep: u8) -> usize {
    if !host_at(u, h) {
        return 0;
    }
    let k = h + 10;
    if k >= u.len() {
        return 0;
    }
    if u[k] != sep {
        return 0;
    }
    k + 1
}

/// The index of the first `x` in `a` at or after `from`, or `a.len()`.
pub fn find_byte(a: &[u8], from: usize, x: u8) -> usize {
    let mut i = from;
    while i < a.len() {
        if a[i] == x {
            return i;
        }
        i += 1;
    }
    a.len()
}

/// The start of the path in `ssh://git@github.com/` or `ssh://git@github.com:<port>/`, or 0.
pub fn ssh_start(u: &[u8]) -> usize {
    let s = after_host(u, 10, b'/');
    if s != 0 {
        return s;
    }
    let c = after_host(u, 10, b':');
    if c == 0 {
        return 0;
    }
    let j = find_byte(u, c, b'/');
    if j >= u.len() {
        return 0;
    }
    if !digits_at(u, c, j) {
        return 0;
    }
    j + 1
}

/// Where the path of a GitHub URL starts, or 0 when `u` does not start as one.
pub fn path_start(u: &[u8]) -> usize {
    if starts(u, b"https://") {
        return after_host(u, 8, b'/');
    }
    if starts(u, b"http://") {
        return after_host(u, 7, b'/');
    }
    if starts(u, b"git://") {
        return after_host(u, 6, b'/');
    }
    if starts(u, b"git@") {
        return after_host(u, 4, b':');
    }
    if starts(u, b"ssh://git@") {
        return ssh_start(u);
    }
    0
}

/// True when `u[from..to]` ends with `.git`. Needs `from <= to <= u.len()`.
pub fn ends_git(u: &[u8], from: usize, to: usize) -> bool {
    if to - from < 4 {
        return false;
    }
    equal_at(u, to - 4, to, b".git")
}

/// The owner and the name of a GitHub remote URL: `https://github.com/o/n`,
/// `http://`, `git://`, `git@github.com:o/n` or `ssh://git@github.com[:port]/o/n`,
/// each with an optional `.git` and then an optional `/`. The host is `github.com`
/// in any case. The path is exactly two parts, not empty, and the name does not
/// end with `.git`.
pub fn github_repo(u: &[u8]) -> Option<Repo> {
    let s = path_start(u);
    if s == 0 {
        return None;
    }
    let mut e = u.len();
    if s < e {
        if u[e - 1] == b'/' {
            e -= 1;
        }
    }
    if ends_git(u, s, e) {
        e -= 4;
    }
    let m = find_byte(u, s, b'/');
    if m >= e {
        return None;
    }
    if m == s {
        return None;
    }
    if m + 1 >= e {
        return None;
    }
    if find_byte(u, m + 1, b'/') < e {
        return None;
    }
    if ends_git(u, m + 1, e) {
        return None;
    }
    Some(Repo {
        owner_start: s,
        owner_end: m,
        name_start: m + 1,
        name_end: e,
    })
}
