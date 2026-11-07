import GitArgCheck.Tables

/-! # The check: options, args, and the whole call -/

open Aeneas Aeneas.Std Result git_argcheck GitArgCheck.Spec

namespace GitArgCheck.Check

set_option linter.unusedTactic false

open GitArgCheck.Small GitArgCheck.Tables

@[simp] theorem byte_U : byte 'U' = 85#u8 := by decide
@[simp] theorem byte_M : byte 'M' = 77#u8 := by decide
@[simp] theorem byte_C : byte 'C' = 67#u8 := by decide
@[simp] theorem byte_l : byte 'l' = 108#u8 := by decide
@[simp] theorem byte_S : byte 'S' = 83#u8 := by decide
@[simp] theorem byte_G : byte 'G' = 71#u8 := by decide
@[simp] theorem byte_L : byte 'L' = 76#u8 := by decide
@[simp] theorem byte_n : byte 'n' = 110#u8 := by decide

theorem pct_not_digit : ¬ isDigit 37#u8 := by simp [isDigit]

/-- The bytes before the trailing `%` are digits exactly when the bytes are a score. -/
theorem score_iff (v : List U8) (m : Nat) (hm : m ≤ v.length)
    (htail : ∀ j, m ≤ j → j < v.length → v[j]? = some 37#u8)
    (hlast : 0 < m → v[m - 1]? ≠ some 37#u8) :
    Digits (v.take m) ↔ Score v := by
  constructor
  · intro hd
    refine ⟨v.take m, v.length - m, ?_, hd⟩
    conv_lhs => rw [← List.take_append_drop m v]
    congr 1
    apply List.ext_getElem?
    intro j
    by_cases hj : j < v.length - m
    · rw [List.getElem?_drop, htail (m + j) (by omega) (by omega)]
      simp [hj]
    · rw [List.getElem?_eq_none (by simp; omega), List.getElem?_eq_none (by simp; omega)]
  · rintro ⟨d, n, rfl, hd⟩
    have hdl : d ≠ [] := hd.1
    have hlen : m = d.length := by
      by_contra hne
      rcases Nat.lt_or_gt_of_ne hne with hlt | hgt
      · -- The last byte of `d` is a digit, but it would be `%`.
        have h1 := htail (d.length - 1) (by omega) (by simp; have := List.length_pos_of_ne_nil hdl; omega)
        have hlt' : d.length - 1 < d.length := by have := List.length_pos_of_ne_nil hdl; omega
        rw [List.getElem?_append_left hlt', List.getElem?_eq_getElem hlt'] at h1
        simp only [Option.some.injEq] at h1
        have := hd.2 _ (List.getElem_mem hlt')
        rw [h1, digit_iff] at this
        exact pct_not_digit this
      · have h1 := hlast (by omega)
        rw [List.getElem?_append_right (by omega)] at h1
        simp only [List.length_append, List.length_replicate] at hm
        rw [List.getElem?_replicate] at h1
        simp only [byte_pct] at h1
        split at h1
        · exact h1 rfl
        · omega
    rw [hlen, List.take_left]
    exact hd

theorem code_blame (sub : List U8) (c : U8) (h : Code sub c) : c = BLAME ↔ sub = bytes "blame" := by
  sub_cases h

theorem code_log (sub : List U8) (c : U8) (h : Code sub c) : c = LOG ↔ sub = bytes "log" := by
  sub_cases h

theorem blame_not_diffs (sub : List U8) (h : sub = bytes "blame") : ¬ Diffs sub := by
  subst h; simp [Diffs, b_diff, b_log, b_show, b_blame]

/-- The trim of `trim_percent` from 2, on the bytes after the first two. -/
theorem score_of_trim (a : List U8) (i : Nat) (h2 : 2 ≤ i) (hi : i ≤ a.length)
    (htail : ∀ j, i ≤ j → j < a.length → a[j]? = some 37#u8)
    (hlast : 2 < i → a[i - 1]? ≠ some 37#u8) :
    Digits ((a.drop 2).take (i - 2)) ↔ Score (a.drop 2) := by
  apply score_iff
  · simp; omega
  · intro j h1 h3
    rw [List.getElem?_drop]
    exact htail (2 + j) (by omega) (by simp at h3; omega)
  · intro hm
    rw [List.getElem?_drop, show 2 + (i - 2 - 1) = i - 1 by omega]
    exact hlast (by omega)

@[step]
theorem score_ok_spec (sub : List U8) (c : U8) (h : Code sub c) (a : Slice U8) (ha : 2 ≤ a.val.length) :
    score_ok c a ⦃ r => (r = true ↔ (Diffs sub ∨ sub = bytes "blame") ∧ Score (a.val.drop 2)) ⦄ := by
  unfold score_ok
  have hb := code_blame sub c h
  step*
  · rw [r_post, score_of_trim a.val i.val i_post1 i_post2 i_post3 i_post4]
    simp_all
  · rw [r_post, score_of_trim a.val i.val i_post1 i_post2 i_post3 i_post4]
    simp_all

theorem glued_short (sub a : List U8) (h : a.length < 3) : ¬ Glued sub a := by
  rintro ⟨f, v, rfl, hv, _⟩
  cases v with
  | nil => exact hv rfl
  | cons _ _ => simp at h; omega

@[step]
theorem glued_ok_spec (sub : List U8) (c : U8) (h : Code sub c) (a : Slice U8)
    (ha : a.val.head? = some 45#u8) :
    glued_ok c a ⦃ r => (r = true ↔ Glued sub a.val) ⦄ := by
  unfold glued_ok
  have hb := code_blame sub c h
  by_cases hl : a.val.length < 3
  · have := glued_short sub a.val hl
    step*
  · obtain ⟨l, hlen⟩ := a
    rcases l with _ | ⟨x, _ | ⟨f, _ | ⟨y, v⟩⟩⟩
    · simp at hl
    · simp at hl
    · simp at hl
    simp only [List.head?_cons, Option.some.injEq] at ha
    subst ha
    step*
    all_goals subst_vars
    all_goals simp_all [Glued, b_diff, b_log, b_show, b_blame]

/-! ## Options -/

def allFlags : List String := diffFlags ++ diffOnly ++ logFlags ++ blameFlags ++ mergeBaseFlags ++ revParseFlags
def allValues : List String := diffValues ++ logValues ++ blameValues ++ revParseValues

theorem flags_sub (sub : List U8) : ∀ x ∈ flags sub, x ∈ allFlags := by
  intro x hx
  unfold flags at hx
  split_ifs at hx <;> simp only [allFlags, List.mem_append] at * <;> tauto

theorem values_sub (sub : List U8) : ∀ x ∈ values sub, x ∈ allValues := by
  intro x hx
  unfold values at hx
  split_ifs at hx <;> simp only [allValues, List.mem_append] at * <;> tauto

set_option maxRecDepth 100000 in
theorem allFlags_no_eq : ∀ x ∈ allFlags, 61#u8 ∉ bytes x := by decide

set_option maxRecDepth 100000 in
theorem allValues_shape : ∀ x ∈ allValues, 61#u8 ∉ bytes x ∧ (bytes x).take 2 = [45#u8, 45#u8] := by decide

/-- `Allowed` for an arg that starts with `-` and is not `--`. -/
def OptAllowed (sub a : List U8) : Prop :=
  a ∈ (flags sub).map bytes ∨
  (∃ n v, a = n ++ byte '=' :: v ∧ n ∈ (values sub).map bytes) ∨
  (sub = bytes "log" ∧ ∃ v, a = byte '-' :: v ∧ Digits v) ∨
  Glued sub a

theorem flags_no_eq (sub x : List U8) (hx : x ∈ (flags sub).map bytes) : 61#u8 ∉ x := by
  obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
  exact allFlags_no_eq y (flags_sub sub y hy)

theorem values_shape (sub n : List U8) (hn : n ∈ (values sub).map bytes) :
    61#u8 ∉ n ∧ n.take 2 = [45#u8, 45#u8] := by
  obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hn
  exact allValues_shape y (values_sub sub y hy)

theorem values_long (sub n : List U8) (hn : n ∈ (values sub).map bytes) : 2 ≤ n.length := by
  have h := (values_shape sub n hn).2
  have := congrArg List.length h
  simp at this; omega

theorem values_second (sub n : List U8) (hn : n ∈ (values sub).map bytes) : n[1]? = some 45#u8 := by
  have h := (values_shape sub n hn).2
  have h1 : (n.take 2)[1]? = some 45#u8 := by rw [h]; rfl
  rw [List.getElem?_take] at h1
  simpa using h1

theorem glued_dash (sub t : List U8) : ¬ Glued sub (45#u8 :: 45#u8 :: t) := by
  rintro ⟨f, v, hfv, _, hc⟩
  simp only [byte_dash, List.cons.injEq] at hfv
  obtain ⟨-, rfl, -⟩ := hfv
  simp at hc

theorem digits_dash (t : List U8) : ¬ Digits (45#u8 :: t) := by
  rintro ⟨-, h⟩
  have := (digit_iff _).mp (h _ List.mem_cons_self)
  simp [isDigit] at this

/-- An option with two dashes and a `=` at `e`, the first one. -/
theorem opt_value (sub t : List U8) (e : Nat) (he : e < t.length + 2)
    (heq : (45#u8 :: 45#u8 :: t)[e]? = some 61#u8)
    (hno : ∀ j, 2 ≤ j → j < e → (45#u8 :: 45#u8 :: t)[j]? ≠ some 61#u8) :
    OptAllowed sub (45#u8 :: 45#u8 :: t) ↔ (45#u8 :: 45#u8 :: t).take e ∈ (values sub).map bytes := by
  set a := 45#u8 :: 45#u8 :: t with ha
  have hmem : 61#u8 ∈ a := List.mem_of_getElem? heq
  constructor
  · rintro (hf | ⟨n, v, hnv, hn⟩ | ⟨-, v, hv, hd⟩ | hg)
    · exact absurd hmem (flags_no_eq sub a hf)
    · have hm := values_long sub n hn
      have hn61 := (values_shape sub n hn).1
      have hat : a[n.length]? = some 61#u8 := by rw [hnv]; simp
      have hlen : n.length = e := by
        by_contra hne
        rcases Nat.lt_or_gt_of_ne hne with hlt | hgt
        · exact hno n.length hm hlt hat
        · apply hn61
          have : a[e]? = n[e]? := by rw [hnv, List.getElem?_append_left hgt]
          rw [heq] at this
          exact List.mem_of_getElem? this.symm
      rw [hnv, ← hlen, List.take_left]
      exact hn
    · simp only [byte_dash, ha, List.cons.injEq] at hv
      obtain ⟨-, rfl⟩ := hv
      exact absurd hd (digits_dash t)
    · exact absurd hg (glued_dash sub t)
  · intro hn
    right; left
    refine ⟨a.take e, a.drop (e + 1), ?_, hn⟩
    have hel : e < a.length := by simp [ha]; omega
    rw [byte_eq]
    conv_lhs => rw [← List.take_append_drop e a]
    congr 1
    rw [List.drop_eq_getElem_cons hel]
    congr 1
    rw [List.getElem?_eq_getElem hel] at heq
    simpa using heq

/-- An option with two dashes and no `=`. -/
theorem opt_flag (sub t : List U8)
    (hno : ∀ j, 2 ≤ j → j < t.length + 2 → (45#u8 :: 45#u8 :: t)[j]? ≠ some 61#u8) :
    OptAllowed sub (45#u8 :: 45#u8 :: t) ↔ (45#u8 :: 45#u8 :: t) ∈ (flags sub).map bytes := by
  set a := 45#u8 :: 45#u8 :: t with ha
  constructor
  · rintro (hf | ⟨n, v, hnv, hn⟩ | ⟨-, v, hv, hd⟩ | hg)
    · exact hf
    · have hm := values_long sub n hn
      have hat : a[n.length]? = some 61#u8 := by rw [hnv]; simp
      have hlt : n.length < t.length + 2 := by
        have := congrArg List.length hnv
        simp [ha] at this; omega
      exact absurd hat (hno n.length hm hlt)
    · simp only [byte_dash, ha, List.cons.injEq] at hv
      obtain ⟨-, rfl⟩ := hv
      exact absurd hd (digits_dash t)
    · exact absurd hg (glued_dash sub t)
  · intro hf; left; exact hf

/-- An option with one dash. -/
theorem opt_short (sub t : List U8) (ht : t.head? ≠ some 45#u8) :
    OptAllowed sub (45#u8 :: t) ↔ (45#u8 :: t) ∈ (flags sub).map bytes ∨
      (sub = bytes "log" ∧ Digits t) ∨ Glued sub (45#u8 :: t) := by
  constructor
  · rintro (hf | ⟨n, v, hnv, hn⟩ | ⟨hs, v, hv, hd⟩ | hg)
    · exact Or.inl hf
    · exfalso
      have h1 := values_second sub n hn
      have hm := values_long sub n hn
      have : (45#u8 :: t)[1]? = some 45#u8 := by
        rw [hnv, List.getElem?_append_left (by omega)]; exact h1
      apply ht
      cases t with
      | nil => simp at this
      | cons y ys => simpa using this
    · simp only [byte_dash, List.cons.injEq, true_and] at hv
      subst hv
      exact Or.inr (Or.inl ⟨hs, hd⟩)
    · exact Or.inr (Or.inr hg)
  · rintro (hf | ⟨hs, hd⟩ | hg)
    · exact Or.inl hf
    · exact Or.inr (Or.inr (Or.inl ⟨hs, t, by simp, hd⟩))
    · exact Or.inr (Or.inr (Or.inr hg))

@[step]
theorem option_ok_spec (sub : List U8) (c : U8) (h : Code sub c) (a : Slice U8)
    (ha : a.val.head? = some 45#u8) :
    option_ok c a ⦃ r => (r = true ↔ OptAllowed sub a.val) ⦄ := by
  have hl := code_log sub c h
  obtain ⟨l, hlen⟩ := a
  rcases l with _ | ⟨x, _ | ⟨y, u⟩⟩
  · simp at ha
  all_goals simp only [List.head?_cons, Option.some.injEq] at ha
  all_goals subst ha
  · -- Just `-`.
    rw [opt_short sub [] (by simp)]
    unfold option_ok
    step*
    all_goals try (with_reducible (show Code _ _); subst_vars; exact h)
    all_goals simp_all
  by_cases hy : y = 45#u8
  · -- Two dashes.
    subst hy
    unfold option_ok
    step*
    all_goals try (with_reducible (show Code _ _); subst_vars; exact h)
    all_goals try (simp_all; done)
    · have hlt : e.val < u.length + 2 := by simp [Slice.len] at *; scalar_tac
      rw [r_post, s_post1, opt_value sub u e.val hlt (e_post4 (by simp; omega)) e_post3]
      simp [List.slice]
    · have hge : u.length + 2 ≤ e.val := by simp [Slice.len] at *; scalar_tac
      rw [r_post, opt_flag sub u (fun j h1 h2 => e_post3 j h1 (by omega))]
  · -- One dash.
    rw [opt_short sub (y :: u) (by simpa using hy)]
    unfold option_ok
    step*
    all_goals try (with_reducible (show Code _ _); subst_vars; exact h)
    all_goals simp_all

/-! ## One arg -/

theorem allowed_opt (sub a : List U8) (ha : a.head? = some 45#u8) (hd : a ≠ [45#u8, 45#u8]) :
    Allowed sub a ↔ OptAllowed sub a := by
  unfold Allowed OptAllowed
  simp only [bytes_dashdash, byte_dash, ha, ne_eq, not_true_eq_false, or_false, hd, false_or]

theorem allowed_path (sub a : List U8) (ha : a.head? ≠ some 45#u8) : Allowed sub a := by
  unfold Allowed
  right; right; right; right; right
  simpa using ha

@[step]
theorem refuse_spec (sub : List U8) (c : U8) (h : Code sub c) (a : Slice U8) :
    refuse c a ⦃ r => (r = 0#u8 ↔ Good sub a.val) ⦄ := by
  have hE : EMPTY = 2#u8 := by simp [global_simps]
  have hC : CONTROL = 3#u8 := by simp [global_simps]
  have hO : OPTION = 4#u8 := by simp [global_simps]
  have hP : PATH = 5#u8 := by simp [global_simps]
  unfold refuse
  obtain ⟨l, hlen⟩ := a
  rcases l with _ | ⟨x, t⟩
  · step*; simp_all [Good]
  by_cases hx : x = 45#u8
  · subst hx
    by_cases hd : t = [45#u8]
    · subst hd
      have hA : Allowed sub [45#u8, 45#u8] := by left; simp
      step*
      all_goals simp_all [Good, Slice.len, Array.repeat, List.replicate]
    · have hA := allowed_opt sub (45#u8 :: t) rfl (by simpa using hd)
      step*
      all_goals simp_all [Good, Slice.len, Array.repeat, List.replicate]
  · have hA := allowed_path sub (x :: t) (by simpa using hx)
    step*
    all_goals simp_all [Good, PathSafe, Slice.len, Array.repeat, List.replicate]

/-! ## The whole call -/

@[simp] theorem deref_val (v : alloc.vec.Vec U8) : (alloc.vec.Vec.deref v).val = v.val := rfl

theorem first_refused_loop_spec (sub : List U8) (c : U8) (h : Code sub c) (args : Slice (alloc.vec.Vec U8))
    (k : Usize) (hk : k.val ≤ args.val.length) (hall : ∀ j < k.val, ∀ hj : j < args.val.length, Good sub (args.val[j]'hj).val) :
    first_refused_loop c args k ⦃ r => (r.1 = 0#u8 ↔ ∀ a ∈ args.val, Good sub a.val) ⦄ := by
  unfold first_refused_loop
  apply loop.spec_decr_nat (fun k => args.val.length - k.val)
    (fun k => k.val ≤ args.val.length ∧ ∀ j < k.val, ∀ hj : j < args.val.length, Good sub (args.val[j]'hj).val) _ _ _ _
    ⟨hk, hall⟩
  rintro k ⟨hk, hall⟩
  unfold first_refused_loop.body
  step*
  · have hne : why ≠ 0#u8 := by simpa using (by assumption : (why != 0#u8) = true)
    simp only [hne, false_iff, not_forall]
    have hlt : k.val < args.val.length := by scalar_tac
    refine ⟨args.val[k.val], List.getElem_mem hlt, fun hg => hne (why_post.mpr ?_)⟩
    simpa [v_post] using hg
  · have heq : why = 0#u8 := by
      have h0 : ¬(why != 0#u8) = true := by assumption
      apply UScalar.eq_of_val_eq
      simpa using h0
    refine ⟨by scalar_tac, fun j hj hjl => ?_, by scalar_tac⟩
    by_cases hjk : j < k.val
    · exact hall j hjk hjl
    · have : j = k.val := by scalar_tac
      subst this
      have := why_post.mp heq
      simpa [v_post] using this
  · simp only [true_iff]
    intro a ha
    obtain ⟨j, hj, rfl⟩ := List.getElem_of_mem ha
    exact hall j (by scalar_tac) hj

@[step]
theorem first_refused_spec (sub : List U8) (c : U8) (h : Code sub c) (args : Slice (alloc.vec.Vec U8)) :
    first_refused c args ⦃ r => (r.why = 0#u8 ↔ ∀ a ∈ args.val, Good sub a.val) ⦄ := by
  unfold first_refused
  have := first_refused_loop_spec sub c h args 0#usize (by simp) (fun j hj => absurd hj (by simp))
  step*

@[step]
theorem clone_spec (v : alloc.vec.Vec U8) : alloc.vec.CloneVec.clone core.clone.CloneU8 v ⦃ v' => v' = v ⦄ := by
  unfold alloc.vec.CloneVec.clone
  apply WP.spec_mono (Slice.clone_spec (fun x _ => rfl))
  intro r hr; exact hr.symm

theorem push_all_loop_spec (args : Slice (alloc.vec.Vec U8)) (out : alloc.vec.Vec (alloc.vec.Vec U8)) (k : Usize)
    (hk : k.val ≤ args.val.length) (hsize : out.val.length + (args.val.length - k.val) ≤ Usize.max) :
    push_all_loop out args k ⦃ out' => out'.val = out.val ++ args.val.drop k.val ⦄ := by
  unfold push_all_loop
  apply loop.spec_decr_nat (fun (st : alloc.vec.Vec (alloc.vec.Vec U8) × Usize) => args.val.length - st.2.val)
    (fun (st : alloc.vec.Vec (alloc.vec.Vec U8) × Usize) => st.2.val ≤ args.val.length ∧
      st.1.val.length + (args.val.length - st.2.val) ≤ Usize.max ∧
      st.1.val ++ args.val.drop st.2.val = out.val ++ args.val.drop k.val) _ _ _ _
    ⟨hk, hsize, rfl⟩
  rintro ⟨o, j⟩ ⟨hj, hs, he⟩
  simp only at hj hs he
  unfold push_all_loop.body
  step*
  · have hlt : j.val < args.val.length := by scalar_tac
    refine ⟨by scalar_tac, by simp [out1_post]; scalar_tac, ?_, by scalar_tac⟩
    rw [out1_post, v1_post, v_post, ← he, List.drop_eq_getElem_cons hlt, k1_post]
    simp
  · have : args.val.drop j.val = [] := List.drop_eq_nil_of_le (by scalar_tac)
    rw [← he, this, List.append_nil]

@[step]
theorem push_all_spec (args : Slice (alloc.vec.Vec U8)) (out : alloc.vec.Vec (alloc.vec.Vec U8))
    (hsize : out.val.length + args.val.length ≤ Usize.max) :
    push_all out args ⦃ out' => out'.val = out.val ++ args.val ⦄ := by
  unfold push_all
  apply WP.spec_mono (push_all_loop_spec args out 0#usize (by simp) (by simpa using hsize))
  intro r hr; simpa using hr

theorem b_no_ext_diff : bytes "--no-ext-diff" =
    [45#u8, 45#u8, 110#u8, 111#u8, 45#u8, 101#u8, 120#u8, 116#u8, 45#u8, 100#u8, 105#u8, 102#u8, 102#u8] := by decide
theorem b_no_textconv : bytes "--no-textconv" =
    [45#u8, 45#u8, 110#u8, 111#u8, 45#u8, 116#u8, 101#u8, 120#u8, 116#u8, 99#u8, 111#u8, 110#u8, 118#u8] := by decide

@[step]
theorem prefix_spec (sub : List U8) (c : U8) (h : Code sub c) :
    «prefix» c ⦃ out => out.val.map (·.val) = pre sub ∧ out.val.length ≤ 2 ⦄ := by
  unfold «prefix»
  have hb := code_blame sub c h
  have hnd := blame_not_diffs sub
  step*
  all_goals simp_all [pre, b_no_ext_diff, b_no_textconv]

theorem code_sub (sub : List U8) (c : U8) (h : Code sub c) : c = NO_SUB ↔ sub ∉ subs.map bytes := by
  rw [subs_bytes]
  sub_cases h

/-- `check` never panics, and it returns `Ok` exactly for a call the spec allows, with
the prefix options first and then the args unchanged. -/
theorem check_spec (sub : Slice U8) (args : Slice (alloc.vec.Vec U8)) (hsize : args.val.length + 2 ≤ Usize.max) :
    check sub args ⦃ r => match r with
      | .Ok out => CallOk sub.val (args.val.map (·.val)) ∧
          out.val.map (·.val) = pre sub.val ++ args.val.map (·.val)
      | .Err _ => ¬ CallOk sub.val (args.val.map (·.val)) ⦄ := by
  unfold check
  step*
  · have := (code_sub _ _ code_post).mp (by assumption)
    exact fun hc => this hc.1
  · have hne : r.why ≠ 0#u8 := by
      have h0 : (r.why != 0#u8) = true := by assumption
      simpa using h0
    intro hc
    apply hne
    rw [r_post]
    intro a ha
    exact hc.2 a.val (List.mem_map_of_mem ha)
  · have hz : r.why = 0#u8 := by
      have h0 : ¬(r.why != 0#u8) = true := by assumption
      apply UScalar.eq_of_val_eq
      simpa using h0
    have hs : ↑sub ∈ subs.map bytes := by
      by_contra hn
      exact (by assumption : ¬code = NO_SUB) ((code_sub _ _ code_post).mpr hn)
    refine ⟨⟨hs, ?_⟩, ?_⟩
    · intro a ha
      obtain ⟨v, hv, rfl⟩ := List.mem_map.mp ha
      exact r_post.mp hz v hv
    · rw [out1_post, List.map_append, out_post1]

end GitArgCheck.Check
