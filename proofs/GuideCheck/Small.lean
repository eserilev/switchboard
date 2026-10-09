import GuideCheck.Spec

/-! # The small functions -/

open Aeneas Aeneas.Std Result guide_check GuideCheck.Spec

namespace GuideCheck.Small

@[simp] theorem deref_val {α : Type} (v : alloc.vec.Vec α) : (alloc.vec.Vec.deref v).val = v.val := rfl

theorem bytes_equal_loop_spec (a b : Slice U8) (i : Usize) (hlen : a.val.length = b.val.length)
    (hi : i.val ≤ a.val.length) (hpre : a.val.take i.val = b.val.take i.val) :
    bytes_equal_loop a b i ⦃ r => (r = true ↔ a.val = b.val) ⦄ := by
  unfold bytes_equal_loop
  apply loop.spec_decr_nat (fun i => a.val.length - i.val)
    (fun i => i.val ≤ a.val.length ∧ a.val.take i.val = b.val.take i.val) _ _ _ _ ⟨hi, hpre⟩
  rintro i ⟨hi, hpre⟩
  unfold bytes_equal_loop.body
  step*
  · -- Same byte: the equal prefix grows by one.
    have hlt : i.val < a.val.length := by scalar_tac
    refine ⟨by scalar_tac, ?_, by scalar_tac⟩
    have heq : ¬(i2 != i3) = true := by assumption
    simp only [bne_iff_ne, ne_eq, not_not] at heq
    rw [i4_post, List.take_add_one, List.take_add_one, hpre, List.getElem?_eq_getElem hlt,
      List.getElem?_eq_getElem (by omega), ← i2_post, ← i3_post, heq]
  · -- Every byte matched.
    simp only [true_iff]
    have : i.val = a.val.length := by scalar_tac
    rw [this, List.take_length, hlen, List.take_length] at hpre
    exact hpre

@[step]
theorem bytes_equal_spec (a b : Slice U8) :
    bytes_equal a b ⦃ r => (r = true ↔ a.val = b.val) ⦄ := by
  unfold bytes_equal
  step*
  apply bytes_equal_loop_spec a b 0#usize (by scalar_tac) (by simp) (by simp)

@[step]
theorem lines_left_spec (i n j m : Usize) :
    lines_left i n j m ⦃ r => (r = true ↔ (i.val < n.val ∨ j.val < m.val)) ⦄ := by
  unfold lines_left
  step*


@[step]
theorem span_holds_spec (s : Span) (file : Usize) (old : Bool) (line : Usize) :
    span_holds s file old line ⦃ r =>
      (r = true ↔ (s.file.val = file.val ∧ s.old = old ∧ s.from.val ≤ line.val ∧ line.val ≤ s.to.val)) ⦄ := by
  unfold span_holds
  step*


@[step]
theorem dist_spec (a b : Usize) : dist a b ⦃ r => r.val = ndist a.val b.val ⦄ := by
  unfold dist
  step*
  all_goals simp_all [ndist]


end GuideCheck.Small
