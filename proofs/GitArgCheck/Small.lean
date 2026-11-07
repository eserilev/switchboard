import GitArgCheck.Spec

/-! # The small functions: byte compares, pieces, digits, and `rel_ok` -/

open Aeneas Aeneas.Std Result git_argcheck GitArgCheck.Spec

namespace GitArgCheck.Small

theorem take_drop_eq_iff (a b : List U8) (f : Nat) :
    (a.drop f).take b.length = b ↔ ∀ j < b.length, a[f + j]? = b[j]? := by
  constructor
  · intro h j hj
    rw [← h]; simp [hj]
  · intro h
    apply List.ext_getElem?
    intro j
    by_cases hj : j < b.length
    · simp [hj, h j hj]
    · simp [hj]

theorem equal_at_loop_spec (a : Slice U8) (fr : Usize) (b : Slice U8) (i : Usize)
    (hfit : fr.val + b.val.length ≤ a.val.length) (hi : i.val ≤ b.val.length)
    (hpre : ∀ j < i.val, a.val[fr.val + j]? = b.val[j]?) :
    equal_at_loop a fr b i ⦃ r => (r = true ↔ (a.val.drop fr.val).take b.val.length = b.val) ⦄ := by
  unfold equal_at_loop
  apply loop.spec_decr_nat (fun i => b.val.length - i.val)
    (fun i => i.val ≤ b.val.length ∧ ∀ j < i.val, a.val[fr.val + j]? = b.val[j]?) _ _ _ _ ⟨hi, hpre⟩
  rintro i ⟨hi, hpre⟩
  unfold equal_at_loop.body
  rw [take_drop_eq_iff]
  step*
  · -- A byte differs.
    simp only [Bool.false_eq_true, false_iff, not_forall]
    refine ⟨i.val, by scalar_tac, ?_⟩
    have h1 : i.val < b.val.length := by scalar_tac
    have h2 : fr.val + i.val < a.val.length := by scalar_tac
    rw [List.getElem?_eq_getElem h1, List.getElem?_eq_getElem h2]
    intro h
    simp only [Option.some.injEq] at h
    simp_all
  · refine ⟨by scalar_tac, ?_, by scalar_tac⟩
    intro j hj
    by_cases hji : j < i.val
    · exact hpre j hji
    · have hj' : j = i.val := by scalar_tac
      subst hj'
      have h1 : i.val < b.val.length := by scalar_tac
      have h2 : fr.val + i.val < a.val.length := by scalar_tac
      rw [List.getElem?_eq_getElem h1, List.getElem?_eq_getElem h2]
      simp only [Option.some.injEq]
      apply UScalar.eq_of_val_eq
      simp_all

@[step]
theorem equal_at_spec (a : Slice U8) (fr t : Usize) (b : Slice U8)
    (h1 : fr.val ≤ t.val) (h2 : t.val ≤ a.val.length) :
    equal_at a fr t b ⦃ r => (r = true ↔ (a.val.drop fr.val).take (t.val - fr.val) = b.val) ⦄ := by
  unfold equal_at
  step*
  have hl : t.val - fr.val = b.val.length := by scalar_tac
  rw [hl]
  exact equal_at_loop_spec a fr b 0#usize (by scalar_tac) (by simp) (by simp)

theorem beq_u8 (x y : U8) : (x == y) = decide (x = y) := by
  by_cases h : x = y <;> simp [h]

/-- No `sep` in `p[s..k)`. -/
def NoSep (p : List U8) (sep : U8) (s k : Nat) : Prop := ∀ j, s ≤ j → j < k → p[j]? ≠ some sep

