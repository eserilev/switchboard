import GuideCheck.Code.Funs

/-! # What the checker means

These definitions say, on plain lists, what `check` must decide.
`Statements.lean` states the theorems with them. -/

open Aeneas Aeneas.Std guide_check

namespace GuideCheck.Spec

/-- The items that a mask keeps: the ones whose mask bit is false. -/
def keep {α : Type} : List α → List Bool → List α
  | x :: xs, b :: bs => if b then keep xs bs else x :: keep xs bs
  | _, _ => []

/-- The lines of a file, as byte lists. -/
def lines (v : alloc.vec.Vec (alloc.vec.Vec U8)) : List (List U8) := v.val.map (·.val)

/-- Some span of the guide holds line `line` of side `old` of file `f`. -/
def Covered (spans : List Span) (f : Nat) (old : Bool) (line : Nat) : Prop :=
  ∃ s ∈ spans, s.file.val = f ∧ s.old = old ∧ s.from.val ≤ line ∧ line ≤ s.to.val

/-- Rebuild and coverage for one file. -/
def FileGood (spans : List Span) (f : Nat) (d : FileDiff) : Prop :=
  d.removed.val.length = d.old.val.length ∧
  d.added.val.length = d.new.val.length ∧
  keep (lines d.old) d.removed.val = keep (lines d.new) d.added.val ∧
  (∀ k, d.removed.val[k]? = some true → Covered spans f true (k + 1)) ∧
  (∀ k, d.added.val[k]? = some true → Covered spans f false (k + 1))

def ndist (a b : Nat) : Nat := if a < b then b - a else a - b

/-- A span against the mask of its side: in bounds, real, and tight. -/
def Fits (mask : List Bool) (lo hi pad : Nat) : Prop :=
  1 ≤ lo ∧ lo ≤ hi ∧ hi ≤ mask.length ∧
  (∃ l, lo ≤ l ∧ l ≤ hi ∧ mask[l - 1]? = some true) ∧
  (∀ l, lo ≤ l → l ≤ hi → ∃ k, mask[k]? = some true ∧ ndist (k + 1) l ≤ pad)

/-- The mask of the side that a span names. -/
def sideMask (s : Span) (d : FileDiff) : List Bool := if s.old then d.removed.val else d.added.val

def SpanGood (files : List FileDiff) (pad : Nat) (s : Span) : Prop :=
  ∃ d, files[s.file.val]? = some d ∧ Fits (sideMask s d) s.from.val s.to.val pad

/-- What `check` must decide. -/
def Accept (files : List FileDiff) (spans : List Span) (pad : Nat) : Prop :=
  (∀ f d, files[f]? = some d → FileGood spans f d) ∧ (∀ s ∈ spans, SpanGood files pad s)

/-! ## The diff rows

`number_rows` walks a file and gives one row per line, with line numbers and no
text. `cut_rows` keeps the rows near a change and puts a header before each run. -/

/-- A row that shows an old line: a removed row or a row on both sides. -/
def showsOld (r : Row) : Bool :=
  match r.kind with
  | .Removed => true
  | .Same => true
  | _ => false

/-- A row that shows a new line: an added row or a row on both sides. -/
def showsNew (r : Row) : Bool :=
  match r.kind with
  | .Added => true
  | .Same => true
  | _ => false

/-- A removed or an added row. -/
def isChange (r : Row) : Bool :=
  match r.kind with
  | .Removed => true
  | .Added => true
  | _ => false

def isHeader (r : Row) : Bool :=
  match r.kind with
  | .Header => true
  | _ => false

/-- The file rebuilds: the masks have the lengths of their line lists, and the old
lines that the diff does not remove are exactly the new lines that it does not add.
These are the first three parts of `FileGood`, so `Accept` gives them for every file. -/
def Rebuilds (d : FileDiff) : Prop :=
  d.removed.val.length = d.old.val.length ∧
  d.added.val.length = d.new.val.length ∧
  keep (lines d.old) d.removed.val = keep (lines d.new) d.added.val

