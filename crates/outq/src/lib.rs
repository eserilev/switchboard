//! The output queue of a remote window (SPEC 29).
//!
//! The tmux reader thread pushes the bytes of each pane on screen. A writer thread
//! takes them and writes them to the window. The reader never waits for the
//! window: when a pane has more than `budget` bytes waiting, its bytes drop and the
//! pane needs a fresh screen. `proofs/` proves in Lean, with Aeneas:
//!
//! - **Budget.** A push never makes a live pane hold more than `budget` bytes.
//! - **Order.** A live pane holds its bytes in the order of the pushes.
//! - **No gap.** After a drop, no byte of that pane goes in until a fresh screen
//!   fills it. So the window never shows a screen with a hole in it.
//!
//! The code stays in the Rust subset that Aeneas translates, as `guide-check` does.

#![forbid(unsafe_code)]
// Nested `if`s on purpose: the kernel avoids `&&` so the Aeneas translation stays plain.
#![allow(clippy::collapsible_if)]
// `mem::replace` on purpose: Aeneas has a model of it, not of `mem::take`.
#![allow(clippy::mem_replace_with_default)]

/// The bytes go to the window.
pub const LIVE: u8 = 0;
/// The bytes dropped. The pane waits for the writer to ask for a fresh screen.
pub const DROPPING: u8 = 1;
/// The writer asked for a fresh screen. The pane waits for it.
pub const RESYNC: u8 = 2;

/// One pane: its key (the tmux pane number), its state and its waiting bytes.
pub struct PaneQ {
    pub key: u32,
    pub state: u8,
    pub bytes: Vec<u8>,
}

pub struct Queue {
    pub budget: usize,
    pub panes: Vec<PaneQ>,
}

pub fn new(budget: usize) -> Queue {
    Queue {
        budget,
        panes: Vec::new(),
    }
}

/// The index of the pane with `key`, or `panes.len()`.
pub fn find(q: &Queue, key: u32) -> usize {
    let mut i = 0;
    while i < q.panes.len() {
        if q.panes[i].key == key {
            return i;
        }
        i += 1;
    }
    i
}

/// The index of the pane with `key`. A new key gets a new live pane.
pub fn slot(q: &mut Queue, key: u32) -> usize {
    let i = find(q, key);
    if i == q.panes.len() {
        q.panes.push(PaneQ {
            key,
            state: LIVE,
            bytes: Vec::new(),
        });
    }
    i
}

/// True when `data` fits after `used` bytes in a budget. No add, so no overflow.
pub fn fits(used: usize, data: usize, budget: usize) -> bool {
    if used <= budget {
        return data <= budget - used;
    }
    false
}

/// Bytes from tmux for a pane. A live pane keeps them in order when they fit.
/// Else the pane drops all its bytes and waits for a fresh screen. A pane that
/// waits takes no bytes.
pub fn push(q: &mut Queue, key: u32, data: &[u8]) {
    let i = slot(q, key);
    let budget = q.budget;
    let p = &mut q.panes[i];
    if p.state == LIVE {
        if fits(p.bytes.len(), data.len(), budget) {
            p.bytes.extend_from_slice(data);
        } else {
            p.bytes = Vec::new();
            p.state = DROPPING;
        }
    }
}

/// The waiting bytes of the pane at index `i`, for the writer. The pane keeps its state.
pub fn take(q: &mut Queue, i: usize) -> Vec<u8> {
    let p = &mut q.panes[i];
    std::mem::replace(&mut p.bytes, Vec::new())
}

/// True once after a drop of the pane at index `i`: the writer then gets a fresh
/// screen for it.
pub fn want_screen(q: &mut Queue, i: usize) -> bool {
    let p = &mut q.panes[i];
    if p.state == DROPPING {
        p.state = RESYNC;
        return true;
    }
    false
}

/// The fresh screen of a pane that waits for it. Its bytes go first; new bytes
/// follow. A pane that does not wait keeps what it has.
pub fn fill(q: &mut Queue, key: u32, screen: Vec<u8>) {
    let i = find(q, key);
    if i < q.panes.len() {
        let p = &mut q.panes[i];
        if p.state == RESYNC {
            p.bytes = screen;
            p.state = LIVE;
        }
    }
}

/// A pane left the screen: it holds nothing and is live for the next time.
pub fn forget(q: &mut Queue, key: u32) {
    let i = find(q, key);
    if i < q.panes.len() {
        let p = &mut q.panes[i];
        p.bytes = Vec::new();
        p.state = LIVE;
    }
}

#[cfg(test)]
mod tests;
