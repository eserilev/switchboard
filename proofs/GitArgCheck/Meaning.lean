import GitArgCheck.Check

/-! # What an accepted call means: no denied option, and paths stay in the tree -/

open Aeneas Aeneas.Std Result git_argcheck GitArgCheck.Spec

namespace GitArgCheck.Meaning

open GitArgCheck.Small GitArgCheck.Tables GitArgCheck.Check

set_option maxRecDepth 100000

theorem prefix_get (d a : List U8) (h : d <+: a) (j : Nat) (hj : j < d.length) : a[j]? = d[j]? := by
  obtain ⟨t, rfl⟩ := h
  exact List.getElem?_append_left hj

theorem denied_shape : ∀ d ∈ denied, (bytes d)[0]? = some 45#u8 ∧
    ((bytes d)[1]? = some 45#u8 ∨ (bytes d)[1]? = some 111#u8 ∨ (bytes d)[1]? = some 79#u8 ∨
      (bytes d)[1]? = some 99#u8) ∧ 61#u8 ∉ bytes d := by decide

theorem denied_not_dashdash : ∀ d ∈ denied, ¬ bytes d <+: bytes "--" := by decide

theorem denied_not_flags : ∀ x ∈ allFlags, ∀ d ∈ denied, ¬ bytes d <+: bytes x := by decide

theorem denied_not_values : ∀ x ∈ allValues, ∀ d ∈ denied, ¬ bytes d <+: bytes x := by decide

/-- A denied name has `-` and then one of `-oOc`, so it is no prefix of an arg whose
second byte is another one. -/
theorem denied_second (d : String) (hd : d ∈ denied) (a : List U8) (y : U8) (hy : a[1]? = some y)
    (hne : y ≠ 45#u8 ∧ y ≠ 111#u8 ∧ y ≠ 79#u8 ∧ y ≠ 99#u8) : ¬ bytes d <+: a := by
  intro hp
  obtain ⟨-, h1, -⟩ := denied_shape d hd
  have hl : 1 < (bytes d).length := by
    rcases h1 with h | h | h | h <;> exact (List.getElem?_eq_some_iff.mp h).1
  rw [← prefix_get _ _ hp 1 hl, hy] at h1
  simp only [Option.some.injEq] at h1
  rcases h1 with h | h | h | h <;> simp_all

theorem digit_ne (y : U8) (h : isDigit y) : y ≠ 45#u8 ∧ y ≠ 111#u8 ∧ y ≠ 79#u8 ∧ y ≠ 99#u8 := by
  unfold isDigit at h
  refine ⟨?_, ?_, ?_, ?_⟩ <;> intro he <;> subst he <;> simp at h

/-- No denied name is a prefix of an arg that `Good` allows. -/
theorem good_not_denied (sub a : List U8) (hg : Good sub a) : ∀ d ∈ denied, ¬ bytes d <+: a := by
  intro d hd hp
  obtain ⟨-, -, -, hA, -⟩ := hg
  rcases hA with rfl | hf | ⟨n, v, rfl, hn⟩ | ⟨-, v, rfl, hv⟩ | ⟨f, v, rfl, hv, hc⟩ | hpath
  · exact denied_not_dashdash d hd hp
  · obtain ⟨x, hx, rfl⟩ := List.mem_map.mp hf
    exact denied_not_flags x (flags_sub sub x hx) d hd hp
  · obtain ⟨x, hx, rfl⟩ := List.mem_map.mp hn
    have hxv := values_sub sub x hx
    by_cases hlen : (bytes d).length ≤ (bytes x).length
    · exact denied_not_values x hxv d hd
        (List.prefix_of_prefix_length_le hp (List.prefix_append _ _) hlen)
    · obtain ⟨-, -, h61⟩ := denied_shape d hd
      apply h61
      have := prefix_get _ _ hp (bytes x).length (by omega)
      simp only [byte_eq, List.getElem?_append_right (le_refl _), Nat.sub_self,
        List.getElem?_cons_zero] at this
      exact List.mem_of_getElem? this.symm
  · obtain ⟨hne, hall⟩ := hv
    cases v with
    | nil => exact hne rfl
    | cons y w =>
      exact denied_second d hd _ y (by simp) (digit_ne y ((digit_iff y).mp (hall y List.mem_cons_self))) hp
  · refine denied_second d hd (byte '-' :: f :: v) f (by simp) ?_ hp
    rcases hc with ⟨rfl, -⟩ | ⟨h | h, -⟩ | ⟨rfl, -⟩ | ⟨h | h, -⟩ | ⟨rfl, -⟩ | ⟨rfl, -⟩ <;>
      (try subst h) <;> simp
  · obtain ⟨h0, -, -⟩ := denied_shape d hd
    have hl : 0 < (bytes d).length := (List.getElem?_eq_some_iff.mp h0).1
    have := prefix_get _ _ hp 0 hl
    rw [h0] at this
    apply hpath
    rw [List.head?_eq_getElem?, this, byte_dash]

/-! ## Paths -/

theorem fold_keeps (st : List (List U8)) (ps : List (List U8)) (h : ∀ x ∈ ps, x ≠ bytes "..") :
    st <+: ps.foldl normStep st := by
  induction ps generalizing st with
  | nil => exact List.prefix_refl _
  | cons x xs ih =>
    simp only [List.foldl_cons]
    have hx := h x List.mem_cons_self
    have hstep : st <+: normStep st x := by
      unfold normStep
      split_ifs
      · exact List.prefix_refl _
      · exact List.prefix_append _ _
    exact hstep.trans (ih _ (fun y hy => h y (List.mem_cons_of_mem _ hy)))

/-- A path with no `..` piece, read from a folder, stays under that folder. -/
theorem norm_under (tree p : List U8) (hp : bytes ".." ∉ p.splitOn (byte '/')) :
    norm tree <+: norm (tree ++ byte '/' :: p) := by
  unfold norm
  have hs : (tree ++ byte '/' :: p).splitOn (byte '/') = tree.splitOn (byte '/') ++ p.splitOn (byte '/') :=
    List.splitOnP_append_cons tree p (by simp)
  rw [hs, List.foldl_append]
  apply fold_keeps
  intro x hx he
  subst he
  exact hp hx

end GitArgCheck.Meaning
