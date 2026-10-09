import GuideCheck.Rebuild

/-! # Coverage: every changed line is in a span -/

open Aeneas Aeneas.Std Result guide_check GuideCheck.Spec GuideCheck.Small GuideCheck.Rebuild

namespace GuideCheck.Cover

theorem covered_loop_spec (spans : Slice Span) (file : Usize) (old : Bool) (line : Usize) (i : Usize)
    (hi : i.val ≤ spans.val.length)
    (hnone : ∀ t (ht : t < i.val), ¬ (let s := spans.val[t]'(by omega);
      s.file.val = file.val ∧ s.old = old ∧ s.from.val ≤ line.val ∧ line.val ≤ s.to.val)) :
    covered_loop spans file old line i ⦃ r => (r = true ↔ Covered spans.val file.val old line.val) ⦄ := by
  unfold covered_loop
  apply loop.spec_decr_nat (fun i => spans.val.length - i.val)
    (fun i => ∃ _ : i.val ≤ spans.val.length, ∀ t (ht : t < i.val), ¬ (let s := spans.val[t]'(by omega);
      s.file.val = file.val ∧ s.old = old ∧ s.from.val ≤ line.val ∧ line.val ≤ s.to.val))
    _ _ _ _ ⟨hi, hnone⟩
  rintro i ⟨hi, hnone⟩
  unfold covered_loop.body
  step*
  · -- This span holds the line.
    simp only [true_iff]
    exact ⟨s, by rw [s_post]; exact List.getElem_mem _, b_post.mp (by assumption)⟩
  · -- Not this span: the invariant holds for the next one.
    refine ⟨⟨by scalar_tac, fun t ht => ?_⟩, by scalar_tac⟩
    by_cases hti : t < i.val
    · exact hnone t hti
    · have : t = i.val := by scalar_tac
      subst this
      intro h
      have hb : ¬ b = true := by assumption
      exact hb (b_post.mpr (by rw [s_post]; exact h))
  · -- No span holds it.
    simp only [Bool.false_eq_true, false_iff]
    rintro ⟨s, hs, h⟩
    obtain ⟨t, ht, rfl⟩ := List.getElem_of_mem hs
    exact hnone t (by scalar_tac) h

@[step]
theorem covered_spec (spans : Slice Span) (file : Usize) (old : Bool) (line : Usize) :
    covered spans file old line ⦃ r => (r = true ↔ Covered spans.val file.val old line.val) ⦄ := by
  unfold covered
  exact covered_loop_spec spans file old line 0#usize (by simp) (fun t ht => absurd ht (by simp))

theorem mask_covered_loop_spec (spans : Slice Span) (file : Usize) (old : Bool) (mask : Slice Bool)
    (k : Usize) (hk : k.val ≤ mask.val.length)
    (hdone : ∀ t, t < k.val → mask.val[t]? = some true → Covered spans.val file.val old (t + 1)) :
    mask_covered_loop spans file old mask k ⦃ r =>
      (r = true ↔ ∀ t, mask.val[t]? = some true → Covered spans.val file.val old (t + 1)) ⦄ := by
  unfold mask_covered_loop
  apply loop.spec_decr_nat (fun k => mask.val.length - k.val)
    (fun k => k.val ≤ mask.val.length ∧
      ∀ t, t < k.val → mask.val[t]? = some true → Covered spans.val file.val old (t + 1))
    _ _ _ _ ⟨hk, hdone⟩
  rintro k ⟨hk, hdone⟩
  unfold mask_covered_loop.body
  step*
  · -- A changed line that a span holds.
    refine ⟨by scalar_tac, fun t ht hm => ?_, by scalar_tac⟩
    by_cases htk : t < k.val
    · exact hdone t htk hm
    · have : t = k.val := by scalar_tac
      subst this
      rw [← i1_post]
      exact b1_post.mp (by assumption)
  · -- A changed line with no span.
    simp only [Bool.false_eq_true, false_iff, not_forall]
    have hlt : k.val < mask.val.length := by scalar_tac
    refine ⟨k.val, ?_, fun h => ?_⟩
    · rw [List.getElem?_eq_getElem hlt, ← b_post]; simp_all
    · have hb1 : ¬ b1 = true := by assumption
      exact hb1 (b1_post.mpr (by rw [i1_post]; exact h))
  · -- A line with no change.
    refine ⟨by scalar_tac, fun t ht hm => ?_, by scalar_tac⟩
    by_cases htk : t < k.val
    · exact hdone t htk hm
    · have : t = k.val := by scalar_tac
      subst this
      have hlt : k.val < mask.val.length := by scalar_tac
      rw [List.getElem?_eq_getElem hlt, ← b_post] at hm
      simp_all
  · -- Every line checked.
    simp only [true_iff]
    intro t hm
    have : t < mask.val.length := by
      by_contra h
      rw [List.getElem?_eq_none (by omega)] at hm
      simp at hm
    exact hdone t (by scalar_tac) hm

@[step]
theorem mask_covered_spec (spans : Slice Span) (file : Usize) (old : Bool) (mask : Slice Bool) :
    mask_covered spans file old mask ⦃ r =>
      (r = true ↔ ∀ t, mask.val[t]? = some true → Covered spans.val file.val old (t + 1)) ⦄ := by
  unfold mask_covered
  exact mask_covered_loop_spec spans file old mask 0#usize (by simp) (fun t ht => absurd ht (by simp))

@[step]
theorem file_ok_spec (d : FileDiff) (f : Usize) (spans : Slice Span) :
    file_ok d f spans ⦃ r => (r = true ↔ FileGood spans.val f.val d) ⦄ := by
  unfold file_ok
  step*
  · -- The removed mask has the wrong length.
    simp only [Bool.false_eq_true, false_iff, FileGood, not_and]
    intro h; simp_all
  · -- The added mask has the wrong length.
    simp only [Bool.false_eq_true, false_iff, FileGood, not_and]
    intro _ h; simp_all
  · simp_all
  · simp_all
  · -- Rebuild and old coverage hold: the result is new coverage.
    simp only [FileGood, lines]
    simp only [rest, List.drop_zero, deref_val] at *
    have h1 : d.removed.val.length = d.old.val.length := by simp_all
    have h2 : d.added.val.length = d.new.val.length := by simp_all
    rename_i hk _ hc _
    simp_all
  · -- Old coverage fails.
    simp only [Bool.false_eq_true, false_iff, FileGood, not_and]
    simp only [rest, List.drop_zero, deref_val] at *
    intro _ _ _ hc
    simp_all
  · -- The rebuild fails.
    simp only [Bool.false_eq_true, false_iff, FileGood, not_and]
    simp only [rest, List.drop_zero, deref_val, lines] at *
    intro _ _ hk
    simp_all

end GuideCheck.Cover
