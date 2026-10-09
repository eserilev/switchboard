import GuideCheck.Rows

/-! # The cut: keep the rows near a change, with a header before each run -/

open Aeneas Aeneas.Std Result guide_check GuideCheck.Spec GuideCheck.Small GuideCheck.Rows

namespace GuideCheck.Cut

@[step]
theorem is_change_spec (r : Row) : is_change r ⦃ b => b = isChange r ⦄ := by
  unfold is_change
  cases h : r.kind <;> simp [isChange, h]

@[step]
theorem window_start_spec (k ctx : Usize) : window_start k ctx ⦃ s => s.val = k.val - ctx.val ⦄ := by
  unfold window_start
  step*
  scalar_tac

@[step]
theorem window_end_spec (k ctx len : Usize) (hk : k.val < len.val) :
    window_end k ctx len ⦃ e => e.val = min (k.val + ctx.val + 1) len.val ⦄ := by
  unfold window_end
  step*

theorem near_change_loop_spec (rows : Slice Row) (stop c0 c : Usize) (hstop : stop.val ≤ rows.val.length)
    (hc : c0.val ≤ c.val)
    (hnone : ∀ t r, c0.val ≤ t → t < c.val → t < stop.val → rows.val[t]? = some r → isChange r = false) :
    near_change_loop rows c stop ⦃ b =>
      (b = true ↔ ∃ t r, c0.val ≤ t ∧ t < stop.val ∧ rows.val[t]? = some r ∧ isChange r = true) ⦄ := by
  unfold near_change_loop
  apply loop.spec_decr_nat (fun c => stop.val - c.val)
    (fun c => c0.val ≤ c.val ∧
      ∀ t r, c0.val ≤ t → t < c.val → t < stop.val → rows.val[t]? = some r → isChange r = false)
    _ _ _ _ ⟨hc, hnone⟩
  rintro c ⟨hc, hnone⟩
  unfold near_change_loop.body
  step*
  · -- A change in the window.
    simp only [true_iff]
    have hlt : c.val < rows.val.length := by scalar_tac
    refine ⟨c.val, r, hc, by scalar_tac, by rw [List.getElem?_eq_getElem hlt, r_post], ?_⟩
    simp_all
  · refine ⟨by scalar_tac, fun t x h1 h2 h3 hx => ?_, by scalar_tac⟩
    by_cases htc : t < c.val
    · exact hnone t x h1 htc h3 hx
    · have : t = c.val := by scalar_tac
      subst this
      have hlt : c.val < rows.val.length := by scalar_tac
      rw [List.getElem?_eq_getElem hlt, ← r_post] at hx
      cases hx
      simp_all

theorem window_iff (c e k ctx n t : Nat) (e1 : c = k - ctx) (e2 : e = min (k + ctx + 1) n) :
    (c ≤ t ∧ t < e) ↔ (t < n ∧ ndist t k ≤ ctx) := by
  subst e1 e2
  simp only [ndist]
  split <;> omega

/-- The window of `near_change` holds exactly the rows at most `ctx` rows from row `k`. -/
@[step]
theorem near_change_spec (rows : Slice Row) (k ctx : Usize) (hk : k.val < rows.val.length) :
    near_change rows k ctx ⦃ b => (b = true ↔ NearChange rows.val ctx.val k.val) ⦄ := by
  unfold near_change
  step*
  apply WP.spec_mono (near_change_loop_spec rows «end» c c (by scalar_tac) (le_refl _)
    (fun t r h1 h2 => absurd h2 (by omega)))
  intro b hb
  rw [hb]
  have e1 : c.val = k.val - ctx.val := c_post
  have e2 : «end».val = min (k.val + ctx.val + 1) rows.val.length := by simpa using end_post
  have key := fun t => window_iff c.val «end».val k.val ctx.val rows.val.length t e1 e2
  unfold NearChange
  constructor
  · rintro ⟨t, r, h1, h2, h3, h4⟩
    exact ⟨t, r, h3, h4, ((key t).mp ⟨h1, h2⟩).2⟩
  · rintro ⟨t, r, h3, h4, h5⟩
    have : t < rows.val.length := by
      by_contra h
      rw [List.getElem?_eq_none (by omega)] at h3
      cases h3
    obtain ⟨h1, h2⟩ := (key t).mpr ⟨this, h5⟩
    exact ⟨t, r, h1, h2, h3, h4⟩

