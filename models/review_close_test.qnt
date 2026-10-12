// -*- mode: Bluespec; -*-
// Fixed traces of the close model (SPEC 26). `scripts/check-models.sh` runs them.
module review_close_test {
  import review_close.* from "./review_close"

  pure val OLD_GATE: Mode = { ...FIXED, gate: false }
  pure val OLD_DISCARD: Mode = { ...FIXED, exact: false }
  pure val OLD_GUARD: Mode = { ...FIXED, guard: false }

  // A close during the network fetch: the checkout then makes no folder.
  run closeDuringFetchMakesNoFolderTest =
    init.then(Open("r1"))
      .then(CloseAsk("r1"))
      .then(Checkout("r1"))
      .expect(and { Always, not(s.disk.get("p1")) })

  // The old order, with no check under the lock, leaks the folder with no note.
  run closeDuringFetchLeakedBeforeTest =
    init.then(Open("r1"))
      .then(CloseAsk("r1"))
      .then(checkout("r1", OLD_GATE))
      .expect(and { s.disk.get("p1"), not(NoSilentLeak) })

  // Changes: a close with no choice changes nothing; Keep the folder keeps them
  // and tells you; the sweep at start removes the folder when you revert them.
  run changesAskThenKeepThenSweepTest =
    init.then(Open("r1"))
      .then(Checkout("r1"))
      .then(Edit("p1", 1))
      .then(CloseAsk("r1"))
      .expect(s.open.get("r1"))
      .then(CloseKeep("r1"))
      .then(Clean({ r: "r1", seen: Set(), every: false }))
      .expect(and { Always, s.disk.get("p1"), s.told.get("p1") })
      .then(Revert("p1"))
      .then(Sweep("r1"))
      .expect(and { Always, not(s.disk.get("p1")) })

  // Discard what you saw, then a new change comes before the cleanup: it stays.
  run discardKeepsALaterChangeTest =
    init.then(Open("r1"))
      .then(Checkout("r1"))
      .then(Edit("p1", 1))
      .then(closeDiscard("r1", Set(1), FIXED))
      .then(Edit("p1", 2))
      .then(Clean({ r: "r1", seen: Set(1), every: false }))
      .expect(and { Always, s.disk.get("p1"), s.told.get("p1") })

  // The old Discard, a bool, dropped the later change too.
  run oldDiscardLostALaterChangeTest =
    init.then(Open("r1"))
      .then(Checkout("r1"))
      .then(Edit("p1", 1))
      .then(closeDiscard("r1", Set(1), OLD_DISCARD))
      .then(Edit("p1", 2))
      .then(clean1({ r: "r1", seen: Set(1), every: true }, OLD_DISCARD))
      .expect(not(NoLostWork))

  // A kept folder with your work: a new full review of the PR refuses to check
  // out over it. The old code checked out, and your work went into the review.
  run keptWorkRefusesACheckoutTest =
    init.then(Open("r1"))
      .then(Checkout("r1"))
      .then(Edit("p1", 1))
      .then(CloseKeep("r1"))
      .then(Clean({ r: "r1", seen: Set(), every: false }))
      .then(Open("r2"))
      .then(Checkout("r2"))
      .expect(and { Always, not(s.tree.get("r2")), s.dirty.get("p1") == Set(1) })
  run keptWorkWasCarriedBeforeTest =
    init.then(Open("r1"))
      .then(Checkout("r1"))
      .then(Edit("p1", 1))
      .then(CloseKeep("r1"))
      .then(Clean({ r: "r1", seen: Set(), every: false }))
      .then(Open("r2"))
      .then(checkout("r2", OLD_GUARD))
      .expect(not(NoCarry))

  // The round 1 folder of a reopened PR: the cleanup of the closed review waits
  // for the lock, and the new review has checked out the same folder. It stays.
  run reopenKeepsTheSharedFolderTest =
    init.then(Open("r1"))
      .then(Checkout("r1"))
      .then(CloseAsk("r1"))
      .then(Open("r2"))
      .then(Checkout("r2"))
      .then(Clean({ r: "r1", seen: Set(), every: false }))
      .expect(and { Always, s.disk.get("p1") })

  // A pane of yours or a lock keeps the folder, with a note.
  run yourPaneKeepsTheFolderTest =
    init.then(Open("r3"))
      .then(Checkout("r3"))
      .then(PaneOpen("p2"))
      .then(CloseAsk("r3"))
      .then(Clean({ r: "r3", seen: Set(), every: false }))
      .expect(and { Always, s.disk.get("p2"), s.told.get("p2") })
  run aLockKeepsTheFolderTest =
    init.then(Open("r3"))
      .then(Checkout("r3"))
      .then(Lock("p2"))
      .then(CloseAsk("r3"))
      .then(Clean({ r: "r3", seen: Set(), every: false }))
      .expect(and { Always, s.disk.get("p2"), s.told.get("p2") })
}
