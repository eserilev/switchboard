import OutQ.Code.Funs

/-! # The output queue (SPEC 29) -/

open Aeneas Aeneas.Std Result outq

namespace OutQ.Queue

/-- The index of the first pane with `key`, or the pane count. -/
def idx (q : Queue) (key : U32) : Nat := q.panes.val.findIdx (fun p => decide (p.key = key))

theorem find_loop_spec (q : Queue) (key : U32) (i : Usize) (hi : i.val ≤ q.panes.val.length)
    (hpre : ∀ t < i.val, ∀ h : t < q.panes.val.length, (q.panes.val[t]'h).key ≠ key) :
    find_loop q key i ⦃ r => r.val = idx q key ⦄ := by
  unfold find_loop
  apply loop.spec_decr_nat (fun i => q.panes.val.length - i.val)
    (fun i => i.val ≤ q.panes.val.length ∧
      ∀ t < i.val, ∀ h : t < q.panes.val.length, (q.panes.val[t]'h).key ≠ key) _ _ _ _ ⟨hi, hpre⟩
  rintro j ⟨hj, hall⟩
  unfold find_loop.body
  step*
  · -- Found: the first pane with the key.
    have hlt : j.val < q.panes.val.length := by scalar_tac
    unfold idx
    rw [eq_comm, List.findIdx_eq hlt]
    rename_i hk
    constructor
    · simpa [← pq_post] using hk
    · intro t ht; simpa using hall t ht _
  · -- Not this one: move on.
    refine ⟨by scalar_tac, fun t ht h => ?_, by scalar_tac⟩
    by_cases htj : t < j.val
    · exact hall t htj h
    · have : t = j.val := by scalar_tac
      subst this
      simp_all
  · -- No pane has the key.
    have : j.val = q.panes.val.length := by scalar_tac
    unfold idx
    rw [this, eq_comm]
    apply (List.findIdx_eq_length).mpr
    intro p hp
    obtain ⟨t, ht, rfl⟩ := List.getElem_of_mem hp
    simp only [decide_eq_false_iff_not]
    exact hall t (by omega) ht

@[step]
theorem find_spec (q : Queue) (key : U32) : find q key ⦃ r => r.val = idx q key ⦄ := by
  unfold find
  exact find_loop_spec q key 0#usize (by simp) (fun t ht => absurd ht (by simp))

/-- The pane with `key`, or the new empty live pane that a push makes for it. -/
def cur (q : Queue) (key : U32) : PaneQ :=
  (q.panes.val[idx q key]?).getD { key, state := LIVE, bytes := alloc.vec.Vec.new U8 }

theorem idx_le (q : Queue) (key : U32) : idx q key ≤ q.panes.val.length := List.findIdx_le_length

theorem cur_key (q : Queue) (key : U32) : (cur q key).key = key := by
  unfold cur
  by_cases h : idx q key < q.panes.val.length
  · rw [List.getElem?_eq_getElem h]
    simp only [Option.getD_some]
    unfold idx at h ⊢
    have := List.findIdx_getElem (p := fun (x : PaneQ) => decide (x.key = key)) (w := h)
    simpa using this
  · rw [List.getElem?_eq_none (by omega)]
    rfl

@[step]
theorem fits_spec (used data budget : Usize) :
    fits used data budget ⦃ r => (r = true ↔ used.val + data.val ≤ budget.val) ⦄ := by
  unfold fits
  step*

@[step]
theorem slot_spec (q : Queue) (key : U32) (h : q.panes.val.length < Usize.max) :
    slot q key ⦃ r =>
      r.1.val = idx q key ∧ r.1.val < r.2.panes.val.length ∧ r.2.budget = q.budget ∧
      r.2.panes.val[r.1.val]? = some (cur q key) ∧
      ∀ j, j ≠ idx q key → r.2.panes.val[j]? = q.panes.val[j]? ⦄ := by
  unfold slot
  step*
  · -- A new pane at the end.
    have hlen : i.val = q.panes.val.length := by simp_all
    have hi : idx q key = q.panes.val.length := by rw [← i_post, hlen]
    have hnone : q.panes.val[idx q key]? = none := by rw [hi]; simp
    refine ⟨by simp_all, by simp_all, ?_, ?_⟩
    · rw [v_post]; simp [cur, hnone]
    · intro j hj
      rw [v_post]
      by_cases hj2 : j < q.panes.val.length
      · rw [List.getElem?_append_left hj2]
      · rw [List.getElem?_eq_none (by simp; omega), List.getElem?_eq_none (by omega)]
  · -- The pane is there.
    have hle := idx_le q key
    have hlt : i.val < q.panes.val.length := by
      have : i.val ≠ q.panes.val.length := by
        intro hc; apply ‹¬i = q.panes.len›; scalar_tac
      omega
    refine ⟨i_post, hlt, ?_, fun _ _ => trivial⟩
    unfold cur
    rw [← i_post, List.getElem?_eq_getElem hlt]
    rfl

/-- `extend_from_slice` on bytes appends the slice. -/
@[step]
theorem extend_u8_spec (v : alloc.vec.Vec U8) (s : Slice U8)
    (h : v.val.length + s.val.length ≤ Usize.max) :
    alloc.vec.Vec.extend_from_slice core.clone.CloneU8 v s ⦃ r => r.val = v.val ++ s.val ⦄ := by
  have hc : ∀ x ∈ s.val, core.clone.CloneU8.clone x = ok x := by intros; rfl
  obtain ⟨s', hs, heq⟩ := WP.spec_imp_exists (Slice.clone_spec hc)
  unfold alloc.vec.Vec.extend_from_slice
  rw [dif_pos (by scalar_tac)]
  split
  · rename_i s'' h''
    rw [hs] at h''
    simp only [ok.injEq] at h''
    subst h''
    simp [heq]
  · rename_i h''; rw [hs] at h''; cases h''
  · rename_i h''; rw [hs] at h''; cases h''

/-- What one push does to the pane `c`, with `data` and a budget. A live pane keeps
the bytes in order when they fit, else it drops all and waits. A pane that waits
does not change. -/
def PushStep (budget : Nat) (c : PaneQ) (data : List U8) (p : PaneQ) : Prop :=
  p.key = c.key ∧
  if c.state = LIVE then
    (c.bytes.val.length + data.length ≤ budget → p.state = LIVE ∧ p.bytes.val = c.bytes.val ++ data) ∧
    (budget < c.bytes.val.length + data.length → p.state = DROPPING ∧ p.bytes.val = [])
  else p = c

@[step]
theorem push_spec (q : Queue) (key : U32) (data : Slice U8) (h : q.panes.val.length < Usize.max) :
    push q key data ⦃ q' =>
      q'.budget = q.budget ∧
      (∀ j, j ≠ idx q key → q'.panes.val[j]? = q.panes.val[j]?) ∧
      ∃ p', q'.panes.val[idx q key]? = some p' ∧ PushStep q.budget.val (cur q key) data.val p' ⦄ := by
  unfold push
  step*
  all_goals
    have hp : p = cur q key := by
      have := i_post4
      rw [List.getElem?_eq_getElem i_post2] at this
      simp only [Option.some.injEq] at this
      rw [p_post1]; exact this
    have hkey := cur_key q key
    subst p_post2
    simp only [alloc.vec.Vec.set_val_eq]
    rw [← i_post1, List.getElem?_set_self i_post2]
    simp only [Option.some.injEq, exists_eq_left']
    refine ⟨i_post3, fun j hj => ?_, ?_⟩
    · rw [List.getElem?_set_ne (by omega)]; exact i_post5 j (by omega)
  · -- The bytes fit: they go after the old ones.
    subst hp
    refine ⟨rfl, ?_⟩
    rw [if_pos ‹_›]
    refine ⟨fun _ => ⟨rfl, ?_⟩, fun hlt => ?_⟩
    · rw [v_post]
    · exfalso
      have := b_post.mp ‹_›
      scalar_tac
  · -- They do not fit: all bytes drop.
    subst hp
    refine ⟨rfl, ?_⟩
    rw [if_pos ‹_›]
    refine ⟨fun hle => ?_, fun _ => ⟨rfl, rfl⟩⟩
    exfalso
    apply ‹¬b = true›
    apply b_post.mpr
    scalar_tac
  · -- A pane that waits takes nothing.
    subst hp
    exact ⟨rfl, by rw [if_neg ‹_›]⟩

theorem cur_of_lt (q : Queue) (key : U32) (h : idx q key < q.panes.val.length) :
    cur q key = q.panes.val[idx q key]'h := by
  unfold cur; rw [List.getElem?_eq_getElem h]; rfl

theorem pane_eta (p : PaneQ) : { key := p.key, state := p.state, bytes := p.bytes : PaneQ } = p := rfl

@[step]
theorem take_spec (q : Queue) (i : Usize) (h : i.val < q.panes.val.length) :
    take q i ⦃ r =>
      r.1.val = (q.panes.val[i.val]'h).bytes.val ∧ r.2.budget = q.budget ∧
      r.2.panes.val = q.panes.val.set i.val { q.panes.val[i.val]'h with bytes := alloc.vec.Vec.new U8 } ⦄ := by
  unfold take
  step*
  all_goals simp_all [alloc.vec.Vec.set_val_eq]

@[step]
theorem want_screen_spec (q : Queue) (i : Usize) (h : i.val < q.panes.val.length) :
    want_screen q i ⦃ r =>
      (r.1 = true ↔ (q.panes.val[i.val]'h).state = DROPPING) ∧ r.2.budget = q.budget ∧
      r.2.panes.val = q.panes.val.set i.val
        (if (q.panes.val[i.val]'h).state = DROPPING then { q.panes.val[i.val]'h with state := RESYNC }
         else q.panes.val[i.val]'h) ⦄ := by
  unfold want_screen
  step*
  all_goals simp_all [alloc.vec.Vec.set_val_eq]

@[step]
theorem fill_spec (q : Queue) (key : U32) (screen : alloc.vec.Vec U8) :
    fill q key screen ⦃ q' =>
      q'.budget = q.budget ∧
      q'.panes.val = (if idx q key < q.panes.val.length ∧ (cur q key).state = RESYNC
        then q.panes.val.set (idx q key) { key, state := LIVE, bytes := screen }
        else q.panes.val) ⦄ := by
  unfold fill
  step*
  · -- The pane waits for this screen.
    have hlt : idx q key < q.panes.val.length := by scalar_tac
    have hp' : q.panes.val[idx q key]? = some p := by
      rw [← i_post, p_post1]; exact List.getElem?_eq_getElem _
    have hpc : cur q key = p := by unfold cur; rw [hp']; rfl
    have hk : p.key = key := by rw [← hpc]; exact cur_key q key
    subst p_post2
    simp only [alloc.vec.Vec.set_val_eq]
    rw [if_pos ⟨hlt, by rw [hpc]; assumption⟩, ← i_post, hk]
  · -- It does not wait: nothing changes.
    have hp' : q.panes.val[idx q key]? = some p := by
      rw [← i_post, p_post1]; exact List.getElem?_eq_getElem _
    have hpc : cur q key = p := by unfold cur; rw [hp']; rfl
    subst p_post2
    simp only [alloc.vec.Vec.set_val_eq]
    rw [if_neg (fun hh => ‹¬p.state = RESYNC› (by rw [← hpc]; exact hh.2)), p_post1]
    simp

@[step]
theorem forget_spec (q : Queue) (key : U32) :
    forget q key ⦃ q' =>
      q'.budget = q.budget ∧
      q'.panes.val = (if idx q key < q.panes.val.length
        then q.panes.val.set (idx q key) { key, state := LIVE, bytes := alloc.vec.Vec.new U8 }
        else q.panes.val) ⦄ := by
  unfold forget
  step*
  · have hlt : idx q key < q.panes.val.length := by scalar_tac
    have hp' : q.panes.val[idx q key]? = some p := by
      rw [← i_post, p_post1]; exact List.getElem?_eq_getElem _
    have hpc : cur q key = p := by unfold cur; rw [hp']; rfl
    have hk : p.key = key := by rw [← hpc]; exact cur_key q key
    subst p_post2
    simp only [alloc.vec.Vec.set_val_eq]
    rw [if_pos hlt, ← i_post, hk]

/-! ## The statements of `Statements.lean`, from the specs -/

theorem drop_ne_live : DROPPING ≠ LIVE := by unfold DROPPING LIVE; decide

theorem Q1_proof (q : Queue) (key : U32) (data : Slice U8) (h : q.panes.val.length < Usize.max) :
    push q key data ⦃ q' => ∀ p', q'.panes.val[idx q key]? = some p' →
      p'.state = LIVE → p'.bytes.val.length ≤ q.budget.val ⦄ := by
  apply WP.spec_mono (push_spec q key data h)
  rintro q' ⟨_, _, p, hp, _, hstep⟩ p' hp' hlive
  rw [hp] at hp'; cases hp'
  by_cases hc : (cur q key).state = LIVE
  · rw [if_pos hc] at hstep
    by_cases hf : (cur q key).bytes.val.length + data.val.length ≤ q.budget.val
    · rw [(hstep.1 hf).2]; simp; omega
    · have := (hstep.2 (by omega)).1; rw [this] at hlive; exact absurd hlive drop_ne_live
  · rw [if_neg hc] at hstep; rw [hstep] at hlive; exact absurd hlive hc

theorem Q2_proof (q : Queue) (key : U32) (data : Slice U8) (h : q.panes.val.length < Usize.max) :
    push q key data ⦃ q' => ∀ p', q'.panes.val[idx q key]? = some p' →
      p'.state = LIVE → p'.bytes.val = (cur q key).bytes.val ++ data.val ⦄ := by
  apply WP.spec_mono (push_spec q key data h)
  rintro q' ⟨_, _, p, hp, _, hstep⟩ p' hp' hlive
  rw [hp] at hp'; cases hp'
  by_cases hc : (cur q key).state = LIVE
  · rw [if_pos hc] at hstep
    by_cases hf : (cur q key).bytes.val.length + data.val.length ≤ q.budget.val
    · exact (hstep.1 hf).2
    · have := (hstep.2 (by omega)).1; rw [this] at hlive; exact absurd hlive drop_ne_live
  · rw [if_neg hc] at hstep; rw [hstep] at hlive; exact absurd hlive hc

theorem Q3_proof (q : Queue) (key : U32) (data : Slice U8) (h : q.panes.val.length < Usize.max)
    (hc : (cur q key).state = LIVE)
    (hf : (cur q key).bytes.val.length + data.val.length ≤ q.budget.val) :
    push q key data ⦃ q' => ∃ p', q'.panes.val[idx q key]? = some p' ∧ p'.state = LIVE ⦄ := by
  apply WP.spec_mono (push_spec q key data h)
  rintro q' ⟨_, _, p, hp, _, hstep⟩
  rw [if_pos hc] at hstep
  exact ⟨p, hp, (hstep.1 hf).1⟩

theorem Q4_proof (q : Queue) (key : U32) (data : Slice U8) (h : q.panes.val.length < Usize.max)
    (hc : (cur q key).state ≠ LIVE) :
    push q key data ⦃ q' => q'.panes.val = q.panes.val ⦄ := by
  apply WP.spec_mono (push_spec q key data h)
  rintro q' ⟨_, hother, p, hp, _, hstep⟩
  rw [if_neg hc] at hstep
  -- A pane that waits is there: a new pane is live.
  have hlt : idx q key < q.panes.val.length := by
    by_contra hn
    apply hc
    unfold cur; rw [List.getElem?_eq_none (by omega)]; rfl
  apply List.ext_getElem?
  intro j
  by_cases hj : j = idx q key
  · subst hj; rw [hp, hstep]; unfold cur; rw [List.getElem?_eq_getElem hlt]; rfl
  · exact hother j hj

theorem Q5_proof (q : Queue) (key : U32) (data : Slice U8) (h : q.panes.val.length < Usize.max) :
    push q key data ⦃ q' => q'.budget = q.budget ∧
      ∀ j, j ≠ idx q key → q'.panes.val[j]? = q.panes.val[j]? ⦄ := by
  apply WP.spec_mono (push_spec q key data h)
  rintro q' ⟨hb, hother, _⟩
  exact ⟨hb, hother⟩

end OutQ.Queue
