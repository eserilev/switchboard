import GuideCheck.Check

/-! # What an accepted guide means for you -/

open Aeneas Aeneas.Std guide_check GuideCheck.Spec

namespace GuideCheck.Meaning

theorem keep_all_kept {α : Type} : ∀ (xs : List α) (bs : List Bool), bs.length = xs.length →
    (∀ k : Nat, bs[k]? ≠ some true) → keep xs bs = xs
  | [], [], _, _ => rfl
  | x :: xs, b :: bs, h, hall => by
    have hb : b = false := by
      have := hall 0
      cases b <;> simp_all
    subst hb
    simp only [keep, Bool.false_eq_true, if_false]
    exact congrArg _ (keep_all_kept xs bs (by simpa using h) (fun k => by simpa using hall (k + 1)))
  | [], _ :: _, h, _ => by simp at h
  | _ :: _, [], h, _ => by simp at h

/-- In an accepted guide, a file where the diff marks no line did not change at all. -/
theorem unmarked_file_is_unchanged (files : List FileDiff) (spans : List Span) (pad : Nat)
    (h : Accept files spans pad) (f : Nat) (d : FileDiff) (hd : files[f]? = some d)
    (hr : ∀ k : Nat, d.removed.val[k]? ≠ some true) (ha : ∀ k : Nat, d.added.val[k]? ≠ some true) :
    lines d.old = lines d.new := by
  obtain ⟨h1, h2, hk, _, _⟩ := h.1 f d hd
  rw [keep_all_kept _ _ (by simp [lines, h1]) hr, keep_all_kept _ _ (by simp [lines, h2]) ha] at hk
  exact hk

/-- In an accepted guide, every line that the diff adds or removes is in a span. -/
theorem every_change_is_covered (files : List FileDiff) (spans : List Span) (pad : Nat)
    (h : Accept files spans pad) (f : Nat) (d : FileDiff) (hd : files[f]? = some d) :
    (∀ k, d.removed.val[k]? = some true → Covered spans f true (k + 1)) ∧
    (∀ k, d.added.val[k]? = some true → Covered spans f false (k + 1)) := by
  obtain ⟨_, _, _, hr, ha⟩ := h.1 f d hd
  exact ⟨hr, ha⟩

end GuideCheck.Meaning