/-- The pairs of neighbours after one more item: the old pairs, and the old last item
with the new one. -/
theorem zip_tail_append {α : Type} (L : List α) (x : α) :
    (L ++ [x]).zip (L ++ [x]).tail = L.zip L.tail ++ (L.getLast?.map (fun y => (y, x))).toList := by
  induction L with
  | nil => simp
  | cons a L ih =>
    cases L with
    | nil => simp
    | cons b L =>
      simp only [List.cons_append, List.tail_cons, List.zip_cons_cons, List.getLast?_cons_cons] at ih ⊢
      rw [ih]

theorem mem_pairs_append {α : Type} (L : List α) (x : α) (a b : α) :
    (a, b) ∈ (L ++ [x]).zip (L ++ [x]).tail ↔ (a, b) ∈ L.zip L.tail ∨ (L.getLast? = some a ∧ b = x) := by
  rw [zip_tail_append]
  cases h : L.getLast? <;> simp [eq_comm]

theorem before_step (rs : List Row) (k : Nat) (r : Row) (hr : rs[k]? = some r) :
    oldBefore rs (k + 1) = oldBefore rs k + (if showsOld r then 1 else 0) ∧
    newBefore rs (k + 1) = newBefore rs k + (if showsNew r then 1 else 0) := by
  simp only [oldBefore, newBefore, List.take_add_one, hr, Option.toList_some, List.filter_append,
    List.length_append, List.filter_cons, List.filter_nil]
  constructor <;> split <;> simp

theorem before_le (rs : List Row) (k : Nat) : oldBefore rs k ≤ k ∧ newBefore rs k ≤ k := by
  simp only [oldBefore, newBefore]
  exact ⟨(List.length_filter_le _ _).trans (List.length_take_le _ _),
    (List.length_filter_le _ _).trans (List.length_take_le _ _)⟩

/-- What a cut has done after rows `0..k`, when it keeps the rows `t` with `P t`.
`kept` is true when row `k - 1` is kept. -/
def CutSoFarP (rs : List Row) (P : Nat → Prop) (out : List Row) (k : Nat) (kept : Bool) : Prop :=
  (out.filter (fun r => !isHeader r)).Sublist (rs.take k) ∧
  (∀ t r, t < k → rs[t]? = some r → P t → r ∈ out) ∧
  (∀ r ∈ out, isHeader r = false → ∃ t, rs[t]? = some r ∧ P t) ∧
  (∀ r, out.head? = some r → isHeader r = true) ∧
  (∀ r, out.getLast? = some r → isHeader r = false) ∧
  (∀ a b, (a, b) ∈ out.zip out.tail → isHeader a = false → isHeader b = false →
    ∃ t, rs[t]? = some a ∧ rs[t + 1]? = some b) ∧
  (∀ h q, (h, q) ∈ out.zip out.tail → isHeader h = true →
    isHeader q = false ∧ ∃ t, rs[t]? = some q ∧
      h.old.val = oldBefore rs t + 1 ∧ h.new.val = newBefore rs t + 1) ∧
  (kept = true → 0 < k ∧ out.getLast? = rs[k - 1]?)

/-- What `cut_rows` has done: it keeps the rows near a change. -/
abbrev CutSoFar (rs : List Row) (ctx : Nat) : List Row → Nat → Bool → Prop :=
  CutSoFarP rs (NearChange rs ctx)

theorem cut_start (rs : List Row) (P : Nat → Prop) : CutSoFarP rs P [] 0 false := by
  simp [CutSoFarP]

