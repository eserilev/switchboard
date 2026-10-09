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

end GuideCheck.Spec
