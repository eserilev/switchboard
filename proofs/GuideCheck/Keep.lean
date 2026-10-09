import GuideCheck.Spec

/-! # Lemmas about `keep` on lists -/

namespace GuideCheck.Spec

theorem keep_nil_right {α : Type} (xs : List α) : keep xs [] = [] := by cases xs <;> rfl

theorem keep_drop_end {α : Type} (xs : List α) (bs : List Bool) (n : Nat) (hn : bs.length ≤ n) :
    keep (xs.drop n) (bs.drop n) = [] := by
  rw [List.drop_eq_nil_of_le hn]; exact keep_nil_right _

/-- Skipping masked items does not change what `keep` returns. -/
theorem keep_drop_skip {α : Type} (xs : List α) (bs : List Bool) (hlen : bs.length = xs.length) :
    ∀ (d i : Nat), i + d ≤ bs.length → (∀ t, i ≤ t → t < i + d → bs[t]? = some true) →
      keep (xs.drop i) (bs.drop i) = keep (xs.drop (i + d)) (bs.drop (i + d)) := by
  intro d
  induction d with
  | zero => intro i _ _; rfl
  | succ d ih =>
    intro i hle hall
    have hi : i < bs.length := by omega
    have hix : i < xs.length := by omega
    have hb : bs[i] = true := by
      have := hall i (le_refl _) (by omega)
      rw [List.getElem?_eq_getElem hi] at this
      exact Option.some.inj this
    rw [List.drop_eq_getElem_cons hix, List.drop_eq_getElem_cons hi, keep, hb, if_pos rfl]
    rw [show i + (d + 1) = (i + 1) + d by omega]
    exact ih (i + 1) (by omega) (fun t h1 h2 => hall t (by omega) (by omega))

/-- At a kept item, `keep` returns that item first. -/
theorem keep_drop_kept {α : Type} (xs : List α) (bs : List Bool) (hlen : bs.length = xs.length)
    (a : Nat) (ha : a < bs.length) (hf : bs[a]? = some false) :
    keep (xs.drop a) (bs.drop a) = xs[a]'(by omega) :: keep (xs.drop (a + 1)) (bs.drop (a + 1)) := by
  have hb : bs[a] = false := by
    rw [List.getElem?_eq_getElem ha] at hf; exact Option.some.inj hf
  rw [List.drop_eq_getElem_cons (show a < xs.length by omega), List.drop_eq_getElem_cons ha, keep, hb]
  simp

end GuideCheck.Spec
