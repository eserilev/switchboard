import GitArgCheck.Code.Funs

/-! # What the git arg check means

These definitions say, on plain byte lists, which git calls the guide agent may
run. They are written from the option lists of the old check in
`crates/core/src/review_git.rs`, not from the Rust tables: a lemma in
`GitArgCheck/Tables.lean` shows that the Rust tables hold the same names.
`Statements.lean` states the theorems with them. -/

open Aeneas Aeneas.Std

namespace GitArgCheck.Spec

/-- The byte of an ASCII char. -/
def byte (c : Char) : U8 := ⟨BitVec.ofNat 8 c.toNat⟩

/-- The bytes of an ASCII string. -/
def bytes (s : String) : List U8 := s.toList.map byte

/-- The subcommands the tool runs. -/
def subs : List String := ["diff", "log", "show", "blame", "merge-base", "rev-parse"]

/-- Diff options of `diff`, `log` and `show` with no value. -/
def diffFlags : List String := [
  "-p", "--patch", "-s", "--no-patch", "--stat", "--numstat", "--shortstat",
  "--name-only", "--name-status", "--summary", "--raw", "--patch-with-stat",
  "--patch-with-raw", "--compact-summary", "--dirstat", "--cumulative", "-M",
  "--find-renames", "-C", "--find-copies", "--find-copies-harder", "-D",
  "--irreversible-delete", "--no-renames", "-w", "--ignore-all-space", "-b",
  "--ignore-space-change", "--ignore-space-at-eol", "--ignore-blank-lines",
  "--ignore-cr-at-eol", "--word-diff", "--color-words", "--no-color", "--relative",
  "--no-relative", "-W", "--function-context", "--histogram", "--patience",
  "--minimal", "--abbrev", "--no-abbrev", "--full-index", "-R", "--no-prefix",
  "-a", "--text"]

/-- Diff options of `diff`, `log` and `show` in the `--name=value` form. -/
def diffValues : List String := [
  "--unified", "--stat", "--dirstat", "--find-renames", "--find-copies",
  "--word-diff", "--word-diff-regex", "--color-words", "--relative",
  "--diff-algorithm", "--diff-filter", "--abbrev", "--src-prefix", "--dst-prefix",
  "--inter-hunk-context", "--anchored"]

/-- Options of `git diff` alone. -/
def diffOnly : List String := ["--cached", "--staged", "--merge-base", "--exit-code", "--quiet", "--check"]

/-- Log options of `log` and `show` with no value. -/
def logFlags : List String := [
  "--oneline", "--graph", "--decorate", "--no-decorate", "--first-parent",
  "--no-merges", "--merges", "--reverse", "--follow", "--all", "--parents",
  "--children", "--topo-order", "--date-order", "--author-date-order",
  "--ancestry-path", "--left-right", "--cherry-pick", "--cherry-mark", "--cherry",
  "--boundary", "--full-history", "--simplify-merges", "--simplify-by-decoration",
  "--dense", "--sparse", "--pickaxe-all", "--pickaxe-regex", "-i",
  "--regexp-ignore-case", "-E", "--extended-regexp", "-F", "--fixed-strings",
  "--all-match", "--invert-grep", "-m", "--cc", "--no-walk", "--source",
  "--abbrev-commit", "--no-abbrev-commit", "--pretty", "-L", "-n", "--quiet"]

/-- Log options of `log` and `show` in the `--name=value` form. -/
def logValues : List String := [
  "--max-count", "--skip", "--since", "--after", "--until", "--before", "--author",
  "--committer", "--grep", "--format", "--pretty", "--date", "--decorate",
  "--diff-merges", "--max-parents", "--min-parents", "--no-walk"]

def blameFlags : List String := [
  "-w", "-M", "-C", "-s", "-e", "--show-email", "-l", "-t", "-f", "--show-name",
  "-n", "--show-number", "-p", "--porcelain", "--line-porcelain", "--incremental",
  "--root", "--show-stats", "-b", "--first-parent", "--reverse", "--abbrev", "-L"]

def blameValues : List String := ["--date", "--abbrev"]

def mergeBaseFlags : List String := ["--is-ancestor", "--fork-point", "--all", "-a", "--octopus", "--independent"]

def revParseFlags : List String := [
  "--verify", "--short", "-q", "--quiet", "--abbrev-ref", "--symbolic",
  "--symbolic-full-name", "--show-toplevel", "--show-prefix", "--is-inside-work-tree"]

def revParseValues : List String := ["--short", "--abbrev-ref"]

/-- `diff`, `log` and `show`: the subcommands that make a diff. -/
abbrev Diffs (sub : List U8) : Prop := sub = bytes "diff" ∨ sub = bytes "log" ∨ sub = bytes "show"

