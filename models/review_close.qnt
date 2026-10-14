// -*- mode: Bluespec; -*-
// A model of review close and worktree cleanup (SPEC 11.10, 26).
//
// It follows the code in crates/core/src/hub_review.rs. Each action names the
// function that it models. Each action is one critical section of the code: the
// hub holds the `trees` lock in `fetch` (the checkout), in `review_close` (the take
// out of the review), and in `clean_after` (the check and the removal).
// `scripts/check-models.sh` runs the checks.
//
// The parts:
// - Reviews. "r1" and "r2" are round 1 of the same PR, so they share the folder
//   "p1" (`worktree_name`). Only one of them is open at a time (`review_open` gives
//   the open one). "r3" is round 2, in "p2", and it can be open with either.
// - A background job of a review: the network fetch, then the checkout.
// - Cleanup threads, one for each close, and the sweep at start.
// - You: your own panes in a folder, locks, and changes in it. A change has a
//   number, so a choice to drop "the changes you saw" is a set.
//
// `Mode` turns one fix off at a time. `step` has all of them. The other steps
// are the code before each fix, and each one must break an invariant.
module review_close {

  // "l1" is a review of local commits (SPEC 11.11). Its folder "a1" is the agent's
  // worktree: it exists from the start and holds the agent's unpushed commits.
  pure val PRS = Set("r1", "r2", "r3")
  pure val REVIEWS = PRS.union(Set("l1"))
  pure val OURS = Set("p1", "p2")
  pure val PATHS = OURS.union(Set("a1"))
  pure def isLocal(r: str): bool = r == "l1"
  pure def path(r: str): str = if (r == "l1") "a1" else if (r == "r3") "p2" else "p1"
  // Reviews that cannot be open together: the same PR and round.
  pure def twin(a: str, b: str): bool = a != b and path(a) == path(b)

  // The fixes. `gate`: the checkout looks at the review first. `exact`: Discard
  // drops only the changes you saw. `guard`: a checkout refuses a folder with your
  // work. `lock`: a removal keeps a locked worktree.
  // `local`: the guards of a local review (no cleanup thread, no sweep, no pin, no
  // change prompt). With it off, a local review is cleaned like a PR review.
  type Mode = { gate: bool, exact: bool, guard: bool, lock: bool, local: bool }
  pure val FIXED: Mode = { gate: true, exact: true, guard: true, lock: true, local: true }
  pure val CHANGES = Set(1, 2)

  // A cleanup thread: the review, the changes you saw and chose to drop, and
  // `every`: the old Discard, a bool that dropped every change.
  type Cleanup = { r: str, seen: Set[int], every: bool }

  type State = {
    // `open`: in `Hub.reviews`. `made`: it was ever opened.
    open: str -> bool,
    made: str -> bool,
    // A background job runs: the fetch and then the checkout (`review_prepare`,
    // `review_update_now`).
    job: str -> bool,
    // `row.tree` is set: the checkout ran for this review.
    tree: str -> bool,
    // The folder: on disk, your changes in it, a pane of yours, a lock.
    disk: str -> bool,
    dirty: str -> Set[int],
    pane: str -> bool,
    locked: str -> bool,
    // A cleanup kept the folder and told you why (`review_cleanup` event).
    told: str -> bool,
    cleanups: Set[Cleanup],
    // The agent's unpushed commits in "a1". They count as work, unless a ref of
    // ours (a pin) reaches them: then `local_changes` does not see them.
    commits: bool,
    pinned: bool,
    // Ghosts. `lost`: a removal dropped a change that you did not choose to drop.
    // `carried`: a checkout ran in a folder with your work. `unlocked`: a removal
    // took a locked worktree.
    lost: bool,
    carried: bool,
    unlocked: bool,
  }

  var s: State