/-- Row `k` is kept, and row `k - 1` was kept too: no header. -/
theorem cut_keep_run (rs : List Row) (P : Nat → Prop) (out : List Row) (k : Nat)
    (hinv : CutSoFarP rs P out k true) (r : Row) (hr : rs[k]? = some r) (hnh : isHeader r = false)
    (hnear : P k) :
    CutSoFarP rs P (out ++ [r]) (k + 1) true := by
  obtain ⟨h1, h2, h3, h4, h5, h6, h7, h8⟩ := hinv
  obtain ⟨hk, hlast⟩ := h8 rfl
  have hklen : k < rs.length := (List.getElem?_eq_some_iff.mp hr).1
  have hne : out ≠ [] := by
    intro h; rw [h] at hlast; simp at hlast; omega
  refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_⟩
  · rw [List.take_add_one, hr, List.filter_append]
    simpa [hnh] using h1
  · intro t x ht hx hc
    by_cases htk : t < k
    · exact List.mem_append_left _ (h2 t x htk hx hc)
    · rw [show t = k by omega, hr] at hx
      cases hx; simp
  · intro x hx hxh
    rcases List.mem_append.mp hx with hx | hx
    · exact h3 x hx hxh
    · rw [List.mem_singleton.mp hx]; exact ⟨k, hr, hnear⟩
  · intro x hx
    apply h4
    rw [List.head?_append] at hx
    cases hh : out.head? with
    | none => simp_all
    | some y => simp_all
  · intro x hx
    rw [List.getLast?_append] at hx
    simp at hx; rw [← hx]; exact hnh
  · intro a b hab ha hb
    rcases (mem_pairs_append out r a b).mp hab with hab | ⟨hla, rfl⟩
    · exact h6 a b hab ha hb
    · refine ⟨k - 1, by rw [← hlast, hla], ?_⟩
      rw [show k - 1 + 1 = k by omega]; exact hr
  · intro h q hhq hh
    rcases (mem_pairs_append out r h q).mp hhq with hhq | ⟨hla, rfl⟩
    · exact h7 h q hhq hh
    · have := h5 h hla; simp_all
  · intro _
    refine ⟨by omega, ?_⟩
    simp [hr]

/-- Row `k` is kept, and row `k - 1` was not: a header first. -/
theorem cut_keep_new (rs : List Row) (P : Nat → Prop) (out : List Row) (k : Nat)
    (hinv : CutSoFarP rs P out k false) (r : Row) (hr : rs[k]? = some r) (hnh : isHeader r = false)
    (hnear : P k) (h : Row) (hh : isHeader h = true)
    (hho : h.old.val = oldBefore rs k + 1) (hhn : h.new.val = newBefore rs k + 1) :
    CutSoFarP rs P (out ++ [h] ++ [r]) (k + 1) true := by
  obtain ⟨h1, h2, h3, h4, h5, h6, h7, _⟩ := hinv
  refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_⟩
  · rw [List.take_add_one, hr, List.filter_append, List.filter_append]
    simpa [hnh, hh] using h1
  · intro t x ht hx hc
    by_cases htk : t < k
    · exact List.mem_append_left _ (List.mem_append_left _ (h2 t x htk hx hc))
    · rw [show t = k by omega, hr] at hx
      cases hx; simp
  · intro x hx hxh
    rcases List.mem_append.mp hx with hx | hx
    · rcases List.mem_append.mp hx with hx | hx
      · exact h3 x hx hxh
      · rw [List.mem_singleton.mp hx] at hxh; simp_all
    · rw [List.mem_singleton.mp hx]; exact ⟨k, hr, hnear⟩
  · intro x hx
    rw [List.append_assoc, List.head?_append] at hx
    cases hho' : out.head? with
    | none => simp_all
    | some y => simp_all
  · intro x hx
    rw [List.getLast?_append] at hx
    simp at hx; rw [← hx]; exact hnh
  · intro a b hab ha hb
    rcases (mem_pairs_append (out ++ [h]) r a b).mp hab with hab | ⟨hla, rfl⟩
    · rcases (mem_pairs_append out h a b).mp hab with hab | ⟨_, rfl⟩
      · exact h6 a b hab ha hb
      · simp_all
    · simp at hla; simp_all
  · intro x q hxq hxh
    rcases (mem_pairs_append (out ++ [h]) r x q).mp hxq with hxq | ⟨hla, rfl⟩
    · rcases (mem_pairs_append out h x q).mp hxq with hxq | ⟨hla, rfl⟩
      · exact h7 x q hxq hxh
      · have := h5 x hla; simp_all
    · simp at hla
      subst hla
      exact ⟨hnh, k, hr, hho, hhn⟩
  · intro _
    refine ⟨by omega, ?_⟩
    simp [hr]