/-- `log` and `show`. -/
abbrev Logs (sub : List U8) : Prop := sub = bytes "log" ∨ sub = bytes "show"

/-- The options of a subcommand with no value. -/
def flags (sub : List U8) : List String :=
  if sub = bytes "diff" then diffFlags ++ diffOnly
  else if sub = bytes "log" ∨ sub = bytes "show" then diffFlags ++ logFlags
  else if sub = bytes "blame" then blameFlags
  else if sub = bytes "merge-base" then mergeBaseFlags
  else if sub = bytes "rev-parse" then revParseFlags
  else []

/-- The options of a subcommand in the `--name=value` form. -/
def values (sub : List U8) : List String :=
  if sub = bytes "diff" then diffValues
  else if sub = bytes "log" ∨ sub = bytes "show" then diffValues ++ logValues
  else if sub = bytes "blame" then blameValues
  else if sub = bytes "rev-parse" then revParseValues
  else []

/-- The options that go before the args: they turn off external diff and textconv programs. -/
def pre (sub : List U8) : List (List U8) :=
  if Diffs sub then [bytes "--no-ext-diff", bytes "--no-textconv"]
  else if sub = bytes "blame" then [bytes "--no-textconv"]
  else []

/-- One digit or more, and nothing else. -/
def Digits (v : List U8) : Prop := v ≠ [] ∧ ∀ x ∈ v, x ∈ bytes "0123456789"

/-- A rename or copy score: digits, then any number of `%`. -/
def Score (v : List U8) : Prop := ∃ d n, v = d ++ List.replicate n (byte '%') ∧ Digits d

/-- A short option with its value glued on, such as `-U0`, `-M50%` or `-Sneedle`. -/
def Glued (sub a : List U8) : Prop :=
  ∃ f v, a = byte '-' :: f :: v ∧ v ≠ [] ∧
    ((f = byte 'U' ∧ Diffs sub ∧ Digits v) ∨
     ((f = byte 'M' ∨ f = byte 'C') ∧ (Diffs sub ∨ sub = bytes "blame") ∧ Score v) ∨
     (f = byte 'l' ∧ Diffs sub ∧ Digits v) ∨
     ((f = byte 'S' ∨ f = byte 'G') ∧ Diffs sub) ∨
     (f = byte 'L' ∧ (Logs sub ∨ sub = bytes "blame")) ∨
     (f = byte 'n' ∧ Logs sub ∧ Digits v))

/-- An arg that may go to `git sub` as far as options go: `--`, an option from the
lists, `--name=value` for a name from the lists, `-<count>` for `log`, a glued short
option, or an arg that is not an option at all (a rev, a range or a path). -/
def Allowed (sub a : List U8) : Prop :=
  a = bytes "--" ∨
  a ∈ (flags sub).map bytes ∨
  (∃ n v, a = n ++ byte '=' :: v ∧ n ∈ (values sub).map bytes) ∨
  (sub = bytes "log" ∧ ∃ v, a = byte '-' :: v ∧ Digits v) ∨
  Glued sub a ∨
  a.head? ≠ some (byte '-')

/-- A path that stays in the folder it is read from: no leading `/` or `~`, and no
piece between two `/` is `..`. -/
def PathSafe (a : List U8) : Prop :=
  a.head? ≠ some (byte '/') ∧ a.head? ≠ some (byte '~') ∧ bytes ".." ∉ a.splitOn (byte '/')

/-- An arg that may go to `git sub`. -/
def Good (sub a : List U8) : Prop :=
  a ≠ [] ∧ byte '\x00' ∉ a ∧ byte '\n' ∉ a ∧ Allowed sub a ∧
  (a.head? ≠ some (byte '-') → PathSafe a)

/-- A git call that the tool may run: a subcommand from the list, and every arg good. -/
def CallOk (sub : List U8) (args : List (List U8)) : Prop :=
  sub ∈ subs.map bytes ∧ ∀ a ∈ args, Good sub a

/-- Option names that read or write files, or run programs. -/
def denied : List String := [
  "--output", "--ext-diff", "--textconv", "--no-index", "--exec-path", "--git-dir",
  "--work-tree", "--contents", "--ignore-revs-file", "--git-path", "-o", "-O", "-c"]

/-! ## Where a path leads

The lexical normal form of a path: split at `/`, drop empty and `.` pieces, and let
`..` drop the piece before it. -/

def normStep (st : List (List U8)) (piece : List U8) : List (List U8) :=
  if piece = [] ∨ piece = bytes "." then st
  else if piece = bytes ".." then st.dropLast
  else st ++ [piece]

def norm (p : List U8) : List (List U8) := (p.splitOn (byte '/')).foldl normStep []

end GitArgCheck.Spec
