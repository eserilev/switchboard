// -*- mode: Bluespec; -*-
// Fixed traces for remote mode (SPEC 29). `quint test` runs them.
module remote_test {
  import remote.* from "./remote"

  action answerAny = { nondet o = s.outbox.oneOf() Answer(o) }
  action makeA = Make("A").then(Read)

  // Window 1 shows pane A, and pane A drops: three chunks and a budget of two.
  // The writer asks for a fresh screen.
  action dropOnScreen =
    Attach
      .then(WinExpand(1, "A"))
      .then(CmdStart(1, 0, true, true))
      .then(Capture)
      .then(ShowOn)
      .then(answerAny)
      .then(makeA).then(makeA).then(makeA)
      .then(Write(1))

  action collapseNow = WinCollapse(1).then(CmdStart(1, 0, true, true))

  // The first screen thread had no owner lock. A collapse between the check of
  // `resume_stream` and its stream on leaves pane A off screen and streaming.
  run oldResumeLeavesAPaneStreamingTest =
    init.then(dropOnScreen)
      .then(JobPause(1))
      .then(JobFill(1))
      .then(JobCheck(1))
      .then(collapseNow)
      .then(JobResume(1))
      .expect(s.live == "" and s.streaming.get("A"))

  // The fix: the fresh screen is one step under the owner lock. The collapse
  // comes before or after it, and pane A stops either way.
  run resyncUnderTheOwnerLockTest =
    init.then(dropOnScreen)
      .then(Resync(1))
      .then(collapseNow)
      .expect(s.live == "" and not(s.streaming.get("A")) and s.filled == 1)

  // A window that lost a takeover never pauses a pane of the new window.
  run oldWindowNeverPausesTheNewOneTest =
    init.then(dropOnScreen)
      .then(Attach)
      .then(WinExpand(2, "A"))
      .then(CmdStart(2, 0, true, true))
      .then(Capture)
      .then(ShowOn)
      .then(Resync(1))
      .expect(s.live == "A" and s.streaming.get("A") and s.filled == 0)
}
