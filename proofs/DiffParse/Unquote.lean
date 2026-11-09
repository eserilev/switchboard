import DiffParse.Spec

/-! # `unquote`: no panic, and it undoes git's quoting -/

open Aeneas Aeneas.Std Result diffparse DiffParse.Spec

namespace DiffParse.Unquote

/-- The byte of a one-letter escape, as `letter` gives it. -/
def letterL (c : U8) : Option U8 :=
  if c = 34#u8 then some c
  else if c = 92#u8 then some c
  else if c = 97#u8 then some 7#u8
  else if c = 98#u8 then some 8#u8
  else if c = 116#u8 then some 9#u8
  else if c = 110#u8 then some 10#u8
  else if c = 118#u8 then some 11#u8
  else if c = 102#u8 then some 12#u8
  else if c = 114#u8 then some 13#u8
  else none

@[step]
theorem letter_spec (c : U8) : letter c ⦃ r => r = letterL c ⦄ := by
  unfold letter letterL
  split_ifs <;> simp

abbrev OctOk (x : U8) : Prop := 48 ≤ x.val ∧ x.val ≤ 55

@[step]
theorem is_octal_spec (x : U8) : is_octal x ⦃ r => (r = true ↔ OctOk x) ⦄ := by
  unfold is_octal
  step*

@[step]
theorem octal_byte_spec (c d e : U8) :
    octal_byte c d e ⦃ r => (r = true ↔ 48 ≤ c.val ∧ c.val ≤ 51 ∧ OctOk d ∧ OctOk e) ⦄ := by
  unfold octal_byte
  step*


/-- The byte that `decode_at` reads at the front of `l`, and how many bytes it takes. -/
def front (l : List U8) : U8 × Nat :=
  let x := l[0]?.getD 0#u8
  let c := l[1]?.getD 0#u8
  let d := l[2]?.getD 0#u8
  let e := l[3]?.getD 0#u8
  if x ≠ 92#u8 then (x, 1)
  else if 4 ≤ l.length ∧ 48 ≤ c.val ∧ c.val ≤ 51 ∧ OctOk d ∧ OctOk e then
    (u8 ((c.val - 48) * 64 + (d.val - 48) * 8 + (e.val - 48)), 4)
  else if 2 ≤ l.length then
    match letterL c with
    | some y => (y, 2)
    | none => (x, 1)
  else (x, 1)

theorem u8_val (n : Nat) : (u8 n).val = n % 256 := by
  simp [u8, UScalar.val]

theorem not_bne (x y : U8) (h : ¬(x != y) = true) : x = y := by
  simp only [bne_iff_ne, ne_eq, Decidable.not_not] at h; exact h

theorem gs (l : List U8) (i : Nat) (h : i < l.length) (x : U8) (hx : x = l[i]) : l[i]? = some x := by
  rw [hx]; exact List.getElem?_eq_getElem h

theorem front_pos (l : List U8) : 1 ≤ (front l).2 := by
  unfold front
  dsimp only
  split_ifs
  all_goals try simp
  split <;> simp

theorem front_le (l : List U8) (h : l ≠ []) : (front l).2 ≤ l.length := by
  have : 1 ≤ l.length := by cases l <;> simp_all
  unfold front
  dsimp only
  split_ifs
  all_goals try (simp; omega)
  split <;> simp <;> omega

