// -*- mode: Bluespec; -*-
// A model of the load cycle of a local review (SPEC 11.11, 11.14, 26.1).
//
// It follows the code in crates/core/src/hub_review.rs. Each action names the
// function that it models, and each action is one lock of the code (or one git
// call). `scripts/check-models.sh` runs the checks.
//
// The parts:
// - The agent's folder: its HEAD and its work tree, as two counters. They only go
//   up, so an old value is never a new one.
// - One local review, in `Hub.reviews` and in the store.
// - The snapshot refs in git: `refs/sb/review/<id>/head` and `.../reviewed`.
// - The threads: loads (`spawn_local`), turn snapshots (`turn_snapshot`), the
//   30 s poll (`review_tick`), a close, and the sweep at start.
// - A crash (a quit) at any time, then a start (`load_reviews`).
//
// `Mode` turns one fix off at a time. `step` has all of them. The other steps
// are the code before each fix, and each one must break `Always`.
module local_load {

  // A snapshot: the HEAD it was made on and the work tree it holds.
  type Snap = { h: int, t: int }
  pure val NONE: Snap = { h: -1, t: -1 }
  pure val MAX = 3

  // The fixes.
  // `claim`: any "fetching" refuses a claim, also with no head yet. The old rule
  //   refused only when the row had a head, so a Retry after a failed load or a
  //   restart could start a second load.
  // `tick`: the poll writes `head_commit` and `new_head` only while the row still
  //   shows the snapshot it read. The old poll wrote the parent of an old snapshot
  //   over the HEAD of a newer one.
  // `sweep`: the sweep at start drops the refs of closed local reviews. The old
  //   sweep left out local rows, so a quit between the store write and the ref drop
  //   of a close leaked them.
  // `repair`: the start points the head ref at the row of the store. A quit during
  //   a load can leave the ref on a newer snapshot than the stored row.
  // `look`: the start makes one turn snapshot for each open local review: `new_head`
  //   is in memory only, so a quit lost New changes.
  type Mode = { claim: bool, tick: bool, sweep: bool, repair: bool, look: bool }
  pure val FIXED: Mode = { claim: true, tick: true, sweep: true, repair: true, look: true }

  // A load thread. `n` tells two loads apart. The phases, in order:
  // "mark", "markCheck" (Mark reviewed, before its load), "start", "snapped",
  // "check", "row", "done", "tail".
  type Load = { n: int, phase: str, snap: Snap }
  // The thread of `turn_snapshot`: the head it saw, and the tree of its snapshot
  // (-1 before the snapshot).
  type Turn = { n: int, head: Snap, now: int }
  // The poll, one at a time: the row head it read, HEAD, and the HEAD the view was
  // made on. Phases: "none", "head", "known", "parent", "decide".
  type Tick = { phase: str, head: Snap, now: int, known: int }
  pure val NO_TICK: Tick = { phase: "none", head: NONE, now: -1, known: -1 }

  type State = {
    // The agent's folder.
    head: int,
    tree: int,
    // `Hub.reviews`. `made`: opened once (an id is never used again).
    made: bool,
    open: bool,
    status: str,
    row: Snap,
    headCommit: int,
    newHead: bool,
    pending: bool,
    // The store: the review is open there, and its saved head.
    stored: bool,
    storedRow: Snap,
    // git.
    ref: Snap,
    reviewed: bool,
    // Threads.
    loads: Set[Load],
    turns: Set[Turn],
    tick: Tick,
    closing: str,
    sweep: bool,
    next: int,
    // Ghosts. `falseNew`: New changes showed while HEAD was the HEAD of the view.
    // `lastTurn`: the tree when the last turn ended.
    falseNew: bool,
    lastTurn: int,
  }

  var s: State