/-- Row `k` is left out. -/
theorem cut_drop (rs : List Row) (P : Nat → Prop) (out : List Row) (k : Nat) (kept : Bool)
    (hinv : CutSoFarP rs P out k kept)
    (hfar : ¬ P k) :
    CutSoFarP rs P out (k + 1) false := by
  obtain ⟨h1, h2, h3, h4, h5, h6, h7, _⟩ := hinv
  refine ⟨h1.trans (List.take_sublist_take_left (by omega)), ?_, h3, h4, h5, h6, h7, by simp⟩
  intro t x ht hx hc
  by_cases htk : t < k
  · exact h2 t x htk hx hc
  · rw [show t = k by omega] at hc
    exact absurd hc hfar

@[step]
theorem shows_old_spec (r : Row) : shows_old r ⦃ b => b = showsOld r ⦄ := by
  unfold shows_old
  cases h : r.kind <;> simp [showsOld, h]

@[step]
theorem shows_new_spec (r : Row) : shows_new r ⦃ b => b = showsNew r ⦄ := by
  unfold shows_new
  cases h : r.kind <;> simp [showsNew, h]

@[step]
theorem keep_row_spec (out : alloc.vec.Vec Row) (kept : Bool) (r : Row) (old new : Usize)
    (hlen : out.val.length + 2 ≤ Usize.max) :
    keep_row out kept r old new ⦃ res =>
      (kept = true → res.val = out.val ++ [r]) ∧
      (kept = false → res.val = out.val ++ [{ kind := .Header, old := old, new := new }] ++ [r]) ⦄ := by
  unfold keep_row
  split
  · step*
  · step*
    simp only [out1_post, List.length_append, List.length_singleton]; scalar_tac

@[step]
theorem bump_spec (n : Usize) (b : Bool) (h : n.val + 1 ≤ Usize.max) :
    bump n b ⦃ m => m.val = n.val + (if b then 1 else 0) ⦄ := by
  unfold bump
  split <;> step*

/-- The facts about row `k` that every step of the cut uses. -/
theorem row_facts (rows : Slice Row) (hnh : ∀ r ∈ rows.val, isHeader r = false)
    (k old new : Usize) (hold : old.val = oldBefore rows.val k.val + 1)
    (hnew : new.val = newBefore rows.val k.val + 1)
    (r : Row) (hr : rows.val[k.val]? = some r)
    (b : Bool) (b_post : b = showsOld r) (b1 : Bool) (b1_post : b1 = showsNew r)
    (old1 : Usize) (old1_post : old1.val = old.val + if b = true then 1 else 0)
    (new1 : Usize) (new1_post : new1.val = new.val + if b1 = true then 1 else 0)
    (k1 : Usize) (k1_post : k1.val = k.val + 1) :
    rows.val[k.val]? = some r ∧ isHeader r = false ∧
    old1.val = oldBefore rows.val k1.val + 1 ∧ new1.val = newBefore rows.val k1.val + 1 := by
  have hrh : isHeader r = false := hnh r (List.mem_of_getElem? hr)
  have hcount := before_step rows.val k.val r hr
  refine ⟨hr, hrh, ?_, ?_⟩
  · rw [old1_post, k1_post, hcount.1, ← b_post, hold]; split <;> omega
  · rw [new1_post, k1_post, hcount.2, ← b1_post, hnew]; split <;> omega

