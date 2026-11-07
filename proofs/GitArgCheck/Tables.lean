import GitArgCheck.Small

/-! # The tables: the Rust lists hold the names of the spec lists -/

open Aeneas Aeneas.Std Result git_argcheck GitArgCheck.Spec

namespace GitArgCheck.Tables

open GitArgCheck.Small

@[step]
theorem in_list_spec (nm list : Slice U8) :
    in_list nm list ⦃ r => (r = true ↔ nm.val ∈ list.val.splitOn 32#u8) ⦄ := by
  unfold in_list
  step*

set_option maxRecDepth 100000

theorem subs_eq : SUBS.val.splitOn 32#u8 = subs.map bytes := by
  simp only [SUBS, global_simps]; decide
theorem diff_flags_eq : DIFF_FLAGS.val.splitOn 32#u8 = diffFlags.map bytes := by
  simp only [DIFF_FLAGS, global_simps]; decide
theorem diff_values_eq : DIFF_VALUES.val.splitOn 32#u8 = diffValues.map bytes := by
  simp only [DIFF_VALUES, global_simps]; decide
theorem diff_only_eq : DIFF_ONLY.val.splitOn 32#u8 = diffOnly.map bytes := by
  simp only [DIFF_ONLY, global_simps]; decide
theorem log_flags_eq : LOG_FLAGS.val.splitOn 32#u8 = logFlags.map bytes := by
  simp only [LOG_FLAGS, global_simps]; decide
theorem log_values_eq : LOG_VALUES.val.splitOn 32#u8 = logValues.map bytes := by
  simp only [LOG_VALUES, global_simps]; decide
theorem blame_flags_eq : BLAME_FLAGS.val.splitOn 32#u8 = blameFlags.map bytes := by
  simp only [BLAME_FLAGS, global_simps]; decide
theorem blame_values_eq : BLAME_VALUES.val.splitOn 32#u8 = blameValues.map bytes := by
  simp only [BLAME_VALUES, global_simps]; decide
theorem merge_base_flags_eq : MERGE_BASE_FLAGS.val.splitOn 32#u8 = mergeBaseFlags.map bytes := by
  simp only [MERGE_BASE_FLAGS, global_simps]; decide
theorem rev_parse_flags_eq : REV_PARSE_FLAGS.val.splitOn 32#u8 = revParseFlags.map bytes := by
  simp only [REV_PARSE_FLAGS, global_simps]; decide
theorem rev_parse_values_eq : REV_PARSE_VALUES.val.splitOn 32#u8 = revParseValues.map bytes := by
  simp only [REV_PARSE_VALUES, global_simps]; decide

/-- `c` is the code of the subcommand `sub` in the Rust (`NO_SUB` = 6 when it is none). -/
def Code (sub : List U8) (c : U8) : Prop :=
  (sub = bytes "diff" ∧ c = 0#u8) ∨ (sub = bytes "log" ∧ c = 1#u8) ∨ (sub = bytes "show" ∧ c = 2#u8) ∨
  (sub = bytes "blame" ∧ c = 3#u8) ∨ (sub = bytes "merge-base" ∧ c = 4#u8) ∨
  (sub = bytes "rev-parse" ∧ c = 5#u8) ∨ (sub ∉ subs.map bytes ∧ c = 6#u8)

@[simp] theorem make_val (n : Usize) (l : List U8) (h : l.length = n.val) : (Array.make n l h).val = l := rfl

theorem b_diff : bytes "diff" = [100#u8, 105#u8, 102#u8, 102#u8] := by decide
theorem b_log : bytes "log" = [108#u8, 111#u8, 103#u8] := by decide
theorem b_show : bytes "show" = [115#u8, 104#u8, 111#u8, 119#u8] := by decide
theorem b_blame : bytes "blame" = [98#u8, 108#u8, 97#u8, 109#u8, 101#u8] := by decide
theorem b_merge_base : bytes "merge-base" = [109#u8, 101#u8, 114#u8, 103#u8, 101#u8, 45#u8, 98#u8, 97#u8, 115#u8, 101#u8] := by decide
theorem b_rev_parse : bytes "rev-parse" = [114#u8, 101#u8, 118#u8, 45#u8, 112#u8, 97#u8, 114#u8, 115#u8, 101#u8] := by decide

theorem subs_bytes : subs.map bytes = [bytes "diff", bytes "log", bytes "show", bytes "blame",
    bytes "merge-base", bytes "rev-parse"] := rfl

@[step]
theorem sub_code_spec (sub : Slice U8) : sub_code sub ⦃ c => Code sub.val c ⦄ := by
  unfold sub_code
  step*
  all_goals simp_all [Code, global_simps, subs_bytes, b_diff, b_log, b_show, b_blame, b_merge_base, b_rev_parse]

theorem not_sub {sub : List U8} (h : sub ∉ subs.map bytes) :
    sub ≠ bytes "diff" ∧ sub ≠ bytes "log" ∧ sub ≠ bytes "show" ∧ sub ≠ bytes "blame" ∧
    sub ≠ bytes "merge-base" ∧ sub ≠ bytes "rev-parse" := by
  simp only [subs_bytes, List.mem_cons, List.not_mem_nil, or_false, not_or] at h
  exact h

theorem DIFF_eq : DIFF = 0#u8 := by simp [global_simps]
theorem LOG_eq : LOG = 1#u8 := by simp [global_simps]
theorem SHOW_eq : SHOW = 2#u8 := by simp [global_simps]
theorem BLAME_eq : BLAME = 3#u8 := by simp [global_simps]
theorem MERGE_BASE_eq : MERGE_BASE = 4#u8 := by simp [global_simps]
theorem REV_PARSE_eq : REV_PARSE = 5#u8 := by simp [global_simps]
theorem NO_SUB_eq : NO_SUB = 6#u8 := by simp [global_simps]

/-- Split on the subcommand. -/
macro "sub_cases" h:ident : tactic => `(tactic| (
  rcases $h:ident with ⟨hs, hc⟩ | ⟨hs, hc⟩ | ⟨hs, hc⟩ | ⟨hs, hc⟩ | ⟨hs, hc⟩ | ⟨hs, hc⟩ | ⟨hs, hc⟩ <;>
  (try have hs := not_sub hs) <;>
  simp only [hc, DIFF_eq, LOG_eq, SHOW_eq, BLAME_eq, MERGE_BASE_eq, REV_PARSE_eq, NO_SUB_eq] at * <;>
  simp_all [Diffs, Logs, flags, values, b_diff, b_log, b_show, b_blame, b_merge_base, b_rev_parse]))

@[step]
theorem diffs_spec (sub : List U8) (c : U8) (h : Code sub c) : diffs c ⦃ r => (r = true ↔ Diffs sub) ⦄ := by
  unfold diffs
  sub_cases h

@[step]
theorem logs_spec (sub : List U8) (c : U8) (h : Code sub c) : logs c ⦃ r => (r = true ↔ Logs sub) ⦄ := by
  unfold logs
  sub_cases h

@[step]
theorem flag_ok_spec (sub : List U8) (c : U8) (h : Code sub c) (nm : Slice U8) :
    flag_ok c nm ⦃ r => (r = true ↔ nm.val ∈ (flags sub).map bytes) ⦄ := by
  unfold flag_ok
  step*
  all_goals try simp only [diff_flags_eq, diff_only_eq, log_flags_eq, blame_flags_eq, merge_base_flags_eq,
    rev_parse_flags_eq] at *
  all_goals sub_cases h

@[step]
theorem value_ok_spec (sub : List U8) (c : U8) (h : Code sub c) (nm : Slice U8) :
    value_ok c nm ⦃ r => (r = true ↔ nm.val ∈ (values sub).map bytes) ⦄ := by
  unfold value_ok
  step*
  all_goals try simp only [diff_values_eq, log_values_eq, blame_values_eq, rev_parse_values_eq] at *
  all_goals sub_cases h

end GitArgCheck.Tables
