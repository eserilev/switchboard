import DiffParse.Spec

/-! # The line counters of a hunk: no overflow, and a well-formed hunk ends at the ends -/

open Aeneas Aeneas.Std Result diffparse DiffParse.Spec

namespace DiffParse.Count

/-- What one `step` does: a row that shows an old (new) line needs a line left on that
side, and moves that counter by one. The ends do not change. -/
def StepOk (c : Counter) (o n : Bool) (r : Option Counter) : Prop :=
  match r with
  | some c' =>
    c'.old.val = c.old.val + (if o then 1 else 0) ∧ c'.new.val = c.new.val + (if n then 1 else 0) ∧
    c'.old_end = c.old_end ∧ c'.new_end = c.new_end ∧
    (o = true → c.old.val < c.old_end.val) ∧ (n = true → c.new.val < c.new_end.val)
  | none => (o = true ∧ c.old_end.val ≤ c.old.val) ∨ (n = true ∧ c.new_end.val ≤ c.new.val)

@[step]
theorem step_spec (c : Counter) (o n : Bool) : step c o n ⦃ r => StepOk c o n r ⦄ := by
  unfold step
  step*
  all_goals (simp only [StepOk]; simp_all; try scalar_tac)

/-- The rows of a well-formed hunk: as many rows show an old (new) line as the hunk
has lines left on that side. Then the counters end at the ends. -/
theorem run_spec (rows : List (Bool × Bool)) : ∀ (c : Counter),
    c.old.val ≤ c.old_end.val → c.new.val ≤ c.new_end.val →
    (rows.filter (·.1)).length = c.old_end.val - c.old.val →
    (rows.filter (·.2)).length = c.new_end.val - c.new.val →
    run c rows ⦃ r => ∃ c', r = some c' ∧ c'.old = c.old_end ∧ c'.new = c.new_end ∧
      c'.old_end = c.old_end ∧ c'.new_end = c.new_end ⦄ := by
  induction rows with
  | nil =>
    intro c h1 h2 h3 h4
    simp only [run, WP.spec_ok]
    refine ⟨c, rfl, ?_, ?_, rfl, rfl⟩
    · apply UScalar.eq_of_val_eq; simp at h3; omega
    · apply UScalar.eq_of_val_eq; simp at h4; omega
  | cons row rows ih =>
    intro c h1 h2 h3 h4
    obtain ⟨o, n⟩ := row
    simp only [run]
    apply WP.spec_bind (step_spec c o n)
    intro r hr
    cases r with
    | none =>
      exfalso
      simp only [StepOk] at hr
      cases o <;> cases n <;> (simp_all; try omega)
    | some c' =>
      obtain ⟨e1, e2, e3, e4, -, -⟩ := hr
      simp only
      have := ih c' (by cases o <;> cases n <;> simp_all <;> omega)
        (by cases o <;> cases n <;> simp_all <;> omega)
        (by rw [e3, e1]; cases o <;> cases n <;> simp_all <;> omega)
        (by rw [e4, e2]; cases o <;> cases n <;> simp_all <;> omega)
      apply WP.spec_mono this
      rintro r ⟨c'', hr, h5, h6, h7, h8⟩
      exact ⟨c'', hr, by rw [h5, e3], by rw [h6, e4], by rw [h7, e3], by rw [h8, e4]⟩

end DiffParse.Count
