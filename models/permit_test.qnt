// -*- mode: Bluespec; -*-
// Fixed traces for the findings of SPEC 25. F1, F2, F5 and F6 are fixed: their
// traces now end in a good state. F3 and F4 are open: their traces still end in
// a state that breaks a property. `quint test` runs them.
module permit_test {
  import permit.* from "./permit"

  // One step of a thread. It takes any one item of its queue.
  action stateAny = { nondet x = s.msgs.oneOf() HubState(x) }
  action clickAny = { nondet x = s.clicks.oneOf() HubClick(x) }
  action replyAny = { nondet x = s.replies.oneOf() ClaudeReadReply(x) }

  // A request of pane A that reaches the tile and the window.
  action askA = ClaudeAsk("A").then(stateAny).then(HubPermit(s.nextReq - 1)).then(WindowSync("A"))

  // F1, fixed: `on_permit` is one step under the `permits` lock. A late
  // `working` line clears both the entry and the buttons, or neither.
  run lateWorkingKeepsTileAndEntryTogetherTest =
    init.then(ClaudeAsk("A"))
      .then(HubPermit(1))
      .then(stateAny)
      .expect(and {
        ButtonsForWaitingRequest,
        s.permits == Set(),
        s.tile.get("A") == NO_VIEW
      })

  // F2, fixed: Claude exits while request 1 waits. `tick` calls
  // `answer_permits`: the buttons go, and `sb permit` gets "none".
  run buttonsGoWithTheProcessTest =
    init.then(askA)
      .then(ClaudeDies("A"))
      .then(HubTickExit("A"))
      .then(WindowSync("A"))
      .expect(and {
        NoButtonsForEndedRequest,
        s.tile.get("A") == NO_VIEW,
        s.replies.exists(x => x.req == 1 and x.answer == "none")
      })

  // F2, fixed: a resume before `tick` saw the exit also clears the buttons.
  run resumeClearsButtonsTest =
    init.then(askA)
      .then(ClaudeDies("A"))
      .then(HubResume("A"))
      .expect(s.tile.get("A") == NO_VIEW and s.permits == Set())

  // F3, open: the user sees request 1 in the pane and types the Allow key.
  // Before the key reaches Claude, a tile click answers request 1 and request
  // 2 opens. The key answers request 2.
  run paneKeyHitsNextDialogTest =
    init.then(askA)
      .then(UserKey("A", "allow"))
      .then(UserClick("A", "allow")).then(clickAny).then(replyAny)
      .then(ClaudeAsk("A"))
      .then({ nondet x = s.inputs.oneOf() HubInput(x) })
      .then(ClaudeReadKey("A"))
      .expect(not(PaneAnswerMatchesRequest) and 2.in(s.ran))

  // F4, open: the `PreToolUse` line ("working") of a request arrives after its
  // `Permit`. `answer_permits` replies "none" and the buttons go at once.
  run lateWorkingClearsTest =
    init.then(ClaudeAsk("A"))
      .then(HubPermit(1))
      .then(stateAny)
      .expect(not(NoSpuriousClear))

  // F5, fixed: the id comes from a counter, so a new request never gets the
  // id of an old one. A click on the old buttons finds no entry.
  run oldIdNeverAllowsNewRequestTest =
    init.then(askA)
      .then(ClaudeDies("A"))
      .then(HubResume("A"))
      .then(ClaudeAsk("A")).then(stateAny).then(HubPermit(2))
      .then(UserClick("A", "allow")).then(clickAny)
      .expect(and {
        s.tile.get("A") == { t: 2, req: 2 },
        s.permits.exists(e => e.req == 2),
        not(2.in(s.ran)),
        TileAnswerMatchesRequest,
        IdsAreUnique
      })

  // F6, fixed: the `Permit` line of request 1 waits in a slow socket thread.
  // Claude exits, the pane is resumed, and request 2 asks. Both entries are in
  // `permits`. A click on the buttons of request 1 leaves the buttons of
  // request 2.
  run oldClickKeepsNewButtonsTest =
    init.then(ClaudeAsk("A"))
      .then(ClaudeDies("A"))
      .then(HubResume("A"))
      .then(ClaudeAsk("A")).then(stateAny).then(stateAny)
      .then(HubPermit(1)).then(WindowSync("A"))
      .then(HubPermit(2))
      .then(UserClick("A", "allow")).then(clickAny)
      .expect(and {
        ClickTakesOnlyItsButtons,
        s.tile.get("A").req == 2,
        s.permits.exists(e => e.req == 2)
      })
}
