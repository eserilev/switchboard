import GitArgCheck.Refs
import GitArgCheck.Tables

/-! # `github_repo`: the owner and the name of a GitHub URL -/

open Aeneas Aeneas.Std Result git_argcheck GitArgCheck.Spec GitArgCheck.RefSpec

namespace GitArgCheck.GitHub

open GitArgCheck.Small GitArgCheck.Refs GitArgCheck.Tables

set_option maxHeartbeats 1000000

/-! ## Small functions -/

@[step]
theorem find_byte_spec (a : Slice U8) (fr : Usize) (x : U8) (h : fr.val ≤ a.val.length) :
    refs.find_byte a fr x ⦃ r => fr.val ≤ r.val ∧ r.val ≤ a.val.length ∧
      (∀ j, fr.val ≤ j → j < r.val → a.val[j]? ≠ some x) ∧
      (r.val < a.val.length → a.val[r.val]? = some x) ⦄ := by
  unfold refs.find_byte refs.find_byte_loop
  apply loop.spec_decr_nat (fun i => a.val.length - i.val)
    (fun i => fr.val ≤ i.val ∧ i.val ≤ a.val.length ∧ ∀ j, fr.val ≤ j → j < i.val → a.val[j]? ≠ some x) _ _ _ _
    ⟨le_refl _, h, fun j h1 h2 => absurd h2 (by omega)⟩
  rintro i ⟨h0, hi, hall⟩
  unfold refs.find_byte_loop.body
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

/-- A byte in lower case, as a number. -/
def low (x : U8) : Nat := if 65 ≤ x.val ∧ x.val ≤ 90 then x.val + 32 else x.val

@[step]
theorem lower_spec (x : U8) : refs.lower x ⦃ y => y.val = low x ⦄ := by
  unfold refs.lower low
  step*

theorem lower_equal_at_loop_spec (u : Slice U8) (h : Usize) (w : Slice U8) (i : Usize)
    (hfit : h.val + w.val.length ≤ u.val.length) (hi : i.val ≤ w.val.length)
    (hpre : ∀ j < i.val, ∃ x c, u.val[h.val + j]? = some x ∧ w.val[j]? = some c ∧ low x = c.val) :
    refs.lower_equal_at_loop u h w i ⦃ r => (r = true ↔
      ∀ j < w.val.length, ∃ x c, u.val[h.val + j]? = some x ∧ w.val[j]? = some c ∧ low x = c.val) ⦄ := by
  unfold refs.lower_equal_at_loop
  apply loop.spec_decr_nat (fun i => w.val.length - i.val)
    (fun i => i.val ≤ w.val.length ∧
      ∀ j < i.val, ∃ x c, u.val[h.val + j]? = some x ∧ w.val[j]? = some c ∧ low x = c.val) _ _ _ _ ⟨hi, hpre⟩
  rintro i ⟨hi, hpre⟩
  unfold refs.lower_equal_at_loop.body
  step*
  · simp only [Bool.false_eq_true, false_iff, not_forall]
    have h1 : i.val < w.val.length := by scalar_tac
    have h2 : h.val + i.val < u.val.length := by scalar_tac
    refine ⟨i.val, h1, ?_⟩
    rintro ⟨x, c, hx, hc, hxc⟩
    rw [List.getElem?_eq_getElem h2, Option.some.injEq] at hx
    rw [List.getElem?_eq_getElem h1, Option.some.injEq] at hc
    subst hx hc
    have : i4 = i5 := by apply UScalar.eq_of_val_eq; simp_all
    simp_all
  · refine ⟨by scalar_tac, fun j hj => ?_, by scalar_tac⟩
    by_cases hji : j < i.val
    · exact hpre j hji
    · have hj' : j = i.val := by scalar_tac
      subst hj'
      have h1 : i.val < w.val.length := by scalar_tac
      have h2 : h.val + i.val < u.val.length := by scalar_tac
      refine ⟨_, _, List.getElem?_eq_getElem h2, List.getElem?_eq_getElem h1, ?_⟩
      have : i4 = i5 := by apply UScalar.eq_of_val_eq; simp_all
      simp_all

@[step]
theorem lower_equal_at_spec (u : Slice U8) (h : Usize) (w : Slice U8) :
    refs.lower_equal_at u h w ⦃ r => (r = true ↔ h.val + w.val.length ≤ u.val.length ∧
      ∀ j < w.val.length, ∃ x c, u.val[h.val + j]? = some x ∧ w.val[j]? = some c ∧ low x = c.val) ⦄ := by
  unfold refs.lower_equal_at
  step*
  · apply WP.spec_mono (lower_equal_at_loop_spec u h w 0#usize (by scalar_tac) (by simp)
      (fun j hj => absurd hj (by simp)))
    intro r hr
    rw [hr]
    constructor
    · intro h1; exact ⟨by scalar_tac, h1⟩
    · intro h1; exact h1.2

/-! ## The host -/

