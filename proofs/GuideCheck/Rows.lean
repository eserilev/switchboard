import GuideCheck.Meaning

/-! # The diff rows: the walk numbers every line once -/

open Aeneas Aeneas.Std Result guide_check GuideCheck.Spec GuideCheck.Small

namespace GuideCheck.Rows

/-- What the kind of the next row tells about the masks. -/
@[step]
theorem next_kind_spec (removed added : Slice Bool) (i j : Usize) :
    next_kind removed added i j ⦃ k =>
      (k = .Removed → i.val < removed.val.length ∧ removed.val[i.val]? = some true) ∧
      (k = .Added → j.val < added.val.length ∧ added.val[j.val]? = some true) ∧
      (k = .Same → i.val < removed.val.length ∧ j.val < added.val.length ∧
        removed.val[i.val]? = some false ∧ added.val[j.val]? = some false) ∧
      (k = .Header → removed.val[i.val]? ≠ some true ∧ added.val[j.val]? ≠ some true ∧
        ¬ (i.val < removed.val.length ∧ j.val < added.val.length)) ⦄ := by
  unfold next_kind
  step*
  all_goals (refine ⟨?_, ?_, ?_, ?_⟩ <;> intro hk <;> simp_all [List.getElem?_eq_some_iff] <;> scalar_tac)

/-- The kept items of a list from index `i` on. -/
def restL {α : Type} (ls : List α) (m : List Bool) (i : Nat) : List α := keep (ls.drop i) (m.drop i)

theorem restL_skip {α : Type} (ls : List α) (m : List Bool) (h : m.length = ls.length) (i : Nat)
    (hi : i < m.length) (ht : m[i]? = some true) : restL ls m i = restL ls m (i + 1) :=
  keep_drop_skip ls m h 1 i (by omega) (fun t h1 h2 => by rw [show t = i by omega]; exact ht)

theorem restL_kept {α : Type} (ls : List α) (m : List Bool) (h : m.length = ls.length) (i : Nat)
    (hi : i < m.length) (hf : m[i]? = some false) :
    restL ls m i = ls[i]'(by omega) :: restL ls m (i + 1) :=
  keep_drop_kept ls m h i hi hf

theorem restL_end {α : Type} (ls : List α) (m : List Bool) (i : Nat) (hi : m.length ≤ i) :
    restL ls m i = [] := keep_drop_end ls m i hi

/-- What the walk has done when it is at old index `i` and new index `j`, on a file that
rebuilds: every row so far names the right lines, the rows so far number the first `i`
old lines and the first `j` new lines, and the kept lines left are equal. -/
def Walked (oldL newL : List (List U8)) (rm ad : List Bool) (out : List Row) (i j : Nat) : Prop :=
  (∀ r ∈ out, RowNames oldL newL rm ad r) ∧
  (out.filter showsOld).map (·.old.val) = List.range' 1 i ∧
  (out.filter showsNew).map (·.new.val) = List.range' 1 j ∧
  restL oldL rm i = restL newL ad j

