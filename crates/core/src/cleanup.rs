//! When a closed review's worktree can go (SPEC 11.10). The rule is one pure
//! function, so the tests and the model in `models/review_close.qnt` check the same thing.

use crate::review::TreeState;

/// What the app knows about one worktree when it wants to remove it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Facts {
    /// The git state of the folder.
    pub tree: TreeState,
    /// A review row of the store names this folder. Only those folders are Switchboard's.
    pub in_store: bool,
    /// An open review uses this folder.
    pub open_review: bool,
    /// Panes in the folder that a close does not stop: agents and shells you opened,
    /// and nvim with unsaved files.
    pub panes: Vec<String>,
    /// The changes that you saw and chose to drop. A removal drops no other change.
    pub discard: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Remove it. `force` drops changes, and only the ones you chose to drop.
    Remove { force: bool },
    /// Nothing to remove: the folder or its worktree entry is gone.
    Gone,
    /// Keep it, for this reason.
    Keep(String),
    /// It has changes: ask before a removal.
    Ask(Vec<String>),
}

pub fn verdict(f: &Facts) -> Verdict {
    if !f.tree.exists || !f.tree.worktree {
        return Verdict::Gone;
    }
    if !f.in_store {
        return Verdict::Keep("no review made this folder".into());
    }
    if !f.tree.detached {
        return Verdict::Keep("a branch is checked out in it".into());
    }
    if f.tree.locked {
        return Verdict::Keep("it is locked (git worktree lock)".into());
    }
    if f.open_review {
        return Verdict::Keep("an open review uses it".into());
    }
    if !f.panes.is_empty() {
        return Verdict::Keep(format!("in use by {}", f.panes.join(", ")));
    }
    if f.tree.changes.is_empty() {
        return Verdict::Remove { force: false };
    }
    let unseen: Vec<String> = f.tree.changes.iter().filter(|c| !f.discard.contains(c)).cloned().collect();
    if unseen.is_empty() {
        return Verdict::Remove { force: true };
    }
    Verdict::Ask(unseen)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small random generator, so the test needs no crate.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn bit(&mut self) -> bool {
            self.next() & 1 == 1
        }
    }

    /// Every combination of the facts: a removal happens only for a detached
    /// Switchboard worktree that no open review and no other pane uses, and it drops
    /// changes only when you chose that.
    #[test]
    fn a_removal_never_loses_work_or_a_folder_in_use() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..20_000 {
            let changes: Vec<String> = (0..rng.next() % 3).map(|i| format!("?? f{i}")).collect();
            // You saw some, all or none of the changes, and maybe others that are gone.
            let mut discard: Vec<String> = changes.iter().filter(|_| rng.bit()).cloned().collect();
            if rng.bit() {
                discard.push("?? gone".into());
            }
            let f = Facts {
                tree: TreeState {
                    exists: rng.bit(),
                    worktree: rng.bit(),
                    detached: rng.bit(),
                    locked: rng.next().is_multiple_of(4),
                    changes,
                },
                in_store: rng.bit(),
                open_review: rng.bit(),
                panes: (0..rng.next() % 2).map(|i| format!("p{i}")).collect(),
                discard,
            };
            let seen_all = f.tree.changes.iter().all(|c| f.discard.contains(c));
            let v = verdict(&f);
            seen.insert(std::mem::discriminant(&v));
            if let Verdict::Remove { force } = v {
                assert!(f.tree.exists && f.tree.worktree && f.tree.detached && !f.tree.locked, "{f:?}");
                assert!(f.in_store && !f.open_review && f.panes.is_empty(), "{f:?}");
                assert_eq!(force, !f.tree.changes.is_empty(), "{f:?}");
                assert!(seen_all, "a change you did not see goes: {f:?}");
            }
            if let Verdict::Ask(c) = &v {
                // It lists only the changes you have not seen.
                assert!(!c.is_empty() && c.iter().all(|x| f.tree.changes.contains(x) && !f.discard.contains(x)), "{f:?}");
            }
            // A removal is the only way a folder goes, and a safe folder always goes.
            let safe = f.tree.exists
                && f.tree.worktree
                && f.tree.detached
                && !f.tree.locked
                && f.in_store
                && !f.open_review
                && f.panes.is_empty()
                && seen_all;
            assert_eq!(matches!(v, Verdict::Remove { .. }), safe, "{f:?}");
        }
        assert_eq!(seen.len(), 4, "every verdict happens");
    }
}