def gl : List U8 := [103#u8, 105#u8, 116#u8, 104#u8, 117#u8, 98#u8, 46#u8, 99#u8, 111#u8, 109#u8]
def Gl : List U8 := [71#u8, 73#u8, 84#u8, 72#u8, 85#u8, 66#u8, 46#u8, 67#u8, 79#u8, 77#u8]

theorem b_host : bytes "github.com" = gl := by decide
theorem b_HOST : bytes "GITHUB.COM" = Gl := by decide

theorem low_pair (x c C : U8)
    (h : (97 ≤ c.val ∧ c.val ≤ 122 ∧ C.val + 32 = c.val) ∨ (c.val = 46 ∧ C.val = 46)) :
    low x = c.val ↔ x.val = c.val ∨ x.val = C.val := by
  unfold low; split_ifs <;> omega

theorem low_at (i : Nat) (hi : i < 10) (x : U8) :
    (∃ c, gl[i]? = some c ∧ low x = c.val) ↔ (some x = gl[i]? ∨ some x = Gl[i]?) := by
  have : i = 0 ∨ i = 1 ∨ i = 2 ∨ i = 3 ∨ i = 4 ∨ i = 5 ∨ i = 6 ∨ i = 7 ∨ i = 8 ∨ i = 9 := by omega
  rcases this with rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl
  all_goals
    simp only [gl, Gl, List.getElem?_cons_succ, List.getElem?_cons_zero, Option.some.injEq,
      exists_eq_left']
    simp only [UScalar.eq_equiv]
    apply low_pair; simp

theorem host_iff (t : List U8) (ht : t.length = 10) :
    (∀ j < 10, ∃ x c, t[j]? = some x ∧ gl[j]? = some c ∧ low x = c.val) ↔ Host t := by
  unfold Host
  rw [b_host, b_HOST]
  constructor
  · intro H
    refine ⟨ht, fun i hi => ?_⟩
    obtain ⟨x, c, hx, hc, hl⟩ := H i hi
    rw [hx]
    exact (low_at i hi x).mp ⟨c, hc, hl⟩
  · rintro ⟨_, H⟩ j hj
    have hjl : j < t.length := by omega
    refine ⟨t[j], ?_⟩
    have h2 := H j hj
    rw [List.getElem?_eq_getElem hjl] at h2
    have := (low_at j hj t[j]).mpr h2
    obtain ⟨c, hc, hl⟩ := this
    exact ⟨c, List.getElem?_eq_getElem hjl, hc, hl⟩

/-- The ten bytes at `h`. -/
abbrev at10 (u : List U8) (h : Nat) : List U8 := (u.drop h).take 10

theorem at10_get (u : List U8) (h j : Nat) (hj : j < 10) : (at10 u h)[j]? = u[h + j]? := by
  simp [at10, hj]

theorem at10_len (u : List U8) (h : Nat) (hl : h + 10 ≤ u.length) : (at10 u h).length = 10 := by
  simp [at10]; omega

@[step]
theorem host_at_spec (u : Slice U8) (h : Usize) :
    refs.host_at u h ⦃ r => (r = true ↔ h.val + 10 ≤ u.val.length ∧ Host (at10 u.val h.val)) ⦄ := by
  unfold refs.host_at
  step*
  have hs : s.val = gl := by rw [s_post]; rfl
  rw [r_post, hs]
  have hgl : gl.length = 10 := rfl
  rw [hgl]
  constructor
  · rintro ⟨hl, H⟩
    refine ⟨hl, (host_iff _ (at10_len _ _ hl)).mp fun j hj => ?_⟩
    obtain ⟨x, c, hx, hc, hlc⟩ := H j hj
    exact ⟨x, c, by rw [at10_get _ _ _ hj, hx], hc, hlc⟩
  · rintro ⟨hl, H⟩
    refine ⟨hl, fun j hj => ?_⟩
    obtain ⟨x, c, hx, hc, hlc⟩ := (host_iff _ (at10_len _ _ hl)).mpr H j hj
    exact ⟨x, c, by rw [← at10_get _ _ _ hj, hx], hc, hlc⟩

/-! ## Prefixes -/

@[step]
theorem starts_spec (u p : Slice U8) : refs.starts u p ⦃ r => (r = true ↔ p.val <+: u.val) ⦄ := by
  unfold refs.starts
  step*
  · simp only [Bool.false_eq_true, false_iff]
    intro h; have := h.length_le; scalar_tac
  · rw [r_post, List.prefix_iff_eq_take]
    simp only [List.drop_zero, Nat.sub_zero]
    exact ⟨fun h => h.symm ▸ (by simp), fun h => h ▸ (by simp)⟩

theorem no_two (a b u : List U8) (ha : a <+: u) (hb : b <+: u) (h1 : ¬ a <+: b) (h2 : ¬ b <+: a) : False := by
  by_cases hl : a.length ≤ b.length
  · exact h1 (List.prefix_of_prefix_length_le ha hb hl)
  · exact h2 (List.prefix_of_prefix_length_le hb ha (by omega))

theorem prefix_parts (sch h tail u : List U8) (c : U8) (hh : h.length = 10) (hp : sch ++ h ++ c :: tail <+: u) :
    sch <+: u ∧ at10 u sch.length = h ∧ u[sch.length + 10]? = some c ∧ sch.length + 11 ≤ u.length := by
  obtain ⟨r, rfl⟩ := hp
  refine ⟨⟨h ++ c :: tail ++ r, by simp⟩, ?_, ?_, ?_⟩
  · simp [at10, hh]
  · rw [List.append_assoc, List.append_assoc, List.getElem?_append_right (by omega), Nat.add_sub_cancel_left,
      List.getElem?_append_right (by omega)]
    simp [hh]
  · simp; omega

theorem take_one (u : List U8) (k : Nat) (c : U8) (hc : u[k]? = some c) : (u.drop k).take 1 = [c] := by
  have hk : k < u.length := by
    by_contra h; rw [List.getElem?_eq_none (by omega)] at hc; simp at hc
  rw [List.drop_eq_getElem_cons hk]
  rw [List.getElem?_eq_getElem hk, Option.some.injEq] at hc
  simp [hc]

theorem prefix_build (sch u : List U8) (c : U8) (hs : sch <+: u) (hl : sch.length + 11 ≤ u.length)
    (hc : u[sch.length + 10]? = some c) : sch ++ at10 u sch.length ++ [c] <+: u := by
  rw [List.prefix_iff_eq_take]
  have hlen : (sch ++ at10 u sch.length ++ [c]).length = sch.length + 10 + 1 := by simp [at10]; omega
  rw [hlen, List.take_add, List.take_add, take_one u _ c hc]
  conv_rhs => rw [← List.prefix_iff_eq_take.mp hs]

/-- `after_host u h sep`: `github.com` at `h`, then `sep`. -/
@[step]
theorem after_host_spec (u : Slice U8) (h : Usize) (sep : U8) :
    refs.after_host u h sep ⦃ r =>
      (r.val ≠ 0 → r.val = h.val + 11 ∧ h.val + 11 ≤ u.val.length ∧ Host (at10 u.val h.val) ∧
        u.val[h.val + 10]? = some sep) ∧
      (h.val + 11 ≤ u.val.length → Host (at10 u.val h.val) → u.val[h.val + 10]? = some sep →
        r.val = h.val + 11) ⦄ := by
  unfold refs.after_host
  step*
  · refine ⟨fun h0 => absurd rfl h0, fun h1 h2 h3 => ?_⟩
    exfalso
    have hl : k.val < u.val.length := by scalar_tac
    rw [← k_post, List.getElem?_eq_getElem hl, Option.some.injEq] at h3
    simp_all
  · have hb : b = true := by assumption
    have hr : r.val = h.val + 11 := by scalar_tac
    refine ⟨fun _ => ⟨hr, by scalar_tac, (b_post.mp hb).2, ?_⟩, fun _ _ _ => hr⟩
    have hl : k.val < u.val.length := by scalar_tac
    rw [← k_post, List.getElem?_eq_getElem hl, Option.some.injEq]
    apply UScalar.eq_of_val_eq
    simp_all

theorem extend (P u : List U8) (j : Nat) (d : U8) (hP : P <+: u) (hj : P.length ≤ j)
    (hd : u[j]? = some d) : P ++ (u.drop P.length).take (j - P.length) ++ [d] <+: u := by
  have hju : j < u.length := by
    by_contra h; rw [List.getElem?_eq_none (by omega)] at hd; simp at hd
  rw [List.prefix_iff_eq_take]
  have hlen : (P ++ (u.drop P.length).take (j - P.length) ++ [d]).length = P.length + (j - P.length) + 1 := by
    simp; omega
  have hj' : P.length + (j - P.length) = j := by omega
  rw [hlen, List.take_add, List.take_add, hj', take_one u j d hd]
  conv_rhs => rw [← List.prefix_iff_eq_take.mp hP]

/-! ## `ssh://` -/

theorem b_ssh : bytes "ssh://git@" = [115#u8, 115#u8, 104#u8, 58#u8, 47#u8, 47#u8, 103#u8, 105#u8, 116#u8, 64#u8] := by decide
theorem b_https : bytes "https://" = [104#u8, 116#u8, 116#u8, 112#u8, 115#u8, 58#u8, 47#u8, 47#u8] := by decide
theorem b_http : bytes "http://" = [104#u8, 116#u8, 116#u8, 112#u8, 58#u8, 47#u8, 47#u8] := by decide
theorem b_git : bytes "git://" = [103#u8, 105#u8, 116#u8, 58#u8, 47#u8, 47#u8] := by decide
theorem b_gitat : bytes "git@" = [103#u8, 105#u8, 116#u8, 64#u8] := by decide
theorem b_slash : bytes "/" = [47#u8] := by decide
theorem b_colon : bytes ":" = [58#u8] := by decide

/-- The two forms of `ssh://`. -/
def SshPre (p : List U8) : Prop := ∃ h, Host h ∧
  (p = bytes "ssh://git@" ++ h ++ bytes "/" ∨
   ∃ port, Digits port ∧ p = bytes "ssh://git@" ++ h ++ bytes ":" ++ port ++ bytes "/")

theorem slash_not_digit (port : List U8) (h : Digits port) : 47#u8 ∉ port := by
  intro hm
  have := (digit_iff _).mp (h.2 _ hm)
  simp [isDigit] at this

/-- In a prefix with a port, the port ends at the first `/` after it. -/
theorem port_parts (h port u : List U8) (hh : h.length = 10) (hd : Digits port)
    (hp : bytes "ssh://git@" ++ h ++ bytes ":" ++ port ++ bytes "/" <+: u) :
    u[20]? = some 58#u8 ∧ u[21 + port.length]? = some 47#u8 ∧
    (∀ i < port.length, u[21 + i]? ≠ some 47#u8) ∧ (u.drop 21).take port.length = port := by
  obtain ⟨r, rfl⟩ := hp
  have hS : (bytes "ssh://git@").length = 10 := by decide
  have hpre : bytes "ssh://git@" ++ h ++ bytes ":" ++ port ++ bytes "/" ++ r =
      (bytes "ssh://git@" ++ h ++ bytes ":") ++ (port ++ 47#u8 :: r) := by simp [b_slash]
  have hl21 : (bytes "ssh://git@" ++ h ++ bytes ":").length = 21 := by simp [hS, hh, b_colon]
  rw [hpre]
  refine ⟨?_, ?_, ?_, ?_⟩
  · rw [List.getElem?_append_left (by omega)]
    rw [List.getElem?_append_right (by simp [hS, hh])]
    simp [hS, hh, b_colon]
  · rw [List.getElem?_append_right (by omega), hl21, Nat.add_sub_cancel_left,
      List.getElem?_append_right (by omega)]
    simp
  · intro i hi
    rw [List.getElem?_append_right (by omega), hl21, Nat.add_sub_cancel_left,
      List.getElem?_append_left hi, List.getElem?_eq_getElem hi]
    intro he
    rw [Option.some.injEq] at he
    exact slash_not_digit port hd (he ▸ List.getElem_mem hi)
  · rw [← hl21, List.drop_left]
    simp

theorem hS : (bytes "ssh://git@").length = 10 := by decide

theorem ssh_plain_parts (h u : List U8) (hh : Host h) (hp : bytes "ssh://git@" ++ h ++ bytes "/" <+: u) :
    21 ≤ u.length ∧ Host (at10 u 10) ∧ u[20]? = some 47#u8 := by
  have := prefix_parts (bytes "ssh://git@") h [] u 47#u8 hh.1 (by rw [b_slash] at hp; exact hp)
  rw [hS] at this
  exact ⟨by omega, this.2.1 ▸ hh, this.2.2.1⟩

theorem ssh_port_host (h port u : List U8) (hh : Host h)
    (hp : bytes "ssh://git@" ++ h ++ bytes ":" ++ port ++ bytes "/" <+: u) :
    21 ≤ u.length ∧ Host (at10 u 10) ∧ u[20]? = some 58#u8 := by
  have := prefix_parts (bytes "ssh://git@") h (port ++ bytes "/") u 58#u8 hh.1
    (by simpa [b_colon] using hp)
  rw [hS] at this
  exact ⟨by omega, this.2.1 ▸ hh, this.2.2.1⟩

theorem port_j (u port : List U8) (j c : Nat) (hc : c = 21) (h1 : u[21 + port.length]? = some 47#u8)
    (h2 : ∀ i < port.length, u[21 + i]? ≠ some 47#u8) (j1 : c ≤ j)
    (j3 : ∀ k, c ≤ k → k < j → u[k]? ≠ some 47#u8) (j4 : j < u.length → u[j]? = some 47#u8) :
    j = 21 + port.length := by
  have hl : 21 + port.length < u.length := by
    by_contra h; rw [List.getElem?_eq_none (by omega)] at h1; simp at h1
  rcases Nat.lt_trichotomy j (21 + port.length) with h | h | h
  · have := h2 (j - 21) (by omega)
    rw [show 21 + (j - 21) = j by omega] at this
    exact absurd (j4 (by omega)) this
  · exact h
  · exact absurd h1 (j3 _ (by omega) h)

theorem ne_of_bne (s : Usize) (h : (s != 0#usize) = true) : s.val ≠ 0 := by
  intro h0; apply absurd h; simp only [bne_iff_ne, ne_eq, Decidable.not_not]
  apply UScalar.eq_of_val_eq; simp [h0]

theorem eq_of_not_bne (s : Usize) (h : ¬(s != 0#usize) = true) : s.val = 0 := by
  simp only [bne_iff_ne, ne_eq, Decidable.not_not] at h; simp [h]

theorem ssh_start_spec (u : Slice U8) (hs : bytes "ssh://git@" <+: u.val) :
    refs.ssh_start u ⦃ r =>
      (r.val ≠ 0 → ∃ pre, SshPre pre ∧ pre.length = r.val ∧ pre <+: u.val) ∧
      (∀ pre, SshPre pre → pre <+: u.val → r.val = pre.length) ⦄ := by
  unfold refs.ssh_start
  step*
  · have F := s_post1 (ne_of_bne s (by assumption))
    refine ⟨fun _ => ⟨bytes "ssh://git@" ++ at10 u.val 10 ++ bytes "/", ⟨_, F.2.2.1, Or.inl rfl⟩, ?_, ?_⟩, ?_⟩
    · have := F.2.1
      rw [F.1, List.length_append, List.length_append, at10_len _ _ (by omega), hS, b_slash]; rfl
    · have := prefix_build _ u.val 47#u8 hs (by rw [hS]; omega) (by rw [hS]; exact F.2.2.2)
      rw [hS] at this; rw [b_slash]; exact this
    · rintro pre ⟨h, hh, rfl | ⟨port, hd, rfl⟩⟩ hp
      · rw [F.1]; simp [hS, hh.1, b_slash]
      · have := (ssh_port_host h port u.val hh hp).2.2
        rw [F.2.2.2] at this; simp at this
  · have hc0 : c = 0#usize := by assumption
    have hs0 := eq_of_not_bne s (by assumption)
    refine ⟨fun h => absurd rfl h, ?_⟩
    rintro pre ⟨h, hh, rfl | ⟨port, hd, rfl⟩⟩ hp
    · have ⟨a, b, c⟩ := ssh_plain_parts h u.val hh hp
      have := s_post2 a b c; omega
    · have ⟨a, b, c'⟩ := ssh_port_host h port u.val hh hp
      have := c_post2 a b c'
      rw [hc0] at this; simp at this
  · have hc0 : ¬c = 0#usize := by assumption
    have hs0 := eq_of_not_bne s (by assumption)
    have hc : c.val = 21 := (c_post1 (by intro h; apply hc0; apply UScalar.eq_of_val_eq; simp [h])).1
    refine ⟨fun h => absurd rfl h, ?_⟩
    rintro pre ⟨h, hh, rfl | ⟨port, hd, rfl⟩⟩ hp
    · have ⟨a, b, c⟩ := ssh_plain_parts h u.val hh hp
      have := s_post2 a b c; omega
    · have P := port_parts h port u.val hh.1 hd hp
      have := port_j u.val port j.val c.val hc P.2.1 P.2.2.1 j_post1 j_post3 j_post4
      have hl : 21 + port.length < u.val.length := by
        by_contra h'; have h1 := P.2.1; rw [List.getElem?_eq_none (by omega)] at h1; simp at h1
      scalar_tac
  · have hc0 : ¬c = 0#usize := by assumption
    have hs0 := eq_of_not_bne s (by assumption)
    have F := c_post1 (by intro h; apply hc0; apply UScalar.eq_of_val_eq; simp [h])
    have hjl : j.val < u.val.length := by scalar_tac
    have hd := b_post.mp (by assumption)
    rw [F.1] at hd
    have hP := prefix_build _ u.val 58#u8 hs (by rw [hS]; omega) (by rw [hS]; exact F.2.2.2)
    rw [hS] at hP
    have hPl : (bytes "ssh://git@" ++ at10 u.val 10 ++ [58#u8]).length = 21 := by
      simp [hS, at10]; omega
    have hE := extend _ u.val j.val 47#u8 hP (by rw [hPl]; scalar_tac) (j_post4 hjl)
    rw [hPl] at hE
    refine ⟨fun _ => ⟨bytes "ssh://git@" ++ at10 u.val 10 ++ bytes ":" ++
      (u.val.drop 21).take (j.val - 21) ++ bytes "/", ⟨_, F.2.2.1, Or.inr ⟨_, hd, rfl⟩⟩, ?_, ?_⟩, ?_⟩
    · simp [hS, at10, b_slash, b_colon]; scalar_tac
    · rw [b_slash, b_colon]; exact hE
    · rintro pre ⟨h, hh, rfl | ⟨port, hd', rfl⟩⟩ hp
      · have ⟨a, b, c⟩ := ssh_plain_parts h u.val hh hp
        have := s_post2 a b c; omega
      · have P := port_parts h port u.val hh.1 hd' hp
        have := port_j u.val port j.val c.val F.1 P.2.1 P.2.2.1 j_post1 j_post3 j_post4
        simp [hS, hh.1, b_slash, b_colon]; scalar_tac
  · have hc0 : ¬c = 0#usize := by assumption
    have hs0 := eq_of_not_bne s (by assumption)
    have F := c_post1 (by intro h; apply hc0; apply UScalar.eq_of_val_eq; simp [h])
    refine ⟨fun h => absurd rfl h, ?_⟩
    rintro pre ⟨h, hh, rfl | ⟨port, hd, rfl⟩⟩ hp
    · have ⟨a, b, c⟩ := ssh_plain_parts h u.val hh hp
      have := s_post2 a b c; omega
    · have P := port_parts h port u.val hh.1 hd hp
      have := port_j u.val port j.val c.val F.1 P.2.1 P.2.2.1 j_post1 j_post3 j_post4
      exfalso; apply (by assumption : ¬b = true); apply b_post.mpr
      rw [this, F.1, Nat.add_sub_cancel_left, P.2.2.2]; exact hd

theorem path_start_spec (u : Slice U8) :
    refs.path_start u ⦃ s =>
      (s.val ≠ 0 → ∃ pre, Pre pre ∧ pre.length = s.val ∧ pre <+: u.val) ∧
      (∀ pre, Pre pre → pre <+: u.val → s.val = pre.length) ⦄ := by
  unfold refs.path_start
  step*
  · -- branch 0
    have e0 : b = true ↔ bytes "https://" <+: u.val := by rw [b_post, s_post, b_https]; rfl
    have y0 : bytes "https://" <+: u.val := e0.mp (by assumption)
    have hX : (bytes "https://").length = 8 := by decide
    refine ⟨fun h0 => ?_, ?_⟩
    · have F := s_post1 h0
      refine ⟨bytes "https://" ++ at10 u.val 8 ++ bytes "/", ⟨_, F.2.2.1, Or.inl rfl⟩, ?_, ?_⟩
      · have := F.2.1
        rw [F.1, List.length_append, List.length_append, at10_len _ _ (by omega), hX]; rfl
      · have := prefix_build _ u.val 47#u8 y0 (by rw [hX]; omega) (by rw [hX]; exact F.2.2.2)
        rw [hX] at this; simpa [b_slash, b_colon] using this
    · rintro pre ⟨h, hh, rfl | rfl | rfl | rfl | rfl | ⟨port, hd, rfl⟩⟩ hp
      · have hY : bytes "https://" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        have P := prefix_parts (bytes "https://") h [] u.val 47#u8 hh.1 (by simpa [b_slash, b_colon] using hp)
        rw [hX] at P
        rw [s_post2 (by omega) (P.2.1 ▸ hh) P.2.2.1]
        simp [hX, hh.1, b_slash, b_colon]
      · have hY : bytes "http://" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact (no_two _ _ _ y0 hY (by decide) (by decide)).elim
      · have hY : bytes "git://" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact (no_two _ _ _ y0 hY (by decide) (by decide)).elim
      · have hY : bytes "git@" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact (no_two _ _ _ y0 hY (by decide) (by decide)).elim
      · have hY : bytes "ssh://git@" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact (no_two _ _ _ y0 hY (by decide) (by decide)).elim
      · have hY : bytes "ssh://git@" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact (no_two _ _ _ y0 hY (by decide) (by decide)).elim
  · -- branch 1
    have e0 : b = true ↔ bytes "https://" <+: u.val := by rw [b_post, s_post, b_https]; rfl
    have e1 : b1 = true ↔ bytes "http://" <+: u.val := by rw [b1_post, s1_post, b_http]; rfl
    have n0 : ¬ bytes "https://" <+: u.val := fun h => (by assumption : ¬ b = true) (e0.mpr h)
    have y1 : bytes "http://" <+: u.val := e1.mp (by assumption)
    have hX : (bytes "http://").length = 7 := by decide
    refine ⟨fun h0 => ?_, ?_⟩
    · have F := s_post1 h0
      refine ⟨bytes "http://" ++ at10 u.val 7 ++ bytes "/", ⟨_, F.2.2.1, Or.inr (Or.inl rfl)⟩, ?_, ?_⟩
      · have := F.2.1
        rw [F.1, List.length_append, List.length_append, at10_len _ _ (by omega), hX]; rfl
      · have := prefix_build _ u.val 47#u8 y1 (by rw [hX]; omega) (by rw [hX]; exact F.2.2.2)
        rw [hX] at this; simpa [b_slash, b_colon] using this
    · rintro pre ⟨h, hh, rfl | rfl | rfl | rfl | rfl | ⟨port, hd, rfl⟩⟩ hp
      · have hY : bytes "https://" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact absurd hY n0
      · have hY : bytes "http://" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        have P := prefix_parts (bytes "http://") h [] u.val 47#u8 hh.1 (by simpa [b_slash, b_colon] using hp)
        rw [hX] at P
        rw [s_post2 (by omega) (P.2.1 ▸ hh) P.2.2.1]
        simp [hX, hh.1, b_slash, b_colon]
      · have hY : bytes "git://" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact (no_two _ _ _ y1 hY (by decide) (by decide)).elim
      · have hY : bytes "git@" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact (no_two _ _ _ y1 hY (by decide) (by decide)).elim
      · have hY : bytes "ssh://git@" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact (no_two _ _ _ y1 hY (by decide) (by decide)).elim
      · have hY : bytes "ssh://git@" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact (no_two _ _ _ y1 hY (by decide) (by decide)).elim
  · -- branch 2
    have e0 : b = true ↔ bytes "https://" <+: u.val := by rw [b_post, s_post, b_https]; rfl
    have e1 : b1 = true ↔ bytes "http://" <+: u.val := by rw [b1_post, s1_post, b_http]; rfl
    have e2 : b2 = true ↔ bytes "git://" <+: u.val := by rw [b2_post, s2_post, b_git]; rfl
    have n0 : ¬ bytes "https://" <+: u.val := fun h => (by assumption : ¬ b = true) (e0.mpr h)
    have n1 : ¬ bytes "http://" <+: u.val := fun h => (by assumption : ¬ b1 = true) (e1.mpr h)
    have y2 : bytes "git://" <+: u.val := e2.mp (by assumption)
    have hX : (bytes "git://").length = 6 := by decide
    refine ⟨fun h0 => ?_, ?_⟩
    · have F := s_post1 h0
      refine ⟨bytes "git://" ++ at10 u.val 6 ++ bytes "/", ⟨_, F.2.2.1, Or.inr (Or.inr (Or.inl rfl))⟩, ?_, ?_⟩
      · have := F.2.1
        rw [F.1, List.length_append, List.length_append, at10_len _ _ (by omega), hX]; rfl
      · have := prefix_build _ u.val 47#u8 y2 (by rw [hX]; omega) (by rw [hX]; exact F.2.2.2)
        rw [hX] at this; simpa [b_slash, b_colon] using this
    · rintro pre ⟨h, hh, rfl | rfl | rfl | rfl | rfl | ⟨port, hd, rfl⟩⟩ hp
      · have hY : bytes "https://" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact absurd hY n0
      · have hY : bytes "http://" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact absurd hY n1
      · have hY : bytes "git://" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        have P := prefix_parts (bytes "git://") h [] u.val 47#u8 hh.1 (by simpa [b_slash, b_colon] using hp)
        rw [hX] at P
        rw [s_post2 (by omega) (P.2.1 ▸ hh) P.2.2.1]
        simp [hX, hh.1, b_slash, b_colon]
      · have hY : bytes "git@" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact (no_two _ _ _ y2 hY (by decide) (by decide)).elim
      · have hY : bytes "ssh://git@" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact (no_two _ _ _ y2 hY (by decide) (by decide)).elim
      · have hY : bytes "ssh://git@" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact (no_two _ _ _ y2 hY (by decide) (by decide)).elim
  · -- branch 3
    have e0 : b = true ↔ bytes "https://" <+: u.val := by rw [b_post, s_post, b_https]; rfl
    have e1 : b1 = true ↔ bytes "http://" <+: u.val := by rw [b1_post, s1_post, b_http]; rfl
    have e2 : b2 = true ↔ bytes "git://" <+: u.val := by rw [b2_post, s2_post, b_git]; rfl
    have e3 : b3 = true ↔ bytes "git@" <+: u.val := by rw [b3_post, s3_post, b_gitat]; rfl
    have n0 : ¬ bytes "https://" <+: u.val := fun h => (by assumption : ¬ b = true) (e0.mpr h)
    have n1 : ¬ bytes "http://" <+: u.val := fun h => (by assumption : ¬ b1 = true) (e1.mpr h)
    have n2 : ¬ bytes "git://" <+: u.val := fun h => (by assumption : ¬ b2 = true) (e2.mpr h)
    have y3 : bytes "git@" <+: u.val := e3.mp (by assumption)
    have hX : (bytes "git@").length = 4 := by decide
    refine ⟨fun h0 => ?_, ?_⟩
    · have F := s_post1 h0
      refine ⟨bytes "git@" ++ at10 u.val 4 ++ bytes ":", ⟨_, F.2.2.1, Or.inr (Or.inr (Or.inr (Or.inl rfl)))⟩, ?_, ?_⟩
      · have := F.2.1
        rw [F.1, List.length_append, List.length_append, at10_len _ _ (by omega), hX]; rfl
      · have := prefix_build _ u.val 58#u8 y3 (by rw [hX]; omega) (by rw [hX]; exact F.2.2.2)
        rw [hX] at this; simpa [b_slash, b_colon] using this
    · rintro pre ⟨h, hh, rfl | rfl | rfl | rfl | rfl | ⟨port, hd, rfl⟩⟩ hp
      · have hY : bytes "https://" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact absurd hY n0
      · have hY : bytes "http://" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact absurd hY n1
      · have hY : bytes "git://" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact absurd hY n2
      · have hY : bytes "git@" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        have P := prefix_parts (bytes "git@") h [] u.val 58#u8 hh.1 (by simpa [b_slash, b_colon] using hp)
        rw [hX] at P
        rw [s_post2 (by omega) (P.2.1 ▸ hh) P.2.2.1]
        simp [hX, hh.1, b_slash, b_colon]
      · have hY : bytes "ssh://git@" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact (no_two _ _ _ y3 hY (by decide) (by decide)).elim
      · have hY : bytes "ssh://git@" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact (no_two _ _ _ y3 hY (by decide) (by decide)).elim
  · -- branch 4
    have e0 : b = true ↔ bytes "https://" <+: u.val := by rw [b_post, s_post, b_https]; rfl
    have e1 : b1 = true ↔ bytes "http://" <+: u.val := by rw [b1_post, s1_post, b_http]; rfl
    have e2 : b2 = true ↔ bytes "git://" <+: u.val := by rw [b2_post, s2_post, b_git]; rfl
    have e3 : b3 = true ↔ bytes "git@" <+: u.val := by rw [b3_post, s3_post, b_gitat]; rfl
    have e4 : b4 = true ↔ bytes "ssh://git@" <+: u.val := by rw [b4_post, s4_post, b_ssh]; rfl
    have n0 : ¬ bytes "https://" <+: u.val := fun h => (by assumption : ¬ b = true) (e0.mpr h)
    have n1 : ¬ bytes "http://" <+: u.val := fun h => (by assumption : ¬ b1 = true) (e1.mpr h)
    have n2 : ¬ bytes "git://" <+: u.val := fun h => (by assumption : ¬ b2 = true) (e2.mpr h)
    have n3 : ¬ bytes "git@" <+: u.val := fun h => (by assumption : ¬ b3 = true) (e3.mpr h)
    have y4 : bytes "ssh://git@" <+: u.val := e4.mp (by assumption)
    apply WP.spec_mono (ssh_start_spec u y4)
    rintro r ⟨h1, h2⟩
    refine ⟨fun h0 => ?_, ?_⟩
    · obtain ⟨pre, ⟨h, hh, hpre⟩, hl, hp⟩ := h1 h0
      refine ⟨pre, ⟨h, hh, ?_⟩, hl, hp⟩
      rcases hpre with e | e
      · exact Or.inr (Or.inr (Or.inr (Or.inr (Or.inl e))))
      · exact Or.inr (Or.inr (Or.inr (Or.inr (Or.inr e))))
    · rintro pre ⟨h, hh, rfl | rfl | rfl | rfl | rfl | ⟨port, hd, rfl⟩⟩ hp
      · have hY : bytes "https://" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact absurd hY n0
      · have hY : bytes "http://" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact absurd hY n1
      · have hY : bytes "git://" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact absurd hY n2
      · have hY : bytes "git@" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact absurd hY n3
      · have hY : bytes "ssh://git@" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact h2 _ ⟨h, hh, Or.inl rfl⟩ hp
      · have hY : bytes "ssh://git@" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
        exact h2 _ ⟨h, hh, Or.inr ⟨port, hd, rfl⟩⟩ hp
  · -- branch 5
    have e0 : b = true ↔ bytes "https://" <+: u.val := by rw [b_post, s_post, b_https]; rfl
    have e1 : b1 = true ↔ bytes "http://" <+: u.val := by rw [b1_post, s1_post, b_http]; rfl
    have e2 : b2 = true ↔ bytes "git://" <+: u.val := by rw [b2_post, s2_post, b_git]; rfl
    have e3 : b3 = true ↔ bytes "git@" <+: u.val := by rw [b3_post, s3_post, b_gitat]; rfl
    have e4 : b4 = true ↔ bytes "ssh://git@" <+: u.val := by rw [b4_post, s4_post, b_ssh]; rfl
    have n0 : ¬ bytes "https://" <+: u.val := fun h => (by assumption : ¬ b = true) (e0.mpr h)
    have n1 : ¬ bytes "http://" <+: u.val := fun h => (by assumption : ¬ b1 = true) (e1.mpr h)
    have n2 : ¬ bytes "git://" <+: u.val := fun h => (by assumption : ¬ b2 = true) (e2.mpr h)
    have n3 : ¬ bytes "git@" <+: u.val := fun h => (by assumption : ¬ b3 = true) (e3.mpr h)
    have n4 : ¬ bytes "ssh://git@" <+: u.val := fun h => (by assumption : ¬ b4 = true) (e4.mpr h)
    have n4 : ¬ bytes "ssh://git@" <+: u.val := fun h => (by assumption : ¬ b4 = true) (e4.mpr h)
    refine ⟨fun h0 => absurd rfl h0, ?_⟩
    rintro pre ⟨h, hh, rfl | rfl | rfl | rfl | rfl | ⟨port, hd, rfl⟩⟩ hp
    · have hY : bytes "https://" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
      exact absurd hY n0
    · have hY : bytes "http://" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
      exact absurd hY n1
    · have hY : bytes "git://" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
      exact absurd hY n2
    · have hY : bytes "git@" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
      exact absurd hY n3
    · have hY : bytes "ssh://git@" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
      exact absurd hY n4
    · have hY : bytes "ssh://git@" <+: u.val := List.IsPrefix.trans (by simp only [List.append_assoc]; exact List.prefix_append _ _) hp
      exact absurd hY n4

/-! ## The end of the path -/

/-- The bytes `u[a..b]`. -/
abbrev slc (u : List U8) (a b : Nat) : List U8 := (u.drop a).take (b - a)

theorem b_dotgit : bytes ".git" = [46#u8, 103#u8, 105#u8, 116#u8] := by decide

theorem slc_drop (u : List U8) (f t k : Nat) (h : f + k ≤ t) : (slc u f t).drop k = slc u (f + k) t := by
  simp only [slc, List.drop_take, List.drop_drop]
  rw [show t - f - k = t - (f + k) by omega]

theorem suffix_slc (u w : List U8) (f t : Nat) (h1 : f ≤ t) (h2 : t ≤ u.length) :
    w <:+ slc u f t ↔ w.length ≤ t - f ∧ slc u (t - w.length) t = w := by
  rw [List.suffix_iff_eq_drop]
  have hl : (slc u f t).length = t - f := by simp; omega
  rw [hl]
  constructor
  · intro h
    have hw : w.length ≤ t - f := by
      have := congrArg List.length h; simp at this; omega
    refine ⟨hw, ?_⟩
    rw [show t - w.length = f + (t - f - w.length) by omega]
    conv_rhs => rw [h]
    rw [slc_drop u f t _ (by omega)]
  · rintro ⟨hw, h⟩
    rw [slc_drop u f t _ (by omega), show f + (t - f - w.length) = t - w.length by omega]
    exact h.symm

@[step]
theorem ends_git_spec (u : Slice U8) (f t : Usize) (h1 : f.val ≤ t.val) (h2 : t.val ≤ u.val.length) :
    refs.ends_git u f t ⦃ r => (r = true ↔ bytes ".git" <:+ slc u.val f.val t.val) ⦄ := by
  unfold refs.ends_git
  have h4 : (bytes ".git").length = 4 := by decide
  step*
  · rw [suffix_slc _ _ _ _ h1 h2, h4]
    simp only [Bool.false_eq_true, false_iff, not_and]
    intro h; scalar_tac
  · have hs : s.val = bytes ".git" := by rw [s_post, b_dotgit]; rfl
    rw [suffix_slc _ _ _ _ h1 h2, r_post, hs, h4, ← i1_post1]
    exact ⟨fun h => ⟨by scalar_tac, h⟩, fun h => h.2⟩

theorem slc_full (u : List U8) (s : Nat) : slc u s u.length = u.drop s := by
  simp only [slc]; apply List.take_of_length_le; simp

theorem slc_of_drop (u y t : List U8) (s : Nat) (h : u.drop s = y ++ t) : slc u s (s + y.length) = y := by
  simp only [slc, h, Nat.add_sub_cancel_left, List.take_left]

theorem slc_append (u : List U8) (s m e : Nat) (h1 : s ≤ m) (h2 : m ≤ e) :
    slc u s e = slc u s m ++ slc u m e := by
  simp only [slc]
  conv_lhs => rw [← List.take_append_drop (m - s) ((u.drop s).take (e - s))]
  rw [List.take_take, List.drop_take, List.drop_drop, Nat.add_comm, Nat.sub_add_cancel h1,
    show min (m - s) (e - s) = m - s by omega, show e - s - (m - s) = e - m by omega]

theorem last_drop (u : List U8) (s : Nat) (h : s < u.length) : (u.drop s).getLast? = u[u.length - 1]? := by
  rw [List.getLast?_eq_getElem?]; simp; congr 1; omega

theorem split_last (u : List U8) (s : Nat) (c : U8) (h : s < u.length) (hc : u[u.length - 1]? = some c) :
    u.drop s = slc u s (u.length - 1) ++ [c] := by
  simp only [slc]
  conv_lhs => rw [← List.take_append_drop (u.length - 1 - s) (u.drop s)]
  rw [List.drop_drop, show s + (u.length - 1 - s) = u.length - 1 by omega]
  congr 1
  rw [List.getElem?_eq_getElem (by omega), Option.some.injEq] at hc
  rw [List.drop_eq_getElem_cons (by omega : u.length - 1 < u.length), hc, List.drop_eq_nil_of_le (by omega)]

theorem trim_slash_spec (u : Slice U8) (s : Usize) (hs : s.val ≤ u.val.length) :
    refs.trim_slash u s ⦃ e => s.val ≤ e.val ∧ e.val ≤ u.val.length ∧
      (∃ t, (t = [] ∨ t = bytes "/") ∧ u.val.drop s.val = slc u.val s.val e.val ++ t) ∧
      (∀ y t, y ≠ [] → y.getLast? ≠ some 47#u8 → (t = [] ∨ t = bytes "/") →
        u.val.drop s.val = y ++ t → e.val = s.val + y.length) ⦄ := by
  unfold refs.trim_slash
  have hL : u.len.val = u.val.length := by simp
  step*
  · have hi : i.val = u.val.length - 1 := by rw [i_post1, hL]
    have hsl : s.val < u.val.length := by scalar_tac
    have key : ∀ k (hk : k < u.val.length), k = i.val → u.val[k] = i1 := by
      intro k hk he; subst he; rw [i1_post]
    have hlast : u.val[u.val.length - 1]? = some 47#u8 := by
      rw [List.getElem?_eq_getElem (by omega), key _ _ hi.symm, (by assumption : i1 = 47#u8)]
    refine ⟨by omega, by omega, ⟨bytes "/", Or.inr rfl, by rw [hi, b_slash]; exact split_last _ _ _ hsl hlast⟩, ?_⟩
    intro y t hy hly ht hd
    rcases ht with rfl | rfl
    · exfalso; apply hly; rw [← List.append_nil y, ← hd, last_drop _ _ hsl, hlast]
    · have := congrArg List.length hd; simp [b_slash] at this; omega
  · have hsl : s.val < u.val.length := by scalar_tac
    have hi : i.val = u.val.length - 1 := by rw [i_post1, hL]
    have key : ∀ k (hk : k < u.val.length), k = i.val → u.val[k] = i1 := by
      intro k hk he; subst he; rw [i1_post]
    have hne : u.val[u.val.length - 1]? ≠ some 47#u8 := by
      rw [List.getElem?_eq_getElem (by omega), key _ _ hi.symm]
      intro h; simp only [Option.some.injEq] at h
      exact (by assumption : ¬ i1 = 47#u8) h
    rw [hL]
    refine ⟨by omega, le_refl _, ⟨[], Or.inl rfl, by rw [List.append_nil, slc_full]⟩, ?_⟩
    intro y t hy hly ht hd
    rcases ht with rfl | rfl
    · have := congrArg List.length hd; simp at this; omega
    · exfalso; apply hne; rw [← last_drop _ _ hsl, hd, b_slash]; simp
  · rw [hL]
    refine ⟨by scalar_tac, le_refl _, ⟨[], Or.inl rfl, by rw [List.append_nil, slc_full]⟩, ?_⟩
    intro y t hy _ _ hd
    exfalso
    have h1 := congrArg List.length hd
    have h2 := List.length_pos_of_ne_nil hy
    rw [List.length_drop, List.length_append] at h1
    scalar_tac

theorem last_xg (x g : List U8) (hx : x.getLast? ≠ some 47#u8) (hg : g = [] ∨ g = bytes ".git") :
    (x ++ g).getLast? ≠ some 47#u8 := by
  rcases hg with rfl | rfl
  · simpa using hx
  · rw [b_dotgit]; simp

theorem path_end_spec (u : Slice U8) (s : Usize) (hs : s.val ≤ u.val.length) :
    refs.path_end u s ⦃ e => s.val ≤ e.val ∧ e.val ≤ u.val.length ∧
      (∃ g t, (g = [] ∨ g = bytes ".git") ∧ (t = [] ∨ t = bytes "/") ∧
        u.val.drop s.val = slc u.val s.val e.val ++ g ++ t) ∧
      (∀ x g t, x ≠ [] → x.getLast? ≠ some 47#u8 → ¬ bytes ".git" <:+ x →
        (g = [] ∨ g = bytes ".git") → (t = [] ∨ t = bytes "/") →
        u.val.drop s.val = x ++ g ++ t → e.val = s.val + x.length) ⦄ := by
  unfold refs.path_end
  have h4 : (bytes ".git").length = 4 := by decide
  apply WP.spec_bind (trim_slash_spec u s hs)
  rintro e0 ⟨he1, he2, ⟨t, ht, hd⟩, hc⟩
  step*
  · have := (suffix_slc _ _ _ _ he1 he2).mp (b_post.mp (by assumption))
    rw [h4] at this; scalar_tac
  · have hb := b_post.mp (by assumption)
    rw [suffix_slc _ _ _ _ he1 he2, h4] at hb
    have he : e.val = e0.val - 4 := e_post1
    have hsplit : slc u.val s.val e0.val = slc u.val s.val e.val ++ bytes ".git" := by
      rw [slc_append u.val s.val (e0.val - 4) e0.val (by omega) (by omega), hb.2, he]
    refine ⟨by omega, by omega, ⟨bytes ".git", t, Or.inr rfl, ht, by rw [hd, hsplit]⟩, ?_⟩
    intro x g t' hx hxl hxs hg ht' hd'
    have hd'' : u.val.drop s.val = (x ++ g) ++ t' := by rw [hd', List.append_assoc]
    have hy := hc (x ++ g) t' (by simp [hx]) (last_xg x g hxl hg) ht' hd''
    have hs' : slc u.val s.val e0.val = x ++ g := by rw [hy]; exact slc_of_drop _ _ t' _ hd''
    have hsuf : bytes ".git" <:+ x ++ g := hs' ▸ b_post.mp (by assumption)
    rcases hg with rfl | rfl
    · rw [List.append_nil] at hsuf; exact absurd hsuf hxs
    · rw [he, hy, List.length_append, h4]; omega
  · refine ⟨he1, he2, ⟨[], t, Or.inl rfl, ht, by rw [hd, List.append_nil]⟩, ?_⟩
    intro x g t' hx hxl hxs hg ht' hd'
    have hd'' : u.val.drop s.val = (x ++ g) ++ t' := by rw [hd', List.append_assoc]
    have hy := hc (x ++ g) t' (by simp [hx]) (last_xg x g hxl hg) ht' hd''
    have hs' : slc u.val s.val e0.val = x ++ g := by rw [hy]; exact slc_of_drop _ _ t' _ hd''
    rcases hg with rfl | rfl
    · rw [hy]; simp
    · exfalso; apply (by assumption : ¬ b = true); apply b_post.mpr; rw [hs']; exact List.suffix_append _ _

/-! ## Owner and name -/

theorem slc_get (u : List U8) (a b k : Nat) (hk : k < b - a) (hb : b ≤ u.length) :
    (slc u a b)[k]? = u[a + k]? := by
  simp [slc, hk]

theorem slc_cons (u : List U8) (m e : Nat) (c : U8) (h1 : m < e) (h2 : e ≤ u.length) (hc : u[m]? = some c) :
    slc u m e = c :: slc u (m + 1) e := by
  simp only [slc]
  rw [List.drop_eq_getElem_cons (by omega), show e - m = (e - (m + 1)) + 1 by omega, List.take_succ_cons]
  rw [List.getElem?_eq_getElem (by omega), Option.some.injEq] at hc
  rw [hc]

theorem no_byte_slc (u : List U8) (a b : Nat) (c : U8) (hb : b ≤ u.length)
    (h : ∀ j, a ≤ j → j < b → u[j]? ≠ some c) : c ∉ slc u a b := by
  intro hm
  rw [List.mem_iff_getElem?] at hm
  obtain ⟨k, hk⟩ := hm
  have hkl : k < b - a := by
    by_contra hc; rw [List.getElem?_eq_none (by simp; omega)] at hk; simp at hk
  rw [slc_get u a b k hkl hb] at hk
  exact h (a + k) (by omega) (by omega) hk

theorem find_at (u : List U8) (st m p : Nat) (c : U8) (hp1 : st ≤ p) (hp : u[p]? = some c)
    (hno : ∀ j, st ≤ j → j < p → u[j]? ≠ some c) (m1 : st ≤ m)
    (m3 : ∀ j, st ≤ j → j < m → u[j]? ≠ some c) (m4 : m < u.length → u[m]? = some c) : m = p := by
  have hpl : p < u.length := by
    by_contra h; rw [List.getElem?_eq_none (by omega)] at hp; simp at hp
  rcases Nat.lt_trichotomy m p with h | h | h
  · exact absurd (m4 (by omega)) (hno m m1 h)
  · exact h
  · exact absurd hp (m3 p hp1 h)

/-- What `slc u s e = o ++ '/' :: n` says about the bytes of `u`. -/
theorem two_parts (u o n : List U8) (s e : Nat) (he : e ≤ u.length) (hs : s ≤ e)
    (h : slc u s e = o ++ 47#u8 :: n) :
    s + o.length + 1 + n.length = e ∧ u[s + o.length]? = some 47#u8 ∧
    (∀ k < o.length, u[s + k]? = o[k]?) ∧ (∀ k < n.length, u[s + o.length + 1 + k]? = n[k]?) := by
  have hl : (slc u s e).length = e - s := by simp; omega
  have hl' := hl; rw [h] at hl'; simp at hl'
  refine ⟨by omega, ?_, fun k hk => ?_, fun k hk => ?_⟩
  · rw [← slc_get u s e _ (by omega) he, h]; simp
  · rw [← slc_get u s e _ (by omega) he, h, List.getElem?_append_left hk]
  · rw [show s + o.length + 1 + k = s + (o.length + 1 + k) by omega, ← slc_get u s e _ (by omega) he, h,
      List.getElem?_append_right (by omega)]
    simp [show o.length + 1 + k - o.length = k + 1 by omega]

theorem owner_facts (u o n : List U8) (s e m : Nat) (he : e ≤ u.length) (hs : s ≤ e)
    (hpo : RefSpec.Part o) (hpn : RefSpec.Part n) (hsl : slc u s e = o ++ 47#u8 :: n)
    (m1 : s ≤ m) (m3 : ∀ j, s ≤ j → j < m → u[j]? ≠ some 47#u8) (m4 : m < u.length → u[m]? = some 47#u8) :
    m = s + o.length ∧ s + o.length + 1 + n.length = e ∧
    (∀ j, s + o.length + 1 ≤ j → j < e → u[j]? ≠ some 47#u8) ∧
    slc u (s + o.length + 1) e = n ∧ 0 < o.length ∧ 0 < n.length := by
  obtain ⟨hl, h47, ho, hn⟩ := two_parts u o n s e he hs hsl
  have hpo2 : 47#u8 ∉ o := by have := hpo.2; simpa using this
  have hpn2 : 47#u8 ∉ n := by have := hpn.2; simpa using this
  have hol : 0 < o.length := List.length_pos_of_ne_nil hpo.1
  have hnl : 0 < n.length := List.length_pos_of_ne_nil hpn.1
  refine ⟨?_, hl, ?_, ?_, hol, hnl⟩
  · apply find_at u s m (s + o.length) 47#u8 (by omega) h47 _ m1 m3 m4
    intro j h1 h2
    rw [show j = s + (j - s) by omega, ho _ (by omega), List.getElem?_eq_getElem (by omega)]
    intro hc; simp only [Option.some.injEq] at hc
    exact hpo2 (hc ▸ List.getElem_mem _)
  · intro j h1 h2
    rw [show j = s + o.length + 1 + (j - (s + o.length + 1)) by omega, hn _ (by omega),
      List.getElem?_eq_getElem (by omega)]
    intro hc; simp only [Option.some.injEq] at hc
    exact hpn2 (hc ▸ List.getElem_mem _)
  · rw [show s + o.length + 1 = s + (o.length + 1) by omega, ← slc_drop u s e _ (by omega), hsl]
    simp

theorem owner_end_spec (u : Slice U8) (s e : Usize) (hs : s.val ≤ e.val) (he : e.val ≤ u.val.length) :
    refs.owner_end u s e ⦃ m =>
      (m.val ≠ 0 → s.val < m.val ∧ m.val < e.val ∧
        slc u.val s.val e.val = slc u.val s.val m.val ++ 47#u8 :: slc u.val (m.val + 1) e.val ∧
        RefSpec.Part (slc u.val s.val m.val) ∧ RefSpec.Part (slc u.val (m.val + 1) e.val) ∧
        ¬ bytes ".git" <:+ slc u.val (m.val + 1) e.val) ∧
      (∀ o n, RefSpec.Part o → RefSpec.Part n → ¬ bytes ".git" <:+ n → slc u.val s.val e.val = o ++ 47#u8 :: n →
        m.val = s.val + o.length) ⦄ := by
  unfold refs.owner_end
  step*
  all_goals first
    | (refine ⟨fun h => absurd rfl h, ?_⟩
       intro o n hpo hpn hnn hsl
       have F := owner_facts u.val o n s.val e.val m.val he hs hpo hpn hsl m_post1 m_post3 m_post4
       exfalso
       first
         | scalar_tac
         | (have hlt : i1.val < u.val.length := by scalar_tac
            exact F.2.2.1 i1.val (by scalar_tac) (by scalar_tac) (i1_post4 hlt)))
    | skip
  have hms : s.val < m.val := by scalar_tac
  have hme : m.val < e.val := by scalar_tac
  have hml : m.val < u.val.length := by omega
  have hm1e : m.val + 1 < e.val := by scalar_tac
  refine ⟨fun _ => ⟨hms, hme, ?_, ⟨?_, ?_⟩, ⟨?_, ?_⟩, ?_⟩, ?_⟩
  · rw [slc_append u.val s.val m.val e.val (by omega) (by omega),
      slc_cons u.val m.val e.val 47#u8 hme he (m_post4 hml)]
  · intro h; have := congrArg List.length h; simp at this; omega
  · rw [byte_slash]; exact no_byte_slc u.val s.val m.val 47#u8 (by omega) m_post3
  · intro h; have := congrArg List.length h; simp at this; omega
  · rw [byte_slash]
    exact no_byte_slc u.val (m.val + 1) e.val 47#u8 he (fun j h1 h2 => i1_post3 j (by scalar_tac) (by scalar_tac))
  · intro h; apply (by assumption : ¬ b = true); apply b_post.mpr; rw [i_post]; exact h
  · intro o n hpo hpn hnn hsl
    exact (owner_facts u.val o n s.val e.val m.val he hs hpo hpn hsl m_post1 m_post3 m_post4).1

/-! ## `github_repo` -/

theorem pre_ne (p : List U8) (h : Pre p) : p.length ≠ 0 := by
  obtain ⟨h, _, hp⟩ := h
  rcases hp with rfl | rfl | rfl | rfl | rfl | ⟨port, _, rfl⟩ <;> simp [b_slash, b_colon]

theorem suffix_nosep (w a b : List U8) (c : U8) (hw : c ∉ w) (h : w <:+ a ++ c :: b) : w <:+ b := by
  have hb : b <:+ a ++ c :: b := (List.suffix_cons c b).trans (List.suffix_append a (c :: b))
  by_cases hl : w.length ≤ b.length
  · exact List.suffix_of_suffix_length_le h hb hl
  · have hcb : (c :: b) <:+ w := List.suffix_of_suffix_length_le (List.suffix_append a (c :: b)) h
      (by simp; omega)
    exact absurd (hcb.subset (List.mem_cons_self ..)) hw

theorem last_name (o n : List U8) (hn : RefSpec.Part n) : (o ++ 47#u8 :: n).getLast? ≠ some 47#u8 := by
  have h2 : 47#u8 ∉ n := by have := hn.2; simpa using this
  rw [show o ++ 47#u8 :: n = (o ++ [47#u8]) ++ n by simp, List.getLast?_append_of_ne_nil _ hn.1]
  intro hx
  exact h2 (List.mem_of_getLast? hx)

theorem part_eq (u : List U8) (a b : Usize) : part u a b = slc u a.val b.val := rfl

theorem github_repo_spec (u : Slice U8) :
    refs.github_repo u ⦃ r =>
      (∀ p, r = some p → p.owner_start.val < p.owner_end.val ∧ p.owner_end.val + 1 = p.name_start.val ∧
        p.name_start.val < p.name_end.val ∧ p.name_end.val ≤ u.val.length ∧
        GitHubUrl u.val (part u.val p.owner_start p.owner_end) (part u.val p.name_start p.name_end)) ∧
      (∀ o n, GitHubUrl u.val o n → ∃ p, r = some p ∧
        part u.val p.owner_start p.owner_end = o ∧ part u.val p.name_start p.name_end = n) ⦄ := by
  unfold refs.github_repo
  apply WP.spec_bind (path_start_spec u)
  rintro s ⟨hs1, hs2⟩
  have hpre_of : ∀ o n, GitHubUrl u.val o n → ∃ pre g t, Pre pre ∧ RefSpec.Part o ∧ RefSpec.Part n ∧
      ¬ bytes ".git" <:+ n ∧ (g = [] ∨ g = bytes ".git") ∧ (t = [] ∨ t = bytes "/") ∧
      u.val = pre ++ o ++ 47#u8 :: n ++ g ++ t ∧ s.val = pre.length := by
    rintro o n ⟨pre, g, t, hpre, hpo, hpn, hnn, hg, ht, hu⟩
    rw [byte_slash] at hu
    refine ⟨pre, g, t, hpre, hpo, hpn, hnn, hg, ht, hu, hs2 pre hpre ?_⟩
    rw [hu]; simp only [List.append_assoc]; exact List.prefix_append _ _
  split
  · refine ⟨fun p hp => absurd hp (by simp), fun o n h => ?_⟩
    exfalso
    obtain ⟨pre, _, _, hpre, -, -, -, -, -, -, hsl⟩ := hpre_of o n h
    have := pre_ne pre hpre
    have : s.val = 0 := by simp_all
    omega
  · have hs0 : s.val ≠ 0 := by
      intro h; apply (by assumption : ¬ s = 0#usize); apply UScalar.eq_of_val_eq; simp [h]
    obtain ⟨pre, hpre, hpl, hpp⟩ := hs1 hs0
    have hsl : s.val ≤ u.val.length := by rw [← hpl]; exact hpp.length_le
    have hu : u.val = pre ++ u.val.drop s.val := by
      rw [← hpl]
      conv_lhs => rw [← List.take_append_drop pre.length u.val]
      rw [← List.prefix_iff_eq_take.mp hpp]
    apply WP.spec_bind (path_end_spec u s hsl)
    rintro e ⟨he1, he2, ⟨g, t, hg, ht, hd⟩, hec⟩
    apply WP.spec_bind (owner_end_spec u s e he1 he2)
    rintro m ⟨hm1, hm2⟩
    have comp : ∀ o n, GitHubUrl u.val o n → m.val = s.val + o.length ∧
        slc u.val s.val (s.val + o.length) = o ∧ slc u.val (s.val + o.length + 1) e.val = n := by
      intro o n h
      obtain ⟨pre', g', t', hpre', hpo, hpn, hnn, hg', ht', hu', hs'⟩ := hpre_of o n h
      have hd' : u.val.drop s.val = o ++ 47#u8 :: n ++ g' ++ t' := by
        rw [hs']; conv_lhs => rw [hu']
        simp only [List.append_assoc, List.drop_left]
      have hx : e.val = s.val + (o ++ 47#u8 :: n).length :=
        hec (o ++ 47#u8 :: n) g' t' (by simp) (last_name o n hpn)
          (fun h => hnn (suffix_nosep _ _ _ _ (by rw [b_dotgit]; decide) h)) hg' ht' hd'
      have hsx : slc u.val s.val e.val = o ++ 47#u8 :: n := by
        rw [hx]; exact slc_of_drop _ _ (g' ++ t') _ (by rw [hd']; simp)
      refine ⟨hm2 o n hpo hpn hnn hsx, slc_of_drop _ _ (47#u8 :: n ++ g' ++ t') _ (by rw [hd']; simp), ?_⟩
      have hd2 : u.val.drop (s.val + o.length + 1) = n ++ (g' ++ t') := by
        rw [show s.val + o.length + 1 = s.val + (o.length + 1) by omega, ← List.drop_drop, hd']
        simp
      rw [hx, show s.val + (o ++ 47#u8 :: n).length = s.val + o.length + 1 + n.length by simp; omega]
      exact slc_of_drop _ _ _ _ hd2
    split
    · refine ⟨fun p hp => absurd hp (by simp), fun o n h => ?_⟩
      exfalso
      have := (comp o n h).1
      have : m.val = 0 := by rw [(by assumption : m = 0#usize)]; rfl
      have := (comp o n h).2.1
      obtain ⟨-, -, -, -, hpo, -⟩ := hpre_of o n h
      have := List.length_pos_of_ne_nil hpo.1
      omega
    · have hm0 : m.val ≠ 0 := by
        intro h; apply (by assumption : ¬ m = 0#usize); apply UScalar.eq_of_val_eq; simp [h]
      obtain ⟨h1, h2, h3, h4, h5, h6⟩ := hm1 hm0
      step*
      refine ⟨fun p hp => ?_, fun o n h => ?_⟩
      · simp only [Option.some.injEq] at hp
        subst hp
        simp only [part_eq]
        rw [i_post]
        have hnl := List.length_pos_of_ne_nil h5.1
        rw [List.length_take, List.length_drop] at hnl
        refine ⟨h1, rfl, by omega, he2, pre, g, t, hpre, h4, h5, h6, hg, ht, ?_⟩
        rw [byte_slash]
        conv_lhs => rw [hu, hd, h3]
        simp only [List.append_assoc, List.cons_append]
      · obtain ⟨hm, ho, hn⟩ := comp o n h
        refine ⟨_, rfl, ?_, ?_⟩
        · simp only [part_eq]; rw [hm]; exact ho
        · simp only [part_eq]; rw [i_post, hm]; exact hn

end GitArgCheck.GitHub