theorem cut_rows_loop_spec (rows : Slice Row) (ctx : Usize)
    (hsize : 2 * rows.val.length ≤ Usize.max) (hnh : ∀ r ∈ rows.val, isHeader r = false)
    (out : alloc.vec.Vec Row) (k old new : Usize) (kept : Bool)
    (hk : k.val ≤ rows.val.length) (hlen : out.val.length ≤ 2 * k.val)
    (hold : old.val = oldBefore rows.val k.val + 1) (hnew : new.val = newBefore rows.val k.val + 1)
    (hinv : CutSoFar rows.val ctx.val out.val k.val kept) :
    cut_rows_loop rows ctx out k old new kept ⦃ res =>
      CutSoFar rows.val ctx.val res.val rows.val.length false ∨
      CutSoFar rows.val ctx.val res.val rows.val.length true ⦄ := by
  unfold cut_rows_loop
  apply loop.spec_decr_nat
    (fun (st : alloc.vec.Vec Row × Usize × Usize × Usize × Bool) => rows.val.length - st.2.1.val)
    (fun (st : alloc.vec.Vec Row × Usize × Usize × Usize × Bool) =>
      st.2.1.val ≤ rows.val.length ∧ st.1.val.length ≤ 2 * st.2.1.val ∧
      st.2.2.1.val = oldBefore rows.val st.2.1.val + 1 ∧
      st.2.2.2.1.val = newBefore rows.val st.2.1.val + 1 ∧
      CutSoFar rows.val ctx.val st.1.val st.2.1.val st.2.2.2.2)
    _ _ _ _ ⟨hk, hlen, hold, hnew, hinv⟩
  rintro ⟨out, k, old, new, kept⟩ ⟨hk, hlen, hold, hnew, hinv⟩
  simp only at hk hlen hold hnew hinv
  unfold cut_rows_loop.body
  have hb := before_le rows.val k.val
  step*
  · by_cases hn : near1 = true
    · simp only [hn, if_true]
      step*
      have hxr : x = r := by rw [x_post, r_post]
      subst hxr
      obtain ⟨hr, hrh, hold1, hnew1⟩ := row_facts rows hnh k old new hold hnew x
        (by rw [List.getElem?_eq_getElem (by scalar_tac), ← r_post])
        b b_post b1 b1_post old1 old1_post new1 new1_post k1 k1_post
      have hnear := near1_post.mp hn
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
      have hfar : ¬ NearChange rows.val ctx.val k.val := fun h => hn (near1_post.mpr h)
      refine ⟨by scalar_tac, by scalar_tac, hold1, hnew1, ?_, by scalar_tac⟩
      rw [k1_post]
      exact cut_drop _ _ _ _ _ hinv hfar
  · -- Every row done.
    have : k.val = rows.val.length := by scalar_tac
    rw [← this]
    cases kept
    · exact Or.inl hinv
    · exact Or.inr hinv

theorem cut_done (rs : List Row) (ctx : Nat) (out : List Row) (kept : Bool)
    (h : CutSoFar rs ctx out rs.length kept) : CutGood rs ctx out := by
  obtain ⟨h1, h2, h3, h4, h5, h6, h7, _⟩ := h
  refine ⟨by simpa using h1, fun r hr hc => ?_, h3, h4, h5, h6, h7⟩
  obtain ⟨t, ht, rfl⟩ := List.getElem_of_mem hr
  exact h2 t _ ht (List.getElem?_eq_getElem ht) ⟨t, _, List.getElem?_eq_getElem ht, hc, by simp [ndist]⟩