/-- Number `n` names line `n` of `ls` (1-based), and the mask bit of that line is `bit`. -/
def LineIs (ls : List (List U8)) (mask : List Bool) (n : Nat) (bit : Bool) : Prop :=
  1 ≤ n ∧ n ≤ ls.length ∧ mask[n - 1]? = some bit

/-- The numbers of row `r` name the right lines. A removed row names a removed old
line. An added row names an added new line. A row on both sides names an old line
that is not removed and a new line that is not added, and these two lines are equal. -/
def RowNames (oldL newL : List (List U8)) (removed added : List Bool) (r : Row) : Prop :=
  match r.kind with
  | .Removed => LineIs oldL removed r.old.val true ∧ r.new.val = 0
  | .Added => r.old.val = 0 ∧ LineIs newL added r.new.val true
  | .Same => LineIs oldL removed r.old.val false ∧ LineIs newL added r.new.val false ∧
      oldL[r.old.val - 1]? = newL[r.new.val - 1]?
  | .Header => False

/-- Row `r` names the right lines of file `d`. -/
def RowGood (d : FileDiff) (r : Row) : Prop :=
  RowNames (lines d.old) (lines d.new) d.removed.val d.added.val r

/-- The text that a row shows on the old side: old line number `r.old`. -/
def oldText (d : FileDiff) (r : Row) : List U8 := (lines d.old).getD (r.old.val - 1) []

/-- The text that a row shows on the new side: new line number `r.new`. -/
def newText (d : FileDiff) (r : Row) : List U8 := (lines d.new).getD (r.new.val - 1) []

/-- Row index `k` is at most `ctx` rows from a removed or added row. -/
def NearChange (rows : List Row) (ctx k : Nat) : Prop :=
  ∃ c r, rows[c]? = some r ∧ isChange r = true ∧ ndist c k ≤ ctx

/-- The count of rows before row index `k` that show an old line. -/
def oldBefore (rows : List Row) (k : Nat) : Nat := ((rows.take k).filter showsOld).length

/-- The count of rows before row index `k` that show a new line. -/
def newBefore (rows : List Row) (k : Nat) : Nat := ((rows.take k).filter showsNew).length

/-- `out` is a correct cut of the full row list `rows` with context `ctx`.
`out.zip out.tail` is the list of the pairs of rows next to each other in `out`. -/
def CutGood (rows : List Row) (ctx : Nat) (out : List Row) : Prop :=
  -- Without the headers, the cut is the full list with some rows left out:
  -- the same rows, with the same numbers, in the same order.
  (out.filter (fun r => !isHeader r)).Sublist rows ∧
  -- Every removed and every added row is kept.
  (∀ r ∈ rows, isChange r = true → r ∈ out) ∧
  -- Every kept row is at most `ctx` rows from a removed or added row.
  (∀ r ∈ out, isHeader r = false → ∃ k, rows[k]? = some r ∧ NearChange rows ctx k) ∧
  -- The cut starts with a header and does not end with one.
  (∀ r, out.head? = some r → isHeader r = true) ∧
  (∀ r, out.getLast? = some r → isHeader r = false) ∧
  -- Two rows next to each other in the cut, with no header between them, are next to
  -- each other in the full list. So a header marks every gap.
  (∀ a b, (a, b) ∈ out.zip out.tail → isHeader a = false → isHeader b = false →
    ∃ k, rows[k]? = some a ∧ rows[k + 1]? = some b) ∧
  -- After each header comes a kept row, row `k` of the full list. The header carries the
  -- number of the next old line and of the next new line at that row: one more than the
  -- count of old (new) lines in the rows before it.
  (∀ h q, (h, q) ∈ out.zip out.tail → isHeader h = true →
    isHeader q = false ∧ ∃ k, rows[k]? = some q ∧
      h.old.val = oldBefore rows k + 1 ∧ h.new.val = newBefore rows k + 1)

end GuideCheck.Spec
