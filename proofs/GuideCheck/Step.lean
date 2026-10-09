import GuideCheck.Cut

/-! # The rows of a guide step -/

open Aeneas Aeneas.Std Result guide_check GuideCheck.Spec GuideCheck.Small GuideCheck.Rows GuideCheck.Cut

namespace GuideCheck.Step

@[step]
theorem side_number_spec (r : Row) (old : Bool) :
    side_number r old ⦃ n => n.val = (if old then r.old.val else r.new.val) ⦄ := by
  unfold side_number
  split <;> step*

@[step]
theorem shows_spec (r : Row) (g : StepRange) : shows r g ⦃ b => (b = true ↔ Shows g r) ⦄ := by
  unfold shows
  step*
  all_goals (simp only [Shows]; rw [← n_post]; scalar_tac)

/-- A fact that holds for every item before index `k` and at `k` holds before `k + 1`. -/
theorem upto_succ {α : Type} (l : List α) (P : Nat → α → Prop) (k : Nat) (x : α) (hx : l[k]? = some x)
    (h : ∀ t r, t < k → l[t]? = some r → P t r) (hk : P k x) :
    ∀ t r, t < k + 1 → l[t]? = some r → P t r := by
  intro t r ht hr
  by_cases htk : t < k
  · exact h t r htk hr
  · rw [show t = k by omega] at hr ⊢
    rw [hx] at hr; cases hr; exact hk

/-- The item at a slice index that `step*` read. -/
theorem at_index {α : Type} (l : Slice α) (k : Usize) (r : α) (hk : k.val < l.val.length)
    (h : r = l.val[k.val]) : l.val[k.val]? = some r := by
  rw [List.getElem?_eq_getElem hk, h]

@[step]
theorem first_shown_spec (rows : Slice Row) (g : StepRange) :
    first_shown rows g ⦃ k => k.val ≤ rows.val.length ∧
      (∀ t r, t < k.val → rows.val[t]? = some r → ¬ Shows g r) ⦄ := by
  unfold first_shown first_shown_loop
  apply loop.spec_decr_nat (fun k => rows.val.length - k.val)
    (fun k => k.val ≤ rows.val.length ∧ ∀ t r, t < k.val → rows.val[t]? = some r → ¬ Shows g r)
    _ _ _ _ ⟨by simp, fun t r h => absurd h (by simp)⟩
  rintro k ⟨hk, hnone⟩
  unfold first_shown_loop.body
  step*
  · have hx := at_index rows k r (by scalar_tac) r_post
    refine ⟨by scalar_tac, ?_, by scalar_tac⟩
    rw [k1_post]
    exact upto_succ rows.val (fun _ r => ¬ Shows g r) k.val r hx hnone (fun h => by simp_all)

@[step]
theorem last_shown_spec (rows : Slice Row) (g : StepRange) (lo : Usize) (hlo : lo.val < rows.val.length) :
    last_shown rows g lo ⦃ k => lo.val ≤ k.val ∧ k.val < rows.val.length ∧
      (∀ t r, k.val < t → rows.val[t]? = some r → ¬ Shows g r) ⦄ := by
  unfold last_shown last_shown_loop
  apply loop.spec_decr_nat (fun k => k.val)
    (fun k => lo.val ≤ k.val ∧ k.val ≤ rows.val.length ∧
      ∀ t r, k.val ≤ t → rows.val[t]? = some r → ¬ Shows g r)
    _ _ _ _ ⟨by scalar_tac, by scalar_tac, fun t r h1 h2 => by
      rw [List.getElem?_eq_none (by scalar_tac)] at h2; cases h2⟩
  rintro k ⟨hk, hk2, hnone⟩
  unfold last_shown_loop.body
  step*
  have hx := at_index rows k1 r (by scalar_tac) r_post
  have hb : ¬ b = true := by assumption
  refine ⟨by scalar_tac, by scalar_tac, fun t x ht hy => ?_, by scalar_tac⟩
  by_cases htk : t = k1.val
  · subst htk; rw [hx] at hy; cases hy; exact fun h => hb (b_post.mpr h)
  · exact hnone t x (by scalar_tac) hy

@[step]
theorem grow_back_spec (rows : Slice Row) (lo ctx : Usize) (hlo : lo.val ≤ rows.val.length) :
    grow_back rows lo ctx ⦃ r => r.val ≤ lo.val ⦄ := by
  unfold grow_back grow_back_loop
  apply loop.spec_decr_nat (fun (st : Usize × Usize) => ctx.val - st.2.val)
    (fun (st : Usize × Usize) => st.1.val ≤ lo.val) _ _ _ _ (le_refl _)
  rintro ⟨l, t⟩ hl
  simp only at hl
  have : l.val ≤ rows.val.length := by omega
  unfold grow_back_loop.body
  step*

@[step]
theorem grow_ahead_spec (rows : Slice Row) (hi ctx : Usize) (hhi : hi.val < rows.val.length) :
    grow_ahead rows hi ctx ⦃ r => hi.val ≤ r.val ⦄ := by
  unfold grow_ahead grow_ahead_loop
  apply loop.spec_decr_nat (fun (st : Usize × Usize) => ctx.val - st.2.val)
    (fun (st : Usize × Usize) => hi.val ≤ st.1.val ∧ st.1.val < rows.val.length) _ _ _ _ ⟨le_refl _, hhi⟩
  rintro ⟨h, t⟩ ⟨hh1, hh2⟩
  simp only at hh1 hh2
  unfold grow_ahead_loop.body
  step*

/-- Some part holds every row that shows a line of `g`. -/
def Covers (rows : List Row) (parts : List Part) (g : StepRange) : Prop :=
  ∀ t r, rows[t]? = some r → Shows g r → ∃ p ∈ parts, p.lo.val ≤ t ∧ t ≤ p.hi.val

theorem covers_more (rows : List Row) (parts : List Part) (x : Part) (g : StepRange)
    (h : Covers rows parts g) : Covers rows (parts ++ [x]) g := by
  intro t r hr hs
  obtain ⟨p, hp, h1, h2⟩ := h t r hr hs
  exact ⟨p, List.mem_append_left _ hp, h1, h2⟩

