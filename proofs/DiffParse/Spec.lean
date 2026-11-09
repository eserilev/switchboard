import DiffParse.Code.Funs

/-! # What the diff parse kernels mean

These definitions say, on plain byte lists, how git quotes a path and how a hunk
header reads. `Statements.lean` states the theorems with them. -/

open Aeneas Aeneas.Std Result

namespace DiffParse.Spec

/-- The byte with value `n`, for `n < 256`. -/
def u8 (n : Nat) : U8 := ⟨BitVec.ofNat 8 n⟩

/-! ## git's quoting of a path

git puts a path with special bytes in quotes (`core.quotePath`). Inside them it
writes `\"` and `\\`, the C escapes `\a \b \t \n \v \f \r`, a three-digit octal
escape `\ooo` for any other control byte and for every byte from 0x7f up, and every
other byte as itself. `quote` is that text without the outer quotes. -/

/-- git's quoting of one byte. -/
def quoteByte (x : U8) : List U8 :=
  if x = 34#u8 then [92#u8, x]
  else if x = 92#u8 then [92#u8, x]
  else if x = 7#u8 then [92#u8, 97#u8]
  else if x = 8#u8 then [92#u8, 98#u8]
  else if x = 9#u8 then [92#u8, 116#u8]
  else if x = 10#u8 then [92#u8, 110#u8]
  else if x = 11#u8 then [92#u8, 118#u8]
  else if x = 12#u8 then [92#u8, 102#u8]
  else if x = 13#u8 then [92#u8, 114#u8]
  else if x.val < 32 ∨ 127 ≤ x.val then
    [92#u8, u8 (48 + x.val / 64), u8 (48 + x.val / 8 % 8), u8 (48 + x.val % 8)]
  else [x]

/-- git's quoting of a path, without the outer quotes. -/
def quote (s : List U8) : List U8 := s.flatMap quoteByte

/-! ## Hunk headers

A hunk header is `@@ -a,b +c,d @@`. The parser gets the text after `@@ `. `,b` and
`,d` may be missing; the count is then 1. The old side has the lines `a` to
`a + b - 1`, and the new side `c` to `c + d - 1`. -/

/-- A decimal digit. -/
def IsDigit (x : U8) : Prop := 48 ≤ x.val ∧ x.val ≤ 57

/-- A number in the text: one digit or more. -/
def Digits (ds : List U8) : Prop := ds ≠ [] ∧ ∀ x ∈ ds, IsDigit x

/-- The value of decimal digits. -/
def dval (ds : List U8) : Nat := ds.foldl (fun n x => 10 * n + (x.val - 48)) 0

/-- `start` or `start,count`. -/
def rangeText (start : List U8) (count : Option (List U8)) : List U8 :=
  match count with
  | none => start
  | some cs => start ++ 44#u8 :: cs

/-- The count of a range: 1 when it is not there. -/
def count (cs : Option (List U8)) : Nat :=
  match cs with
  | none => 1
  | some cs => dval cs

/-- `-a[,b] +c[,d]`. -/
def headerText (a : List U8) (b : Option (List U8)) (c : List U8) (d : Option (List U8)) : List U8 :=
  45#u8 :: (rangeText a b ++ 32#u8 :: 43#u8 :: rangeText c d)

/-- The counters after the rows of a hunk, one `step` per row. A row is a pair:
does it show an old line, does it show a new line. -/
def run (c : diffparse.Counter) : List (Bool × Bool) → Result (Option diffparse.Counter)
  | [] => ok (some c)
  | (o, n) :: rows => do
    let r ← diffparse.step c o n
    match r with
    | none => ok none
    | some c' => run c' rows

end DiffParse.Spec