theorem splitOn_drop_step (p : List U8) (sep : U8) (s k : Nat) (hsk : s ≤ k) (hk : k < p.length)
    (hsep : p[k]? = some sep) (hno : NoSep p sep s k) :
    (p.drop s).splitOn sep = (p.drop s).take (k - s) :: (p.drop (k + 1)).splitOn sep := by
  have hsplit : p.drop s = (p.drop s).take (k - s) ++ sep :: p.drop (k + 1) := by
    apply List.ext_getElem?
    intro j
    by_cases hj : j < k - s
    · rw [List.getElem?_append_left (by simp; omega)]
      simp [hj]
    · rw [List.getElem?_append_right (by simp; omega)]
      have hl : ((p.drop s).take (k - s)).length = k - s := by simp; omega
      rw [hl]
      by_cases hj2 : j = k - s
      · subst hj2; simp; rw [← hsep]; congr 1; omega
      · have : j - (k - s) = (j - (k - s) - 1) + 1 := by omega
        rw [this, List.getElem?_cons_succ]
        simp only [List.getElem?_drop]
        congr 1; omega
  conv_lhs => rw [hsplit]
  rw [List.splitOn_eq_splitOnP, List.splitOnP_append_cons_of_forall_mem]
  · rfl
  · intro x hx
    rw [List.mem_iff_getElem?] at hx
    obtain ⟨j, hj⟩ := hx
    have hjl : j < k - s := by
      by_contra hc
      rw [List.getElem?_eq_none (by simp; omega)] at hj
      simp at hj
    simp only [List.getElem?_take, hjl, if_true, List.getElem?_drop] at hj
    have := hno (s + j) (by omega) (by omega)
    rw [beq_u8]
    simp only [decide_eq_false_iff_not]
    intro h; subst h; exact this hj
  · simp

theorem splitOn_drop_end (p : List U8) (sep : U8) (s : Nat) (hno : NoSep p sep s p.length) :
    (p.drop s).splitOn sep = [p.drop s] := by
  rw [List.splitOn_eq_splitOnP, List.splitOnP_eq_singleton]
  intro x hx
  rw [List.mem_iff_getElem?] at hx
  obtain ⟨j, hj⟩ := hx
  simp only [List.getElem?_drop] at hj
  have hjl : s + j < p.length := by
    by_contra hc
    rw [List.getElem?_eq_none (by omega)] at hj
    simp at hj
  have := hno (s + j) (by omega) hjl
  rw [beq_u8]
  simp only [decide_eq_false_iff_not]
  intro h; subst h; exact this hj