@[step]
theorem part_of_spec (rows : Slice Row) (g : StepRange) (lo ctx : Usize) (hlo : lo.val < rows.val.length)
    (hfirst : ∀ t r, t < lo.val → rows.val[t]? = some r → ¬ Shows g r) :
    part_of rows g lo ctx ⦃ p => ∀ t r, rows.val[t]? = some r → Shows g r → p.lo.val ≤ t ∧ t ≤ p.hi.val ⦄ := by
  unfold part_of
  step*

@[step]
theorem step_parts_spec (rows : Slice Row) (ranges : Slice StepRange) (ctx : Usize) :
    step_parts rows ranges ctx ⦃ parts => ∀ g ∈ ranges.val, Covers rows.val parts.val g ⦄ := by
  unfold step_parts step_parts_loop
  apply loop.spec_decr_nat (fun (st : alloc.vec.Vec Part × Usize) => ranges.val.length - st.2.val)
    (fun (st : alloc.vec.Vec Part × Usize) => st.2.val ≤ ranges.val.length ∧
      st.1.val.length ≤ st.2.val ∧
      ∀ j g, j < st.2.val → ranges.val[j]? = some g → Covers rows.val st.1.val g)
    _ _ _ _ ⟨by simp, by simp, fun j g h => absurd h (by simp)⟩
  rintro ⟨parts, i⟩ ⟨hik, hlen, hcov⟩
  simp only at hik hlen hcov
  unfold step_parts_loop.body
  step*
  · have hg := at_index ranges i sr (by scalar_tac) sr_post
    by_cases hlo : lo.val < rows.val.length
    · simp only [show lo < rows.len by scalar_tac, if_true]
      step*
      refine ⟨by scalar_tac, by simp [parts1_post]; scalar_tac, ?_, by scalar_tac⟩
      rw [i3_post, parts1_post]
      refine upto_succ ranges.val (fun _ g => Covers rows.val (parts.val ++ _) g) i.val sr hg
        (fun j g hj hgj => covers_more _ _ _ _ (hcov j g hj hgj)) ?_
      intro t r hr hs
      exact ⟨_, List.mem_append_right _ (List.mem_singleton_self _), x_post t r hr hs⟩
    · simp only [show ¬ lo < rows.len by scalar_tac, if_false]
      step*
      refine ⟨by scalar_tac, by scalar_tac, ?_, by scalar_tac⟩
      rw [i3_post]
      refine upto_succ ranges.val (fun _ g => Covers rows.val parts.val g) i.val sr hg hcov ?_
      intro t r hr hs
      have : t < rows.val.length := (List.getElem?_eq_some_iff.mp hr).1
      exact absurd hs (lo_post2 t r (by omega) hr)
  · intro g hg
    obtain ⟨j, hj, rfl⟩ := List.getElem_of_mem hg
    exact hcov j _ (by scalar_tac) (List.getElem?_eq_getElem hj)

/-- Some part holds row index `k`. -/
def InParts (parts : List Part) (k : Nat) : Prop := ∃ p ∈ parts, p.lo.val ≤ k ∧ k ≤ p.hi.val

@[step]
theorem in_parts_spec (parts : Slice Part) (k : Usize) :
    in_parts parts k ⦃ b => (b = true ↔ InParts parts.val k.val) ⦄ := by
  unfold in_parts in_parts_loop
  apply loop.spec_decr_nat (fun i => parts.val.length - i.val)
    (fun i => i.val ≤ parts.val.length ∧
      ∀ t p, t < i.val → parts.val[t]? = some p → ¬ (p.lo.val ≤ k.val ∧ k.val ≤ p.hi.val))
    _ _ _ _ ⟨by simp, fun t p h => absurd h (by simp)⟩
  rintro i ⟨hi, hnone⟩
  unfold in_parts_loop.body
  step*
  · simp only [true_iff]
    exact ⟨p, by rw [p_post]; exact List.getElem_mem _, by scalar_tac, by scalar_tac⟩
  · have hx := at_index parts i p (by scalar_tac) p_post
    refine ⟨by scalar_tac, ?_, by scalar_tac⟩
    rw [i2_post]
    exact upto_succ parts.val (fun _ p => ¬ (p.lo.val ≤ k.val ∧ k.val ≤ p.hi.val)) i.val p hx hnone
      (by intro h; scalar_tac)
  · have hx := at_index parts i p (by scalar_tac) p_post
    refine ⟨by scalar_tac, ?_, by scalar_tac⟩
    rw [i2_post]
    exact upto_succ parts.val (fun _ p => ¬ (p.lo.val ≤ k.val ∧ k.val ≤ p.hi.val)) i.val p hx hnone
      (by intro h; scalar_tac)
  · simp only [Bool.false_eq_true, false_iff, InParts, not_exists, not_and]
    intro p hp h1 h2
    obtain ⟨t, ht, rfl⟩ := List.getElem_of_mem hp
    exact hnone t _ (by scalar_tac) (List.getElem?_eq_getElem ht) ⟨h1, h2⟩

/-- What `step_mask` gives: one bit per row, true for every row that shows a line of a range. -/
def MaskGood (rows : List Row) (ranges : List StepRange) (mask : List Bool) : Prop :=
  mask.length = rows.length ∧
  ∀ g ∈ ranges, ∀ (t : Nat) r, rows[t]? = some r → Shows g r → mask[t]? = some true

@[step]
theorem step_mask_loop_spec (rows : Slice Row) (parts : alloc.vec.Vec Part) (mask : alloc.vec.Vec Bool)
    (k : Usize) (hk : k.val ≤ rows.val.length) (hlen : mask.val.length = k.val)
    (hdone : ∀ (t : Nat), t < k.val → InParts parts.val t → mask.val[t]? = some true) :
    step_mask_loop rows parts mask k ⦃ res => res.val.length = rows.val.length ∧
      ∀ (t : Nat), t < rows.val.length → InParts parts.val t → res.val[t]? = some true ⦄ := by
  unfold step_mask_loop
  apply loop.spec_decr_nat (fun (st : alloc.vec.Vec Bool × Usize) => rows.val.length - st.2.val)
    (fun (st : alloc.vec.Vec Bool × Usize) => st.2.val ≤ rows.val.length ∧
      st.1.val.length = st.2.val ∧
      ∀ (t : Nat), t < st.2.val → InParts parts.val t → st.1.val[t]? = some true)
    _ _ _ _ ⟨hk, hlen, hdone⟩
  rintro ⟨mask, k⟩ ⟨hk, hlen, hdone⟩
  simp only at hk hlen hdone
  unfold step_mask_loop.body
  step*
  · refine ⟨by scalar_tac, by simp [mask1_post]; scalar_tac, fun t ht hin => ?_, by scalar_tac⟩
    rw [mask1_post]
    by_cases htk : t < k.val
    · rw [List.getElem?_append_left (by omega)]; exact hdone t htk hin
    · have : t = k.val := by scalar_tac
      subst this
      rw [List.getElem?_append_right (by omega), show k.val - mask.val.length = 0 by omega]
      simp only [List.getElem?_cons_zero, Option.some.injEq]
      exact b_post.mpr (by simpa using hin)

