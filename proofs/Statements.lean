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

G5 to G9 are about the diff rows that the review window draws. `number_rows` gives
every row of a file with its line numbers and no text; the window gets the text
with the number. `cut_rows` keeps the rows near a change. They use:
- `Rebuilds`: the first three parts of `FileGood`. `Accept` gives it for every file.
- `RowGood`: the numbers of a row name real lines of the right kind (see `RowNames`).
- `showsOld`, `showsNew`: the row shows an old (new) line: removed (added) or on both sides.
- `CutGood`: what a correct cut is.
The bound on the line counts is always true in practice: a `Vec<u8>` takes 24 bytes,
so a line list has fewer than `Usize.max / 24` lines.
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

/-- **G5.** On a file that rebuilds, every row names the right lines: an old number `n`
is a line `1 ≤ n ≤` the old line count, removed for a removed row and not removed for a
row on both sides; the same for new numbers. A row on both sides pairs two equal lines. -/
def G5_rows_name_their_lines : Prop :=
  ∀ (d : FileDiff), Rebuilds d → d.removed.val.length + d.added.val.length ≤ Usize.max →
    number_rows (alloc.vec.Vec.deref d.removed) (alloc.vec.Vec.deref d.added) ⦃ rows =>
      ∀ r ∈ rows.val, RowGood d r ⦄

/-- **G6.** On a file that rebuilds, the text of the removed and both-sides rows, in order,
is exactly the old file, and the text of the added and both-sides rows is exactly the new file. -/
def G6_rows_rebuild_both_files : Prop :=
  ∀ (d : FileDiff), Rebuilds d → d.removed.val.length + d.added.val.length ≤ Usize.max →
    number_rows (alloc.vec.Vec.deref d.removed) (alloc.vec.Vec.deref d.added) ⦃ rows =>
      (rows.val.filter showsOld).map (oldText d) = lines d.old ∧
      (rows.val.filter showsNew).map (newText d) = lines d.new ⦄

/-- **G7.** On a file that rebuilds, each line comes once: on each side the numbers of the
rows go 1, 2, …, line count, with no gap and no repeat. -/
def G7_each_line_once : Prop :=
  ∀ (d : FileDiff), Rebuilds d → d.removed.val.length + d.added.val.length ≤ Usize.max →
    number_rows (alloc.vec.Vec.deref d.removed) (alloc.vec.Vec.deref d.added) ⦃ rows =>
      (rows.val.filter showsOld).map (·.old.val) = List.range' 1 d.old.val.length ∧
      (rows.val.filter showsNew).map (·.new.val) = List.range' 1 d.new.val.length ⦄

/-- **G8.** The cut of the rows of a file that rebuilds is correct (`CutGood`): it keeps every
removed and added row, keeps only rows near one, changes no row, keeps the order, and marks
every gap with a header. A header carries the numbers of the next kept row; on a side where
that row has no line, it carries the number of the next line of that side. -/
def G8_the_cut : Prop :=
  ∀ (d : FileDiff) (ctx : Usize), Rebuilds d →
    2 * (d.removed.val.length + d.added.val.length) ≤ Usize.max →
    number_rows (alloc.vec.Vec.deref d.removed) (alloc.vec.Vec.deref d.added) ⦃ rows =>
      cut_rows (alloc.vec.Vec.deref rows) ctx ⦃ out =>
        CutGood rows.val ctx.val out.val ∧
        (∀ h q, (h, q) ∈ out.val.zip out.val.tail → isHeader h = true →
          (showsOld q = true → h.old = q.old) ∧ (showsNew q = true → h.new = q.new)) ⦄ ⦄

/-- **G9.** The row functions never panic, on any input within the size bound. Even on a
file that does not rebuild, the rows are at most one per line. The cut is correct on any
rows with no header. -/
def G9_rows_never_panic : Prop :=
  (∀ (removed added : Slice Bool), removed.val.length + added.val.length ≤ Usize.max →
    number_rows removed added ⦃ rows => rows.val.length ≤ removed.val.length + added.val.length ⦄) ∧
  (∀ (rows : Slice Row) (ctx : Usize), 2 * rows.val.length ≤ Usize.max →
    (∀ r ∈ rows.val, isHeader r = false) →
    cut_rows rows ctx ⦃ out => CutGood rows.val ctx.val out.val ⦄)

theorem check_G1 : G1_check_decides_accept := GuideCheck.Check.check_spec

theorem check_G2 : G2_every_change_is_covered :=
  fun files spans pad f d h hd => GuideCheck.Meaning.every_change_is_covered files spans pad h f d hd

theorem check_G3 : G3_unmarked_lines_are_unchanged :=
  fun _ _ _ f d h hd => (h.1 f d hd).2.2.1

theorem check_G4 : G4_unmarked_file_is_unchanged :=
  fun files spans pad f d h hd => GuideCheck.Meaning.unmarked_file_is_unchanged files spans pad h f d hd

theorem check_G5 : G5_rows_name_their_lines := GuideCheck.Rows.G5_proof

theorem check_G6 : G6_rows_rebuild_both_files := GuideCheck.Rows.G6_proof

theorem check_G7 : G7_each_line_once := GuideCheck.Rows.G7_proof

theorem check_G8 : G8_the_cut := GuideCheck.Cut.G8_proof

theorem check_G9 : G9_rows_never_panic :=
  ⟨GuideCheck.Rows.number_rows_safe, GuideCheck.Cut.cut_rows_spec⟩

end Statements
