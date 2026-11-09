import GitArgCheck.Small
import GitArgCheck.RefSpec

/-! # `ref_ok` and `split_remote`, and the small functions of `refs` -/

open Aeneas Aeneas.Std Result git_argcheck GitArgCheck.Spec GitArgCheck.RefSpec

namespace GitArgCheck.Refs

open GitArgCheck.Small

@[simp] theorem byte_space : byte ' ' = 32#u8 := by decide

@[step]
theorem bad_ref_byte_spec (x : U8) : refs.bad_ref_byte x ⦃ r => (r = true ↔ RefusedByte x) ⦄ := by
  unfold refs.bad_ref_byte
  step*
  · simp only [RefusedByte, byte_space, true_iff]
    by_cases h : x.val < 32
    · exact Or.inl h
    · right; left; apply UScalar.eq_of_val_eq; simp; scalar_tac
  · simp only [RefusedByte, byte_space, decide_eq_true_eq]
    constructor
    · intro h; right; right; rw [h]; rfl
    · rintro (h | h | h)
      · scalar_tac
      · subst h; scalar_tac
      · apply UScalar.eq_of_val_eq; simp [h]

theorem ref_ok_loop_spec (r : Slice U8) (i : Usize) (hi : i.val ≤ r.val.length)
    (hpre : ∀ j < i.val, ∀ x, r.val[j]? = some x → ¬ RefusedByte x) :
    refs.ref_ok_loop r i ⦃ b => (b = true ↔ ∀ x ∈ r.val, ¬ RefusedByte x) ⦄ := by
  unfold refs.ref_ok_loop
  apply loop.spec_decr_nat (fun i => r.val.length - i.val)
    (fun i => i.val ≤ r.val.length ∧ ∀ j < i.val, ∀ x, r.val[j]? = some x → ¬ RefusedByte x) _ _ _ _
    ⟨hi, hpre⟩
  rintro i ⟨hi, hpre⟩
  unfold refs.ref_ok_loop.body
  step*
  · simp only [Bool.false_eq_true, false_iff, not_forall, not_not]
    have hl : i.val < r.val.length := by scalar_tac
    exact ⟨_, List.getElem_mem hl, by simp_all⟩
  · refine ⟨by scalar_tac, fun j hj x hx => ?_, by scalar_tac⟩
    by_cases hji : j < i.val
    · exact hpre j hji x hx
    · have : j = i.val := by scalar_tac
      subst this
      have hl : i.val < r.val.length := by scalar_tac
      rw [List.getElem?_eq_getElem hl] at hx
      simp only [Option.some.injEq] at hx
      subst hx
      simp_all
  · simp only [true_iff]
    intro x hx
    rw [List.mem_iff_getElem?] at hx
    obtain ⟨j, hj⟩ := hx
    have : j < r.val.length := by
      by_contra hc
      rw [List.getElem?_eq_none (by omega)] at hj
      simp at hj
    exact hpre j (by scalar_tac) x hj

theorem REF_MAX_eq : refs.REF_MAX = 256#usize := by simp [global_simps]

