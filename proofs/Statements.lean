import GuideCheck

/-!
# The theorems, stated

Each `def` below is a statement that a person approved. The proofs live in other
files. A `check_` theorem at the end checks each proof against its statement, so a
proof cannot quietly prove something weaker: if a theorem changes, this file fails.

`f x ⦃ r => P r ⦄` means: `f x` returns without a panic, and its result `r` has
property `P`. So the first statement also says "`check` never panics".

The definitions it uses are in `GuideCheck/Spec.lean`:
- `Accept`: every file rebuilds and is covered, and every span is real and tight.
- `FileGood`: the masks have the right lengths; the old lines that the diff does
  not remove are exactly the new lines that it does not add; every removed and
  added line is in a span of the same file and side.
- `SpanGood`: the span names a file, has `1 ≤ from ≤ to ≤` the line count, holds a
  changed line, and every line of it is at most `pad` lines from a changed line.
-/

open Aeneas Aeneas.Std guide_check GuideCheck.Spec

namespace Statements

/-- **G1.** `check` never panics, and it returns `true` exactly when the guide is accepted. -/
def G1_check_decides_accept : Prop :=
  ∀ (files : Slice FileDiff) (spans : Slice Span) (pad : Usize),
    check files spans pad ⦃ r => (r = true ↔ Accept files.val spans.val pad.val) ⦄

/-- **G2.** In an accepted guide, every added and every removed line is in a span. -/
def G2_every_change_is_covered : Prop :=
  ∀ (files : List FileDiff) (spans : List Span) (pad f : Nat) (d : FileDiff),
    Accept files spans pad → files[f]? = some d →
    (∀ k, d.removed.val[k]? = some true → Covered spans f true (k + 1)) ∧
    (∀ k, d.added.val[k]? = some true → Covered spans f false (k + 1))

/-- **G3.** In an accepted guide, the diff hides nothing: the lines it does not mark are
the same in the old and the new file, in the same order. -/
def G3_unmarked_lines_are_unchanged : Prop :=
  ∀ (files : List FileDiff) (spans : List Span) (pad f : Nat) (d : FileDiff),
    Accept files spans pad → files[f]? = some d →
    keep (lines d.old) d.removed.val = keep (lines d.new) d.added.val

/-- **G4.** In an accepted guide, a file where the diff marks no line did not change. -/
def G4_unmarked_file_is_unchanged : Prop :=
  ∀ (files : List FileDiff) (spans : List Span) (pad f : Nat) (d : FileDiff),
    Accept files spans pad → files[f]? = some d →
    (∀ k : Nat, d.removed.val[k]? ≠ some true) → (∀ k : Nat, d.added.val[k]? ≠ some true) →
    lines d.old = lines d.new

theorem check_G1 : G1_check_decides_accept := GuideCheck.Check.check_spec

theorem check_G2 : G2_every_change_is_covered :=
  fun files spans pad f d h hd => GuideCheck.Meaning.every_change_is_covered files spans pad h f d hd

theorem check_G3 : G3_unmarked_lines_are_unchanged :=
  fun _ _ _ f d h hd => (h.1 f d hd).2.2.1

theorem check_G4 : G4_unmarked_file_is_unchanged :=
  fun files spans pad f d h hd => GuideCheck.Meaning.unmarked_file_is_unchanged files spans pad h f d hd

end Statements
