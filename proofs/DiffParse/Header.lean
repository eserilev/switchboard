import DiffParse.Spec

/-! # Hunk headers: no panic, and a header too big to count from is refused -/

open Aeneas Aeneas.Std Result diffparse DiffParse.Spec

namespace DiffParse.Header

theorem MAX_val : diffparse.MAX.val = 4294967295 := by
  unfold diffparse.MAX; rfl

/-- A digit at index `j`. -/
def DigitAt (l : List U8) (j : Nat) : Prop := ∃ x, l[j]? = some x ∧ IsDigit x

/-- `l[k..e)` are digits, and `l[e]` is not a digit (or not there). -/
def Run (l : List U8) (k e : Nat) : Prop :=
  k ≤ e ∧ (∀ j, k ≤ j → j < e → DigitAt l j) ∧ ¬ DigitAt l e

/-- The bytes `l[a..b)`. -/
def seg (l : List U8) (a b : Nat) : List U8 := (l.drop a).take (b - a)

theorem run_unique (l : List U8) (k e1 e2 : Nat) (h1 : Run l k e1) (h2 : Run l k e2) : e1 = e2 := by
  obtain ⟨a1, b1, c1⟩ := h1
  obtain ⟨a2, b2, c2⟩ := h2
  by_contra hne
  rcases Nat.lt_or_gt_of_ne hne with h | h
  · exact c1 (b2 e1 a1 h)
  · exact c2 (b1 e2 a2 h)

theorem dval_append (a b : List U8) : dval (a ++ b) = b.foldl (fun n x => 10 * n + (x.val - 48)) (dval a) := by
  simp [dval, List.foldl_append]

theorem foldl_ge (b : List U8) : ∀ n, n ≤ b.foldl (fun n x => 10 * n + (x.val - 48)) n := by
  induction b with
  | nil => intro n; simp
  | cons x t ih =>
    intro n
    simp only [List.foldl_cons]
    have := ih (10 * n + (x.val - 48))
    omega

theorem dval_le_append (a b : List U8) : dval a ≤ dval (a ++ b) := by
  rw [dval_append]; exact foldl_ge b _

theorem dval_snoc (a : List U8) (x : U8) : dval (a ++ [x]) = 10 * dval a + (x.val - 48) := by
  simp [dval, List.foldl_append]

theorem seg_succ (l : List U8) (a i : Nat) (hai : a ≤ i) (hi : i < l.length) :
    seg l a (i + 1) = seg l a i ++ [l[i]] := by
  unfold seg
  have : i + 1 - a = (i - a) + 1 := by omega
  rw [this, List.take_add_one]
  congr
  rw [List.getElem?_drop]
  have : a + (i - a) = i := by omega
  rw [this, List.getElem?_eq_getElem hi]
  rfl

theorem seg_split (l : List U8) (a i b : Nat) (hai : a ≤ i) (hib : i ≤ b) :
    seg l a b = seg l a i ++ seg l i b := by
  unfold seg
  have hb : b - a = (i - a) + (b - i) := by omega
  rw [hb, List.take_add, List.drop_drop]
  have e1 : a + (i - a) = i := by omega
  have e2 : i - a + a = i := by omega
  simp only [e1]

theorem seg_self (l : List U8) (a : Nat) : seg l a a = [] := by simp [seg]

@[step]
theorem is_digit_spec (x : U8) : is_digit x ⦃ r => (r = true ↔ IsDigit x) ⦄ := by
  unfold is_digit IsDigit
  step*

