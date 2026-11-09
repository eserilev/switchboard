import GitArgCheck.Spec

/-! # What the checks of a ref, a remote branch and a GitHub URL mean (SPEC 24.2)

Plain definitions on byte lists. `Statements.lean` states the theorems R1, R2, B1,
H1 and H2 with them. -/

open Aeneas Aeneas.Std

namespace GitArgCheck.RefSpec

open GitArgCheck.Spec

/-! ## A ref -/

/-- A byte that a ref may not hold: a control byte (0 to 31, and 127) or a space. -/
def RefusedByte (x : U8) : Prop := x.val < 32 ∨ x = byte ' ' ∨ x.val = 127

/-- A ref that may go to git: not empty, no leading `-`, at most 256 bytes, and no
refused byte. -/
def RefOk (r : List U8) : Prop :=
  r ≠ [] ∧ r.head? ≠ some (byte '-') ∧ r.length ≤ 256 ∧ ∀ x ∈ r, ¬ RefusedByte x

/-! ## A remote branch name -/

/-- The branch name `nm` is the remote `rm`, then `/`, then a branch that is not empty. -/
def Fits (nm rm : List U8) : Prop := ∃ b, b ≠ [] ∧ nm = rm ++ byte '/' :: b

/-! ## A GitHub URL -/

/-- The host `github.com`, with each letter in either case. -/
def Host (h : List U8) : Prop :=
  h.length = 10 ∧ ∀ i < 10, h[i]? = (bytes "github.com")[i]? ∨ h[i]? = (bytes "GITHUB.COM")[i]?

/-- What comes before the owner: `https://github.com/`, `http://github.com/`,
`git://github.com/`, `git@github.com:`, `ssh://git@github.com/` or
`ssh://git@github.com:<port>/`. -/
def Pre (p : List U8) : Prop := ∃ h, Host h ∧
  (p = bytes "https://" ++ h ++ bytes "/" ∨
   p = bytes "http://" ++ h ++ bytes "/" ∨
   p = bytes "git://" ++ h ++ bytes "/" ∨
   p = bytes "git@" ++ h ++ bytes ":" ∨
   p = bytes "ssh://git@" ++ h ++ bytes "/" ∨
   ∃ port, Digits port ∧ p = bytes "ssh://git@" ++ h ++ bytes ":" ++ port ++ bytes "/")

/-- An owner or a name: not empty, and no `/`. -/
def Part (x : List U8) : Prop := x ≠ [] ∧ byte '/' ∉ x

/-- `u` is a GitHub URL of the repo `o/n`: a `Pre`, the owner, `/`, the name, then an
optional `.git` and then an optional `/`. The name does not end with `.git`, so the
parts are clear. -/
def GitHubUrl (u o n : List U8) : Prop :=
  ∃ pre g t, Pre pre ∧ Part o ∧ Part n ∧ ¬ bytes ".git" <:+ n ∧
    (g = [] ∨ g = bytes ".git") ∧ (t = [] ∨ t = bytes "/") ∧
    u = pre ++ o ++ byte '/' :: n ++ g ++ t

/-- The bytes `u[a..b]`. -/
def part (u : List U8) (a b : Usize) : List U8 := (u.drop a.val).take (b.val - a.val)

end GitArgCheck.RefSpec
