// -*- mode: Bluespec; -*-
// Fixed traces of the local load model (SPEC 26.1). Each finding has a trace of
// the old code, which ends in a bad state, and the same trace with the fix.
// `scripts/check-models.sh` runs them.
module local_load_test {
  import local_load.* from "./local_load"

  pure val OLD_CLAIM: Mode = { ...FIXED, claim: false }
  pure val OLD_TICK: Mode = { ...FIXED, tick: false }
  pure val NO_SWEEP: Mode = { ...FIXED, sweep: false }
  pure val NO_REPAIR: Mode = { ...FIXED, repair: false }
  pure val NO_LOOK: Mode = { ...FIXED, look: false }

  pure def L(n: int, phase: str, snap: Snap): Load = { n: n, phase: phase, snap: snap }
  pure val S00: Snap = { h: 0, t: 0 }
  pure val S01: Snap = { h: 0, t: 1 }
  pure val S10: Snap = { h: 1, t: 0 }

  // L1. The first load fails: no head, "error". Two Retry clicks: the old rule
  // let both through. One load writes the head ref after the other made the view
  // ready, so the ref no longer keeps the snapshot that the view shows.
  run oldClaimStartsTwoLoadsTest =
    init.then(OpenLocal)
      .then(LoadFails(L(0, "start", NONE)))
      .then(LoadTail(L(0, "tail", NONE)))
      .expect(s.status == "error" and s.row == NONE)
      .then(claim(OLD_CLAIM))
      .then(claim(OLD_CLAIM))
      .expect(not(OneLoad))
      .then(LoadSnap(L(1, "start", NONE)))
      .then(Work)
      .then(LoadSnap(L(2, "start", NONE)))
      .then(LoadRef(L(2, "snapped", S01)))
      .then(LoadCheck(L(2, "check", S01)))
      .then(LoadRow(L(2, "row", S01)))
      .then(LoadDone(L(2, "done", S01)))
      .then(LoadRef(L(1, "snapped", S00)))
      .expect(and { s.status == "ready", s.row == S01, s.ref == S00, not(RefKeepsView) })
  run claimRefusesWhileLoadingTest =
    init.then(OpenLocal)
      .then(LoadFails(L(0, "start", NONE)))
      .then(LoadTail(L(0, "tail", NONE)))
      .then(Claim)
      .expect(and { Always, not(claimOk(FIXED)) })

  // L1, after a restart: the first load never ended, so the row has no head and
  // says Retry. The old rule let two Retry clicks through; the open itself never
  // goes through the claim now, so a click during the first load is refused too.
  run oldClaimAfterRestartTest =
    init.then(OpenLocal)
      .then(restart(OLD_CLAIM))
      .expect(s.status == "error")
      .then(claim(OLD_CLAIM))
      .then(claim(OLD_CLAIM))
      .expect(not(OneLoad))
  run claimAfterRestartTest =
    init.then(OpenLocal)
      .expect(not(claimOk(FIXED)))
      .then(Restart)
      .then(Claim)
      .expect(and { Always, not(claimOk(FIXED)) })

  // L2. After a restart `head_commit` is not in memory. The poll reads the view
  // (S00), a Refresh loads S10 with HEAD 1, and then the poll writes the parent of
  // S00 over it: New changes for the HEAD the view was made on.
  run oldTickAfterRestartTest =
    init.then(OpenLocal)
      .then(LoadSnap(L(0, "start", NONE)))
      .then(LoadRef(L(0, "snapped", S00)))
      .then(LoadCheck(L(0, "check", S00)))
      .then(LoadRow(L(0, "row", S00)))
      .then(LoadDone(L(0, "done", S00)))
      .then(LoadTail(L(0, "tail", S00)))
      .then(restart(OLD_TICK))
      .then(TickStart)
      .then(Commit)
      .then(TickHead)
      .then(TickKnown)
      .then(claim(OLD_TICK))
      .then(LoadSnap(L(2, "start", NONE)))
      .then(LoadRef(L(2, "snapped", S10)))
      .then(LoadCheck(L(2, "check", S10)))
      .then(LoadRow(L(2, "row", S10)))
      .then(LoadDone(L(2, "done", S10)))
      .then(tickParent(OLD_TICK))
      .expect(s.headCommit == 0)
      .then(tickDecide(OLD_TICK))
      .expect(and { s.newHead, not(NoFalseNew) })
  run tickAfterRestartTest =
    init.then(OpenLocal)
      .then(LoadSnap(L(0, "start", NONE)))
      .then(LoadRef(L(0, "snapped", S00)))
      .then(LoadCheck(L(0, "check", S00)))
      .then(LoadRow(L(0, "row", S00)))
      .then(LoadDone(L(0, "done", S00)))
      .then(LoadTail(L(0, "tail", S00)))
      .then(Restart)
      .then(TickStart)
      .then(Commit)
      .then(TickHead)
      .then(TickKnown)
      .then(Claim)
      .then(LoadSnap(L(2, "start", NONE)))
      .then(LoadRef(L(2, "snapped", S10)))
      .then(LoadCheck(L(2, "check", S10)))
      .then(LoadRow(L(2, "row", S10)))
      .then(LoadDone(L(2, "done", S10)))
      .then(TickParent)
      .then(TickDecide)
      .expect(and { Always, s.headCommit == 1, not(s.newHead) })