theorem digits_end_loop_spec (h : Slice U8) (k i : Usize) (hki : k.val ≤ i.val) (hi : i.val ≤ h.val.length)
    (hpre : ∀ j, k.val ≤ j → j < i.val → DigitAt h.val j) :
    digits_end_loop h i ⦃ e => e.val ≤ h.val.length ∧ Run h.val k.val e.val ⦄ := by
  unfold digits_end_loop
  apply loop.spec_decr_nat (fun i => h.val.length - i.val)
    (fun i => k.val ≤ i.val ∧ i.val ≤ h.val.length ∧ ∀ j, k.val ≤ j → j < i.val → DigitAt h.val j) _ _ _ _
    ⟨hki, hi, hpre⟩
  rintro i ⟨hki, hi, hpre⟩
  unfold digits_end_loop.body
  step*
  · refine ⟨by scalar_tac, by scalar_tac, ?_, by scalar_tac⟩
    intro j hj1 hj2
    by_cases hj : j < i.val
    · exact hpre j hj1 hj
    · have : j = i.val := by scalar_tac
      subst this
      exact ⟨_, List.getElem?_eq_getElem (by scalar_tac), by rw [← i2_post]; exact b_post.mp (by assumption)⟩
  · refine ⟨by scalar_tac, hki, hpre, ?_⟩
    rintro ⟨x, hx, hd⟩
    rw [List.getElem?_eq_getElem (by scalar_tac)] at hx
    simp only [Option.some.injEq] at hx
    have : b = true := b_post.mpr (by rw [i2_post, hx]; exact hd)
    simp_all
  · refine ⟨by scalar_tac, hki, hpre, ?_⟩
    rintro ⟨x, hx, -⟩
    rw [List.getElem?_eq_none (by scalar_tac)] at hx
    cases hx

@[step]
theorem digits_end_spec (h : Slice U8) (k : Usize) (hk : k.val ≤ h.val.length) :
    digits_end h k ⦃ e => e.val ≤ h.val.length ∧ Run h.val k.val e.val ⦄ :=
  digits_end_loop_spec h k k (le_refl _) hk (fun j h1 h2 => absurd h2 (by omega))

/-- The state of the `value` loop. -/
def VInv (h : List U8) (fr t : Nat) (st : U32 × Bool × Usize) : Prop :=
  fr ≤ st.2.2.val ∧ st.2.2.val ≤ t ∧
  (st.2.1 = false → st.1.val = dval (seg h fr st.2.2.val)) ∧
  (st.2.1 = true → st.2.2.val = t ∧ U32.max < dval (seg h fr t))

