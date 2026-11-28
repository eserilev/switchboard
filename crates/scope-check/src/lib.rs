//! Which window may run which command of the hub (SPEC 41.7).
//!
//! Every command of `dispatch` is in one table, by name, with its class. A window
//! is trusted (the app's own window, or `sb attach` on the hub's socket) or a
//! device (a phone). A trusted window runs every command of the table. A device
//! runs a command only when its classes hold the command's class, never an
//! `admin` command, and only when the arguments pass their checks.
//!
//! The statements to prove in Lean with Aeneas (SPEC 41.14):
//!
//! - **Table.** A command that is not in the table is refused, for every window.
//! - **Admin.** A device never runs an `admin` command, whatever its classes.
//! - **Class.** A device runs a command only when its classes hold the command's class.
//! - **Args.** A device runs `review_open` only with a GitHub PR URL.
//! - **More is more.** Adding a class to a device never refuses a command that it ran before.
//!
//! The code stays in the Rust subset that Aeneas translates, as `git-argcheck`
//! does: `while` loops with an index, no `?`, no closures, no `&&` or `||`. A
//! list of names is one byte string, with a space between two names.

#![forbid(unsafe_code)]
// Nested `if`s on purpose: the kernel avoids `&&` so the Aeneas translation stays plain.
#![allow(clippy::collapsible_if)]
#![allow(clippy::needless_bool)]

/// The classes, as bits. A device holds a set of them.
pub const READ: u8 = 1;
pub const REVIEW: u8 = 2;
pub const ASK: u8 = 4;
pub const ACT: u8 = 8;
pub const TERMINAL: u8 = 16;
pub const ADMIN: u8 = 32;

/// The classes a new phone gets: all but `terminal` and `admin`.
pub const PHONE: u8 = READ | REVIEW | ASK | ACT;

/// The windows.
pub const TRUSTED: u8 = 0;
pub const DEVICE: u8 = 1;

/// Why a command is refused. `OK` runs it.
pub const OK: u8 = 0;
pub const UNKNOWN: u8 = 1;
pub const NEVER: u8 = 2;
pub const CLASS: u8 = 3;
pub const ARGS: u8 = 4;

/// Commands that only read.
pub const READ_CMDS: &[u8] = b"panes mail_list mail_agents connections repos layouts settings reviews review_prior \
review_chat review_view review_diff review_file_rows review_context review_files \
review_file_diff review_drafts_text review_post_preview tasks task_history today \
today_job today_repo_check review_step_note";

/// Commands that change a review, a list or a mark, and post a review to GitHub.
/// They run no agent and no program of yours.
pub const REVIEW_CMDS: &[u8] = b"review_open_local review_step_seen review_dismiss review_mark review_pin \
review_draft review_pin_edit review_draft_edit review_comment review_summary \
review_post review_range review_mark_reviewed review_base review_base_update \
today_done today_later today_looked pane_seen pane_rename pane_reorder task_add \
task_edit task_status task_move task_delete task_copy task_restore sprint_new \
sprint_edit sprint_close task_link task_unlink";

/// Commands that start, or can start, a run of Claude that only reads: a guide, an
/// answer, or the order of Today.
pub const ASK_CMDS: &[u8] = b"review_ask review_retake review_open review_update review_round review_retry \
today_forget today_refresh";

/// Commands that make an agent act (free text to an agent with Bash, or a permit),
/// or that close its pane or drop its changes.
pub const ACT_CMDS: &[u8] = b"permit_answer pane_say review_send_agent review_work pane_resume connection_resume \
task_resume today_tell today_job_save today_job_remove review_restart review_close";

/// Commands that show or type in a terminal.
pub const TERMINAL_CMDS: &[u8] = b"pane_input pane_expand pane_attach pane_detach \
pane_collapse pane_image";

/// Commands that change the hub itself, open a program, or set a pane's size.
pub const ADMIN_CMDS: &[u8] = b"pane_open shell_open nvim_open layout_open layout_save \
settings_save connection_set trust_answer hub_restart task_start_plan handover_out \
handover_take review_term review_nvim review_edit pane_close pane_resize part_resize";

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

/// True when `name` is one of the names of `list`, split at every space.
pub fn in_list(name: &[u8], list: &[u8]) -> bool {
    let mut start = 0;
    let mut k = 0;
    while k < list.len() {
        if list[k] == b' ' {
            if equal_at(list, start, k, name) {
                return true;
            }
            start = k + 1;
        }
        k += 1;
    }
    equal_at(list, start, list.len(), name)
}

