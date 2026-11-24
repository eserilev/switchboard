// -*- mode: Bluespec; -*-
// Fixed traces for several windows on one hub (SPEC 41.8). `quint test` runs them.
module windows_test {
  import windows.* from "./windows"

  action answerAny = { nondet o = s.outbox.oneOf() Answer(o) }
  action makeA = Make("A").then(Read)

  // Window `e` shows pane `p`: the expand runs and its answer is queued.
  action showWith(e: int, p: str, locked: bool): bool =
    WinExpand(e, p)
      .then(CmdStart(e, 0, true, locked))
      .then(Capture)
      .then(ShowOn)
      .then(answerAny)
  action show(e: int, p: str): bool = showWith(e, p, true)

  // Two windows show pane A.
  action twoOnA = Attach(0, true).then(Attach(0, true)).then(show(1, "A")).then(show(2, "A"))

  // Both windows get each chunk of the pane.
  run bothWindowsGetTheBytesTest =
    init.then(twoOnA).then(makeA)
      .expect(s.outq.get(1).get("A").bytes.length() == 1 and s.outq.get(2).get("A").bytes.length() == 1)

  // Pane A drops in the queue of window 1: three chunks and a budget of two.
  action dropInOne = twoOnA.then(makeA).then(makeA).then(makeA).then(Write(1))

  // The fresh screen comes from the emulator: pane A keeps its stream for window 2.
  run aFreshScreenKeepsTheOtherStreamTest =
    init.then(dropInOne)
      .then(Resync(1))
      .expect(s.streaming.get("A") and s.emu.get("A").on and s.filled == 1 and s.shared == 1)

  // The fresh screen of remote.qnt paused pane A: window 2 then gets no bytes of it.
  run thePauseStopsTheOtherWindowTest =
    init.then(dropInOne)
      .then(PauseFill(1))
      .expect(s.live.get(2) == "A" and not(s.streaming.get("A")))

  // One window leaves the pane: it streams on for the other.
  run aCollapseKeepsTheOtherStreamTest =
    init.then(twoOnA)
      .then(WinCollapse(1))
      .then(CmdStart(1, 0, true, true))
      .expect(s.live.get(1) == "" and s.streaming.get("A"))

  // With no screen lock, a collapse decides to stop pane A, window 2 shows it
  // meanwhile, and then the stop comes: window 2 shows a pane with no stream.
  run aCollapseWithNoLockStopsTheOtherTest =
    init.then(Attach(0, true)).then(Attach(0, true)).then(show(1, "A"))
      .then(WinCollapse(1))
      .then(CmdStart(1, 0, true, false))
      .then(showWith(2, "A", false))
      .then(StopStream("A"))
      .expect(s.live.get(2) == "A" and not(s.streaming.get("A")))

  // Window 1 sees an event, leaves, and comes back as window 2 after two more.
  action backAfterTwo(locked: bool): bool =
    Attach(0, true).then(Emit).then(LinkDrop(1)).then(Exit(1, true))
      .then(Emit).then(Emit).then(Attach(1, locked))

  // Under the log lock, the window that comes back gets the two it missed, then new ones.
  run aWindowThatComesBackGetsWhatItMissedTest =
    init.then(backAfterTwo(true)).then(Emit)
      .expect(s.evGot.get(2) == List(2, 3, 4) and NoLostEvent)

  // With no log lock, an event between the replay and the join is lost.
  run anEventBetweenReplayAndJoinIsLostTest =
    init.then(backAfterTwo(false)).then(Emit).then(Join)
      .expect(s.evGot.get(2) == List(2, 3) and s.evN == 4 and not(NoLostEvent))
}