  action init = s' = {
    open: REVIEWS.mapBy(_ => false),
    made: REVIEWS.mapBy(_ => false),
    job: REVIEWS.mapBy(_ => false),
    tree: REVIEWS.mapBy(_ => false),
    disk: PATHS.mapBy(p => p == "a1"),
    dirty: PATHS.mapBy(_ => Set()),
    pane: PATHS.mapBy(p => p == "a1"),
    locked: PATHS.mapBy(_ => false),
    told: PATHS.mapBy(_ => false),
    cleanups: Set(),
    commits: true,
    pinned: false,
    lost: false,
    carried: false,
    unlocked: false,
  }

  // ---------- the hub ----------

  // `review_open` and `start_round`: a new review starts its job.
  action Open(r: str): bool = all {
    PRS.contains(r),
    not(s.made.get(r)),
    REVIEWS.forall(o => twin(r, o) implies not(s.open.get(o))),
    s' = { ...s,
      open: s.open.set(r, true), made: s.made.set(r, true), job: s.job.set(r, true) },
  }

  // `review_update`: a fetch again for an open review with no job.
  action Update(r: str): bool = all {
    PRS.contains(r), s.open.get(r), not(s.job.get(r)), s.tree.get(r),
    s' = { ...s, job: s.job.set(r, true) },
  }

  // The network part failed: the job ends with no checkout.
  action FetchFails(r: str): bool = all {
    s.job.get(r),
    s' = { ...s, job: s.job.set(r, false) },
  }

  // `fetch`, the part under the lock, with `review::checkout_tree`.
  action checkout(r: str, m: Mode): bool = all {
    s.job.get(r),
    val p = path(r)
    // `own`: an Update of this review in its own folder. Git carries your edits
    // along there, as you expect; the guard is for the folders of other reviews.
    val own = s.tree.get(r)
    val work = s.disk.get(p) and s.dirty.get(p) != Set() and not(own)
    if (m.gate and not(s.open.get(r)))
      // The review closed during the fetch: no folder.
      s' = { ...s, job: s.job.set(r, false) }
    else if (m.guard and work)
      // The folder has your work: the job ends with an error, and nothing changes.
      s' = { ...s, job: s.job.set(r, false) }
    else
      s' = { ...s,
        job: s.job.set(r, false),
        tree: s.tree.set(r, s.open.get(r) or s.tree.get(r)),
        disk: s.disk.set(p, true),
        told: s.told.set(p, false),
        carried: s.carried or work },
  }
  action Checkout(r: str): bool = checkout(r, FIXED)

  // The review leaves the map, and a cleanup thread starts when it has a folder.
  // A local review starts no cleanup thread (`clean_after` returns at once).
  def closed(r: str, c: Cleanup, m: Mode): State = { ...s,
    open: s.open.set(r, false),
    cleanups: if (s.tree.get(r) and not(m.local and isLocal(r))) s.cleanups.union(Set(c)) else s.cleanups }

  // `review_close` with no choice. With changes, nothing happens: the window asks.
  // A local review never asks: the changes are the agent's.
  action closeAsk(r: str, m: Mode): bool = all {
    s.open.get(r),
    val asks = s.tree.get(r) and s.disk.get(path(r)) and s.dirty.get(path(r)) != Set() and not(m.local and isLocal(r))
    s' = if (asks) s else closed(r, { r: r, seen: Set(), every: false }, m),
  }
  action CloseAsk(r: str): bool = closeAsk(r, FIXED)
  // `review_close` with Keep the folder.
  action closeKeep(r: str, m: Mode): bool = all {
    s.open.get(r),
    s' = closed(r, { r: r, seen: Set(), every: false }, m),
  }
  action CloseKeep(r: str): bool = closeKeep(r, FIXED)
  // `review_close` with Discard and the list `seen` that the window showed. A
  // change that is not on the list makes it ask again. The old code had no list.
  action closeDiscard(r: str, seen: Set[int], m: Mode): bool = all {
    s.open.get(r),
    val asks = s.tree.get(r) and s.disk.get(path(r)) and not(s.dirty.get(path(r)).subseteq(seen))
    s' = if (m.exact)
        (if (asks) s else closed(r, { r: r, seen: seen, every: false }, m))
      else closed(r, { r: r, seen: seen, every: true }, m),
  }

  // `clean_after` for review `r`: the rule of `cleanup::verdict`, under the lock.
  def clean(c: Cleanup, m: Mode): State = {
    val p = path(c.r)
    val used = REVIEWS.exists(o => s.open.get(o) and s.tree.get(o) and path(o) == p)
    // `pin` runs first. In the agent's folder it hides the agent's commits.
    val pins = isLocal(c.r) or s.pinned
    // The agent's commits that `local_changes` sees: none once a pin reaches them.
    val seenCommits = p == "a1" and s.commits and not(pins)
    val drop = if (m.exact) s.dirty.get(p).subseteq(c.seen) and not(seenCommits)
      else (s.dirty.get(p) == Set() and not(seenCommits)) or c.every
    if (m.local and isLocal(c.r)) s
    else if (not(s.disk.get(p))) s
    else if (used) { ...s, pinned: pins }
    else if (s.pane.get(p)) { ...s, pinned: pins, told: s.told.set(p, true) }
    else if (m.lock and s.locked.get(p)) { ...s, pinned: pins, told: s.told.set(p, true) }
    else if (not(drop)) { ...s, pinned: pins, told: s.told.set(p, true) }
    else if (s.pane.get(p)) { ...s, told: s.told.set(p, true) }
    else if (m.lock and s.locked.get(p)) { ...s, told: s.told.set(p, true) }
    else if (not(drop)) { ...s, told: s.told.set(p, true) }
    else { ...s,
      disk: s.disk.set(p, false),
      dirty: s.dirty.set(p, Set()),
      locked: s.locked.set(p, false),
      pinned: pins,
      commits: if (p == "a1") false else s.commits,
      lost: s.lost or not(s.dirty.get(p).subseteq(c.seen)) or (p == "a1" and s.commits),
      unlocked: s.unlocked or s.locked.get(p) }
  }
  action clean1(c: Cleanup, m: Mode): bool = all {
    s.cleanups.contains(c),
    val t = clean(c, m)
    s' = { ...t, cleanups: s.cleanups.exclude(Set(c)) },
  }
  action Clean(c: Cleanup): bool = clean1(c, FIXED)

  // `sweep_closed` at start: the same rule for each closed review with a folder.
  // `closed_reviews` leaves out local rows, so the sweep never sees one.
  action sweep(r: str, m: Mode): bool = all {
    s.made.get(r), not(s.open.get(r)), s.tree.get(r),
    not(m.local and isLocal(r)),
    s' = clean({ r: r, seen: Set(), every: false }, m),
  }
  action Sweep(r: str): bool = sweep(r, FIXED)

  // `review_open_local`: no job and no checkout; it reads the agent's folder.
  action OpenLocal: bool = all {
    not(s.open.get("l1")), s.disk.get("a1"),
    s' = { ...s, open: s.open.set("l1", true), made: s.made.set("l1", true), tree: s.tree.set("l1", true) },
  }

  // The agent commits, or pushes its commits.
  action AgentCommits: bool = all { s.disk.get("a1"), s' = { ...s, commits: true } }
  action AgentPushes: bool = all { s.commits, s' = { ...s, commits: false } }

  // ---------- you ----------

  action Edit(p: str, c: int): bool = all { s.disk.get(p), s' = { ...s, dirty: s.dirty.set(p, s.dirty.get(p).union(Set(c))) } }
  action Revert(p: str): bool = all { s.dirty.get(p) != Set(), s' = { ...s, dirty: s.dirty.set(p, Set()) } }
  action PaneOpen(p: str): bool = all { s.disk.get(p), s' = { ...s, pane: s.pane.set(p, true) } }
  action PaneClose(p: str): bool = all { s.pane.get(p), s' = { ...s, pane: s.pane.set(p, false) } }
  action Lock(p: str): bool = all { s.disk.get(p), s' = { ...s, locked: s.locked.set(p, true) } }
  action Unlock(p: str): bool = all { s.locked.get(p), s' = { ...s, locked: s.locked.set(p, false) } }

  action stepWith(m: Mode): bool = {
    nondet r = REVIEWS.oneOf()
    nondet p = PATHS.oneOf()
    nondet c = CHANGES.oneOf()
    nondet seen = CHANGES.powerset().oneOf()
    any {
      Open(r), Update(r), FetchFails(r), checkout(r, m),
      OpenLocal, AgentCommits, AgentPushes,
      closeAsk(r, m), closeKeep(r, m), closeDiscard(r, seen, m), sweep(r, m),
      Edit(p, c), Revert(p), PaneOpen(p), PaneClose(p), Lock(p), Unlock(p),
      all { s.cleanups != Set(), nondet x = s.cleanups.oneOf() clean1(x, m) },
    }
  }
  action step = stepWith(FIXED)
  // The code before each fix. Each one must break `Always`.
  action stepNoGate = stepWith({ ...FIXED, gate: false })
  action stepBoolDiscard = stepWith({ ...FIXED, exact: false })
  action stepNoGuard = stepWith({ ...FIXED, guard: false })
  action stepForceLock = stepWith({ ...FIXED, lock: false })
  action stepNoLocalGuard = stepWith({ ...FIXED, local: false })

  // ---------- properties ----------

  // An open review that has its folder keeps it: no cleanup removes it.
  val OpenKeepsFolder = REVIEWS.forall(r =>
    (s.open.get(r) and s.tree.get(r)) implies s.disk.get(path(r)))

  // A removal never drops a change that you did not see and choose to drop.
  val NoLostWork = not(s.lost)

  // A checkout never runs over your work in a kept folder.
  val NoCarry = not(s.carried)

  // A removal never takes a locked worktree.
  val LockHolds = not(s.unlocked)

  // No silent leak: when no job and no cleanup is left, each folder on disk is in
  // use by an open review, has a pane of yours, or a note told you why it stays.
  val Quiet = s.cleanups == Set() and REVIEWS.forall(r => not(s.job.get(r)))
  // Only our folders: the agent's folder is not ours to remove.
  val NoSilentLeak = Quiet implies OURS.forall(p =>
    s.disk.get(p) implies (
      REVIEWS.exists(r => s.open.get(r) and path(r) == p)
      or s.pane.get(p) or s.told.get(p)))

  // The agent's folder is never removed: not by a close, a cleanup or the sweep.
  val AgentFolderStays = s.disk.get("a1")

  val Always = OpenKeepsFolder and NoLostWork and NoCarry and LockHolds and NoSilentLeak and AgentFolderStays
}