theorem decode_at_front (b : Slice U8) (k : Usize) (hk : k.val < b.val.length) :
    decode_at b k ⦃ r => r.1 = (front (b.val.drop k.val)).1 ∧ r.2.val = (front (b.val.drop k.val)).2 ⦄ := by
  unfold decode_at
  step*
  all_goals
    have hx := gs b.val k.val hk x x_post
    simp only [front, List.getElem?_drop, List.length_drop, Nat.add_zero, hx, Option.getD_some]
  · simp_all
  all_goals (try (have hc := gs b.val i1.val (by scalar_tac) _ (by assumption); rw [i1_post] at hc))
  all_goals (try (have hd := gs b.val i2.val (by scalar_tac) _ (by assumption); rw [i2_post] at hd))
  all_goals (try (have he := gs b.val i3.val (by scalar_tac) _ (by assumption); rw [i3_post] at he))
  all_goals (try simp only [hc, Option.getD_some])
  all_goals (try simp only [hd, Option.getD_some])
  all_goals (try simp only [he, Option.getD_some])
  all_goals (have hx92 : x = 92#u8 := not_bne x 92#u8 (by assumption))
  all_goals (try simp only [hx92, ne_eq, not_true_eq_false, if_false])
  all_goals (try (have hci : c = i4 := (by rw [i4_post, c_post])); subst hci)
  all_goals (try (have hcond := b1_post.mp (by assumption)))
  all_goals (try (have hcond : ¬(48 ≤ c.val ∧ c.val ≤ 51 ∧ 48 ≤ d.val ∧ d.val ≤ 55 ∧ OctOk e) := fun h => (by assumption : ¬b1 = true) (b1_post.mpr h)))
  all_goals (try (have hlen : (↑b.len : Nat) = b.val.length := (by simp)))
  all_goals split_ifs
  all_goals simp only [OctOk] at *
  all_goals first
    | (refine ⟨?_, rfl⟩; apply UScalar.eq_of_val_eq; rw [u8_val]; scalar_tac)
    | (simp; done)
    | (exfalso; scalar_tac)
    | (rw [← o_post]; subst_vars; simp)

@[step]
theorem decode_at_spec (b : Slice U8) (k : Usize) (hk : k.val < b.val.length) :
    decode_at b k ⦃ r => r.1 = (front (b.val.drop k.val)).1 ∧ r.2.val = (front (b.val.drop k.val)).2 ∧
      1 ≤ r.2.val ∧ k.val + r.2.val ≤ b.val.length ⦄ := by
  apply WP.spec_mono (decode_at_front b k hk)
  rintro r ⟨h1, h2⟩
  have hne : b.val.drop k.val ≠ [] := by simp; omega
  have := front_pos (b.val.drop k.val)
  have := front_le (b.val.drop k.val) hne
  simp only [List.length_drop] at this
  refine ⟨h1, h2, by omega, by omega⟩

set_option linter.unusedVariables false in
/-- The bytes that `unquote` gives for `l`: the front byte, then the rest. -/
def dec (l : List U8) : List U8 :=
  if h : l = [] then [] else (front l).1 :: dec (l.drop (front l).2)
termination_by l.length
decreasing_by
  have := front_pos l
  have : 0 < l.length := List.length_pos_of_ne_nil h
  simp only [List.length_drop]
  omega

theorem dec_nil : dec [] = [] := by rw [dec]; simp

theorem dec_cons (l : List U8) (h : l ≠ []) : dec l = (front l).1 :: dec (l.drop (front l).2) := by
  rw [dec]; simp [h]

theorem dec_length (l : List U8) : (dec l).length ≤ l.length := by
  induction h : l.length using Nat.strong_induction_on generalizing l with
  | _ n ih =>
    by_cases hl : l = []
    · subst hl; simp [dec_nil]
    · rw [dec_cons l hl]
      have h1 := front_pos l
      have h2 := front_le l hl
      have := ih (l.drop (front l).2).length (by simp; omega) _ rfl
      simp only [List.length_cons, List.length_drop] at this ⊢
      omega

theorem unquote_loop_spec (b : Slice U8) (out : alloc.vec.Vec U8) (k : Usize)
    (hk : k.val ≤ b.val.length) (ho : out.val.length ≤ k.val) :
    unquote_loop b out k ⦃ r => r.val = out.val ++ dec (b.val.drop k.val) ⦄ := by
  unfold unquote_loop
  apply loop.spec_decr_nat (fun (st : alloc.vec.Vec U8 × Usize) => b.val.length - st.2.val)
    (fun (st : alloc.vec.Vec U8 × Usize) => st.2.val ≤ b.val.length ∧ st.1.val.length ≤ st.2.val ∧
      st.1.val ++ dec (b.val.drop st.2.val) = out.val ++ dec (b.val.drop k.val)) _ _ _ _ ⟨hk, ho, rfl⟩
  rintro ⟨o, j⟩ ⟨hj, hs, he⟩
  simp only at hj hs he
  unfold unquote_loop.body
  step*
  · have hne : b.val.drop j.val ≠ [] := by simp; scalar_tac
    refine ⟨by scalar_tac, by simp [out1_post]; scalar_tac, ?_, by scalar_tac⟩
    rw [out1_post, ← he, dec_cons _ hne, k1_post, ← y_post1, ← y_post2, List.drop_drop]
    simp
  · have : j.val = b.val.length := by scalar_tac
    rw [← he, this]
    simp [dec_nil]

/-- `unquote` gives `dec` of its input. -/
@[step]
theorem unquote_spec (b : Slice U8) : unquote b ⦃ r => r.val = dec b.val ⦄ := by
  unfold unquote
  apply WP.spec_mono (unquote_loop_spec b (alloc.vec.Vec.new U8) 0#usize (by simp) (by simp))
  intro r hr
  simpa using hr

/-! ## `unquote` undoes git's quoting -/

theorem front_quote (x : U8) (r : List U8) : front (quoteByte x ++ r) = (x, (quoteByte x).length) := by
  unfold quoteByte
  split_ifs
  all_goals (try subst_vars)
  all_goals (try (simp [front, letterL]; done))
  · have hx : x.val < 256 := by scalar_tac
    have h1 : (u8 (48 + x.val / 64)).val = 48 + x.val / 64 := by rw [u8_val]; omega
    have h2 : (u8 (48 + x.val / 8 % 8)).val = 48 + x.val / 8 % 8 := by rw [u8_val]; omega
    have h3 : (u8 (48 + x.val % 8)).val = 48 + x.val % 8 := by rw [u8_val]; omega
    simp only [front, List.cons_append, List.getElem?_cons_zero, List.getElem?_cons_succ,
      Option.getD_some, List.length_cons, h1, h2, h3, ne_eq, not_true_eq_false, if_false]
    rw [if_pos (by simp only [OctOk, List.length_append, h3]; omega)]
    simp only [Prod.mk.injEq]
    refine ⟨?_, by simp⟩
    apply UScalar.eq_of_val_eq
    rw [u8_val]
    omega
  · simp_all [front]

theorem dec_quote (s : List U8) : dec (quote s) = s := by
  induction s with
  | nil => simp [quote, dec_nil]
  | cons x t ih =>
    have hq : quote (x :: t) = quoteByte x ++ quote t := by simp [quote]
    have hne : quoteByte x ++ quote t ≠ [] := by unfold quoteByte; split_ifs <;> simp
    rw [hq, dec_cons _ hne, front_quote]
    simp only [List.drop_left]
    rw [ih]

/-- A path with no backslash comes out as it is. -/
theorem dec_plain (l : List U8) (h : 92#u8 ∉ l) : dec l = l := by
  induction hl : l.length using Nat.strong_induction_on generalizing l with
  | _ n ih =>
    cases l with
    | nil => simp [dec_nil]
    | cons x t =>
      have hx : x ≠ 92#u8 := fun e => h (e ▸ List.mem_cons_self)
      have hf : front (x :: t) = (x, 1) := by simp [front, hx]
      rw [dec_cons _ (by simp), hf]
      simp only [List.drop_succ_cons, List.drop_zero]
      rw [ih t.length (by simp at hl; omega) t (fun e => h (List.mem_cons_of_mem _ e)) rfl]

end DiffParse.Unquote