@[step]
theorem ref_ok_spec (r : Slice U8) : refs.ref_ok r ⦃ b => (b = true ↔ RefOk r.val) ⦄ := by
  unfold refs.ref_ok
  obtain ⟨l, hl⟩ := r
  cases l with
  | nil => step*; simp [RefOk]
  | cons y ys =>
    simp only [REF_MAX_eq]
    step*
    · simp_all [RefOk]
    · simp only [Bool.false_eq_true, false_iff, RefOk, not_and]
      intro _ _ h; scalar_tac
    · apply WP.spec_mono (ref_ok_loop_spec _ 0#usize (by simp) (fun j hj => absurd hj (by simp)))
      intro b hb
      rw [hb]
      simp only [RefOk, ne_eq, reduceCtorEq, not_false_eq_true, List.head?_cons, Option.some.injEq,
        true_and]
      constructor
      · intro h; refine ⟨?_, by scalar_tac, h⟩
        simp_all
      · intro h; exact h.2.2

/-- An accepted ref is one git arg that git does not read as an option. -/
theorem ref_ok_one_arg (r : List U8) (h : RefOk r) :
    (∀ s : List U8, r ++ s ≠ [] ∧ (r ++ s).head? ≠ some (byte '-')) ∧
    byte '\x00' ∉ r ∧ byte '\n' ∉ r ∧ byte ' ' ∉ r := by
  obtain ⟨hne, hhd, -, hall⟩ := h
  refine ⟨fun s => ⟨by simp [hne], ?_⟩, fun hm => hall _ hm (Or.inl (by decide)),
    fun hm => hall _ hm (Or.inl (by decide)), fun hm => hall _ hm (Or.inr (Or.inl rfl))⟩
  cases r with
  | nil => exact absurd rfl hne
  | cons x xs => simpa using hhd

/-! ## `split_remote` -/

theorem fits_iff (nm rm : List U8) :
    Fits nm rm ↔ rm.length + 1 < nm.length ∧ nm[rm.length]? = some (byte '/') ∧ nm.take rm.length = rm := by
  constructor
  · rintro ⟨b, hb, rfl⟩
    have : b.length ≠ 0 := by simpa using hb
    refine ⟨by simp; omega, by simp, by simp⟩
  · rintro ⟨hl, hs, ht⟩
    refine ⟨nm.drop (rm.length + 1), by simp; omega, ?_⟩
    have hlt : rm.length < nm.length := by omega
    rw [List.getElem?_eq_getElem hlt, Option.some.injEq] at hs
    conv_lhs => rw [← List.take_append_drop rm.length nm, List.drop_eq_getElem_cons hlt]
    rw [ht, hs]

@[step]
theorem fits_spec (nm r : Slice U8) : refs.fits nm r ⦃ b => (b = true ↔ Fits nm.val r.val) ⦄ := by
  unfold refs.fits
  step*
  all_goals rw [fits_iff]
  · simp only [Bool.false_eq_true, false_iff, not_and]; intro h; scalar_tac
  · simp only [Bool.false_eq_true, false_iff, not_and]; intro h; scalar_tac
  · simp only [Bool.false_eq_true, false_iff, not_and]
    intro _ h
    have hl : r.val.length < nm.val.length := by scalar_tac
    rw [List.getElem?_eq_getElem hl, Option.some.injEq] at h
    simp_all
  · have hl : r.val.length < nm.val.length := by scalar_tac
    have h47 : nm.val[r.val.length] = 47#u8 := by
      apply UScalar.eq_of_val_eq; simp_all
    have hrl : r.len.val = r.val.length := by simp
    rw [b_post, hrl, List.getElem?_eq_getElem hl, byte_slash]
    simp only [Nat.sub_zero, List.drop_zero]
    simp only [h47, true_and]
    constructor
    · intro h2; exact ⟨by scalar_tac, h2⟩
    · intro h2; exact h2.2

/-- What the loop of `split_remote` knows after the first `k` remotes. -/
def Best (nm : List U8) (rs : List (alloc.vec.Vec U8)) (found : Bool) (best k : Nat) : Prop :=
  (found = false → ∀ j < k, ∀ v, rs[j]? = some v → ¬ Fits nm v.val) ∧
  (found = true → (∃ j < k, ∃ v, rs[j]? = some v ∧ Fits nm v.val ∧ v.val.length = best) ∧
    ∀ j < k, ∀ v, rs[j]? = some v → Fits nm v.val → v.val.length ≤ best)

@[simp] theorem deref_val (v : alloc.vec.Vec U8) : (alloc.vec.Vec.deref v).val = v.val := rfl

theorem lt_succ_cases (j k : Nat) (h : j < k + 1) : j < k ∨ j = k := by omega

theorem best_skip (nm : List U8) (rs : List (alloc.vec.Vec U8)) (f : Bool) (b k : Nat)
    (v : alloc.vec.Vec U8) (hv : rs[k]? = some v) (hn : ¬ Fits nm v.val) (h : Best nm rs f b k) :
    Best nm rs f b (k + 1) := by
  obtain ⟨h1, h2⟩ := h
  refine ⟨fun hf j hj w hw => ?_, fun hf => ?_⟩
  · rcases lt_succ_cases j k hj with hj | rfl
    · exact h1 hf j hj w hw
    · rw [hv, Option.some.injEq] at hw; subst hw; exact hn
  · obtain ⟨⟨j, hj, w, hw, hfw, hl⟩, h3⟩ := h2 hf
    refine ⟨⟨j, by omega, w, hw, hfw, hl⟩, fun j' hj' w' hw' hf' => ?_⟩
    rcases lt_succ_cases j' k hj' with hj' | rfl
    · exact h3 j' hj' w' hw' hf'
    · rw [hv, Option.some.injEq] at hw'; subst hw'; exact absurd hf' hn

theorem best_take (nm : List U8) (rs : List (alloc.vec.Vec U8)) (f : Bool) (b k : Nat)
    (v : alloc.vec.Vec U8) (hv : rs[k]? = some v) (hy : Fits nm v.val) (h : Best nm rs f b k)
    (hbig : f = true → b ≤ v.val.length) :
    Best nm rs true v.val.length (k + 1) := by
  obtain ⟨h1, h2⟩ := h
  refine ⟨fun hf => absurd hf (by simp), fun _ => ⟨⟨k, by omega, v, hv, hy, rfl⟩, fun j hj w hw hfw => ?_⟩⟩
  rcases lt_succ_cases j k hj with hj | rfl
  · cases f with
    | false => exact absurd hfw (h1 rfl j hj w hw)
    | true => exact le_trans ((h2 rfl).2 j hj w hw hfw) (hbig rfl)
  · rw [hv, Option.some.injEq] at hw; subst hw; exact le_refl _

theorem best_keep (nm : List U8) (rs : List (alloc.vec.Vec U8)) (b k : Nat)
    (v : alloc.vec.Vec U8) (hv : rs[k]? = some v) (hle : v.val.length ≤ b) (h : Best nm rs true b k) :
    Best nm rs true b (k + 1) := by
  obtain ⟨_, h2⟩ := h
  obtain ⟨⟨j, hj, w, hw, hfw, hl⟩, h3⟩ := h2 rfl
  refine ⟨fun hf => absurd hf (by simp), fun _ => ⟨⟨j, by omega, w, hw, hfw, hl⟩, fun j' hj' w' hw' hf' => ?_⟩⟩
  rcases lt_succ_cases j' k hj' with hj' | rfl
  · exact h3 j' hj' w' hw' hf'
  · rw [hv, Option.some.injEq] at hw'; subst hw'; exact hle

theorem split_remote_loop_spec (nm : Slice U8) (rs : Slice (alloc.vec.Vec U8)) (found : Bool)
    (best k : Usize) (hk : k.val ≤ rs.val.length) (hb : Best nm.val rs.val found best.val k.val) :
    refs.split_remote_loop nm rs found best k ⦃ r =>
      Best nm.val rs.val r.1 r.2.val rs.val.length ⦄ := by
  unfold refs.split_remote_loop
  apply loop.spec_decr_nat (fun (st : Bool × Usize × Usize) => rs.val.length - st.2.2.val)
    (fun (st : Bool × Usize × Usize) => st.2.2.val ≤ rs.val.length ∧
      Best nm.val rs.val st.1 st.2.1.val st.2.2.val) _ _ _ _ ⟨hk, hb⟩
  rintro ⟨f, b, k⟩ ⟨hk, hb⟩
  simp only at hk hb
  unfold refs.split_remote_loop.body
  step*
  · rename_i bst _ _
    by_cases hbt : b = true <;> by_cases hf : f = true <;> by_cases hgt : v.len > bst <;>
      simp only [hbt, hf, hgt, if_true, if_false, Bool.false_eq_true, bind_tc_ok] <;> step* <;>
      refine ⟨by scalar_tac, ?_, by scalar_tac⟩ <;> rw [k1_post] <;>
      have hkl : k.val < rs.val.length := by scalar_tac
    all_goals have hv : rs.val[k.val]? = some v := by rw [List.getElem?_eq_getElem hkl, v_post]
    all_goals have hlen : (alloc.vec.Vec.len v).val = v.val.length := by simp
    all_goals cases f <;> simp only [Bool.false_eq_true, not_false_eq_true, not_true_eq_false] at hf ⊢
    all_goals first
      | (have hn : ¬ Fits nm.val v.val := by simp_all
         exact best_skip _ _ _ _ _ v hv hn hb)
      | (have hle : v.val.length ≤ bst.val := by scalar_tac
         exact best_keep _ _ _ _ v hv hle hb)
      | (have hy : Fits nm.val v.val := by simp_all
         rw [hlen]; exact best_take _ _ _ _ _ v hv hy hb (by intro; scalar_tac))

theorem split_remote_spec (nm : Slice U8) (rs : Slice (alloc.vec.Vec U8)) :
    refs.split_remote nm rs ⦃ r => match r with
      | some n => ∃ rm ∈ rs.val, rm.val.length = n.val ∧
          nm.val = rm.val ++ byte '/' :: nm.val.drop (n.val + 1) ∧ nm.val.drop (n.val + 1) ≠ [] ∧
          ∀ rm' ∈ rs.val, Fits nm.val rm'.val → rm'.val.length ≤ n.val
      | none => ∀ rm ∈ rs.val, ¬ Fits nm.val rm.val ⦄ := by
  unfold refs.split_remote
  have h0 : Best nm.val rs.val false 0 0 := ⟨fun _ j hj => absurd hj (by omega), fun h => absurd h (by simp)⟩
  apply WP.spec_bind (split_remote_loop_spec nm rs false 0#usize 0#usize (by simp) h0)
  rintro ⟨f, b⟩ ⟨h1, h2⟩
  cases f with
  | false =>
    apply (WP.spec_ok _).mpr
    intro rm hrm
    obtain ⟨j, hj, hjr⟩ := List.getElem_of_mem hrm
    exact h1 rfl j hj rm (by rw [List.getElem?_eq_getElem hj, hjr])
  | true =>
    apply (WP.spec_ok _).mpr
    obtain ⟨⟨j, hj, v, hv, hfv, hl⟩, h3⟩ := h2 rfl
    obtain ⟨br, hbr, he⟩ := hfv
    refine ⟨v, List.mem_of_getElem? hv, hl, ?_, ?_, ?_⟩
    · have : nm.val.drop (b.val + 1) = br := by rw [he, ← hl]; simp
      rw [this]; exact he
    · have : nm.val.drop (b.val + 1) = br := by rw [he, ← hl]; simp
      rw [this]; exact hbr
    · intro rm' hrm' hf'
      obtain ⟨j', hj', hjr'⟩ := List.getElem_of_mem hrm'
      exact h3 j' (by simpa using hj') rm' (by rw [List.getElem?_eq_getElem hj', hjr']) hf'

end GitArgCheck.Refs
