import GuideCheck.Small
import GuideCheck.Keep

/-! # The rebuild check: kept old lines equal kept new lines -/

open Aeneas Aeneas.Std Result guide_check GuideCheck.Spec

namespace GuideCheck.Rebuild

/-- `next_kept` returns the first kept index at or after `i`. -/
@[step]
theorem next_kept_spec (mask : Slice Bool) (i : Usize) (hi : i.val ≤ mask.val.length) :
    next_kept mask i ⦃ j =>
      i.val ≤ j.val ∧ j.val ≤ mask.val.length ∧
      (∀ t, i.val ≤ t → t < j.val → mask.val[t]? = some true) ∧
      (j.val < mask.val.length → mask.val[j.val]? = some false) ⦄ := by
  unfold next_kept next_kept_loop
  apply loop.spec_decr_nat (fun j => mask.val.length - j.val)
    (fun j => i.val ≤ j.val ∧ j.val ≤ mask.val.length ∧
      ∀ t, i.val ≤ t → t < j.val → mask.val[t]? = some true) _ _ _ _
    ⟨le_refl _, hi, fun t h1 h2 => absurd h2 (by omega)⟩
  rintro j ⟨hij, hjl, hall⟩
  unfold next_kept_loop.body
  step*
  · -- A masked item: move on.
    have hlt : j.val < mask.val.length := by scalar_tac
    refine ⟨by scalar_tac, by scalar_tac, fun t h1 h2 => ?_, by scalar_tac⟩
    by_cases ht : t < j.val
    · exact hall t h1 ht
    · have : t = j.val := by scalar_tac
      subst this
      rw [List.getElem?_eq_getElem hlt, ← b_post]
      simp_all
  · -- A kept item: stop here.
    have hlt : j.val < mask.val.length := by scalar_tac
    refine ⟨hij, hjl, hall, fun _ => ?_⟩
    rw [List.getElem?_eq_getElem hlt, ← b_post]
    simp_all

/-- The kept lines of a slice of lines, from `i` on. -/
def rest (ls : Slice (alloc.vec.Vec U8)) (m : Slice Bool) (i : Nat) : List (List U8) :=
  keep ((ls.val.map (·.val)).drop i) (m.val.drop i)

theorem rest_skip (ls : Slice (alloc.vec.Vec U8)) (m : Slice Bool) (h : m.val.length = ls.val.length)
    (i j : Nat) (hij : i ≤ j) (hj : j ≤ m.val.length) (hall : ∀ t, i ≤ t → t < j → m.val[t]? = some true) :
    rest ls m i = rest ls m j := by
  unfold rest
  have := keep_drop_skip (ls.val.map (·.val)) m.val (by simp [h]) (j - i) i (by omega)
    (fun t h1 h2 => hall t h1 (by omega))
  rw [show i + (j - i) = j by omega] at this
  exact this

theorem rest_end (ls : Slice (alloc.vec.Vec U8)) (m : Slice Bool) (n : Nat) (hn : m.val.length ≤ n) :
    rest ls m n = [] := keep_drop_end _ _ _ hn