@[step]
theorem step_mask_spec (rows : Slice Row) (ranges : Slice StepRange) (ctx : Usize) :
    step_mask rows ranges ctx ⦃ mask => MaskGood rows.val ranges.val mask.val ⦄ := by
  unfold step_mask
  step*
  refine ⟨mask_post1, fun g hg t r hr hs => ?_⟩
  have ht : t < rows.val.length := (List.getElem?_eq_some_iff.mp hr).1
  obtain ⟨p, hp, h1, h2⟩ := parts_post g hg t r hr hs
  exact mask_post2 t ht ⟨p, hp, h1, h2⟩

theorem change_back_loop_spec (rows : Slice Row) (mask : Slice Bool) (start c k : Usize)
    (hm : mask.val.length = rows.val.length) (hk : k.val < rows.val.length)
    (hc1 : start.val ≤ c.val) (hc2 : c.val ≤ k.val + 1)
    (hrun : ∀ (t : Nat), c.val ≤ t → t ≤ k.val → mask.val[t]? = some true)
    (hnc : ∀ (t : Nat) (r : Row), c.val ≤ t → t ≤ k.val → rows.val[t]? = some r → isChange r = false) :
    change_back_loop rows mask start c ⦃ b => (b = true ↔ ∃ (c' : Nat) (r : Row), start.val ≤ c' ∧ c' ≤ k.val ∧
      rows.val[c']? = some r ∧ isChange r = true ∧
      ∀ (t : Nat), c' ≤ t → t ≤ k.val → mask.val[t]? = some true) ⦄ := by
  unfold change_back_loop
  apply loop.spec_decr_nat (fun c => c.val)
    (fun c => start.val ≤ c.val ∧ c.val ≤ k.val + 1 ∧
      (∀ (t : Nat), c.val ≤ t → t ≤ k.val → mask.val[t]? = some true) ∧
      (∀ (t : Nat) (r : Row), c.val ≤ t → t ≤ k.val → rows.val[t]? = some r → isChange r = false))
    _ _ _ _ ⟨hc1, hc2, hrun, hnc⟩
  rintro c ⟨hc1, hc2, hrun, hnc⟩
  unfold change_back_loop.body
  step*
  · -- A change row in the run: found.
    have hx := at_index rows c1 r (by scalar_tac) r_post
    have hmx := at_index mask c1 b (by scalar_tac) b_post
    simp only [true_iff]
    refine ⟨c1.val, r, by scalar_tac, by scalar_tac, hx, by simp_all, fun t h1 h2 => ?_⟩
    by_cases htc : t = c1.val
    · subst htc; rw [hmx]; simp_all
    · exact hrun t (by scalar_tac) h2
  · -- Not a change: go on back.
    have hx := at_index rows c1 r (by scalar_tac) r_post
    have hmx := at_index mask c1 b (by scalar_tac) b_post
    refine ⟨by scalar_tac, by scalar_tac, fun t h1 h2 => ?_, fun t x h1 h2 hy => ?_, by scalar_tac⟩
    · by_cases htc : t = c1.val
      · subst htc; rw [hmx]; simp_all
      · exact hrun t (by scalar_tac) h2
    · by_cases htc : t = c1.val
      · subst htc; rw [hx] at hy; cases hy; simp_all
      · exact hnc t x (by scalar_tac) h2 hy
  · -- The run ends here.
    have hmx := at_index mask c1 b (by scalar_tac) b_post
    simp only [Bool.false_eq_true, false_iff, not_exists, not_and]
    intro c' x h1 h2 hx hch hrun'
    by_cases hcc : c.val ≤ c'
    · have := hnc c' x hcc h2 hx; simp_all
    · have := hrun' c1.val (by scalar_tac) (by scalar_tac)
      rw [hmx] at this; simp_all

@[step]
theorem change_back_spec (rows : Slice Row) (mask : Slice Bool) (k ctx : Usize)
    (hm : mask.val.length = rows.val.length) (hk : k.val < rows.val.length) :
    change_back rows mask k ctx ⦃ b => (b = true ↔ ∃ (c : Nat) (r : Row), c ≤ k.val ∧ k.val - c ≤ ctx.val ∧
      rows.val[c]? = some r ∧ isChange r = true ∧
      ∀ (t : Nat), c ≤ t → t ≤ k.val → mask.val[t]? = some true) ⦄ := by
  unfold change_back
  step*
  apply WP.spec_mono (change_back_loop_spec rows mask start c k hm hk (by scalar_tac) (by scalar_tac)
    (fun t h1 h2 => by scalar_tac) (fun t r h1 h2 => by scalar_tac))
  intro b hb
  rw [hb]
  constructor
  · rintro ⟨c', r, h1, h2, h3, h4, h5⟩; exact ⟨c', r, h2, by scalar_tac, h3, h4, h5⟩
  · rintro ⟨c', r, h1, h2, h3, h4, h5⟩; exact ⟨c', r, by scalar_tac, h1, h3, h4, h5⟩

theorem change_ahead_loop_spec (rows : Slice Row) (mask : Slice Bool) (stop c k : Usize)
    (hm : mask.val.length = rows.val.length) (hstop : stop.val ≤ rows.val.length)
    (hc1 : k.val ≤ c.val) (hc2 : c.val ≤ stop.val)
    (hrun : ∀ (t : Nat), k.val ≤ t → t < c.val → mask.val[t]? = some true)
    (hnc : ∀ (t : Nat) (r : Row), k.val ≤ t → t < c.val → rows.val[t]? = some r → isChange r = false) :
    change_ahead_loop rows mask stop c ⦃ b => (b = true ↔ ∃ (c' : Nat) (r : Row), k.val ≤ c' ∧ c' < stop.val ∧
      rows.val[c']? = some r ∧ isChange r = true ∧
      ∀ (t : Nat), k.val ≤ t → t ≤ c' → mask.val[t]? = some true) ⦄ := by
  unfold change_ahead_loop
  apply loop.spec_decr_nat (fun c => stop.val - c.val)
    (fun c => k.val ≤ c.val ∧ c.val ≤ stop.val ∧
      (∀ (t : Nat), k.val ≤ t → t < c.val → mask.val[t]? = some true) ∧
      (∀ (t : Nat) (r : Row), k.val ≤ t → t < c.val → rows.val[t]? = some r → isChange r = false))
    _ _ _ _ ⟨hc1, hc2, hrun, hnc⟩
  rintro c ⟨hc1, hc2, hrun, hnc⟩
  unfold change_ahead_loop.body
  step*
  · -- A change row in the run: found.
    have hx := at_index rows c r (by scalar_tac) r_post
    have hmx := at_index mask c b (by scalar_tac) b_post
    simp only [true_iff]
    refine ⟨c.val, r, hc1, by scalar_tac, hx, by simp_all, fun t h1 h2 => ?_⟩
    by_cases htc : t = c.val
    · subst htc; rw [hmx]; simp_all
    · exact hrun t h1 (by omega)
  · -- Not a change: go on.
    have hx := at_index rows c r (by scalar_tac) r_post
    have hmx := at_index mask c b (by scalar_tac) b_post
    refine ⟨by scalar_tac, by scalar_tac, fun t h1 h2 => ?_, fun t x h1 h2 hy => ?_, by scalar_tac⟩
    · by_cases htc : t = c.val
      · subst htc; rw [hmx]; simp_all
      · exact hrun t h1 (by scalar_tac)
    · by_cases htc : t = c.val
      · subst htc; rw [hx] at hy; cases hy; simp_all
      · exact hnc t x h1 (by scalar_tac) hy
  · -- The run ends here.
    have hmx := at_index mask c b (by scalar_tac) b_post
    simp only [Bool.false_eq_true, false_iff, not_exists, not_and]
    intro c' x h1 h2 hx hch hrun'
    by_cases hcc : c' < c.val
    · have := hnc c' x h1 hcc hx; simp_all
    · have := hrun' c.val hc1 (by omega)
      rw [hmx] at this; simp_all

@[step]
theorem change_ahead_spec (rows : Slice Row) (mask : Slice Bool) (k ctx : Usize)
    (hm : mask.val.length = rows.val.length) (hk : k.val < rows.val.length) :
    change_ahead rows mask k ctx ⦃ b => (b = true ↔ ∃ (c : Nat) (r : Row), k.val ≤ c ∧ c - k.val ≤ ctx.val ∧
      rows.val[c]? = some r ∧ isChange r = true ∧
      ∀ (t : Nat), k.val ≤ t → t ≤ c → mask.val[t]? = some true) ⦄ := by
  unfold change_ahead
  step*
  apply WP.spec_mono (change_ahead_loop_spec rows mask «end» k k hm (by scalar_tac) (le_refl _) (by scalar_tac)
    (fun t h1 h2 => by omega) (fun t r h1 h2 => by omega))
  intro b hb
  rw [hb]
  have e2 : «end».val = min (k.val + ctx.val + 1) rows.val.length := by simpa using end_post
  constructor
  · rintro ⟨c', r, h1, h2, h3, h4, h5⟩; exact ⟨c', r, h1, by omega, h3, h4, h5⟩
  · rintro ⟨c', r, h1, h2, h3, h4, h5⟩
    have : c' < rows.val.length := (List.getElem?_eq_some_iff.mp h3).1
    exact ⟨c', r, h1, by omega, h3, h4, h5⟩

/-- The cut of a step keeps row `k`: the mask holds it, and a change row is at most `ctx`
rows away, with every row between them in the mask. -/
def KeptIn (rows : List Row) (mask : List Bool) (ctx k : Nat) : Prop :=
  mask[k]? = some true ∧ ∃ (c : Nat) (r : Row), rows[c]? = some r ∧ isChange r = true ∧
    ndist c k ≤ ctx ∧ ∀ (t : Nat), (c ≤ t ∧ t ≤ k ∨ k ≤ t ∧ t ≤ c) → mask[t]? = some true

theorem kept_iff (rows : List Row) (mask : List Bool) (ctx k : Nat) :
    KeptIn rows mask ctx k ↔ mask[k]? = some true ∧
      ((∃ (c : Nat) (r : Row), c ≤ k ∧ k - c ≤ ctx ∧ rows[c]? = some r ∧ isChange r = true ∧
        ∀ (t : Nat), c ≤ t → t ≤ k → mask[t]? = some true) ∨
       (∃ (c : Nat) (r : Row), k ≤ c ∧ c - k ≤ ctx ∧ rows[c]? = some r ∧ isChange r = true ∧
        ∀ (t : Nat), k ≤ t → t ≤ c → mask[t]? = some true)) := by
  unfold KeptIn
  constructor
  · rintro ⟨hm, c, r, hr, hc, hd, hrun⟩
    refine ⟨hm, ?_⟩
    simp only [ndist] at hd
    by_cases hck : c ≤ k
    · exact Or.inl ⟨c, r, hck, by split at hd <;> omega, hr, hc, fun t h1 h2 => hrun t (Or.inl ⟨h1, h2⟩)⟩
    · exact Or.inr ⟨c, r, by omega, by split at hd <;> omega, hr, hc, fun t h1 h2 => hrun t (Or.inr ⟨h1, h2⟩)⟩
  · rintro ⟨hm, ⟨c, r, h1, h2, hr, hc, hrun⟩ | ⟨c, r, h1, h2, hr, hc, hrun⟩⟩
    · refine ⟨hm, c, r, hr, hc, by simp only [ndist]; split <;> omega, fun t ht => ?_⟩
      rcases ht with ⟨a, b⟩ | ⟨a, b⟩
      · exact hrun t a b
      · exact hrun t (by omega) (by omega)
    · refine ⟨hm, c, r, hr, hc, by simp only [ndist]; split <;> omega, fun t ht => ?_⟩
      rcases ht with ⟨a, b⟩ | ⟨a, b⟩
      · exact hrun t (by omega) (by omega)
      · exact hrun t a b

@[step]
theorem kept_in_spec (rows : Slice Row) (mask : Slice Bool) (k ctx : Usize)
    (hm : mask.val.length = rows.val.length) (hk : k.val < rows.val.length) :
    kept_in rows mask k ctx ⦃ b => (b = true ↔ KeptIn rows.val mask.val ctx.val k.val) ⦄ := by
  unfold kept_in
  have hmk : ∀ b, b = mask.val[k.val]'(by omega) → (b = true ↔ mask.val[k.val]? = some true) := by
    intro b hb; rw [hb, List.getElem?_eq_getElem (by omega)]; simp
  step*
  all_goals rw [kept_iff]
  · simp only [true_iff]
    exact ⟨(hmk _ b_post).mp (by assumption), Or.inl (b1_post.mp (by assumption))⟩
  · have hm0 : mask.val[k.val]? = some true := by
      rw [List.getElem?_eq_getElem (by omega)]; simp_all
    have hback : ¬ b1 = true := by assumption
    rw [b_post]
    constructor
    · intro h; exact ⟨hm0, Or.inr h⟩
    · rintro ⟨_, h | h⟩
      · exact absurd (b1_post.mpr h) hback
      · exact h
  · simp only [Bool.false_eq_true, false_iff, not_and]
    intro h
    exact absurd ((hmk _ b_post).mpr h) (by assumption)

theorem cut_in_loop_spec (rows : Slice Row) (mask : Slice Bool) (ctx : Usize)
    (hm : mask.val.length = rows.val.length)
    (hsize : 2 * rows.val.length ≤ Usize.max) (hnh : ∀ r ∈ rows.val, isHeader r = false)
    (out : alloc.vec.Vec Row) (k old new : Usize) (kept : Bool)
    (hk : k.val ≤ rows.val.length) (hlen : out.val.length ≤ 2 * k.val)
    (hold : old.val = oldBefore rows.val k.val + 1) (hnew : new.val = newBefore rows.val k.val + 1)
    (hinv : CutSoFarP rows.val (KeptIn rows.val mask.val ctx.val) out.val k.val kept) :
    cut_in_loop rows mask ctx out k old new kept ⦃ res =>
      CutSoFarP rows.val (KeptIn rows.val mask.val ctx.val) res.val rows.val.length false ∨
      CutSoFarP rows.val (KeptIn rows.val mask.val ctx.val) res.val rows.val.length true ⦄ := by
  unfold cut_in_loop
  apply loop.spec_decr_nat
    (fun (st : alloc.vec.Vec Row × Usize × Usize × Usize × Bool) => rows.val.length - st.2.1.val)
    (fun (st : alloc.vec.Vec Row × Usize × Usize × Usize × Bool) =>
      st.2.1.val ≤ rows.val.length ∧ st.1.val.length ≤ 2 * st.2.1.val ∧
      st.2.2.1.val = oldBefore rows.val st.2.1.val + 1 ∧
      st.2.2.2.1.val = newBefore rows.val st.2.1.val + 1 ∧
      CutSoFarP rows.val (KeptIn rows.val mask.val ctx.val) st.1.val st.2.1.val st.2.2.2.2)
    _ _ _ _ ⟨hk, hlen, hold, hnew, hinv⟩
  rintro ⟨out, k, old, new, kept⟩ ⟨hk, hlen, hold, hnew, hinv⟩
  simp only at hk hlen hold hnew hinv
  unfold cut_in_loop.body
  have hb := before_le rows.val k.val
  step*
  · by_cases hn : keep = true
    · simp only [hn, if_true]
      step*
      have hxr : x = r := by rw [x_post, r_post]
      subst hxr
      obtain ⟨hr, hrh, hold1, hnew1⟩ := row_facts rows hnh k old new hold hnew x
        (by rw [List.getElem?_eq_getElem (by scalar_tac), ← r_post])
        b b_post b1 b1_post old1 old1_post new1 new1_post k1 k1_post
      have hnear := keep_post.mp hn
      refine ⟨by scalar_tac, ?_, hold1, hnew1, ?_, by scalar_tac⟩
      · cases kept
        · rw [out1_post2 rfl]
          simp only [List.length_append, List.length_singleton]; scalar_tac
        · rw [out1_post1 rfl]
          simp only [List.length_append, List.length_singleton]; scalar_tac
      · rw [k1_post]
        cases kept
        · rw [out1_post2 rfl]
          exact cut_keep_new _ _ _ _ hinv x hr hrh hnear _ rfl hold hnew
        · rw [out1_post1 rfl]
          exact cut_keep_run _ _ _ _ hinv x hr hrh hnear
    · simp only [hn, Bool.false_eq_true, if_false]
      step*
      obtain ⟨_, _, hold1, hnew1⟩ := row_facts rows hnh k old new hold hnew r
        (by rw [List.getElem?_eq_getElem (by scalar_tac), ← r_post])
        b b_post b1 b1_post old1 old1_post new1 new1_post k1 k1_post
      have hfar : ¬ KeptIn rows.val mask.val ctx.val k.val := fun h => hn (keep_post.mpr h)
      refine ⟨by scalar_tac, by scalar_tac, hold1, hnew1, ?_, by scalar_tac⟩
      rw [k1_post]
      exact cut_drop _ _ _ _ _ hinv hfar
  · -- Every row done.
    have : k.val = rows.val.length := by scalar_tac
    rw [← this]
    cases kept
    · exact Or.inl hinv
    · exact Or.inr hinv

@[step]
theorem cut_in_spec (rows : Slice Row) (mask : Slice Bool) (ctx : Usize)
    (hm : mask.val.length = rows.val.length)
    (hsize : 2 * rows.val.length ≤ Usize.max) (hnh : ∀ r ∈ rows.val, isHeader r = false) :
    cut_in rows mask ctx ⦃ out => ∃ kept,
      CutSoFarP rows.val (KeptIn rows.val mask.val ctx.val) out.val rows.val.length kept ⦄ := by
  unfold cut_in
  apply WP.spec_mono (cut_in_loop_spec rows mask ctx hm hsize hnh _ 0#usize 1#usize 1#usize false
    (by simp) (by simp) (by simp [oldBefore]) (by simp [newBefore])
    (by simpa using cut_start rows.val (KeptIn rows.val mask.val ctx.val)))
  intro out h
  rcases h with h | h
  · exact ⟨_, h⟩
  · exact ⟨_, h⟩

/-- What `hidden_changes` gives: the change rows that the mask leaves out, in order. -/
def HiddenGood (rows : List Row) (mask : List Bool) (hid : List Row) : Prop :=
  hid.Sublist rows ∧
  ∀ r, r ∈ hid ↔ ∃ (t : Nat), rows[t]? = some r ∧ isChange r = true ∧ mask[t]? = some false

theorem hidden_changes_loop_spec (rows : Slice Row) (mask : Slice Bool)
    (hm : mask.val.length = rows.val.length)
    (out : alloc.vec.Vec Row) (k : Usize) (hk : k.val ≤ rows.val.length) (hlen : out.val.length ≤ k.val)
    (hsub : out.val.Sublist (rows.val.take k.val))
    (hmem : ∀ r, r ∈ out.val ↔ ∃ (t : Nat), t < k.val ∧ rows.val[t]? = some r ∧ isChange r = true ∧
      mask.val[t]? = some false) :
    hidden_changes_loop rows mask out k ⦃ hid => HiddenGood rows.val mask.val hid.val ⦄ := by
  unfold hidden_changes_loop
  apply loop.spec_decr_nat (fun (st : alloc.vec.Vec Row × Usize) => rows.val.length - st.2.val)
    (fun (st : alloc.vec.Vec Row × Usize) => st.2.val ≤ rows.val.length ∧ st.1.val.length ≤ st.2.val ∧
      st.1.val.Sublist (rows.val.take st.2.val) ∧
      ∀ r, r ∈ st.1.val ↔ ∃ (t : Nat), t < st.2.val ∧ rows.val[t]? = some r ∧ isChange r = true ∧
        mask.val[t]? = some false)
    _ _ _ _ ⟨hk, hlen, hsub, hmem⟩
  rintro ⟨out, k⟩ ⟨hk, hlen, hsub, hmem⟩
  simp only at hk hlen hsub hmem
  unfold hidden_changes_loop.body
  step*
  · have hx := at_index rows k r (by scalar_tac) r_post
    have htake : rows.val.take (k.val + 1) = rows.val.take k.val ++ [r] := by
      rw [List.take_add_one, hx]; rfl
    by_cases hb : b = true
    · simp only [hb, if_true]
      step*
      by_cases hb1 : x = true
      · simp only [hb1, if_true]
        step*
        have hmx := at_index mask k x (by scalar_tac) x_post
        refine ⟨by scalar_tac, by scalar_tac, ?_, fun y => ?_, by scalar_tac⟩
        · rw [k1_post, htake]; exact hsub.trans (List.sublist_append_left _ _)
        · rw [hmem y]
          constructor
          · rintro ⟨t, h1, h2, h3, h4⟩; exact ⟨t, by scalar_tac, h2, h3, h4⟩
          · rintro ⟨t, h1, h2, h3, h4⟩
            by_cases htk : t < k.val
            · exact ⟨t, htk, h2, h3, h4⟩
            · have : t = k.val := by scalar_tac
              subst this
              rw [hmx, hb1] at h4; cases h4
      · have hmx := at_index mask k x (by scalar_tac) x_post
        simp only [hb1, Bool.false_eq_true, if_false]
        step*
        refine ⟨by scalar_tac, by simp [out1_post]; scalar_tac, ?_, fun y => ?_, by scalar_tac⟩
        · rw [k1_post, htake, out1_post]; exact hsub.append (List.Sublist.refl _)
        · rw [out1_post, List.mem_append, hmem y, List.mem_singleton]
          constructor
          · rintro (⟨t, h1, h2, h3, h4⟩ | rfl)
            · exact ⟨t, by scalar_tac, h2, h3, h4⟩
            · exact ⟨k.val, by scalar_tac, hx, by simp_all, by rw [hmx]; simp_all⟩
          · rintro ⟨t, h1, h2, h3, h4⟩
            by_cases htk : t < k.val
            · exact Or.inl ⟨t, htk, h2, h3, h4⟩
            · have : t = k.val := by scalar_tac
              subst this
              rw [hx] at h2; cases h2; exact Or.inr rfl
    · simp only [hb, Bool.false_eq_true, if_false]
      step*
      refine ⟨by scalar_tac, by scalar_tac, ?_, fun y => ?_, by scalar_tac⟩
      · rw [k1_post, htake]; exact hsub.trans (List.sublist_append_left _ _)
      · rw [hmem y]
        constructor
        · rintro ⟨t, h1, h2, h3, h4⟩; exact ⟨t, by scalar_tac, h2, h3, h4⟩
        · rintro ⟨t, h1, h2, h3, h4⟩
          by_cases htk : t < k.val
          · exact ⟨t, htk, h2, h3, h4⟩
          · have : t = k.val := by scalar_tac
            subst this
            rw [hx] at h2; cases h2; simp_all
  · -- Every row done.
    have hkl : k.val = rows.val.length := by scalar_tac
    refine ⟨by rw [hkl, List.take_length] at hsub; exact hsub, fun y => ?_⟩
    rw [hmem y]
    constructor
    · rintro ⟨t, _, h2, h3, h4⟩; exact ⟨t, h2, h3, h4⟩
    · rintro ⟨t, h2, h3, h4⟩
      exact ⟨t, by rw [hkl]; exact (List.getElem?_eq_some_iff.mp h2).1, h2, h3, h4⟩

@[step]
theorem hidden_changes_spec (rows : Slice Row) (mask : Slice Bool) (hm : mask.val.length = rows.val.length) :
    hidden_changes rows mask ⦃ hid => HiddenGood rows.val mask.val hid.val ⦄ := by
  unfold hidden_changes
  exact hidden_changes_loop_spec rows mask hm _ 0#usize (by simp) (by simp) (by simp) (by simp)

/-- What `step_cut` gives, on rows with no header. -/
theorem step_cut_spec (rows : Slice Row) (ranges : Slice StepRange) (ctx : Usize)
    (hsize : 2 * rows.val.length ≤ Usize.max) (hnh : ∀ r ∈ rows.val, isHeader r = false) :
    step_cut rows ranges ctx ⦃ res => ∃ mask, MaskGood rows.val ranges.val mask ∧
      (∃ kept, CutSoFarP rows.val (KeptIn rows.val mask ctx.val) res.1.val rows.val.length kept) ∧
      HiddenGood rows.val mask res.2.val ⦄ := by
  unfold step_cut
  step*
  · simpa using mask_post.1
  · simpa using mask_post.1
  · simp only [deref_val] at *
    exact ⟨mask.val, mask_post, ⟨_, v_post⟩, v1_post⟩

/-- A change row in the mask is kept: it is its own near change. -/
theorem kept_change (rows : List Row) (mask : List Bool) (ctx t : Nat) (r : Row)
    (hr : rows[t]? = some r) (hc : isChange r = true) (hm : mask[t]? = some true) :
    KeptIn rows mask ctx t :=
  ⟨hm, t, r, hr, hc, by simp [ndist], fun u hu => by
    rw [show u = t by omega]; exact hm⟩

theorem kept_near (rows : List Row) (mask : List Bool) (ctx k : Nat) (h : KeptIn rows mask ctx k) :
    NearChange rows ctx k := by
  obtain ⟨_, c, r, hr, hc, hd, _⟩ := h
  exact ⟨c, r, hr, hc, hd⟩

theorem so_far_headers (rows : List Row) (P : Nat → Prop) (out : List Row) (kept : Bool)
    (h : CutSoFarP rows P out rows.length kept) :
    (out.filter (fun r => !isHeader r)).Sublist rows ∧ HeadersGood rows out := by
  obtain ⟨h1, _, _, h4, h5, h6, h7, _⟩ := h
  exact ⟨by simpa using h1, h4, h5, h6, h7⟩

theorem S5_proof (rows : Slice Row) (ranges : Slice StepRange) (ctx : Usize)
    (hsize : 2 * rows.val.length ≤ Usize.max) (hnh : ∀ r ∈ rows.val, isHeader r = false) :
    step_cut rows ranges ctx ⦃ res =>
      (res.1.val.filter (fun r => !isHeader r)).Sublist rows.val ∧
      HeadersGood rows.val res.1.val ∧ res.2.val.Sublist rows.val ⦄ := by
  apply WP.spec_mono (step_cut_spec rows ranges ctx hsize hnh)
  rintro res ⟨mask, _, ⟨kept, hc⟩, hh⟩
  exact ⟨(so_far_headers _ _ _ _ hc).1, (so_far_headers _ _ _ _ hc).2, hh.1⟩

/-- The rows of a file that rebuilds have no repeat: each row has its own line number. -/
theorem rows_nodup (l : List Row) (n m : Nat)
    (hO : (l.filter showsOld).map (·.old.val) = List.range' 1 n)
    (hN : (l.filter showsNew).map (·.new.val) = List.range' 1 m)
    (hall : ∀ r ∈ l, showsOld r = true ∨ showsNew r = true) : l.Nodup := by
  classical
  have dO : (l.filter showsOld).Nodup := List.Nodup.of_map _ (by rw [hO]; exact List.nodup_range' _)
  have dN : (l.filter showsNew).Nodup := List.Nodup.of_map _ (by rw [hN]; exact List.nodup_range' _)
  rw [List.nodup_iff_count_le_one] at dO dN ⊢
  intro a
  by_cases ha : a ∈ l
  · rcases hall a ha with h | h
    · rw [← List.count_filter h]; exact dO a
    · rw [← List.count_filter h]; exact dN a
  · rw [List.count_eq_zero_of_not_mem ha]; omega

theorem index_unique (l : List Row) (hl : l.Nodup) (t u : Nat) (r : Row)
    (ht : l[t]? = some r) (hu : l[u]? = some r) : t = u := by
  obtain ⟨h1, e1⟩ := List.getElem?_eq_some_iff.mp ht
  obtain ⟨h2, e2⟩ := List.getElem?_eq_some_iff.mp hu
  exact (hl.getElem_inj_iff (hi := h1) (hj := h2)).mp (e1.trans e2.symm)

/-- The facts about the rows of a file that rebuilds that S1 to S4 use. -/
theorem file_rows (d : FileDiff) (hd : Rebuilds d)
    (hsize : 2 * (d.removed.val.length + d.added.val.length) ≤ Usize.max) :
    number_rows (alloc.vec.Vec.deref d.removed) (alloc.vec.Vec.deref d.added) ⦃ rows =>
      2 * rows.val.length ≤ Usize.max ∧ (∀ r ∈ rows.val, isHeader r = false) ∧ rows.val.Nodup ∧
      (rows.val.filter showsOld).map (·.old.val) = List.range' 1 d.old.val.length ∧
      (rows.val.filter showsNew).map (·.new.val) = List.range' 1 d.new.val.length ⦄ := by
  apply WP.spec_mono (number_rows_file d hd (by omega))
  intro rows ⟨hlen, hgood, hO, hN, _⟩
  have hk : ∀ r ∈ rows.val, isHeader r = false ∧ (showsOld r = true ∨ showsNew r = true) := by
    intro r hr
    have := hgood r hr
    unfold RowNames at this
    cases hk : r.kind <;> simp_all [isHeader, showsOld, showsNew]
  exact ⟨by omega, fun r hr => (hk r hr).1, rows_nodup _ _ _ hO hN (fun r hr => (hk r hr).2), hO, hN⟩

theorem S1_proof (d : FileDiff) (ranges : Slice StepRange) (ctx : Usize) (hd : Rebuilds d)
    (hsize : 2 * (d.removed.val.length + d.added.val.length) ≤ Usize.max) :
    number_rows (alloc.vec.Vec.deref d.removed) (alloc.vec.Vec.deref d.added) ⦃ rows =>
      step_cut (alloc.vec.Vec.deref rows) ranges ctx ⦃ res =>
        ∀ r ∈ rows.val, isChange r = true → (∃ g ∈ ranges.val, Shows g r) → r ∈ res.1.val ⦄ ⦄ := by
  apply WP.spec_mono (file_rows d hd hsize)
  rintro rows ⟨hs, hnh, _, _, _⟩
  apply WP.spec_mono (step_cut_spec (alloc.vec.Vec.deref rows) ranges ctx (by simpa using hs) (by simpa using hnh))
  rintro res ⟨mask, hmask, ⟨kept, hc⟩, _⟩
  simp only [deref_val] at hmask hc
  intro r hr hch ⟨g, hg, hsh⟩
  obtain ⟨t, ht, rfl⟩ := List.getElem_of_mem hr
  have hrt := List.getElem?_eq_getElem ht
  exact hc.2.1 t _ ht hrt (kept_change _ _ _ t _ hrt hch (hmask.2 g hg t _ hrt hsh))

theorem S2_proof (d : FileDiff) (ranges : Slice StepRange) (ctx : Usize) (hd : Rebuilds d)
    (hsize : 2 * (d.removed.val.length + d.added.val.length) ≤ Usize.max) :
    number_rows (alloc.vec.Vec.deref d.removed) (alloc.vec.Vec.deref d.added) ⦃ rows =>
      step_cut (alloc.vec.Vec.deref rows) ranges ctx ⦃ res =>
        (res.1.val.filter (fun r => !isHeader r)).Sublist rows.val ∧
        (∀ r ∈ res.1.val, isHeader r = false → ∃ k, rows.val[k]? = some r ∧ NearChange rows.val ctx.val k) ⦄ ⦄ := by
  apply WP.spec_mono (file_rows d hd hsize)
  rintro rows ⟨hs, hnh, _, _, _⟩
  apply WP.spec_mono (step_cut_spec (alloc.vec.Vec.deref rows) ranges ctx (by simpa using hs) (by simpa using hnh))
  rintro res ⟨mask, _, ⟨kept, hc⟩, _⟩
  simp only [deref_val] at hc
  refine ⟨(so_far_headers _ _ _ _ hc).1, fun r hr hh => ?_⟩
  obtain ⟨k, hk, hkept⟩ := hc.2.2.1 r hr hh
  exact ⟨k, hk, kept_near _ _ _ _ hkept⟩

theorem S3_proof (d : FileDiff) (ranges : Slice StepRange) (ctx : Usize) (hd : Rebuilds d)
    (hsize : 2 * (d.removed.val.length + d.added.val.length) ≤ Usize.max) :
    number_rows (alloc.vec.Vec.deref d.removed) (alloc.vec.Vec.deref d.added) ⦃ rows =>
      step_cut (alloc.vec.Vec.deref rows) ranges ctx ⦃ res =>
        HeadersGood rows.val res.1.val ∧
        (∀ h q, (h, q) ∈ res.1.val.zip res.1.val.tail → isHeader h = true →
          (showsOld q = true → h.old = q.old) ∧ (showsNew q = true → h.new = q.new)) ⦄ ⦄ := by
  apply WP.spec_mono (file_rows d hd hsize)
  rintro rows ⟨hs, hnh, _, hO, hN⟩
  apply WP.spec_mono (step_cut_spec (alloc.vec.Vec.deref rows) ranges ctx (by simpa using hs) (by simpa using hnh))
  rintro res ⟨mask, _, ⟨kept, hc⟩, _⟩
  simp only [deref_val] at hc
  have hg := (so_far_headers _ _ _ _ hc).2
  refine ⟨hg, fun h q hhq hh => ?_⟩
  obtain ⟨_, t, hq, ho, hn⟩ := hg.2.2.2 h q hhq hh
  refine ⟨fun hsO => UScalar.eq_of_val_eq ?_, fun hsN => UScalar.eq_of_val_eq ?_⟩
  · rw [ho, number_at rows.val showsOld (·.old.val) _ hO t q hq hsO]; rfl
  · rw [hn, number_at rows.val showsNew (·.new.val) _ hN t q hq hsN]; rfl

theorem S4_proof (d : FileDiff) (ranges : Slice StepRange) (ctx : Usize) (hd : Rebuilds d)
    (hsize : 2 * (d.removed.val.length + d.added.val.length) ≤ Usize.max) :
    number_rows (alloc.vec.Vec.deref d.removed) (alloc.vec.Vec.deref d.added) ⦃ rows =>
      step_cut (alloc.vec.Vec.deref rows) ranges ctx ⦃ res =>
        res.2.val.Sublist rows.val ∧
        (∀ r, r ∈ res.2.val ↔ (r ∈ rows.val ∧ isChange r = true ∧ r ∉ res.1.val)) ∧
        (∀ r ∈ rows.val, isChange r = true → r ∈ res.1.val ∨ r ∈ res.2.val) ⦄ ⦄ := by
  apply WP.spec_mono (file_rows d hd hsize)
  rintro rows ⟨hs, hnh, hnd, _, _⟩
  apply WP.spec_mono (step_cut_spec (alloc.vec.Vec.deref rows) ranges ctx (by simpa using hs) (by simpa using hnh))
  rintro res ⟨mask, hmask, ⟨kept, hc⟩, hsub, hmem⟩
  simp only [deref_val] at hmask hc hsub hmem
  -- A change row is shown exactly when the mask holds it.
  have shown : ∀ (t : Nat) (r : Row), rows.val[t]? = some r → isChange r = true →
      (r ∈ res.1.val ↔ mask[t]? = some true) := by
    intro t r hr hch
    constructor
    · intro hin
      have hrh : isHeader r = false := hnh r (List.mem_of_getElem? hr)
      obtain ⟨u, hu, hkept⟩ := hc.2.2.1 r hin hrh
      rw [index_unique rows.val hnd t u r hr hu]
      exact hkept.1
    · intro hm
      exact hc.2.1 t r (List.getElem?_eq_some_iff.mp hr).1 hr (kept_change _ _ _ t r hr hch hm)
  have hiff : ∀ r, r ∈ res.2.val ↔ (r ∈ rows.val ∧ isChange r = true ∧ r ∉ res.1.val) := by
    intro r
    rw [hmem r]
    constructor
    · rintro ⟨t, hr, hch, hm⟩
      refine ⟨List.mem_of_getElem? hr, hch, fun hin => ?_⟩
      rw [(shown t r hr hch).mp hin] at hm; cases hm
    · rintro ⟨hr, hch, hout⟩
      obtain ⟨t, ht, rfl⟩ := List.getElem_of_mem hr
      have hrt := List.getElem?_eq_getElem ht
      refine ⟨t, hrt, hch, ?_⟩
      have hml : t < mask.length := by rw [hmask.1]; exact ht
      cases hb : mask[t]'hml
      · rw [List.getElem?_eq_getElem hml, hb]
      · exact absurd ((shown t _ hrt hch).mpr (by rw [List.getElem?_eq_getElem hml, hb])) hout
  refine ⟨hsub, hiff, fun r hr hch => ?_⟩
  by_cases hin : r ∈ res.1.val
  · exact Or.inl hin
  · exact Or.inr ((hiff r).mpr ⟨hr, hch, hin⟩)

end GuideCheck.Step
