import GuideCheck.Cover

/-! # Every file, every span, and `check` -/

open Aeneas Aeneas.Std Result guide_check GuideCheck.Spec GuideCheck.Small GuideCheck.Rebuild GuideCheck.Cover

namespace GuideCheck.Check

theorem files_ok_loop_spec (files : Slice FileDiff) (spans : Slice Span) (f : Usize)
    (hf : f.val ≤ files.val.length)
    (hdone : ∀ g d, g < f.val → files.val[g]? = some d → FileGood spans.val g d) :
    files_ok_loop files spans f ⦃ r =>
      (r = true ↔ ∀ g d, files.val[g]? = some d → FileGood spans.val g d) ⦄ := by
  unfold files_ok_loop
  apply loop.spec_decr_nat (fun f => files.val.length - f.val)
    (fun f => f.val ≤ files.val.length ∧ ∀ g d, g < f.val → files.val[g]? = some d → FileGood spans.val g d)
    _ _ _ _ ⟨hf, hdone⟩
  rintro f ⟨hf, hdone⟩
  unfold files_ok_loop.body
  step*
  · -- This file is good.
    refine ⟨by scalar_tac, fun g d hg hd => ?_, by scalar_tac⟩
    by_cases hgf : g < f.val
    · exact hdone g d hgf hd
    · have : g = f.val := by scalar_tac
      subst this
      have hlt : f.val < files.val.length := by scalar_tac
      rw [List.getElem?_eq_getElem hlt] at hd
      have hd' := Option.some.inj hd
      rw [← hd', ← fd_post]
      exact b_post.mp (by assumption)
  · -- This file is bad.
    simp only [Bool.false_eq_true, false_iff, not_forall]
    have hlt : f.val < files.val.length := by scalar_tac
    refine ⟨f.val, fd, by rw [List.getElem?_eq_getElem hlt, fd_post], fun h => ?_⟩
    have hb : ¬ b = true := by assumption
    exact hb (b_post.mpr h)
  · -- Every file checked.
    simp only [true_iff]
    intro g d hd
    have : g < files.val.length := by
      by_contra h
      rw [List.getElem?_eq_none (by omega)] at hd
      simp at hd
    exact hdone g d (by scalar_tac) hd

@[step]
theorem files_ok_spec (files : Slice FileDiff) (spans : Slice Span) :
    files_ok files spans ⦃ r => (r = true ↔ ∀ g d, files.val[g]? = some d → FileGood spans.val g d) ⦄ := by
  unfold files_ok
  exact files_ok_loop_spec files spans 0#usize (by simp) (fun g d hg => absurd hg (by simp))

theorem has_change_loop_spec (mask : Slice Bool) (top : Usize) (k0 k : Usize) (htop : top.val ≤ mask.val.length)
    (hk : k0.val ≤ k.val) (hnone : ∀ t, k0.val ≤ t → t < k.val → t < top.val → mask.val[t]? ≠ some true) :
    has_change_loop mask top k ⦃ r =>
      (r = true ↔ ∃ t, k0.val ≤ t ∧ t < top.val ∧ mask.val[t]? = some true) ⦄ := by
  unfold has_change_loop
  apply loop.spec_decr_nat (fun k => top.val - k.val)
    (fun k => k0.val ≤ k.val ∧ ∀ t, k0.val ≤ t → t < k.val → t < top.val → mask.val[t]? ≠ some true)
    _ _ _ _ ⟨hk, hnone⟩
  rintro k ⟨hk, hnone⟩
  unfold has_change_loop.body
  step*
  · -- A changed line.
    simp only [true_iff]
    have hlt : k.val < mask.val.length := by scalar_tac
    refine ⟨k.val, hk, by scalar_tac, ?_⟩
    rw [List.getElem?_eq_getElem hlt, ← b_post]; simp_all
  · refine ⟨by scalar_tac, fun t h1 h2 h3 => ?_, by scalar_tac⟩
    by_cases htk : t < k.val
    · exact hnone t h1 htk h3
    · have : t = k.val := by scalar_tac
      subst this
      have hlt : k.val < mask.val.length := by scalar_tac
      rw [List.getElem?_eq_getElem hlt, ← b_post]; simp_all

@[step]
theorem has_change_spec (mask : Slice Bool) (lo hi : Usize) (hlo : 1 ≤ lo.val) (hhi : hi.val ≤ mask.val.length) :
    has_change mask lo hi ⦃ r =>
      (r = true ↔ ∃ l, lo.val ≤ l ∧ l ≤ hi.val ∧ mask.val[l - 1]? = some true) ⦄ := by
  unfold has_change
  step*
  apply WP.spec_mono (has_change_loop_spec mask hi k k hhi (le_refl _) (fun t h1 h2 => absurd h2 (by omega)))
  intro r hr
  rw [hr]
  constructor
  · rintro ⟨t, h1, h2, h3⟩; exact ⟨t + 1, by scalar_tac, by scalar_tac, by simpa using h3⟩
  · rintro ⟨l, h1, h2, h3⟩; exact ⟨l - 1, by scalar_tac, by scalar_tac, h3⟩

theorem near_loop_spec (mask : Slice Bool) (line pad k : Usize) (hk : k.val ≤ mask.val.length)
    (hnone : ∀ t, t < k.val → mask.val[t]? = some true → pad.val < ndist (t + 1) line.val) :
    near_loop mask line pad k ⦃ r =>
      (r = true ↔ ∃ t, mask.val[t]? = some true ∧ ndist (t + 1) line.val ≤ pad.val) ⦄ := by
  unfold near_loop
  apply loop.spec_decr_nat (fun k => mask.val.length - k.val)
    (fun k => k.val ≤ mask.val.length ∧
      ∀ t, t < k.val → mask.val[t]? = some true → pad.val < ndist (t + 1) line.val)
    _ _ _ _ ⟨hk, hnone⟩
  rintro k ⟨hk, hnone⟩
  unfold near_loop.body
  step*
  · -- A changed line close enough.
    simp only [true_iff]
    have hlt : k.val < mask.val.length := by scalar_tac
    refine ⟨k.val, ?_, ?_⟩
    · rw [List.getElem?_eq_getElem hlt, ← b_post]; simp_all
    · have h2 := i2_post
      rw [i1_post] at h2
      scalar_tac
  · -- A changed line too far.
    refine ⟨by scalar_tac, fun t ht hm => ?_, by scalar_tac⟩
    by_cases htk : t < k.val
    · exact hnone t htk hm
    · have : t = k.val := by scalar_tac
      subst this
      rw [← i1_post, ← i2_post]; scalar_tac
  · -- A line with no change.
    refine ⟨by scalar_tac, fun t ht hm => ?_, by scalar_tac⟩
    by_cases htk : t < k.val
    · exact hnone t htk hm
    · have : t = k.val := by scalar_tac
      subst this
      have hlt : k.val < mask.val.length := by scalar_tac
      rw [List.getElem?_eq_getElem hlt, ← b_post] at hm
      simp_all
  · -- No changed line close enough.
    simp only [Bool.false_eq_true, false_iff, not_exists, not_and, not_le]
    intro t hm
    have : t < mask.val.length := by
      by_contra h
      rw [List.getElem?_eq_none (by omega)] at hm
      simp at hm
    exact hnone t (by scalar_tac) hm

@[step]
theorem near_spec (mask : Slice Bool) (line pad : Usize) :
    near mask line pad ⦃ r =>
      (r = true ↔ ∃ t, mask.val[t]? = some true ∧ ndist (t + 1) line.val ≤ pad.val) ⦄ := by
  unfold near
  exact near_loop_spec mask line pad 0#usize (by simp) (fun t ht => absurd ht (by simp))

/-- Line `l` has a changed line at most `pad` lines away. -/
def Near (mask : List Bool) (pad l : Nat) : Prop := ∃ t, mask[t]? = some true ∧ ndist (t + 1) l ≤ pad

theorem all_near_loop_spec (mask : Slice Bool) (top pad : Usize) (k0 k : Usize) (hk : k0.val ≤ k.val)
    (hdone : ∀ t, k0.val ≤ t → t < k.val → t < top.val → Near mask.val pad.val (t + 1)) :
    all_near_loop mask top pad k ⦃ r =>
      (r = true ↔ ∀ t, k0.val ≤ t → t < top.val → Near mask.val pad.val (t + 1)) ⦄ := by
  unfold all_near_loop
  apply loop.spec_decr_nat (fun k => top.val - k.val)
    (fun k => k0.val ≤ k.val ∧ ∀ t, k0.val ≤ t → t < k.val → t < top.val → Near mask.val pad.val (t + 1))
    _ _ _ _ ⟨hk, hdone⟩
  rintro k ⟨hk, hdone⟩
  unfold all_near_loop.body
  step*
  · refine ⟨by scalar_tac, fun t h1 h2 h3 => ?_, by scalar_tac⟩
    by_cases htk : t < k.val
    · exact hdone t h1 htk h3
    · have : t = k.val := by scalar_tac
      subst this
      have := b_post.mp (by assumption)
      rw [i_post] at this
      exact this
  · simp only [Bool.false_eq_true, false_iff, not_forall]
    refine ⟨k.val, hk, by scalar_tac, fun h => ?_⟩
    have hb : ¬ b = true := by assumption
    exact hb (b_post.mpr (by rw [i_post]; exact h))

@[step]
theorem all_near_spec (mask : Slice Bool) (lo hi pad : Usize) (hlo : 1 ≤ lo.val) :
    all_near mask lo hi pad ⦃ r =>
      (r = true ↔ ∀ l, lo.val ≤ l → l ≤ hi.val → Near mask.val pad.val l) ⦄ := by
  unfold all_near
  step*
  apply WP.spec_mono (all_near_loop_spec mask hi pad k k (le_refl _) (fun t h1 h2 => absurd h2 (by omega)))
  intro r hr
  rw [hr]
  constructor
  · intro h l h1 h2
    have := h (l - 1) (by scalar_tac) (by scalar_tac)
    rwa [show l - 1 + 1 = l by omega] at this
  · intro h t h1 h2
    exact h (t + 1) (by scalar_tac) (by scalar_tac)

@[step]
theorem span_fits_spec (mask : Slice Bool) (lo hi pad : Usize) :
    span_fits mask lo hi pad ⦃ r => (r = true ↔ Fits mask.val lo.val hi.val pad.val) ⦄ := by
  unfold span_fits
  step*
  · simp [Fits]
  · simp only [Bool.false_eq_true, false_iff, Fits, not_and]
    intro _ h
    scalar_tac
  · simp only [Bool.false_eq_true, false_iff, Fits, not_and]
    intro _ _ h
    scalar_tac
  · -- In bounds with a changed line: the result is the tightness check.
    have hc := b_post.mp (by assumption)
    simp only [Fits]
    rw [r_post]
    exact ⟨fun h => ⟨by scalar_tac, by scalar_tac, by scalar_tac, hc, h⟩, fun h => h.2.2.2.2⟩
  · -- No changed line in the span.
    simp only [Bool.false_eq_true, false_iff, Fits, not_and]
    intro _ _ _ h
    have hb : ¬ b = true := by assumption
    exact absurd (b_post.mpr h) hb

@[step]
theorem span_ok_spec (s : Span) (files : Slice FileDiff) (pad : Usize) :
    span_ok s files pad ⦃ r => (r = true ↔ SpanGood files.val pad.val s) ⦄ := by
  unfold span_ok
  step*
  · -- No such file.
    simp only [Bool.false_eq_true, false_iff, SpanGood, not_exists, not_and]
    intro d hd
    have : s.file.val < files.val.length := by
      by_contra h
      rw [List.getElem?_eq_none (by omega)] at hd
      simp at hd
    scalar_tac
  · -- The old side.
    have hlt : s.file.val < files.val.length := by scalar_tac
    have hold : s.old = true := by assumption
    rw [r_post]
    simp only [SpanGood, sideMask, hold, if_true, deref_val]
    constructor
    · intro h; exact ⟨fd, by rw [List.getElem?_eq_getElem hlt, fd_post], h⟩
    · rintro ⟨d, hd, h⟩
      rw [List.getElem?_eq_getElem hlt] at hd
      rw [fd_post, Option.some.inj hd]; exact h
  · -- The new side.
    have hlt : s.file.val < files.val.length := by scalar_tac
    have hold : s.old = false := by simp_all
    rw [r_post]
    simp only [SpanGood, sideMask, hold, Bool.false_eq_true, if_false, deref_val]
    constructor
    · intro h; exact ⟨fd, by rw [List.getElem?_eq_getElem hlt, fd_post], h⟩
    · rintro ⟨d, hd, h⟩
      rw [List.getElem?_eq_getElem hlt] at hd
      rw [fd_post, Option.some.inj hd]; exact h

theorem spans_ok_loop_spec (spans : Slice Span) (files : Slice FileDiff) (pad i : Usize)
    (hi : i.val ≤ spans.val.length)
    (hdone : ∀ t (ht : t < i.val), SpanGood files.val pad.val (spans.val[t]'(by omega))) :
    spans_ok_loop spans files pad i ⦃ r => (r = true ↔ ∀ s ∈ spans.val, SpanGood files.val pad.val s) ⦄ := by
  unfold spans_ok_loop
  apply loop.spec_decr_nat (fun i => spans.val.length - i.val)
    (fun i => ∃ _ : i.val ≤ spans.val.length,
      ∀ t (ht : t < i.val), SpanGood files.val pad.val (spans.val[t]'(by omega)))
    _ _ _ _ ⟨hi, hdone⟩
  rintro i ⟨hi, hdone⟩
  unfold spans_ok_loop.body
  step*
  · -- This span is good.
    refine ⟨⟨by scalar_tac, fun t ht => ?_⟩, by scalar_tac⟩
    by_cases hti : t < i.val
    · exact hdone t hti
    · have : t = i.val := by scalar_tac
      subst this
      have := b_post.mp (by assumption)
      rw [s_post] at this
      exact this
  · -- This span is bad.
    simp only [Bool.false_eq_true, false_iff, not_forall]
    refine ⟨s, by rw [s_post]; exact List.getElem_mem _, fun h => ?_⟩
    have hb : ¬ b = true := by assumption
    exact hb (b_post.mpr h)
  · -- Every span checked.
    simp only [true_iff]
    intro s hs
    obtain ⟨t, ht, rfl⟩ := List.getElem_of_mem hs
    exact hdone t (by scalar_tac)

@[step]
theorem spans_ok_spec (spans : Slice Span) (files : Slice FileDiff) (pad : Usize) :
    spans_ok spans files pad ⦃ r => (r = true ↔ ∀ s ∈ spans.val, SpanGood files.val pad.val s) ⦄ := by
  unfold spans_ok
  exact spans_ok_loop_spec spans files pad 0#usize (by simp) (fun t ht => absurd ht (by simp))

/-- **The main theorem.** `check` never panics, and it returns `true` exactly when
the guide is accepted: every file rebuilds, every changed line is covered, and
every span is real and tight. -/
theorem check_spec (files : Slice FileDiff) (spans : Slice Span) (pad : Usize) :
    check files spans pad ⦃ r => (r = true ↔ Accept files.val spans.val pad.val) ⦄ := by
  unfold check
  step*
  · simp only [Accept]
    rw [r_post]
    exact ⟨fun h => ⟨b_post.mp (by assumption), h⟩, fun h => h.2⟩
  · simp only [Bool.false_eq_true, false_iff, Accept, not_and]
    intro h
    have hb : ¬ b = true := by assumption
    exact absurd (b_post.mpr h) hb

end GuideCheck.Check