set_option maxRecDepth 20000 in
theorem value_loop_spec (h : Slice U8) (fr t : Usize) (n : U32) (big : Bool) (i : Usize)
    (hto : t.val ≤ h.val.length) (hdig : ∀ j, fr.val ≤ j → j < t.val → DigitAt h.val j)
    (hinv : VInv h.val fr.val t.val (n, big, i)) :
    value_loop h t n big i ⦃ r => (r.2 = false → r.1.val = dval (seg h.val fr.val t.val)) ∧
      (r.2 = true → U32.max < dval (seg h.val fr.val t.val)) ⦄ := by
  unfold value_loop
  apply loop.spec_decr_nat (fun (st : U32 × Bool × Usize) => t.val - st.2.2.val)
    (VInv h.val fr.val t.val) _ _ _ _ hinv
  rintro ⟨n, big, i⟩ ⟨h1, h2, h3, h4⟩
  simp only at h1 h2 h3 h4
  unfold value_loop.body
  have hM := MAX_val
  have hdi : ∀ (hi : i.val < h.val.length), i.val < t.val → IsDigit (h.val[i.val]'hi) := by
    intro hi hit
    obtain ⟨x, hx, hd⟩ := hdig i.val h1 hit
    rw [List.getElem?_eq_getElem hi] at hx
    simp only [Option.some.injEq] at hx
    rw [hx]; exact hd
  step*
  · rw [i1_post]; exact (hdi (by scalar_tac) (by scalar_tac)).1
  · -- Too big: the number of the digits so far is over `MAX`, and so is the full number.
    have hbig : big = false := by
      cases big with
      | false => rfl
      | true => exfalso; have := (h4 rfl).1; scalar_tac
    have hn := h3 hbig
    have hd : d.val = i1.val - 48 := by
      rw [d_post, UScalar.cast_val_mod_pow_greater_numBits_eq _ _ (by simp), i2_post1]
    have hs := seg_succ h.val fr.val i.val h1 (by scalar_tac)
    have hsplit := seg_split h.val fr.val (i.val + 1) t.val (by omega) (by scalar_tac)
    have hle := dval_le_append (seg h.val fr.val (i.val + 1)) (seg h.val (i.val + 1) t.val)
    rw [← hsplit, hs, dval_snoc, ← hn] at hle
    have : (h.val[i.val]'(by scalar_tac)) = i1 := i1_post.symm
    rw [this] at hle
    have hlt : i4.val < n.val := ‹n > i4›
    have hmax := U32.max_eq
    have hit : i.val < t.val := ‹i < t›
    unfold VInv; dsimp only
    refine ⟨⟨by omega, le_refl _, (fun h => nomatch h), fun _ => ⟨rfl, ?_⟩⟩, by omega⟩
    omega
  · have hbig : big = false := by
      cases big with
      | false => rfl
      | true => exfalso; have := (h4 rfl).1; scalar_tac
    have hn := h3 hbig
    have hd : d.val = i1.val - 48 := by
      rw [d_post, UScalar.cast_val_mod_pow_greater_numBits_eq _ _ (by simp), i2_post1]
    have hs := seg_succ h.val fr.val i.val h1 (by scalar_tac)
    have : (h.val[i.val]'(by scalar_tac)) = i1 := i1_post.symm
    rw [this] at hs
    have hit : i.val < t.val := ‹i < t›
    unfold VInv; dsimp only
    refine ⟨⟨by omega, by omega, fun _ => ?_, fun h => by rw [hbig] at h; cases h⟩, by omega⟩
    rw [i6_post, hs, dval_snoc, ← hn, n1_post, i5_post, hd]
    ring

@[step]
theorem value_spec (h : Slice U8) (fr t : Usize) (hft : fr.val ≤ t.val) (hto : t.val ≤ h.val.length)
    (hdig : ∀ j, fr.val ≤ j → j < t.val → DigitAt h.val j) :
    value h fr t ⦃ r => match r with
      | none => U32.max < dval (seg h.val fr.val t.val)
      | some n => n.val = dval (seg h.val fr.val t.val) ⦄ := by
  unfold value
  have hl := value_loop_spec h fr t 0#u32 false fr hto hdig
    ⟨le_refl _, hft, fun _ => by simp [seg_self, dval], fun h => nomatch h⟩
  apply WP.spec_bind hl
  rintro ⟨n, big⟩ ⟨h1, h2⟩
  cases big
  · exact h1 rfl
  · exact h2 rfl

@[step]
theorem number_spec (h : Slice U8) (k : Usize) (hk : k.val ≤ h.val.length) :
    number h k ⦃ r => ∃ e, e ≤ h.val.length ∧ Run h.val k.val e ∧ match r with
      | none => e = k.val ∨ U32.max < dval (seg h.val k.val e)
      | some p => p.2.val = e ∧ k.val < e ∧ p.1.val = dval (seg h.val k.val e) ⦄ := by
  unfold number
  step*
  rw [‹o = some n›] at o_post
  refine ⟨_, end_post1, end_post2, rfl, ?_, o_post⟩
  have := end_post2.1
  have : «end».val ≠ k.val := fun h => ‹¬«end» = k› (UScalar.eq_of_val_eq h)
  omega

theorem getElem?_of_drop (l : List U8) (k : Nat) (x : U8) (r : List U8) (h : l.drop k = x :: r) :
    l[k]? = some x := by
  have := congrArg (·[0]?) h
  simpa [List.getElem?_drop] using this

theorem run_of_text (l : List U8) (k : Nat) (ds rest : List U8) (hd : l.drop k = ds ++ rest)
    (hds : ∀ x ∈ ds, IsDigit x) (hr : ∀ x, rest.head? = some x → ¬IsDigit x) :
    Run l k (k + ds.length) := by
  refine ⟨by omega, ?_, ?_⟩
  · intro j h1 h2
    have : l[j]? = ds[j - k]? := by
      have := congrArg (·[j - k]?) hd
      simp only [List.getElem?_drop] at this
      rw [show k + (j - k) = j by omega] at this
      rw [this, List.getElem?_append_left (by omega)]
    rw [List.getElem?_eq_getElem (by omega : j - k < ds.length)] at this
    exact ⟨_, this, hds _ (List.getElem_mem _)⟩
  · rintro ⟨x, hx, hdx⟩
    have := congrArg (·[ds.length]?) hd
    simp only [List.getElem?_drop, List.getElem?_append_right (le_refl _), Nat.sub_self] at this
    rw [hx] at this
    exact hr x (by rw [List.head?_eq_getElem?]; exact this.symm) hdx

theorem rest_not_digit (rest : List U8) (h : rest = [] ∨ rest.head? = some 32#u8) :
    ∀ x, rest.head? = some x → ¬IsDigit x := by
  intro x hx
  rcases h with h | h
  · subst h; simp at hx
  · rw [h] at hx; simp only [Option.some.injEq] at hx; subst hx; simp [IsDigit]

theorem seg_of_text (l : List U8) (k : Nat) (ds rest : List U8) (hd : l.drop k = ds ++ rest) :
    seg l k (k + ds.length) = ds := by
  simp [seg, hd]

/-- What follows the start of a range. -/
def tailOf (cs : Option (List U8)) (rest : List U8) : List U8 :=
  match cs with
  | none => rest
  | some c => 44#u8 :: (c ++ rest)

/-- `l[k..]` is a range `ds[,cs]`, and then the end or a space. -/
def RangeHyp (l : List U8) (k : Nat) (ds : List U8) (cs : Option (List U8)) (rest : List U8) : Prop :=
  l.drop k = rangeText ds cs ++ rest ∧ Digits ds ∧ (∀ c, cs = some c → Digits c) ∧
  (rest = [] ∨ rest.head? = some 32#u8)

@[step]
theorem range_spec (h : Slice U8) (k : Usize) (hk : k.val ≤ h.val.length) :
    range h k ⦃ r => (∀ s en a, r = some (s, en, a) → s.val ≤ en.val ∧ k.val < a.val ∧ a.val ≤ h.val.length) ∧
      (∀ ds cs rest, RangeHyp h.val k.val ds cs rest → match r with
        | none => U32.max < dval ds + count cs
        | some (s, en, a) => s.val = dval ds ∧ en.val = dval ds + count cs ∧
            a.val = k.val + (rangeText ds cs).length) ⦄ := by
  unfold range
  have hM := MAX_val
  have hmax := U32.max_eq
  apply WP.spec_bind (number_spec h k hk)
  rintro o ⟨e, he1, he2, he3⟩
  -- On a range text, the digits end after `ds`.
  have htext : ∀ ds cs rest, RangeHyp h.val k.val ds cs rest →
      e = k.val + ds.length ∧ seg h.val k.val e = ds ∧
      (h.val[k.val + ds.length]? = some 44#u8 ↔ cs.isSome) ∧
      (∀ c, cs = some c → h.val.drop (k.val + ds.length + 1) = c ++ rest) := by
    intro ds cs rest ⟨hd, hds, hcs, hrest⟩
    have hd' : h.val.drop k.val = ds ++ tailOf cs rest := by
      rw [hd]; cases cs <;> simp [rangeText, tailOf]
    have hR : ∀ x, (tailOf cs rest).head? = some x → ¬IsDigit x := by
      intro x hx
      unfold tailOf at hx
      cases cs with
      | none =>
        rcases hrest with hr | hr
        · subst hr; simp at hx
        · simp only at hx; rw [hr] at hx; simp only [Option.some.injEq] at hx; subst hx
          simp [IsDigit]
      | some c => simp only [List.head?_cons, Option.some.injEq] at hx; subst hx; simp [IsDigit]
    have hrun := run_of_text h.val k.val ds _ hd' hds.2 hR
    have he := run_unique _ _ _ _ he2 hrun
    refine ⟨he, by rw [he]; exact seg_of_text _ _ _ _ hd', ?_, ?_⟩
    · have := congrArg (·[ds.length]?) hd'
      simp only [List.getElem?_drop, List.getElem?_append_right (le_refl _), Nat.sub_self] at this
      rw [this]
      unfold tailOf
      cases cs with
      | none =>
        rcases hrest with hr | hr
        · subst hr; simp
        · simp only [Option.isSome_none, Bool.false_eq_true, iff_false]
          rw [← List.head?_eq_getElem?, hr]; simp only [Option.some.injEq]; decide
      | some c => simp
    · intro c hc
      subst hc
      rw [show k.val + ds.length + 1 = (k.val + ds.length) + 1 by rfl, ← List.drop_drop, ← List.drop_drop, hd']
      simp [tailOf]
  cases o with
  | none =>
    simp only [WP.spec_ok]
    refine ⟨by simp, ?_⟩
    intro ds cs rest hr
    obtain ⟨he, hs, -, -⟩ := htext ds cs rest hr
    have hne : ds ≠ [] := hr.2.1.1
    rcases he3 with h0 | h0
    · have : ds.length = 0 := by omega
      simp_all
    · rw [hs] at h0
      cases cs <;> simp only [count] <;> omega
  | some p =>
    obtain ⟨start, next⟩ := p
    obtain ⟨hn, hkn, hv⟩ := he3
    simp only at hn hv ⊢
    -- The comma is there exactly for a range with a count.
    have hcomma : ∀ ds cs rest, RangeHyp h.val k.val ds cs rest →
        ((next.val < h.val.length ∧ h.val[next.val]? = some 44#u8) ↔ cs.isSome) := by
      intro ds cs rest hr
      obtain ⟨he, -, hc, -⟩ := htext ds cs rest hr
      rw [hn, he, ← hc]
      constructor
      · exact fun h => h.2
      · intro h'
        refine ⟨?_, h'⟩
        by_contra hc'
        rw [List.getElem?_eq_none (by omega)] at h'
        cases h'
    -- With no comma, the range has no count.
    have hnone : ∀ ds cs rest, RangeHyp h.val k.val ds cs rest →
        ¬(next.val < h.val.length ∧ h.val[next.val]? = some 44#u8) →
        cs = none ∧ start.val = dval ds ∧ next.val = k.val + ds.length := by
      intro ds cs rest hr hno
      obtain ⟨he, hs, -, -⟩ := htext ds cs rest hr
      refine ⟨?_, by rw [hv, ← hs], by rw [hn, he]⟩
      have := mt (hcomma ds cs rest hr).mpr hno
      cases cs <;> simp_all
    step*
    · -- `,` and then no count: refused, and the text has no number there either.
      refine ⟨by simp, ?_⟩
      intro ds cs rest hr
      obtain ⟨he, hs, -, hdrop⟩ := htext ds cs rest hr
      have h44 : next.val < h.val.length ∧ h.val[next.val]? = some 44#u8 := by
        refine ⟨by scalar_tac, ?_⟩
        rw [List.getElem?_eq_getElem (by scalar_tac), ← i1_post, ‹i1 = 44#u8›]
      obtain ⟨c, rfl⟩ := Option.isSome_iff_exists.mp ((hcomma ds cs rest hr).mp h44)
      have hd2 := hdrop c rfl
      have hc := hr.2.2.1 c rfl
      have hrun := run_of_text h.val (k.val + ds.length + 1) c rest hd2 hc.2 (rest_not_digit rest hr.2.2.2)
      have hi2 : i2.val = k.val + ds.length + 1 := by scalar_tac
      rw [hi2] at o1_post2 o1_post3
      have := run_unique _ _ _ _ o1_post2 hrun
      subst this
      rw [seg_of_text _ _ _ _ hd2] at o1_post3
      simp only [‹_ = none›] at o1_post3
      simp only [count]
      rcases o1_post3 with h0 | h0
      · have : c.length = 0 := by omega
        exact absurd (List.length_eq_zero_iff.mp this) hc.1
      · omega
    · obtain ⟨cv, a⟩ := p1
      simp only [‹_ = some (cv, a)›] at o1_post3
      obtain ⟨ha, hia, hcv⟩ := o1_post3
      have hsome : ∀ ds cs rest, RangeHyp h.val k.val ds cs rest → ∃ c, cs = some c ∧
          cv.val = dval c ∧ a.val = k.val + ds.length + 1 + c.length ∧ start.val = dval ds := by
        intro ds cs rest hr
        obtain ⟨he, hs, -, hdrop⟩ := htext ds cs rest hr
        have h44 : next.val < h.val.length ∧ h.val[next.val]? = some 44#u8 := by
          refine ⟨by scalar_tac, ?_⟩
          rw [List.getElem?_eq_getElem (by scalar_tac), ← i1_post, ‹i1 = 44#u8›]
        obtain ⟨c, rfl⟩ := Option.isSome_iff_exists.mp ((hcomma ds cs rest hr).mp h44)
        have hd2 := hdrop c rfl
        have hc := hr.2.2.1 c rfl
        have hrun := run_of_text h.val (k.val + ds.length + 1) c rest hd2 hc.2 (rest_not_digit rest hr.2.2.2)
        have hi2 : i2.val = k.val + ds.length + 1 := by scalar_tac
        rw [hi2] at o1_post2 hcv
        have := run_unique _ _ _ _ o1_post2 hrun
        subst this
        rw [seg_of_text _ _ _ _ hd2] at hcv
        exact ⟨c, rfl, hcv, ha, by rw [hv, ← hs, he]⟩
      step*
      · refine ⟨by simp, ?_⟩
        intro ds cs rest hr
        obtain ⟨c, rfl, h1, -, h3⟩ := hsome ds cs rest hr
        simp only [count]
        scalar_tac
      · refine ⟨?_, ?_⟩
        · intro s en a' heq
          simp only [Option.some.injEq, Prod.mk.injEq] at heq
          obtain ⟨rfl, rfl, rfl⟩ := heq
          refine ⟨by scalar_tac, by scalar_tac, by scalar_tac⟩
        · intro ds cs rest hr
          obtain ⟨c, rfl, h1, h2, h3⟩ := hsome ds cs rest hr
          simp only [count, rangeText, List.length_append, List.length_cons]
          refine ⟨h3, by scalar_tac, by omega⟩
    all_goals (have hno : ¬(next.val < h.val.length ∧ h.val[next.val]? = some 44#u8) := fun ⟨hl, hx⟩ => by first | exact ‹¬next < h.len› (by scalar_tac) | (rw [List.getElem?_eq_getElem hl, ← i1_post] at hx; exact ‹¬i1 = 44#u8› (by simpa using hx)))
    all_goals first
      | (refine ⟨by simp, ?_⟩
         intro ds cs rest hr
         obtain ⟨rfl, h2, h3⟩ := hnone ds cs rest hr hno
         simp only [count]
         have : i2.val = 0 := by scalar_tac
         omega)
      | (refine ⟨?_, ?_⟩
         · intro s en a' heq
           simp only [Option.some.injEq, Prod.mk.injEq] at heq
           obtain ⟨rfl, rfl, rfl⟩ := heq
           refine ⟨by scalar_tac, by scalar_tac, by scalar_tac⟩
         · intro ds cs rest hr
           obtain ⟨rfl, h2, h3⟩ := hnone ds cs rest hr hno
           simp only [count, rangeText]
           refine ⟨h2, by scalar_tac, h3⟩)

/-- `l` is the text of a hunk header after `@@ `, then the end or a space. -/
def HeaderHyp (l : List U8) (a : List U8) (b : Option (List U8)) (c : List U8) (d : Option (List U8))
    (tail : List U8) : Prop :=
  l = headerText a b c d ++ tail ∧ Digits a ∧ (∀ x, b = some x → Digits x) ∧ Digits c ∧
  (∀ x, d = some x → Digits x) ∧ (tail = [] ∨ tail.head? = some 32#u8)

theorem header_text_facts (l a : List U8) (b : Option (List U8)) (c : List U8) (d : Option (List U8))
    (tail : List U8) (hh : HeaderHyp l a b c d tail) :
    l[0]? = some 45#u8 ∧
    RangeHyp l 1 a b (32#u8 :: 43#u8 :: (rangeText c d ++ tail)) ∧
    l[1 + (rangeText a b).length]? = some 32#u8 ∧
    l[1 + (rangeText a b).length + 1]? = some 43#u8 ∧
    1 + (rangeText a b).length + 2 ≤ l.length ∧
    RangeHyp l (1 + (rangeText a b).length + 2) c d tail ∧
    (1 + (rangeText a b).length + 2 + (rangeText c d).length < l.length →
      l[1 + (rangeText a b).length + 2 + (rangeText c d).length]? = some 32#u8) := by
  obtain ⟨hl, ha, hb, hc, hd, ht⟩ := hh
  have hl' : l = 45#u8 :: (rangeText a b ++ 32#u8 :: 43#u8 :: (rangeText c d ++ tail)) := by
    rw [hl]; simp [headerText]
  have h1 : l.drop 1 = rangeText a b ++ 32#u8 :: 43#u8 :: (rangeText c d ++ tail) := by
    rw [hl']; simp
  have hK : l.drop (1 + (rangeText a b).length) = 32#u8 :: 43#u8 :: (rangeText c d ++ tail) := by
    rw [← List.drop_drop, h1]; simp
  have hK1 : l.drop (1 + (rangeText a b).length + 1) = 43#u8 :: (rangeText c d ++ tail) := by
    rw [← List.drop_drop, hK]; simp
  have hK2 : l.drop (1 + (rangeText a b).length + 2) = rangeText c d ++ tail := by
    rw [← List.drop_drop, hK]; simp
  have hJ : l.drop (1 + (rangeText a b).length + 2 + (rangeText c d).length) = tail := by
    rw [← List.drop_drop, hK2]; simp
  refine ⟨by rw [hl']; simp, ⟨h1, ha, hb, Or.inr (by simp)⟩, getElem?_of_drop _ _ _ _ hK,
    getElem?_of_drop _ _ _ _ hK1, ?_, ⟨by rw [hK2], hc, hd, ht⟩, ?_⟩
  · have := congrArg List.length hK
    simp only [List.length_drop, List.length_cons] at this
    omega
  · intro hlt
    rcases ht with ht | ht
    · subst ht
      have := congrArg List.length hJ
      simp only [List.length_drop, List.length_nil] at this
      omega
    · have := congrArg (·[0]?) hJ
      simp only [List.getElem?_drop, Nat.add_zero] at this
      rw [this, ← List.head?_eq_getElem?, ht]

theorem header_spec (h : Slice U8) :
    header h ⦃ r => (∀ ctr, r = some ctr → ctr.old.val ≤ ctr.old_end.val ∧ ctr.new.val ≤ ctr.new_end.val) ∧
      (∀ a b c d tail, HeaderHyp h.val a b c d tail → match r with
        | none => U32.max < dval a + count b ∨ U32.max < dval c + count d
        | some ctr => ctr.old.val = dval a ∧ ctr.old_end.val = dval a + count b ∧
            ctr.new.val = dval c ∧ ctr.new_end.val = dval c + count d) ⦄ := by
  unfold header
  have hmax := U32.max_eq
  step*
  · refine ⟨by simp, ?_⟩
    intro ra rb rc rd tail hh
    exfalso
    have h0 := (header_text_facts _ _ _ _ _ _ hh).1
    have h1 : h.length = 0 := b_post ▸ ‹b = true›
    have : h.val = [] := by simpa [Slice.length] using h1
    rw [this] at h0
    cases h0
  · refine ⟨by simp, ?_⟩
    intro ra rb rc rd tail hh
    exfalso
    have hne : i ≠ 45#u8 := by simpa using ‹(i != 45#u8) = true›
    have h0 := (header_text_facts _ _ _ _ _ _ hh).1
    rw [List.getElem?_eq_getElem (by scalar_tac)] at h0
    simp only [Option.some.injEq] at h0
    exact hne (i_post.trans h0)
  · refine ⟨by simp, ?_⟩
    intro ra rb rc rd tail hh
    have := o_post2 _ _ _ (header_text_facts _ _ _ _ _ _ hh).2.1
    rw [‹o = none›] at this
    exact Or.inl this
  · rename_i t ht
    obtain ⟨a', ae, k⟩ := t
    have hs1 := o_post1 a' ae k ht
    have ht2 := o_post2
    simp only [ht] at ht2
    have hk : ∀ ra rb rc rd tail, HeaderHyp h.val ra rb rc rd tail → k.val = 1 + (rangeText ra rb).length :=
      fun ra rb rc rd tail hh => (ht2 _ _ _ (header_text_facts _ _ _ _ _ _ hh).2.1).2.2
    have hlen : (↑h.len : Nat) = h.val.length := by simp
    step*
    · refine ⟨by simp, ?_⟩
      intro ra rb rc rd tail hh
      exfalso
      have F := header_text_facts _ _ _ _ _ _ hh
      have := hk _ _ _ _ _ hh
      have := F.2.2.2.2.1
      scalar_tac
    · refine ⟨by simp, ?_⟩
      intro ra rb rc rd tail hh
      exfalso
      have hne : i3 ≠ 32#u8 := by simpa using ‹(i3 != 32#u8) = true›
      have F := (header_text_facts _ _ _ _ _ _ hh).2.2.1
      rw [← hk _ _ _ _ _ hh, List.getElem?_eq_getElem (by scalar_tac)] at F
      simp only [Option.some.injEq] at F
      exact hne (i3_post.trans F)
    · refine ⟨by simp, ?_⟩
      intro ra rb rc rd tail hh
      exfalso
      have hne : i5 ≠ 43#u8 := by simpa using ‹(i5 != 43#u8) = true›
      have F := (header_text_facts _ _ _ _ _ _ hh).2.2.2.1
      rw [← hk _ _ _ _ _ hh, ← i4_post, List.getElem?_eq_getElem (by scalar_tac)] at F
      simp only [Option.some.injEq] at F
      exact hne (i5_post.trans F)
    · refine ⟨by simp, ?_⟩
      intro ra rb rc rd tail hh
      have F := (header_text_facts _ _ _ _ _ _ hh).2.2.2.2.2.1
      rw [← hk _ _ _ _ _ hh, ← i6_post] at F
      have := o1_post2 _ _ _ F
      rw [‹o1 = none›] at this
      exact Or.inr this
    · rename_i t1 ht1
      obtain ⟨c', ce, j⟩ := t1
      have hs2 := o1_post1 c' ce j ht1
      have ht3 := o1_post2
      simp only [ht1] at ht3
      have hj : ∀ ra rb rc rd tail, HeaderHyp h.val ra rb rc rd tail →
          c'.val = dval rc ∧ ce.val = dval rc + count rd ∧
          j.val = 1 + (rangeText ra rb).length + 2 + (rangeText rc rd).length := by
        intro ra rb rc rd tail hh
        have F := (header_text_facts _ _ _ _ _ _ hh).2.2.2.2.2.1
        rw [← hk _ _ _ _ _ hh, ← i6_post] at F
        have := ht3 _ _ _ F
        refine ⟨this.1, this.2.1, ?_⟩
        rw [this.2.2, i6_post, hk _ _ _ _ _ hh]
      step*
      · refine ⟨by simp, ?_⟩
        intro ra rb rc rd tail hh
        exfalso
        have hne : i8 ≠ 32#u8 := by simpa using ‹(i8 != 32#u8) = true›
        have F := (header_text_facts _ _ _ _ _ _ hh).2.2.2.2.2.2
        rw [← (hj _ _ _ _ _ hh).2.2] at F
        have F := F (by scalar_tac)
        rw [List.getElem?_eq_getElem (by scalar_tac)] at F
        simp only [Option.some.injEq] at F
        exact hne (i8_post.trans F)
      all_goals
        refine ⟨fun ctr hc => ?_, fun ra rb rc rd tail hh => ?_⟩
        · simp only [Option.some.injEq] at hc
          subst hc
          exact ⟨hs1.1, hs2.1⟩
        · have h1 := ht2 _ _ _ (header_text_facts _ _ _ _ _ _ hh).2.1
          have h2 := hj _ _ _ _ _ hh
          exact ⟨h1.1, h1.2.1, h2.1, h2.2.1⟩

end DiffParse.Header