theorem rest_kept (ls : Slice (alloc.vec.Vec U8)) (m : Slice Bool) (h : m.val.length = ls.val.length)
    (a : Nat) (ha : a < m.val.length) (hf : m.val[a]? = some false) :
    rest ls m a = (ls.val[a]'(by omega)).val :: rest ls m (a + 1) := by
  unfold rest
  rw [keep_drop_kept (ls.val.map (·.val)) m.val (by simp [h]) a ha hf]
  simp

@[step]
theorem kept_equal_from_spec (old : Slice (alloc.vec.Vec U8)) (removed : Slice Bool)
    (new : Slice (alloc.vec.Vec U8)) (added : Slice Bool)
    (hr : removed.val.length = old.val.length) (ha : added.val.length = new.val.length)
    (i0 j0 : Usize) (hi : i0.val ≤ old.val.length) (hj : j0.val ≤ new.val.length) :
    kept_equal_from old removed new added i0 j0 ⦃ r =>
      (r = true ↔ rest old removed i0.val = rest new added j0.val) ⦄ := by
  unfold kept_equal_from kept_equal_from_loop
  apply loop.spec_decr_nat
    (fun (st : Usize × Usize) => (old.val.length - st.1.val) + (new.val.length - st.2.val))
    (fun (st : Usize × Usize) => st.1.val ≤ old.val.length ∧ st.2.val ≤ new.val.length ∧
      (rest old removed st.1.val = rest new added st.2.val ↔
        rest old removed i0.val = rest new added j0.val)) _ _ _ _
    ⟨hi, hj, Iff.rfl⟩
  rintro ⟨i, j⟩ ⟨hi, hj, hinv⟩
  simp only at hi hj hinv
  unfold kept_equal_from_loop.body
  step*
  · -- The old side has no kept line left: equal only if the new side has none either.
    have hsI := rest_skip old removed hr i.val a.val a_post1 a_post2 a_post3
    have hsJ := rest_skip new added ha j.val b1.val b1_post1 b1_post2 b1_post3
    rw [← hinv, hsI, hsJ, rest_end old removed a.val (by scalar_tac)]
    by_cases hb : b1.val < added.val.length
    · rw [rest_kept new added ha b1.val hb (b1_post4 hb)]
      simp only [List.nil_eq, reduceCtorEq, iff_false, decide_eq_true_eq]
      scalar_tac
    · rw [rest_end new added b1.val (by scalar_tac)]
      simp only [iff_true, decide_eq_true_eq]
      scalar_tac
  · -- The new side has no kept line left, the old side has one.
    have hsI := rest_skip old removed hr i.val a.val a_post1 a_post2 a_post3
    have hsJ := rest_skip new added ha j.val b1.val b1_post1 b1_post2 b1_post3
    have hlt : a.val < removed.val.length := by scalar_tac
    rw [← hinv, hsI, hsJ, rest_kept old removed hr a.val hlt (a_post4 hlt),
      rest_end new added b1.val (by scalar_tac)]
    simp
  · -- The same line on both sides: go on with the rest.
    have hsI := rest_skip old removed hr i.val a.val a_post1 a_post2 a_post3
    have hsJ := rest_skip new added ha j.val b1.val b1_post1 b1_post2 b1_post3
    have hlt : a.val < removed.val.length := by scalar_tac
    have hlt1 : b1.val < added.val.length := by scalar_tac
    have hv : (old.val[a.val]'(by omega)).val = (new.val[b1.val]'(by omega)).val := by
      have := b2_post.mp (by assumption)
      simpa [v_post, v1_post] using this
    refine ⟨by scalar_tac, by scalar_tac, ?_, by scalar_tac⟩
    rw [← hinv, hsI, hsJ, rest_kept old removed hr a.val hlt (a_post4 hlt),
      rest_kept new added ha b1.val hlt1 (b1_post4 hlt1), i5_post, j1_post, hv]
    simp
  · -- Different lines: the kept lines differ.
    have hsI := rest_skip old removed hr i.val a.val a_post1 a_post2 a_post3
    have hsJ := rest_skip new added ha j.val b1.val b1_post1 b1_post2 b1_post3
    have hlt : a.val < removed.val.length := by scalar_tac
    have hlt1 : b1.val < added.val.length := by scalar_tac
    have hne : (old.val[a.val]'(by omega)).val ≠ (new.val[b1.val]'(by omega)).val := by
      intro h
      have : b2 = true := b2_post.mpr (by simpa [v_post, v1_post] using h)
      simp_all
    rw [← hinv, hsI, hsJ, rest_kept old removed hr a.val hlt (a_post4 hlt),
      rest_kept new added ha b1.val hlt1 (b1_post4 hlt1)]
    simp [hne]
  · -- No lines left on either side.
    rw [← hinv, rest_end old removed i.val (by scalar_tac), rest_end new added j.val (by scalar_tac)]
    simp

end GuideCheck.Rebuild