  action init = s' = {
    head: 0, tree: 0,
    made: false, open: false, status: "none", row: NONE, headCommit: -1, newHead: false, pending: false,
    stored: false, storedRow: NONE,
    ref: NONE, reviewed: false,
    loads: Set(), turns: Set(), tick: NO_TICK, closing: "none", sweep: false, next: 0,
    falseNew: false, lastTurn: -1,
  }

  def newLoad(phase: str, snap: Snap): Set[Load] = s.loads.union(Set({ n: s.next, phase: phase, snap: snap }))
  def swap(l: Load, l2: Load): Set[Load] = s.loads.exclude(Set(l)).union(Set(l2))

  // ---------- the agent ----------

  action Work: bool = all { s.tree < MAX, s' = { ...s, tree: s.tree + 1 } }
  action Commit: bool = all { s.head < MAX, s' = { ...s, head: s.head + 1 } }

  // ---------- opening and claims ----------

  // `review_open_local`: the insert as "fetching", `save_row`, and the first load.
  action OpenLocal: bool = all {
    not(s.made),
    s' = { ...s,
      made: true, open: true, status: "fetching", row: NONE, stored: true, storedRow: NONE,
      loads: newLoad("start", NONE), next: s.next + 1 },
  }

  // `claim_local` from Retry, Refresh, Restart (`local_refresh_bg`) and New, All,
  // Base and Update base, then `spawn_local`.
  def claimOk(m: Mode): bool =
    s.open and (if (m.claim) s.status != "fetching" else not(s.status == "fetching" and s.row != NONE))
  action claim(m: Mode): bool = all {
    claimOk(m),
    s' ={ ...s, status: "fetching", loads: newLoad("start", NONE), next: s.next + 1 },
  }
  action Claim: bool = claim(FIXED)

  // `review_mark_reviewed`: the claim, with the head of the view.
  action Mark: bool = all {
    s.open, s.status != "fetching", s.row != NONE,
    s' = { ...s, status: "fetching", loads: newLoad("mark", s.row), next: s.next + 1 },
  }

  // ---------- a load ----------

  // `set_status` on a row that is still there.
  def status(st: str): str = if (s.open) st else s.status

  // `turn_snapshot`, the part under the lock: a thread for a ready review, a kept
  // turn for a review that loads.
  def turnSnapshot(t: State): State =
    if (t.open and t.status == "ready")
      { ...t, turns: t.turns.union(Set({ n: t.next, head: t.row, now: -1 })), next: t.next + 1 }
    else if (t.open and t.status == "fetching") { ...t, pending: true }
    else t

  // Mark reviewed: `update-ref` of `reviewed`, then the open check.
  action MarkRef(l: Load): bool = all {
    l.phase == "mark",
    s' = { ...s, reviewed: true, loads: swap(l, { ...l, phase: "markCheck" }) },
  }
  action MarkFails(l: Load): bool = all {
    l.phase == "mark",
    s' = { ...s, status: status("error"), loads: s.loads.exclude(Set(l)) },
  }
  action MarkCheck(l: Load): bool = all {
    l.phase == "markCheck",
    if (s.open) s' = { ...s, loads: swap(l, { ...l, phase: "start" }) }
    else s' = { ...s, ref: NONE, reviewed: false, loads: s.loads.exclude(Set(l)) },
  }

  // `local_refresh`: `review::snapshot`, the work tree as a commit on HEAD.
  action LoadSnap(l: Load): bool = all {
    l.phase == "start",
    s' = { ...s, loads: swap(l, { ...l, phase: "snapped", snap: { h: s.head, t: s.tree } }) },
  }
  // A git error before the ref: the status goes to "error".
  action LoadFails(l: Load): bool = all {
    l.phase == "start" or l.phase == "snapped",
    s' = { ...s, status: status("error"), loads: swap(l, { ...l, phase: "tail" }) },
  }
  // The `update-ref` of the head ref.
  action LoadRef(l: Load): bool = all {
    l.phase == "snapped",
    s' = { ...s, ref: l.snap, loads: swap(l, { ...l, phase: "check" }) },
  }
  // The open check: a close that took the row maybe dropped the refs already.
  action LoadCheck(l: Load): bool = all {
    l.phase == "check",
    if (s.open) s' = { ...s, loads: swap(l, { ...l, phase: "row" }) }
    else s' = { ...s, ref: NONE, reviewed: false, loads: swap(l, { ...l, phase: "tail" }) },
  }
  // The row takes the snapshot; `new_head` goes.
  action LoadRow(l: Load): bool = all {
    l.phase == "row",
    if (s.open) s' = { ...s, row: l.snap, headCommit: l.snap.h, newHead: false, loads: swap(l, { ...l, phase: "done" }) }
    else s' = { ...s, loads: swap(l, { ...l, phase: "tail" }) },
  }
  // `save_row` (the row as it is now), `load_model`, and "ready".
  action LoadDone(l: Load): bool = all {
    l.phase == "done",
    s' = { ...s,
      storedRow: if (s.open) s.row else s.storedRow,
      status: status("ready"),
      loads: swap(l, { ...l, phase: "tail" }) },
  }
  // The end of `spawn_local`: a turn that ended during the load runs now, unless a
  // newer load runs and takes it.
  action LoadTail(l: Load): bool = all {
    l.phase == "tail",
    val t = { ...s, loads: s.loads.exclude(Set(l)) }
    s' = if (s.open and s.pending and s.status != "fetching") turnSnapshot({ ...t, pending: false }) else t,
  }

  // ---------- a turn ----------

  // The Stop hook of the agent: `turn_snapshot`.
  action TurnEnd: bool = s' = turnSnapshot({ ...s, lastTurn: s.tree })

  // The thread: a snapshot, then the check under the lock.
  action TurnSnap(u: Turn): bool = all {
    u.now < 0,
    s' = { ...s, turns: s.turns.exclude(Set(u)).union(Set({ ...u, now: s.tree })) },
  }
  action TurnDecide(u: Turn): bool = all {
    u.now >= 0,
    val t = { ...s, turns: s.turns.exclude(Set(u)) }
    s' = if (u.now == u.head.t) t
      else if (s.open and s.status == "ready" and s.row == u.head) { ...t, newHead: true }
      else if (s.open and s.status == "fetching") { ...t, pending: true }
      else t,
  }

  // ---------- the poll ----------

  // `review_tick`: the ready local reviews and their heads.
  action TickStart: bool = all {
    s.tick.phase == "none", s.open, s.status == "ready",
    s' = { ...s, tick: { ...NO_TICK, phase: "head", head: s.row } },
  }
  // `rev-parse HEAD`.
  action TickHead: bool = all {
    s.tick.phase == "head",
    s' = { ...s, tick: { ...s.tick, phase: "known", now: s.head } },
  }
  // `head_commit` in memory, or none after a restart.
  action TickKnown: bool = all {
    s.tick.phase == "known",
    val hc = if (s.open) s.headCommit else -1
    s' = { ...s, tick: if (hc >= 0) { ...s.tick, phase: "decide", known: hc } else { ...s.tick, phase: "parent" } },
  }
  // The parent of the snapshot read at the start, written to `head_commit`.
  action tickParent(m: Mode): bool = all {
    s.tick.phase == "parent",
    val madeOn = s.tick.head.h
    val write = s.open and (m.tick implies (s.row == s.tick.head and s.headCommit < 0))
    s' = { ...s,
      headCommit: if (write) madeOn else s.headCommit,
      tick: { ...s.tick, phase: "decide", known: madeOn } },
  }
  action TickParent: bool = tickParent(FIXED)
  // The compare with the row as it is now.
  action tickDecide(m: Mode): bool = all {
    s.tick.phase == "decide",
    val hc = if (s.headCommit >= 0) s.headCommit else s.tick.known
    val show = s.open and s.status == "ready" and hc != s.tick.now and (m.tick implies s.row == s.tick.head)
    s' = { ...s,
      newHead: s.newHead or show,
      falseNew: s.falseNew or (show and s.head == s.row.h),
      tick: NO_TICK },
  }
  action TickDecide: bool = tickDecide(FIXED)

  // ---------- close ----------

  // `review_close`: the row leaves the map, the store says closed, then
  // `drop_snap_refs`.
  action CloseTake: bool = all { s.open, s.closing == "none", s' = { ...s, open: false, closing: "store" } }
  action CloseStore: bool = all { s.closing == "store", s' = { ...s, stored: false, closing: "drop" } }
  action CloseDrop: bool = all { s.closing == "drop", s' = { ...s, ref: NONE, reviewed: false, closing: "none" } }

  // ---------- a quit, and the start ----------

  // Every thread and all memory go; git and the store stay. Then `Hub::start`:
  // `load_reviews` (a row with a head is "ready", else "error"), and the sweep in
  // the background.
  action restart(m: Mode): bool = all {
    s.made,
    val status = if (not(s.stored)) "none" else if (s.storedRow != NONE) "ready" else "error"
    val t = { ...s,
      open: s.stored, status: status, row: s.storedRow, headCommit: -1, newHead: false, pending: false,
      ref: if (m.repair and s.stored and s.storedRow != NONE) s.storedRow else s.ref,
      loads: Set(), turns: Set(), tick: NO_TICK, closing: "none", sweep: true }
    s' = if (m.look) turnSnapshot(t) else t,
  }
  action Restart: bool = restart(FIXED)

  // `sweep_snap_refs`: the refs of a review that the store has as closed.
  action sweep(m: Mode): bool = all {
    s.sweep,
    val drop = m.sweep and not(s.stored) and not(s.open)
    s' = { ...s, sweep: false, ref: if (drop) NONE else s.ref, reviewed: s.reviewed and not(drop) },
  }
  action Sweep: bool = sweep(FIXED)

  action stepWith(m: Mode): bool = any {
    Work, Commit, OpenLocal, claim(m), Mark, TurnEnd,
    TickStart, TickHead, TickKnown, tickParent(m), tickDecide(m),
    CloseTake, CloseStore, CloseDrop, restart(m), sweep(m),
    all {
      s.loads != Set(),
      nondet l = s.loads.oneOf()
      any { MarkRef(l), MarkFails(l), MarkCheck(l), LoadSnap(l), LoadFails(l), LoadRef(l), LoadCheck(l), LoadRow(l), LoadDone(l), LoadTail(l) },
    },
    all {
      s.turns != Set(),
      nondet u = s.turns.oneOf()
      any { TurnSnap(u), TurnDecide(u) },
    },
  }
  // Only the agent's commits, loads, restarts and the poll: more runs reach a
  // poll that a load overtakes.
  action pollWith(m: Mode): bool = any {
    Commit, OpenLocal, claim(m), restart(m),
    TickStart, TickHead, TickKnown, tickParent(m), tickDecide(m),
    all {
      s.loads != Set(),
      nondet l = s.loads.oneOf()
      any { LoadSnap(l), LoadRef(l), LoadCheck(l), LoadRow(l), LoadDone(l), LoadTail(l) },
    },
  }
  action step = stepWith(FIXED)
  action stepPoll = pollWith(FIXED)
  action stepPollOldTick = pollWith({ ...FIXED, tick: false })
  // The code before each fix. Each one must break `Always`.
  action stepOldClaim = stepWith({ ...FIXED, claim: false })
  action stepNoSweep = stepWith({ ...FIXED, sweep: false })
  action stepNoRepair = stepWith({ ...FIXED, repair: false })
  action stepNoLook = stepWith({ ...FIXED, look: false })

  // ---------- properties ----------

  // At most one load of the review runs. A load in its tail (after "ready" or
  // "error") only looks at the kept turn: a new claim may start then.
  val OneLoad = s.loads.filter(l => l.phase != "tail").size() <= 1

  // While the review shows a snapshot and no load runs, the head ref keeps that
  // snapshot from `git gc`.
  val RefKeepsView = (s.open and s.status != "fetching" and s.row != NONE) implies s.ref == s.row

  // After a close, and once its threads and the sweep at start end, no ref is left.
  val Quiet = s.loads == Set() and s.turns == Set() and s.closing == "none" and not(s.sweep)
  val NoLeak = (s.made and not(s.open) and not(s.stored) and Quiet) implies (s.ref == NONE and not(s.reviewed))

  // New changes from the poll show only when HEAD is not the HEAD of the view.
  val NoFalseNew = not(s.falseNew)

  // A turn that ended after the tree changed is not lost: once the threads end, the
  // ready view has that tree, or it shows New changes.
  val TurnKept = (Quiet and s.open and s.status == "ready" and s.lastTurn >= 0)
    implies (s.row.t >= s.lastTurn or s.newHead)

  val Always = OneLoad and RefKeepsView and NoLeak and NoFalseNew and TurnKept

  // Witnesses. Each must fail in `step`: else the runs never reach a turn kept
  // during a load, Mark reviewed, New changes, or a sweep with a ref to drop.
  val NoKeptTurn = not(s.pending)
  val NoMarkRef = not(s.reviewed)
  val NoNewChanges = not(s.newHead)
  val NoSweepOfARef = not(s.sweep and s.ref != NONE and not(s.open) and not(s.stored))
}
