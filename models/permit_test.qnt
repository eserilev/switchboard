// -*- mode: Bluespec; -*-
// Fixed traces for the findings of SPEC 25. Each test ends in a state that
// breaks one property of permit.qnt. `quint test` runs them.
module permit_test {
  import permit.* from "./permit"

  // One step of a thread. It takes any one item of its queue.
  action stateAny = { nondet x = s.msgs.oneOf() HubState(x) }
  action insertAny = { nondet x = s.half.oneOf() HubPermitInsert(x) }
  action clickAny = { nondet x = s.clicks.oneOf() HubClick(x) }
  action replyAny = { nondet x = s.replies.oneOf() ClaudeReadReply(x) }

  // A request of pane A that reaches the tile and the window, with id nanos `t`.
  action askA(t: int): bool = ClaudeAsk("A").then(stateAny).then(HubPermitTile(s.nextReq - 1, t)).then(insertAny).then(WindowSync("A"))

  // F1: the `PreToolUse` line arrives between the two halves of `on_permit`.
  // The tile loses its buttons, and the entry stays in `permits`.
  run lateWorkingBetweenHalvesTest =
    init.then(ClaudeAsk("A"))
      .then(HubPermitTile(1, 1))
      .then(stateAny)
      .then(insertAny)
      .expect(not(ButtonsForWaitingRequest))

  // F2: Claude exits while request 1 waits. The tile keeps Allow and Deny, also
  // after a resume.
  run buttonsOutliveTheProcessTest =
    init.then(askA(1))
      .then(ClaudeDies("A"))
      .then(HubResume("A"))
      .expect(not(NoButtonsForEndedRequest))

  // F3: the user sees request 1 in the pane and types the Allow key. Before the
  // key reaches Claude, a tile click answers request 1 and request 2 opens. The
  // key answers request 2.
  run paneKeyHitsNextDialogTest =
    init.then(askA(1))
      .then(UserKey("A", "allow"))
      .then(UserClick("A", "allow")).then(clickAny).then(replyAny)
      .then(ClaudeAsk("A"))
      .then({ nondet x = s.inputs.oneOf() HubInput(x) })
      .then(ClaudeReadKey("A"))
      .expect(not(PaneAnswerMatchesRequest) and 2.in(s.ran))

  // F4: the `PreToolUse` line ("working") of a request arrives after its
  // `Permit`. `answer_permits` replies "none" and the buttons go at once.
  run lateWorkingClearsTest =
    init.then(ClaudeAsk("A"))
      .then(HubPermitTile(1, 1)).then(insertAny)
      .then(stateAny)
      .expect(not(NoSpuriousClear))

  // F5: `now_nanos` gives the same value twice. The new entry replaces the old
  // one in `permits`. A click on the old buttons allows the new request.
  run repeatedIdAllowsNewRequestTest =
    init.then(askA(1))
      .then(ClaudeDies("A"))
      .then(HubResume("A"))
      .then(ClaudeAsk("A")).then(stateAny).then(HubPermitTile(2, 1)).then(insertAny)
      .then(UserClick("A", "allow")).then(clickAny)
      .then({ nondet x = s.replies.filter(r => r.req == 2).oneOf() ClaudeReadReply(x) })
      .expect(not(TileAnswerMatchesRequest) and 2.in(s.ran))

  // F6: the `Permit` line of request 1 waits in a slow socket thread. Claude
  // exits, the pane is resumed, and request 2 asks. Then both entries are in
  // `permits`. A click on the buttons of request 1 takes the buttons of request 2.
  run oldClickTakesNewButtonsTest =
    init.then(ClaudeAsk("A"))
      .then(ClaudeDies("A"))
      .then(HubResume("A"))
      .then(ClaudeAsk("A")).then(stateAny).then(stateAny)
      .then(HubPermitTile(1, 1)).then(insertAny).then(WindowSync("A"))
      .then(HubPermitTile(2, 2)).then(insertAny)
      .then(UserClick("A", "allow")).then(clickAny)
      .expect(not(ClickTakesOnlyItsButtons) and s.permits.exists(e => e.req == 2))
}