theorem walked_removed (oldL newL : List (List U8)) (rm ad : List Bool) (ho : rm.length = oldL.length)
    (out : List Row) (i j : Nat) (hw : Walked oldL newL rm ad out i j)
    (hi : i < rm.length) (ht : rm[i]? = some true)
    (r : Row) (hk : r.kind = .Removed) (hro : r.old.val = i + 1) (hrn : r.new.val = 0) :
    Walked oldL newL rm ad (out ++ [r]) (i + 1) j := by
  obtain ⟨hg, hO, hN, hR⟩ := hw
  refine ⟨fun x hx => ?_, ?_, ?_, ?_⟩
  · rcases List.mem_append.mp hx with hx | hx
    · exact hg x hx
    · rw [List.mem_singleton.mp hx]
      simp only [RowNames, hk, LineIs, hro, hrn]
      exact ⟨⟨by omega, by omega, by simpa using ht⟩, trivial⟩
  · simp [List.filter_append, showsOld, hk, hO, hro, List.range'_concat, Nat.add_comm]
  · simp [List.filter_append, showsNew, hk, hN]
  · rw [← restL_skip oldL rm ho i hi ht]; exact hR

theorem walked_added (oldL newL : List (List U8)) (rm ad : List Bool) (hn : ad.length = newL.length)
    (out : List Row) (i j : Nat) (hw : Walked oldL newL rm ad out i j)
    (hj : j < ad.length) (ht : ad[j]? = some true)
    (r : Row) (hk : r.kind = .Added) (hro : r.old.val = 0) (hrn : r.new.val = j + 1) :
    Walked oldL newL rm ad (out ++ [r]) i (j + 1) := by
  obtain ⟨hg, hO, hN, hR⟩ := hw
  refine ⟨fun x hx => ?_, ?_, ?_, ?_⟩
  · rcases List.mem_append.mp hx with hx | hx
    · exact hg x hx
    · rw [List.mem_singleton.mp hx]
      simp only [RowNames, hk, LineIs, hro, hrn]
      exact ⟨trivial, by omega, by omega, by simpa using ht⟩
  · simp [List.filter_append, showsOld, hk, hO]
  · simp [List.filter_append, showsNew, hk, hN, hrn, List.range'_concat, Nat.add_comm]
  · rw [← restL_skip newL ad hn j hj ht]; exact hR

theorem walked_same (oldL newL : List (List U8)) (rm ad : List Bool)
    (ho : rm.length = oldL.length) (hn : ad.length = newL.length)
    (out : List Row) (i j : Nat) (hw : Walked oldL newL rm ad out i j)
    (hi : i < rm.length) (hj : j < ad.length) (hfi : rm[i]? = some false) (hfj : ad[j]? = some false)
    (r : Row) (hk : r.kind = .Same) (hro : r.old.val = i + 1) (hrn : r.new.val = j + 1) :
    Walked oldL newL rm ad (out ++ [r]) (i + 1) (j + 1) := by
  obtain ⟨hg, hO, hN, hR⟩ := hw
  rw [restL_kept oldL rm ho i hi hfi, restL_kept newL ad hn j hj hfj] at hR
  obtain ⟨hhead, htail⟩ := List.cons.inj hR
  refine ⟨fun x hx => ?_, ?_, ?_, htail⟩
  · rcases List.mem_append.mp hx with hx | hx
    · exact hg x hx
    · rw [List.mem_singleton.mp hx]
      simp only [RowNames, hk, LineIs, hro, hrn, Nat.add_sub_cancel]
      refine ⟨⟨by omega, by omega, hfi⟩, ⟨by omega, by omega, hfj⟩, ?_⟩
      rw [List.getElem?_eq_getElem (by omega), List.getElem?_eq_getElem (by omega), hhead]
  · simp [List.filter_append, showsOld, hk, hO, hro, List.range'_concat, Nat.add_comm]
  · simp [List.filter_append, showsNew, hk, hN, hrn, List.range'_concat, Nat.add_comm]

/-- On a file that rebuilds, the walk is never stuck. -/
theorem walked_not_stuck (oldL newL : List (List U8)) (rm ad : List Bool)
    (ho : rm.length = oldL.length) (hn : ad.length = newL.length)
    (out : List Row) (i j : Nat) (hw : Walked oldL newL rm ad out i j)
    (hleft : i < rm.length ∨ j < ad.length)
    (hti : rm[i]? ≠ some true) (htj : ad[j]? ≠ some true) (hboth : ¬ (i < rm.length ∧ j < ad.length)) :
    False := by
  have hR := hw.2.2.2
  rcases hleft with h | h
  · have hfi : rm[i]? = some false := by
      rw [List.getElem?_eq_getElem h] at hti ⊢; simpa using hti
    rw [restL_kept oldL rm ho i h hfi, restL_end newL ad j (by omega)] at hR
    exact List.cons_ne_nil _ _ hR
  · have hfj : ad[j]? = some false := by
      rw [List.getElem?_eq_getElem h] at htj ⊢; simpa using htj
    rw [restL_end oldL rm i (by omega), restL_kept newL ad hn j h hfj] at hR
    exact List.cons_ne_nil _ _ hR.symm

theorem number_rows_loop_spec (removed added : Slice Bool) (oldL newL : List (List U8))
    (ho : removed.val.length = oldL.length) (hn : added.val.length = newL.length)
    (hsize : removed.val.length + added.val.length ≤ Usize.max)
    (out : alloc.vec.Vec Row) (i j : Usize)
    (hi : i.val ≤ removed.val.length) (hj : j.val ≤ added.val.length)
    (hlen : out.val.length ≤ i.val + j.val)
    (hw : keep oldL removed.val = keep newL added.val →
      Walked oldL newL removed.val added.val out.val i.val j.val) :
    number_rows_loop removed added out i j ⦃ rows =>
      rows.val.length ≤ removed.val.length + added.val.length ∧
      (keep oldL removed.val = keep newL added.val →
        Walked oldL newL removed.val added.val rows.val removed.val.length added.val.length) ⦄ := by
  unfold number_rows_loop
  apply loop.spec_decr_nat
    (fun (st : alloc.vec.Vec Row × Usize × Usize) =>
      (removed.val.length - st.2.1.val) + (added.val.length - st.2.2.val))
    (fun (st : alloc.vec.Vec Row × Usize × Usize) =>
      st.2.1.val ≤ removed.val.length ∧ st.2.2.val ≤ added.val.length ∧
      st.1.val.length ≤ st.2.1.val + st.2.2.val ∧
      (keep oldL removed.val = keep newL added.val →
        Walked oldL newL removed.val added.val st.1.val st.2.1.val st.2.2.val))
    _ _ _ _ ⟨hi, hj, hlen, hw⟩
  rintro ⟨out, i, j⟩ ⟨hi, hj, hlen, hw⟩
  simp only at hi hj hlen hw
  unfold number_rows_loop.body
  step*
  · -- A line on both sides.
    obtain ⟨hlt1, hlt2, hf1, hf2⟩ := kind_post3 (by assumption)
    refine ⟨by scalar_tac, by scalar_tac, by simp [out1_post]; scalar_tac, fun hk => ?_, by scalar_tac⟩
    rw [out1_post, i3_post, j1_post]
    exact walked_same oldL newL _ _ ho hn _ _ _ (hw hk) hlt1 hlt2 hf1 hf2
      { kind := Kind.Same, old := i3, new := j1 } rfl i3_post j1_post
  · -- A removed line.
    obtain ⟨hlt, ht⟩ := kind_post1 (by assumption)
    refine ⟨by scalar_tac, hj, by simp [out1_post]; scalar_tac, fun hk => ?_, by scalar_tac⟩
    rw [out1_post, i3_post]
    exact walked_removed oldL newL _ _ ho _ _ _ (hw hk) hlt ht
      { kind := Kind.Removed, old := i3, new := 0#usize } rfl i3_post (by scalar_tac)
  · -- An added line.
    obtain ⟨hlt, ht⟩ := kind_post2 (by assumption)
    refine ⟨hi, by scalar_tac, by simp [out1_post]; scalar_tac, fun hk => ?_, by scalar_tac⟩
    rw [out1_post, j1_post]
    exact walked_added oldL newL _ _ hn _ _ _ (hw hk) hlt ht
      { kind := Kind.Added, old := 0#usize, new := j1 } rfl (by scalar_tac) j1_post
  · -- Stuck: only when the file does not rebuild.
    obtain ⟨h1, h2, h3⟩ := kind_post4 (by assumption)
    refine ⟨by scalar_tac, fun hk => ?_⟩
    have hleft := b_post.mp (by assumption)
    simp only [Slice.len_val] at hleft
    exact (walked_not_stuck oldL newL _ _ ho hn _ _ _ (hw hk) hleft h1 h2 h3).elim

theorem number_rows_spec (removed added : Slice Bool) (oldL newL : List (List U8))
    (ho : removed.val.length = oldL.length) (hn : added.val.length = newL.length)
    (hsize : removed.val.length + added.val.length ≤ Usize.max) :
    number_rows removed added ⦃ rows =>
      rows.val.length ≤ removed.val.length + added.val.length ∧
      (keep oldL removed.val = keep newL added.val →
        Walked oldL newL removed.val added.val rows.val removed.val.length added.val.length) ⦄ := by
  unfold number_rows
  exact number_rows_loop_spec removed added oldL newL ho hn hsize _ 0#usize 0#usize
    (by simp) (by simp) (by simp)
    (fun hk => ⟨by simp, by simp, by simp, by simpa [restL] using hk⟩)

/-- `number_rows` never panics, and gives at most one row per line. -/
theorem number_rows_safe (removed added : Slice Bool)
    (hsize : removed.val.length + added.val.length ≤ Usize.max) :
    number_rows removed added ⦃ rows => rows.val.length ≤ removed.val.length + added.val.length ⦄ :=
  WP.spec_mono (number_rows_spec removed added (List.replicate removed.val.length [])
    (List.replicate added.val.length []) (by simp) (by simp) hsize) (fun _ h => h.1)

/-- `Accept` gives `Rebuilds` for every file. -/
theorem accept_rebuilds (files : List FileDiff) (spans : List Span) (pad f : Nat) (d : FileDiff)
    (h : Accept files spans pad) (hd : files[f]? = some d) : Rebuilds d := by
  obtain ⟨h1, h2, h3, _, _⟩ := h.1 f d hd
  exact ⟨h1, h2, h3⟩

/-- The rows of a file that rebuilds. -/
theorem number_rows_file (d : FileDiff) (hd : Rebuilds d)
    (hsize : d.removed.val.length + d.added.val.length ≤ Usize.max) :
    number_rows (alloc.vec.Vec.deref d.removed) (alloc.vec.Vec.deref d.added) ⦃ rows =>
      rows.val.length ≤ d.removed.val.length + d.added.val.length ∧
      Walked (lines d.old) (lines d.new) d.removed.val d.added.val rows.val
        d.old.val.length d.new.val.length ⦄ := by
  obtain ⟨h1, h2, h3⟩ := hd
  apply WP.spec_mono (number_rows_spec (alloc.vec.Vec.deref d.removed) (alloc.vec.Vec.deref d.added)
    (lines d.old) (lines d.new) (by simp [lines, h1]) (by simp [lines, h2]) (by simpa using hsize))
  intro rows h
  have hw := h.2 (by simpa using h3)
  simp only [deref_val] at h hw
  rw [h1, h2] at hw
  exact ⟨h.1, hw⟩

/-- Numbers 1, 2, …, n, each looked up in a list of length n, give the list. -/
theorem map_range_getD {α : Type} (l : List α) (x : α) :
    (List.range' 1 l.length).map (fun n => l.getD (n - 1) x) = l := by
  apply List.ext_getElem (by simp)
  intro t h1 h2
  simp [List.getElem_range', List.getElem?_eq_getElem h2]

theorem G5_proof (d : FileDiff) (hd : Rebuilds d)
    (hsize : d.removed.val.length + d.added.val.length ≤ Usize.max) :
    number_rows (alloc.vec.Vec.deref d.removed) (alloc.vec.Vec.deref d.added) ⦃ rows =>
      ∀ r ∈ rows.val, RowGood d r ⦄ :=
  WP.spec_mono (number_rows_file d hd hsize) (fun _ h => h.2.1)

theorem G7_proof (d : FileDiff) (hd : Rebuilds d)
    (hsize : d.removed.val.length + d.added.val.length ≤ Usize.max) :
    number_rows (alloc.vec.Vec.deref d.removed) (alloc.vec.Vec.deref d.added) ⦃ rows =>
      (rows.val.filter showsOld).map (·.old.val) = List.range' 1 d.old.val.length ∧
      (rows.val.filter showsNew).map (·.new.val) = List.range' 1 d.new.val.length ⦄ :=
  WP.spec_mono (number_rows_file d hd hsize) (fun _ h => ⟨h.2.2.1, h.2.2.2.1⟩)

theorem G6_proof (d : FileDiff) (hd : Rebuilds d)
    (hsize : d.removed.val.length + d.added.val.length ≤ Usize.max) :
    number_rows (alloc.vec.Vec.deref d.removed) (alloc.vec.Vec.deref d.added) ⦃ rows =>
      (rows.val.filter showsOld).map (oldText d) = lines d.old ∧
      (rows.val.filter showsNew).map (newText d) = lines d.new ⦄ := by
  apply WP.spec_mono (G7_proof d hd hsize)
  intro rows ⟨hO, hN⟩
  have eO : oldText d = (fun n => (lines d.old).getD (n - 1) []) ∘ (fun r : Row => r.old.val) := rfl
  have eN : newText d = (fun n => (lines d.new).getD (n - 1) []) ∘ (fun r : Row => r.new.val) := rfl
  rw [eO, eN, ← List.map_map, ← List.map_map, hO, hN]
  have lo : d.old.val.length = (lines d.old).length := by simp [lines]
  have ln : d.new.val.length = (lines d.new).length := by simp [lines]
  rw [lo, ln, map_range_getD, map_range_getD]
  exact ⟨rfl, rfl⟩

end GuideCheck.Rows