/-- `cut_rows` never panics, and its cut is correct on any rows with no header. -/
theorem cut_rows_spec (rows : Slice Row) (ctx : Usize)
    (hsize : 2 * rows.val.length ≤ Usize.max) (hnh : ∀ r ∈ rows.val, isHeader r = false) :
    cut_rows rows ctx ⦃ out => CutGood rows.val ctx.val out.val ⦄ := by
  unfold cut_rows
  apply WP.spec_mono (cut_rows_loop_spec rows ctx hsize hnh _ 0#usize 1#usize 1#usize false
    (by simp) (by simp) (by simp [oldBefore]) (by simp [newBefore]) (by simpa using cut_start rows.val (NearChange rows.val ctx.val)))
  intro out h
  rcases h with h | h
  · exact cut_done _ _ _ _ h
  · exact cut_done _ _ _ _ h

/-- In a list whose rows of one side are numbered 1, 2, …, a row of that side has the
number one more than the count of rows of that side before it. -/
theorem number_at (l : List Row) (p : Row → Bool) (f : Row → Nat) (n : Nat)
    (h : (l.filter p).map f = List.range' 1 n) (t : Nat) (q : Row) (hq : l[t]? = some q)
    (hp : p q = true) : f q = ((l.take t).filter p).length + 1 := by
  have hlt : t < l.length := (List.getElem?_eq_some_iff.mp hq).1
  have hq' : l[t] = q := by rw [List.getElem?_eq_getElem hlt] at hq; exact Option.some.inj hq
  have e : l = l.take t ++ q :: l.drop (t + 1) := by
    conv_lhs => rw [← List.take_append_drop t l, List.drop_eq_getElem_cons hlt]
    rw [hq']
  rw [e, List.filter_append, List.filter_cons, if_pos hp, List.map_append, List.map_cons] at h
  have hl := congrArg List.length h
  have hi := congrArg (fun L => L[((l.take t).filter p).length]?) h
  simp only [List.length_append, List.length_map, List.length_cons, List.length_range'] at hl
  simp only [List.getElem?_append_right (by simp : ((List.take t l).filter p |>.map f).length ≤
    ((l.take t).filter p).length), List.length_map, Nat.sub_self, List.getElem?_cons_zero] at hi
  rw [List.getElem?_range' (by omega)] at hi
  have := Option.some.inj hi
  omega

theorem G8_proof (d : FileDiff) (ctx : Usize) (hd : Rebuilds d)
    (hsize : 2 * (d.removed.val.length + d.added.val.length) ≤ Usize.max) :
    number_rows (alloc.vec.Vec.deref d.removed) (alloc.vec.Vec.deref d.added) ⦃ rows =>
      cut_rows (alloc.vec.Vec.deref rows) ctx ⦃ out =>
        CutGood rows.val ctx.val out.val ∧
        (∀ h q, (h, q) ∈ out.val.zip out.val.tail → isHeader h = true →
          (showsOld q = true → h.old = q.old) ∧ (showsNew q = true → h.new = q.new)) ⦄ ⦄ := by
  apply WP.spec_mono (number_rows_file d hd (by omega))
  intro rows ⟨hlen, hgood, hO, hN, _⟩
  have hnh : ∀ r ∈ rows.val, isHeader r = false := by
    intro r hr
    have := hgood r hr
    unfold RowNames at this
    cases hk : r.kind <;> simp_all [isHeader]
  apply WP.spec_mono (cut_rows_spec (alloc.vec.Vec.deref rows) ctx (by simp; omega) (by simpa using hnh))
  intro out hc
  simp only [deref_val] at hc
  refine ⟨hc, fun h q hhq hh => ?_⟩
  obtain ⟨_, t, hq, ho, hn⟩ := hc.2.2.2.2.2.2 h q hhq hh
  refine ⟨fun hs => UScalar.eq_of_val_eq ?_, fun hs => UScalar.eq_of_val_eq ?_⟩
  · rw [ho, number_at rows.val showsOld (·.old.val) _ hO t q hq hs]; rfl
  · rw [hn, number_at rows.val showsNew (·.new.val) _ hN t q hq hs]; rfl

end GuideCheck.Cut
