//! The verified kernel of the `git` tool of the guide agent (SPEC 11.4, 24.1).
//!
//! The guide agent only reads. It runs git through one tool: a subcommand from a
//! fixed list, and args that must each pass a list of safe options. This crate
//! decides if a call may run, and gives the full argv after the subcommand, with
//! the options that turn off external diff and textconv programs first.
//! `proofs/GitArgCheck` proves in Lean, with Aeneas, what `check` means.
//!
//! It also has `rel_ok`, the check that a path stays inside a folder, and in
//! `refs` the checks of a ref, a remote branch name and a GitHub URL (SPEC 24.2).
//!
//! The code stays in the Rust subset that Aeneas translates: `while` loops,
//! small functions, no `?`, no closures, no `&&` or `||`. It works on bytes.
//! A list of names is one byte string, with a space between two names.

#![forbid(unsafe_code)]
// Nested `if`s on purpose: the kernel avoids `&&` so the Aeneas translation stays plain.
#![allow(clippy::collapsible_if)]
#![allow(clippy::needless_bool)]

pub mod refs;

#[cfg(test)]
mod tests;

/// The subcommands, as codes.
pub const DIFF: u8 = 0;
pub const LOG: u8 = 1;
pub const SHOW: u8 = 2;
pub const BLAME: u8 = 3;
pub const MERGE_BASE: u8 = 4;
pub const REV_PARSE: u8 = 5;
/// Not a subcommand the tool runs.
pub const NO_SUB: u8 = 6;

/// Why a call is refused.
pub const BAD_SUB: u8 = 1;
pub const EMPTY: u8 = 2;
pub const CONTROL: u8 = 3;
pub const OPTION: u8 = 4;
pub const PATH: u8 = 5;

/// A refused call: why, and the index of the arg (0 for a bad subcommand).
pub struct Refusal {
    pub why: u8,
    pub at: usize,
}

/// The subcommands the tool runs.
pub const SUBS: &[u8] = b"diff log show blame merge-base rev-parse";

/// Diff options of `diff`, `log` and `show` with no value.
pub const DIFF_FLAGS: &[u8] = b"-p --patch -s --no-patch --stat --numstat --shortstat \
--name-only --name-status --summary --raw --patch-with-stat \
--patch-with-raw --compact-summary --dirstat --cumulative -M \
--find-renames -C --find-copies --find-copies-harder -D \
--irreversible-delete --no-renames -w --ignore-all-space -b \
--ignore-space-change --ignore-space-at-eol --ignore-blank-lines \
--ignore-cr-at-eol --word-diff --color-words --no-color --relative \
--no-relative -W --function-context --histogram --patience \
--minimal --abbrev --no-abbrev --full-index -R --no-prefix \
-a --text";

/// Diff options of `diff`, `log` and `show` in the `--name=value` form.
pub const DIFF_VALUES: &[u8] = b"--unified --stat --dirstat --find-renames --find-copies \
--word-diff --word-diff-regex --color-words --relative \
--diff-algorithm --diff-filter --abbrev --src-prefix --dst-prefix \
--inter-hunk-context --anchored";

/// Options of `git diff` alone.
pub const DIFF_ONLY: &[u8] = b"--cached --staged --merge-base --exit-code --quiet --check";

/// Log options of `log` and `show` with no value.
pub const LOG_FLAGS: &[u8] = b"--oneline --graph --decorate --no-decorate --first-parent \
--no-merges --merges --reverse --follow --all --parents \
--children --topo-order --date-order --author-date-order \
--ancestry-path --left-right --cherry-pick --cherry-mark --cherry \
--boundary --full-history --simplify-merges --simplify-by-decoration \
--dense --sparse --pickaxe-all --pickaxe-regex -i \
--regexp-ignore-case -E --extended-regexp -F --fixed-strings \
--all-match --invert-grep -m --cc --no-walk --source \
--abbrev-commit --no-abbrev-commit --pretty -L -n --quiet";

/// Log options of `log` and `show` in the `--name=value` form.
pub const LOG_VALUES: &[u8] = b"--max-count --skip --since --after --until --before --author \
--committer --grep --format --pretty --date --decorate \
--diff-merges --max-parents --min-parents --no-walk";

pub const BLAME_FLAGS: &[u8] = b"-w -M -C -s -e --show-email -l -t -f --show-name \
-n --show-number -p --porcelain --line-porcelain --incremental \
--root --show-stats -b --first-parent --reverse --abbrev -L";

pub const BLAME_VALUES: &[u8] = b"--date --abbrev";

pub const MERGE_BASE_FLAGS: &[u8] = b"--is-ancestor --fork-point --all -a --octopus --independent";