  // L2, with no restart: the poll reads HEAD 0, the agent commits, a Refresh loads
  // S10, and the old poll compared the new view with the old HEAD.
  run oldTickStaleHeadTest =
    init.then(OpenLocal)
      .then(LoadSnap(L(0, "start", NONE)))
      .then(LoadRef(L(0, "snapped", S00)))
      .then(LoadCheck(L(0, "check", S00)))
      .then(LoadRow(L(0, "row", S00)))
      .then(LoadDone(L(0, "done", S00)))
      .then(LoadTail(L(0, "tail", S00)))
      .then(TickStart)
      .then(TickHead)
      .then(TickKnown)
      .then(Commit)
      .then(Claim)
      .then(LoadSnap(L(1, "start", NONE)))
      .then(LoadRef(L(1, "snapped", S10)))
      .then(LoadCheck(L(1, "check", S10)))
      .then(LoadRow(L(1, "row", S10)))
      .then(LoadDone(L(1, "done", S10)))
      .then(tickDecide(OLD_TICK))
      .expect(not(NoFalseNew))
  run tickStaleHeadTest =
    init.then(OpenLocal)
      .then(LoadSnap(L(0, "start", NONE)))
      .then(LoadRef(L(0, "snapped", S00)))
      .then(LoadCheck(L(0, "check", S00)))
      .then(LoadRow(L(0, "row", S00)))
      .then(LoadDone(L(0, "done", S00)))
      .then(LoadTail(L(0, "tail", S00)))
      .then(TickStart)
      .then(TickHead)
      .then(TickKnown)
      .then(Commit)
      .then(Claim)
      .then(LoadSnap(L(1, "start", NONE)))
      .then(LoadRef(L(1, "snapped", S10)))
      .then(LoadCheck(L(1, "check", S10)))
      .then(LoadRow(L(1, "row", S10)))
      .then(LoadDone(L(1, "done", S10)))
      .then(TickDecide)
      .expect(and { Always, not(s.newHead) })

  // L3. A quit between the store write of a close and the ref drop. The old sweep
  // left out local rows: the refs stayed for good.
  run oldSweepLeaksTest =
    init.then(OpenLocal)
      .then(LoadSnap(L(0, "start", NONE)))
      .then(LoadRef(L(0, "snapped", S00)))
      .then(CloseTake)
      .then(CloseStore)
      .then(restart(NO_SWEEP))
      .then(sweep(NO_SWEEP))
      .expect(and { s.ref == S00, not(NoLeak) })
  run sweepDropsTheRefsTest =
    init.then(OpenLocal)
      .then(LoadSnap(L(0, "start", NONE)))
      .then(LoadRef(L(0, "snapped", S00)))
      .then(CloseTake)
      .then(CloseStore)
      .then(Restart)
      .then(Sweep)
      .expect(and { Always, s.ref == NONE })

  // L4. A quit during a Refresh, after its ref and before `save_row`: the start
  // shows the stored head, and the ref is on the newer snapshot.
  run noRepairTest =
    init.then(OpenLocal)
      .then(LoadSnap(L(0, "start", NONE)))
      .then(LoadRef(L(0, "snapped", S00)))
      .then(LoadCheck(L(0, "check", S00)))
      .then(LoadRow(L(0, "row", S00)))
      .then(LoadDone(L(0, "done", S00)))
      .then(LoadTail(L(0, "tail", S00)))
      .then(Work)
      .then(Claim)
      .then(LoadSnap(L(1, "start", NONE)))
      .then(LoadRef(L(1, "snapped", S01)))
      .then(restart(NO_REPAIR))
      .expect(and { s.row == S00, s.ref == S01, not(RefKeepsView) })
  run repairTest =
    init.then(OpenLocal)
      .then(LoadSnap(L(0, "start", NONE)))
      .then(LoadRef(L(0, "snapped", S00)))
      .then(LoadCheck(L(0, "check", S00)))
      .then(LoadRow(L(0, "row", S00)))
      .then(LoadDone(L(0, "done", S00)))
      .then(LoadTail(L(0, "tail", S00)))
      .then(Work)
      .then(Claim)
      .then(LoadSnap(L(1, "start", NONE)))
      .then(LoadRef(L(1, "snapped", S01)))
      .then(Restart)
      .expect(and { Always, s.ref == S00 })

  // L5. A turn ends with new work, and the app quits before its thread shows New
  // changes (or after: New changes is in memory only). The start looks once.
  run noLookTest =
    init.then(OpenLocal)
      .then(LoadSnap(L(0, "start", NONE)))
      .then(LoadRef(L(0, "snapped", S00)))
      .then(LoadCheck(L(0, "check", S00)))
      .then(LoadRow(L(0, "row", S00)))
      .then(LoadDone(L(0, "done", S00)))
      .then(LoadTail(L(0, "tail", S00)))
      .then(Work)
      .then(TurnEnd)
      .then(restart(NO_LOOK))
      .then(sweep(NO_LOOK))
      .expect(not(TurnKept))
  run lookTest =
    init.then(OpenLocal)
      .then(LoadSnap(L(0, "start", NONE)))
      .then(LoadRef(L(0, "snapped", S00)))
      .then(LoadCheck(L(0, "check", S00)))
      .then(LoadRow(L(0, "row", S00)))
      .then(LoadDone(L(0, "done", S00)))
      .then(LoadTail(L(0, "tail", S00)))
      .then(Work)
      .then(TurnEnd)
      .then(Restart)
      .then(Sweep)
      .then(TurnSnap({ n: 2, head: S00, now: -1 }))
      .then(TurnDecide({ n: 2, head: S00, now: 1 }))
      .expect(and { Always, s.newHead })
}