theorem has_piece_loop_spec (p : Slice U8) (sep : U8) (w : Slice U8) (start k : Usize)
    (hsk : start.val ≤ k.val) (hk : k.val ≤ p.val.length) (hno : NoSep p.val sep start.val k.val)
    (W : List (List U8)) (hW : p.val.splitOn sep = W ++ (p.val.drop start.val).splitOn sep) (hw : w.val ∉ W) :
    has_piece_loop p sep w start k ⦃ r => (r = true ↔ w.val ∈ p.val.splitOn sep) ⦄ := by
  unfold has_piece_loop
  apply loop.spec_decr_nat (fun (st : Usize × Usize) => p.val.length - st.2.val)
    (fun (st : Usize × Usize) => st.1.val ≤ st.2.val ∧ st.2.val ≤ p.val.length ∧
      NoSep p.val sep st.1.val st.2.val ∧
      ∃ W : List (List U8), p.val.splitOn sep = W ++ (p.val.drop st.1.val).splitOn sep ∧ w.val ∉ W) _ _ _ _
    ⟨hsk, hk, hno, W, hW, hw⟩
  rintro ⟨s, k⟩ ⟨hsk, hk, hno, W, hW, hw⟩
  simp only at hsk hk hno hW
  unfold has_piece_loop.body
  step*
  · -- The piece before this `sep` is `w`.
    have hkl : k.val < p.val.length := by scalar_tac
    have hsep : p.val[k.val]? = some sep := by rw [List.getElem?_eq_getElem hkl]; simp_all
    rw [hW, splitOn_drop_step p.val sep s.val k.val hsk hkl hsep hno]
    simp_all
  · -- Another piece: go on after this `sep`.
    have hkl : k.val < p.val.length := by scalar_tac
    have hsep : p.val[k.val]? = some sep := by rw [List.getElem?_eq_getElem hkl]; simp_all
    refine ⟨le_refl _, by scalar_tac, fun j h1 h2 => absurd h2 (by omega), ?_, by scalar_tac⟩
    refine ⟨W ++ [(p.val.drop s.val).take (k.val - s.val)], ?_, ?_⟩
    · rw [hW, splitOn_drop_step p.val sep s.val k.val hsk hkl hsep hno, start1_post]; simp
    · simp only [List.mem_append, List.mem_singleton, not_or]
      exact ⟨hw, fun h => by simp_all⟩
  · -- Not a `sep`: the piece grows.
    have hkl : k.val < p.val.length := by scalar_tac
    refine ⟨by scalar_tac, by scalar_tac, fun j h1 h2 => ?_, ⟨W, hW, hw⟩, by scalar_tac⟩
    by_cases hj : j < k.val
    · exact hno j h1 hj
    · have : j = k.val := by scalar_tac
      subst this
      rw [List.getElem?_eq_getElem hkl]
      simp_all
  · -- The last piece.
    have hkl : k.val = p.val.length := by scalar_tac
    have hno' : NoSep p.val sep s.val p.val.length := by rw [← hkl]; exact hno
    rw [hW, splitOn_drop_end p.val sep s.val hno', b_post]
    have : (p.val.drop s.val).take (p.len.val - s.val) = p.val.drop s.val := by
      apply List.take_of_length_le; simp
    rw [this]
    simp only [List.mem_append, List.mem_singleton]
    constructor
    · intro h; right; exact h.symm
    · rintro (h | h)
      · exact absurd h hw
      · exact h.symm

@[step]
theorem has_piece_spec (p : Slice U8) (sep : U8) (w : Slice U8) :
    has_piece p sep w ⦃ r => (r = true ↔ w.val ∈ p.val.splitOn sep) ⦄ :=
  has_piece_loop_spec p sep w 0#usize 0#usize (by simp) (by simp)
    (fun j h1 h2 => absurd h2 (by simp)) [] (by simp) (by simp)

@[step]
theorem has_byte_spec (p : Slice U8) (x : U8) :
    has_byte p x ⦃ r => (r = true ↔ x ∈ p.val) ⦄ := by
  unfold has_byte has_byte_loop
  apply loop.spec_decr_nat (fun i => p.val.length - i.val)
    (fun i => i.val ≤ p.val.length ∧ ∀ j < i.val, p.val[j]? ≠ some x) _ _ _ _
    ⟨by simp, fun j hj => absurd hj (by simp)⟩
  rintro i ⟨hi, hall⟩
  unfold has_byte_loop.body
  step*
  · simp only [true_iff]
    have hl : i.val < p.val.length := by scalar_tac
    rw [List.mem_iff_getElem]
    exact ⟨i.val, hl, by simp_all⟩
  · refine ⟨by scalar_tac, fun j hj => ?_, by scalar_tac⟩
    by_cases hji : j < i.val
    · exact hall j hji
    · have : j = i.val := by scalar_tac
      subst this
      have hl : i.val < p.val.length := by scalar_tac
      rw [List.getElem?_eq_getElem hl]
      simp_all
  · simp only [Bool.false_eq_true, false_iff]
    intro hx
    rw [List.mem_iff_getElem?] at hx
    obtain ⟨j, hj⟩ := hx
    have : j < p.val.length := by
      by_contra hc
      rw [List.getElem?_eq_none (by omega)] at hj
      simp at hj
    exact hall j (by scalar_tac) hj

/-- A digit byte, as numbers. -/
def isDigit (x : U8) : Prop := 48 ≤ x.val ∧ x.val ≤ 57

theorem digits_at_loop_spec (a : Slice U8) (t i : Usize) (ht : t.val ≤ a.val.length) (hi : i.val ≤ t.val) :
    digits_at_loop a t i ⦃ r => (r = true ↔ ∀ j, i.val ≤ j → j < t.val → ∃ x, a.val[j]? = some x ∧ isDigit x) ⦄ := by
  unfold digits_at_loop
  apply loop.spec_decr_nat (fun i => t.val - i.val)
    (fun i' => i.val ≤ i'.val ∧ i'.val ≤ t.val ∧ ∀ j, i.val ≤ j → j < i'.val → ∃ x, a.val[j]? = some x ∧ isDigit x) _ _ _ _
    ⟨le_refl _, hi, fun j h1 h2 => absurd h2 (by omega)⟩
  rintro i' ⟨h0, hi', hall⟩
  unfold digits_at_loop.body
  step*
  · simp only [Bool.false_eq_true, false_iff, not_forall]
    have hl : i'.val < a.val.length := by scalar_tac
    refine ⟨i'.val, h0, by scalar_tac, ?_⟩
    rw [List.getElem?_eq_getElem hl]
    simp only [Option.some.injEq, exists_eq_left', isDigit, not_and, not_le]
    intro h; scalar_tac
  · simp only [Bool.false_eq_true, false_iff, not_forall]
    have hl : i'.val < a.val.length := by scalar_tac
    refine ⟨i'.val, h0, by scalar_tac, ?_⟩
    rw [List.getElem?_eq_getElem hl]
    simp only [Option.some.injEq, exists_eq_left', isDigit, not_and, not_le]
    intro h; scalar_tac
  · refine ⟨by scalar_tac, by scalar_tac, fun j h1 h2 => ?_, by scalar_tac⟩
    by_cases hj : j < i'.val
    · exact hall j h1 hj
    · have : j = i'.val := by scalar_tac
      subst this
      have hl : i'.val < a.val.length := by scalar_tac
      rw [List.getElem?_eq_getElem hl]
      simp only [Option.some.injEq, exists_eq_left', isDigit]
      constructor <;> scalar_tac

theorem digit_iff (x : U8) : x ∈ bytes "0123456789" ↔ isDigit x := by
  have : bytes "0123456789" = [48#u8, 49#u8, 50#u8, 51#u8, 52#u8, 53#u8, 54#u8, 55#u8, 56#u8, 57#u8] := by decide
  rw [this]
  simp only [List.mem_cons, List.not_mem_nil, or_false, UScalar.eq_equiv, isDigit]
  simp
  omega

theorem forall_mem_take_drop (a : List U8) (P : U8 → Prop) (f n : Nat) (h : f + n ≤ a.length) :
    (∀ x ∈ (a.drop f).take n, P x) ↔ ∀ j, f ≤ j → j < f + n → ∃ x, a[j]? = some x ∧ P x := by
  constructor
  · intro hall j h1 h2
    refine ⟨a[j]'(by omega), by simp, hall _ ?_⟩
    rw [List.mem_iff_getElem]
    refine ⟨j - f, by simp; omega, ?_⟩
    simp only [List.getElem_take, List.getElem_drop]
    congr 1; omega
  · intro hall x hx
    rw [List.mem_iff_getElem] at hx
    obtain ⟨k, hk, hx⟩ := hx
    simp only [List.length_take, List.length_drop] at hk
    obtain ⟨y, hy, hp⟩ := hall (f + k) (by omega) (by omega)
    simp only [List.getElem_take, List.getElem_drop] at hx
    rw [List.getElem?_eq_getElem (by omega)] at hy
    simp only [Option.some.injEq] at hy
    rw [← hx, hy]; exact hp

@[step]
theorem digits_at_spec (a : Slice U8) (fr t : Usize) (ht : t.val ≤ a.val.length) :
    digits_at a fr t ⦃ r => (r = true ↔ Digits ((a.val.drop fr.val).take (t.val - fr.val))) ⦄ := by
  unfold digits_at
  step*
  · simp only [Bool.false_eq_true, false_iff, Digits, not_and]
    intro h; exfalso; apply h
    apply List.eq_nil_of_length_eq_zero; simp; scalar_tac
  · apply WP.spec_mono (digits_at_loop_spec a t fr ht (by scalar_tac))
    intro r hr
    rw [hr, Digits]
    have hne : (a.val.drop fr.val).take (t.val - fr.val) ≠ [] := by
      intro h; have := congrArg List.length h; simp at this; scalar_tac
    simp only [hne, ne_eq, not_false_eq_true, true_and, digit_iff]
    rw [forall_mem_take_drop _ _ _ _ (by scalar_tac)]
    have : fr.val + (t.val - fr.val) = t.val := by scalar_tac
    rw [this]

@[step]
theorem is_digits_spec (s : Slice U8) : is_digits s ⦃ r => (r = true ↔ Digits s.val) ⦄ := by
  unfold is_digits
  step*
  simp_all

@[step]
theorem find_eq_spec (a : Slice U8) (fr : Usize) (h : fr.val ≤ a.val.length) :
    find_eq a fr ⦃ r => fr.val ≤ r.val ∧ r.val ≤ a.val.length ∧
      (∀ j, fr.val ≤ j → j < r.val → a.val[j]? ≠ some 61#u8) ∧
      (r.val < a.val.length → a.val[r.val]? = some 61#u8) ⦄ := by
  unfold find_eq find_eq_loop
  apply loop.spec_decr_nat (fun i => a.val.length - i.val)
    (fun i => fr.val ≤ i.val ∧ i.val ≤ a.val.length ∧ ∀ j, fr.val ≤ j → j < i.val → a.val[j]? ≠ some 61#u8) _ _ _ _
    ⟨le_refl _, h, fun j h1 h2 => absurd h2 (by omega)⟩
  rintro i ⟨h0, hi, hall⟩
  unfold find_eq_loop.body
  step*
  · refine ⟨h0, hi, hall, fun hl => ?_⟩
    rw [List.getElem?_eq_getElem hl]; simp_all
  · refine ⟨by scalar_tac, by scalar_tac, fun j h1 h2 => ?_, by scalar_tac⟩
    by_cases hj : j < i.val
    · exact hall j h1 hj
    · have : j = i.val := by scalar_tac
      subst this
      have hl : i.val < a.val.length := by scalar_tac
      rw [List.getElem?_eq_getElem hl]; simp_all

@[step]
theorem trim_percent_spec (a : Slice U8) (fr : Usize) (h : fr.val ≤ a.val.length) :
    trim_percent a fr ⦃ r => fr.val ≤ r.val ∧ r.val ≤ a.val.length ∧
      (∀ j, r.val ≤ j → j < a.val.length → a.val[j]? = some 37#u8) ∧
      (fr.val < r.val → a.val[r.val - 1]? ≠ some 37#u8) ⦄ := by
  unfold trim_percent trim_percent_loop
  apply loop.spec_decr_nat (fun n => n.val)
    (fun n => fr.val ≤ n.val ∧ n.val ≤ a.val.length ∧ ∀ j, n.val ≤ j → j < a.val.length → a.val[j]? = some 37#u8) _ _ _ _
    ⟨h, by simp, fun j h1 h2 => absurd h2 (by scalar_tac)⟩
  rintro n ⟨h0, hn, hall⟩
  unfold trim_percent_loop.body
  step*
  · refine ⟨h0, hn, hall, fun _ => ?_⟩
    have hl : n.val - 1 < a.val.length := by scalar_tac
    rw [List.getElem?_eq_getElem hl]
    intro hc
    simp only [Option.some.injEq] at hc
    have : i1 = 37#u8 := by rw [i1_post]; simp only [i_post1]; exact hc
    simp_all
  · refine ⟨by scalar_tac, by scalar_tac, fun j h1 h2 => ?_, by scalar_tac⟩
    by_cases hj : n.val ≤ j
    · exact hall j hj h2
    · have : j = i.val := by scalar_tac
      subst this
      have hl : i.val < a.val.length := by scalar_tac
      rw [List.getElem?_eq_getElem hl]; simp only [Option.some.injEq]
      apply UScalar.eq_of_val_eq; simp_all

@[simp] theorem byte_slash : byte '/' = 47#u8 := by decide
@[simp] theorem byte_tilde : byte '~' = 126#u8 := by decide
@[simp] theorem byte_nul : byte '\x00' = 0#u8 := by decide
@[simp] theorem byte_nl : byte '\n' = 10#u8 := by decide
@[simp] theorem byte_dash : byte '-' = 45#u8 := by decide
@[simp] theorem byte_eq : byte '=' = 61#u8 := by decide
@[simp] theorem byte_pct : byte '%' = 37#u8 := by decide
@[simp] theorem bytes_dotdot : bytes ".." = [46#u8, 46#u8] := by decide
@[simp] theorem bytes_dashdash : bytes "--" = [45#u8, 45#u8] := by decide

theorem head_eq (l : List U8) (h : l ≠ []) (x : U8) : l.head? = some x ↔ l[0]! = x := by
  cases l with
  | nil => exact absurd rfl h
  | cons y ys => simp

@[step]
theorem rel_ok_spec (p : Slice U8) :
    rel_ok p ⦃ r => (r = true ↔ p.val ≠ [] ∧ p.val.head? ≠ some (byte '/') ∧
      p.val.head? ≠ some (byte '~') ∧ bytes ".." ∉ p.val.splitOn (byte '/') ∧ byte '\x00' ∉ p.val) ⦄ := by
  unfold rel_ok
  obtain ⟨l, hl⟩ := p
  cases l with
  | nil => step*
  | cons y ys =>
    step*
    all_goals simp_all [Slice.length, Array.repeat, List.replicate]

end GitArgCheck.Small