pub const REV_PARSE_FLAGS: &[u8] = b"--verify --short -q --quiet --abbrev-ref --symbolic \
--symbolic-full-name --show-toplevel --show-prefix --is-inside-work-tree";

pub const REV_PARSE_VALUES: &[u8] = b"--short --abbrev-ref";

/// True when `a[from..to]` equals `b`. Needs `from <= to <= a.len()`.
pub fn equal_at(a: &[u8], from: usize, to: usize, b: &[u8]) -> bool {
    if to - from != b.len() {
        return false;
    }
    let mut i = 0;
    while i < b.len() {
        if a[from + i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// True when `w` is one of the pieces of `p`, split at every `sep` byte.
pub fn has_piece(p: &[u8], sep: u8, w: &[u8]) -> bool {
    let mut start = 0;
    let mut k = 0;
    while k < p.len() {
        if p[k] == sep {
            if equal_at(p, start, k, w) {
                return true;
            }
            start = k + 1;
        }
        k += 1;
    }
    equal_at(p, start, p.len(), w)
}

/// True when `name` is in the list.
pub fn in_list(name: &[u8], list: &[u8]) -> bool {
    has_piece(list, b' ', name)
}

/// True when the byte `x` is in `p`.
pub fn has_byte(p: &[u8], x: u8) -> bool {
    let mut i = 0;
    while i < p.len() {
        if p[i] == x {
            return true;
        }
        i += 1;
    }
    false
}

/// True when `a[from..to]` is not empty and every byte of it is a digit.
pub fn digits_at(a: &[u8], from: usize, to: usize) -> bool {
    if to <= from {
        return false;
    }
    let mut i = from;
    while i < to {
        if a[i] < b'0' {
            return false;
        }
        if a[i] > b'9' {
            return false;
        }
        i += 1;
    }
    true
}

/// True when `s` is not empty and every byte of it is a digit.
pub fn is_digits(s: &[u8]) -> bool {
    digits_at(s, 0, s.len())
}

/// True when `p` is a path that stays inside the folder it is read from: not empty,
/// no NUL byte, no leading `/` or `~`, and no piece between two `/` is `..`.
pub fn rel_ok(p: &[u8]) -> bool {
    if p.is_empty() {
        return false;
    }
    if p[0] == b'/' {
        return false;
    }
    if p[0] == b'~' {
        return false;
    }
    if has_byte(p, 0) {
        return false;
    }
    if has_piece(p, b'/', b"..") {
        return false;
    }
    true
}

/// The code of a subcommand, or `NO_SUB`.
pub fn sub_code(sub: &[u8]) -> u8 {
    if equal_at(sub, 0, sub.len(), b"diff") {
        return DIFF;
    }
    if equal_at(sub, 0, sub.len(), b"log") {
        return LOG;
    }
    if equal_at(sub, 0, sub.len(), b"show") {
        return SHOW;
    }
    if equal_at(sub, 0, sub.len(), b"blame") {
        return BLAME;
    }
    if equal_at(sub, 0, sub.len(), b"merge-base") {
        return MERGE_BASE;
    }
    if equal_at(sub, 0, sub.len(), b"rev-parse") {
        return REV_PARSE;
    }
    NO_SUB
}

/// True for `diff`, `log` and `show`.
pub fn diffs(code: u8) -> bool {
    if code == DIFF {
        return true;
    }
    if code == LOG {
        return true;
    }
    code == SHOW
}

/// True for `log` and `show`.
pub fn logs(code: u8) -> bool {
    if code == LOG {
        return true;
    }
    code == SHOW
}

/// True when `name` is an option of the subcommand with no value.
pub fn flag_ok(code: u8, name: &[u8]) -> bool {
    if code == DIFF {
        if in_list(name, DIFF_FLAGS) {
            return true;
        }
        return in_list(name, DIFF_ONLY);
    }
    if logs(code) {
        if in_list(name, DIFF_FLAGS) {
            return true;
        }
        return in_list(name, LOG_FLAGS);
    }
    if code == BLAME {
        return in_list(name, BLAME_FLAGS);
    }
    if code == MERGE_BASE {
        return in_list(name, MERGE_BASE_FLAGS);
    }
    if code == REV_PARSE {
        return in_list(name, REV_PARSE_FLAGS);
    }
    false
}

/// True when `name` is an option of the subcommand in the `--name=value` form.
pub fn value_ok(code: u8, name: &[u8]) -> bool {
    if code == DIFF {
        return in_list(name, DIFF_VALUES);
    }
    if logs(code) {
        if in_list(name, DIFF_VALUES) {
            return true;
        }
        return in_list(name, LOG_VALUES);
    }
    if code == BLAME {
        return in_list(name, BLAME_VALUES);
    }
    if code == REV_PARSE {
        return in_list(name, REV_PARSE_VALUES);
    }
    false
}

/// The index of the first `=` in `a` at or after `from`, or `a.len()`.
pub fn find_eq(a: &[u8], from: usize) -> usize {
    let mut i = from;
    while i < a.len() {
        if a[i] == b'=' {
            return i;
        }
        i += 1;
    }
    a.len()
}

/// The end of `a` without its trailing `%` bytes. It stops at `from`.
pub fn trim_percent(a: &[u8], from: usize) -> usize {
    let mut n = a.len();
    while from < n {
        if a[n - 1] != b'%' {
            return n;
        }
        n -= 1;
    }
    n
}

/// A short option with its value glued on, such as `-U0` or `-n5`. Needs `a[0] == '-'`.
pub fn glued_ok(code: u8, a: &[u8]) -> bool {
    if a.len() < 3 {
        return false;
    }
    let f = a[1];
    if f == b'U' {
        if diffs(code) {
            return digits_at(a, 2, a.len());
        }
        return false;
    }
    if f == b'M' {
        return score_ok(code, a);
    }
    if f == b'C' {
        return score_ok(code, a);
    }
    if f == b'l' {
        if diffs(code) {
            return digits_at(a, 2, a.len());
        }
        return false;
    }
    if f == b'S' {
        return diffs(code);
    }
    if f == b'G' {
        return diffs(code);
    }
    if f == b'L' {
        if logs(code) {
            return true;
        }
        return code == BLAME;
    }
    if f == b'n' {
        if logs(code) {
            return digits_at(a, 2, a.len());
        }
        return false;
    }
    false
}

/// A rename or copy score: `-M50` or `-M50%`. Needs `a.len() >= 2`.
pub fn score_ok(code: u8, a: &[u8]) -> bool {
    if diffs(code) {
        return digits_at(a, 2, trim_percent(a, 2));
    }
    if code == BLAME {
        return digits_at(a, 2, trim_percent(a, 2));
    }
    false
}

/// Why the arg may not go to the subcommand, or 0 when it may.
pub fn refuse(code: u8, a: &[u8]) -> u8 {
    if a.is_empty() {
        return EMPTY;
    }
    if has_byte(a, 0) {
        return CONTROL;
    }
    if has_byte(a, b'\n') {
        return CONTROL;
    }
    if equal_at(a, 0, a.len(), b"--") {
        return 0;
    }
    if a[0] == b'-' {
        if option_ok(code, a) {
            return 0;
        }
        return OPTION;
    }
    // A rev, a range or a path. It must stay in the review worktree: git diff with
    // a path outside the tree compares files on disk, as with `--no-index`.
    if rel_ok(a) {
        return 0;
    }
    PATH
}

/// True when the option `a` may go to the subcommand. Needs `a[0] == '-'`.
pub fn option_ok(code: u8, a: &[u8]) -> bool {
    if a.len() >= 2 {
        if a[1] == b'-' {
            let e = find_eq(a, 2);
            if e < a.len() {
                return value_ok(code, &a[0..e]);
            }
            return flag_ok(code, a);
        }
    }
    if flag_ok(code, a) {
        return true;
    }
    if code == LOG {
        if digits_at(a, 1, a.len()) {
            return true;
        }
    }
    glued_ok(code, a)
}

/// The index of the first arg that the subcommand refuses, with why. `why` is 0
/// when it refuses none.
pub fn first_refused(code: u8, args: &[Vec<u8>]) -> Refusal {
    let mut k = 0;
    while k < args.len() {
        let why = refuse(code, &args[k]);
        if why != 0 {
            return Refusal { why, at: k };
        }
        k += 1;
    }
    Refusal { why: 0, at: args.len() }
}

/// The options that come before the args: they turn off external diff and
/// textconv programs.
pub fn prefix(code: u8) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    if diffs(code) {
        out.push(b"--no-ext-diff".to_vec());
        out.push(b"--no-textconv".to_vec());
    } else if code == BLAME {
        out.push(b"--no-textconv".to_vec());
    }
    out
}

/// Adds a copy of every arg to `out`, in order.
pub fn push_all(out: &mut Vec<Vec<u8>>, args: &[Vec<u8>]) {
    let mut k = 0;
    while k < args.len() {
        out.push(args[k].clone());
        k += 1;
    }
}

/// Checks a git call. Returns the args to pass after the subcommand, with the
/// options that turn off external diff and textconv programs first.
pub fn check(sub: &[u8], args: &[Vec<u8>]) -> Result<Vec<Vec<u8>>, Refusal> {
    let code = sub_code(sub);
    if code == NO_SUB {
        return Err(Refusal { why: BAD_SUB, at: 0 });
    }
    let r = first_refused(code, args);
    if r.why != 0 {
        return Err(r);
    }
    let mut out = prefix(code);
    push_all(&mut out, args);
    Ok(out)
}