/// The class of a command, or 0 when it is not in the table.
pub fn class_of(cmd: &[u8]) -> u8 {
    if in_list(cmd, READ_CMDS) {
        return READ;
    }
    if in_list(cmd, REVIEW_CMDS) {
        return REVIEW;
    }
    if in_list(cmd, ASK_CMDS) {
        return ASK;
    }
    if in_list(cmd, ACT_CMDS) {
        return ACT;
    }
    if in_list(cmd, TERMINAL_CMDS) {
        return TERMINAL;
    }
    if in_list(cmd, ADMIN_CMDS) {
        return ADMIN;
    }
    0
}

/// True for a byte of a GitHub owner or repo name: a letter, a digit, `.`, `_` or `-`.
pub fn name_byte(x: u8) -> bool {
    if x >= b'a' {
        if x <= b'z' {
            return true;
        }
    }
    if x >= b'A' {
        if x <= b'Z' {
            return true;
        }
    }
    if x >= b'0' {
        if x <= b'9' {
            return true;
        }
    }
    if x == b'.' {
        return true;
    }
    if x == b'_' {
        return true;
    }
    x == b'-'
}

/// The end of the name that starts at `from`: the first byte that is not a name byte.
pub fn name_end(u: &[u8], from: usize) -> usize {
    let mut i = from;
    while i < u.len() {
        if !name_byte(u[i]) {
            return i;
        }
        i += 1;
    }
    u.len()
}

/// True when `u[from..to]` is `.` or `..`: a name that is not a GitHub name.
/// Needs `from <= to <= u.len()`.
pub fn dots(u: &[u8], from: usize, to: usize) -> bool {
    if equal_at(u, from, to, b".") {
        return true;
    }
    equal_at(u, from, to, b"..")
}

/// The end of the digits that start at `from`.
pub fn digits_end(u: &[u8], from: usize) -> usize {
    let mut i = from;
    while i < u.len() {
        if u[i] < b'0' {
            return i;
        }
        if u[i] > b'9' {
            return i;
        }
        i += 1;
    }
    u.len()
}

pub const PREFIX: &[u8] = b"https://github.com/";
pub const PULL: &[u8] = b"/pull/";

/// True when `u` is `https://github.com/<owner>/<repo>/pull/<number>`, then
/// nothing, or a `/`, `#` or `?` and anything. The owner and the repo are not
/// empty and hold only name bytes; the number has 1 to 10 digits.
pub fn pr_url_ok(u: &[u8]) -> bool {
    if u.len() < PREFIX.len() {
        return false;
    }
    if !equal_at(u, 0, PREFIX.len(), PREFIX) {
        return false;
    }
    let o = PREFIX.len();
    let o_end = name_end(u, o);
    if o_end == o {
        return false;
    }
    if dots(u, o, o_end) {
        return false;
    }
    if o_end >= u.len() {
        return false;
    }
    if u[o_end] != b'/' {
        return false;
    }
    let r = o_end + 1;
    let r_end = name_end(u, r);
    if r_end == r {
        return false;
    }
    if dots(u, r, r_end) {
        return false;
    }
    if u.len() - r_end < PULL.len() {
        return false;
    }
    if !equal_at(u, r_end, r_end + PULL.len(), PULL) {
        return false;
    }
    let n = r_end + PULL.len();
    let n_end = digits_end(u, n);
    if n_end == n {
        return false;
    }
    if n_end - n > 10 {
        return false;
    }
    if n_end == u.len() {
        return true;
    }
    let x = u[n_end];
    if x == b'/' {
        return true;
    }
    if x == b'#' {
        return true;
    }
    x == b'?'
}

/// Decides one command. `origin` is `TRUSTED` or `DEVICE`, `classes` the classes
/// of a device, and `url` the `url` argument of the command (empty when it has none).
/// Returns `OK`, or why the command is refused.
pub fn check(origin: u8, classes: u8, cmd: &[u8], url: &[u8]) -> u8 {
    let c = class_of(cmd);
    if c == 0 {
        return UNKNOWN;
    }
    if origin == TRUSTED {
        return OK;
    }
    if c == ADMIN {
        return NEVER;
    }
    if classes & c == 0 {
        return CLASS;
    }
    if equal_at(cmd, 0, cmd.len(), b"review_open") {
        if !pr_url_ok(url) {
            return ARGS;
        }
    }
    OK
}

#[cfg(test)]
mod tests;
