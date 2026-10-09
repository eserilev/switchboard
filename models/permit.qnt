// -*- mode: Bluespec; -*-
// A model of the permission flow of the board (SPEC 6.5, 13.2, 25).
//
// The model follows the code, not an ideal design. Each action names the
// function that it models. `scripts/check-models.sh` runs the checks.
//
// The parts:
// - Claude in a tmux pane. It opens a permission dialog in the pane and runs
//   the `PermissionRequest` hook (`sb permit`) for the same request. The first
//   answer wins (spike S2). An answer from a hook applies only to the request
//   of that hook.
// - `sb permit` (crates/sb/src/main.rs:permit). It sends one `Permit` request on
//   the socket and waits for one reply line.
// - The hub (crates/core/src/hub.rs). One thread for each socket connection, and
//   the calls from the window. The threads run in any order.
// - The window. It shows a copy of each tile, and that copy can be old.
//
// The state is one record, so each action changes only the fields it names.
module permit {

  // ---------- sizes ----------

  pure val PANES = Set("A", "B")
  // Requests in one run, all panes together.
  pure val MAX_REQ = 4
  // Process starts of one pane: the first start and one resume.
  pure val MAX_GEN = 2
  // A bound on the events that the user and Claude start in one run.
  pure val MAX_EVENTS = 16
  pure val ANSWERS = Set("allow", "deny")
  // Keys typed in a pane. "allow" and "deny" answer a dialog. "other" is any
  // other byte, for example a letter or a focus report.
  pure val KEYS = Set("allow", "deny", "other")

  // ---------- types ----------

  // One request of Claude. `gen` is the process start of the pane.
  // `status` is "none" (not made yet), "pending", "allowed", "denied" or "gone".
  type Req = { pane: str, gen: int, status: str }
  // One answer that Claude applied. `src` is "tile" or "pane". `meant` is the
  // request that the user saw when they gave the answer (0: no request).
  type Applied = { src: str, answer: str, meant: int }
  // The permit id of the hub is "{pane}-{counter}". `t` is the counter part.
  // `req` is a ghost: the request that the permit view on the tile describes.
  type View = { t: int, req: int }
  // One entry of `Hub.permits`. `req` names the socket of its `Sender`.
  type Entry = { pane: str, t: int, req: int }
  // One reply line on the socket of `sb permit`. `answer` is "allow", "deny"
  // or "none" (`PermitReply { behavior: None }`).
  type Reply = { req: int, answer: str, meant: int, n: int }
  type Msg = { pane: str, event: str, n: int }
  type Click = { pane: str, t: int, answer: str, meant: int, n: int }
  type Input = { pane: str, key: str, meant: int, n: int }
  type Key = { key: str, meant: int }
  type Clear = { pane: str, meant: int, n: int }

  type State = {
    // Claude, for each pane.
    alive: str -> bool,
    gen: str -> int,
    // The request whose dialog shows in the pane. 0: no dialog.
    dialog: str -> int,
    // Bytes in the tmux pane that Claude did not read yet.
    keys: str -> List[Key],
    // Claude, for each request.
    reqs: int -> Req,
    nextReq: int,
    // The `sb permit` process of each request runs.
    hookAlive: int -> bool,
    // The tool of these requests ran.
    ran: Set[int],
    applied: int -> List[Applied],
    // In flight between threads and processes.
    // `sb permit` connected, and the hub did not run `on_permit` yet.
    conns: Set[int],
    // `sb state` lines that the hub did not read yet.
    msgs: Set[Msg],
    // Reply lines that `sb permit` did not read yet.
    replies: Set[Reply],
    // Clicks on Allow or Deny: `permit_answer` calls that did not run yet.
    clicks: Set[Click],
    // Keys from the window: `input` calls that did not run yet.
    inputs: Set[Input],
    // `input` sent the bytes, and did not run its `answer_permits` yet.
    clears: Set[Clear],
    n: int,
    // The hub.
    hubPanes: Set[str],
    // `Pane.dead`: `tick` saw the process exit.
    dead: str -> bool,
    permits: Set[Entry],
    // `Pane.permit`. t = 0: no buttons.
    tile: str -> View,
    clock: int,
    // The window.
    shown: str -> View,
    // Ghosts for the checks.
    sent: Set[Reply],
    spurious: Set[int],
    // A click cleared `Pane.permit` while it held another request.
    taken: Set[int]
  }

  var s: State

  pure val NO_VIEW = { t: 0, req: 0 }
  pure val REQ_IDS = 1.to(MAX_REQ)

  // ---------- helpers ----------

  // Claude applies an answer to the dialog of request `r`. The first answer
  // wins: the dialog closes, so a later answer finds no dialog. Then Claude
  // goes on, and its next hook event is a `PreToolUse` or a `Stop`. Both clear
  // the buttons of the pane; the model sends "working" for both.
  pure def resolve(st: State, r: int, src: str, answer: str, meant: int): State = {
    val q = st.reqs.get(r)
    { ...st,
      msgs: st.msgs.union(Set({ pane: q.pane, event: "working", n: st.n })),
      n: st.n + 1,
      reqs: st.reqs.set(r, { ...q, status: if (answer == "allow") "allowed" else "denied" }),
      dialog: st.dialog.set(q.pane, 0),
      ran: if (answer == "allow") st.ran.union(Set(r)) else st.ran,
      applied: st.applied.set(r, st.applied.get(r).append({ src: src, answer: answer, meant: meant }))
    }
  }

  // A "none" reply to an open dialog that no key of the user explains: the tile
  // buttons go, but Claude still waits. `cause` is "state", "input" or "close".
  pure def isSpurious(st: State, r: int, cause: str, meant: int): bool = and {
    cause != "close",
    st.reqs.get(r).status == "pending",
    not(cause == "input" and meant == r)
  }

  // hub.rs:answer_permits(pane, None). It removes every entry of the pane and
  // sends "none" on each socket. Then it takes `Pane.permit`, if the pane is
  // still in the list.
  pure def answerPermits(st: State, p: str, cause: str, meant: int): State = {
    val mine = st.permits.filter(e => e.pane == p)
    val out = mine.map(e => { req: e.req, answer: "none", meant: 0, n: st.n })
    { ...st,
      permits: st.permits.exclude(mine),
      replies: st.replies.union(out),
      sent: st.sent.union(out),
      tile: if (st.hubPanes.contains(p)) st.tile.set(p, NO_VIEW) else st.tile,
      spurious: st.spurious.union(mine.filter(e => isSpurious(st, e.req, cause, meant)).map(e => e.req)),
      n: st.n + 1
    }
  }

  // ---------- Claude and `sb` ----------

  // Claude wants to run a tool that needs permission. The `PreToolUse` hook
  // runs `sb state working` first. `sb state` writes its line and exits, so the
  // hub can read that line late. Then the `PermissionRequest` hook starts
  // `sb permit` (main.rs:permit), which connects, and the dialog shows in the pane.
  action ClaudeAsk(p: str): bool = all {
    s.alive.get(p),
    s.dialog.get(p) == 0,
    s.nextReq <= MAX_REQ,
    s.n < MAX_EVENTS,
    val r = s.nextReq
    s' = { ...s,
      reqs: s.reqs.set(r, { pane: p, gen: s.gen.get(p), status: "pending" }),
      nextReq: r + 1,
      dialog: s.dialog.set(p, r),
      hookAlive: s.hookAlive.set(r, true),
      conns: s.conns.union(Set(r)),
      msgs: s.msgs.union(Set({ pane: p, event: "working", n: s.n })),
      n: s.n + 1
    }
  }

  // Claude sends another hook event: `UserPromptSubmit` or `PreToolUse`
  // ("working"), or `Stop` ("turn"). A subagent or a tool that needs no
  // permission sends "working" also while a dialog waits.
  action ClaudeEvent(p: str, ev: str): bool = all {
    s.alive.get(p),
    s.n < MAX_EVENTS,
    ev == "working" or s.dialog.get(p) == 0,
    s' = { ...s, msgs: s.msgs.union(Set({ pane: p, event: ev, n: s.n })), n: s.n + 1 }
  }

  // Claude reads one key from the pane. An answer key answers the dialog that
  // shows now. A key has no request id.
  action ClaudeReadKey(p: str): bool = all {
    s.keys.get(p).length() > 0,
    val k = s.keys.get(p).head()
    val st = { ...s, keys: s.keys.set(p, s.keys.get(p).tail()) }
    val r = s.dialog.get(p)
    s' = if (s.alive.get(p) and r != 0 and ANSWERS.contains(k.key))
      resolve(st, r, "pane", k.key, k.meant)
    else st
  }

  // `sb permit` reads its reply line, prints the decision and exits
  // (main.rs:permit). Claude applies "allow" or "deny" to the request of this
  // hook, if its dialog still shows. "none" prints nothing: the dialog stays.
  // If the hook is dead, the line is lost.
  action ClaudeReadReply(rep: Reply): bool = all {
    s.replies.contains(rep),
    val r = rep.req
    val p = s.reqs.get(r).pane
    val st = { ...s,
      replies: s.replies.exclude(Set(rep)),
      hookAlive: s.hookAlive.set(r, false)
    }
    s' = if (s.hookAlive.get(r) and ANSWERS.contains(rep.answer) and s.alive.get(p) and s.dialog.get(p) == r)
      resolve(st, r, "tile", rep.answer, rep.meant)
    else st
  }

  // The hook timeout (600 s, hooks.rs:settings). Claude kills `sb permit`. The
  // dialog stays. The hub thread still waits on its channel and does not know.
  action HookTimeout(r: int): bool = all {
    s.hookAlive.get(r),
    s.reqs.get(r).status == "pending",
    s' = { ...s, hookAlive: s.hookAlive.set(r, false) }
  }

  // The request ended another way (an answer in the pane, or Claude exited).
  // Claude can kill the hook, or the hook can live on. Both are possible here.
  action HookKilled(r: int): bool = all {
    s.hookAlive.get(r),
    s.reqs.get(r).status != "pending",
    s' = { ...s, hookAlive: s.hookAlive.set(r, false) }
  }

  // Claude exits or crashes. The pane stays (`remain-on-exit`). The open request
  // is gone. Nothing tells the hub about the request (hub.rs:tick only sets the lamp).
  action ClaudeDies(p: str): bool = all {
    s.alive.get(p),
    s.n < MAX_EVENTS,
    val r = s.dialog.get(p)
    s' = { ...s,
      alive: s.alive.set(p, false),
      dialog: s.dialog.set(p, 0),
      reqs: if (r == 0) s.reqs else s.reqs.set(r, { ...s.reqs.get(r), status: "gone" }),
      n: s.n + 1
    }
  }

  // ---------- the hub ----------

  // hub.rs:serve, `Request::Permit`, and hub.rs:on_permit. The id takes the
  // next value of the counter `next_permit`, so it is new. Under the `permits`
  // lock, `update_pane` sets `Pane.permit` and the entry goes in `permits`: one
  // step. If the pane is not in the list, or `tick` saw its process exit,
  // `serve` replies "none" at once.
  action HubPermit(r: int): bool = all {
    s.conns.contains(r),
    val p = s.reqs.get(r).pane
    val t = s.clock + 1
    val st = { ...s, conns: s.conns.exclude(Set(r)) }
    val none = { req: r, answer: "none", meant: 0, n: s.n }
    s' = if (s.hubPanes.contains(p) and not(s.dead.get(p))) { ...st,
      tile: st.tile.set(p, { t: t, req: r }),
      permits: st.permits.union(Set({ pane: p, t: t, req: r })),
      clock: t
    } else { ...st,
      replies: st.replies.union(Set(none)),
      sent: st.sent.union(Set(none)),
      n: st.n + 1
    }
  }

  // hub.rs:on_state. "working" and "turn" call `answer_permits(pane, None)`.
  // A pane not in the list ends the call early.
  action HubState(m: Msg): bool = all {
    s.msgs.contains(m),
    val st = { ...s, msgs: s.msgs.exclude(Set(m)) }
    s' = if (s.hubPanes.contains(m.pane)) answerPermits(st, m.pane, "state", 0) else st
  }

  // hub.rs:permit_answer. Under the `permits` lock, it removes the entry of
  // this id and sends the answer on its socket. Then `with_pane` sets
  // `Pane.permit = None`, but only if `Pane.permit` holds this id.
  // No entry: an error ("the request is gone"), and no change.
  action HubClick(c: Click): bool = all {
    s.clicks.contains(c),
    val st = { ...s, clicks: s.clicks.exclude(Set(c)) }
    val hit = s.permits.filter(e => e.pane == c.pane and e.t == c.t)
    s' = if (hit == Set()) st else {
      val e = hit.getOnlyElement()
      val rep = { req: e.req, answer: c.answer, meant: c.meant, n: s.n }
      val clear = st.hubPanes.contains(e.pane) and st.tile.get(e.pane).t == e.t
      { ...st,
        permits: st.permits.exclude(hit),
        replies: st.replies.union(Set(rep)),
        sent: st.sent.union(Set(rep)),
        tile: if (clear) st.tile.set(e.pane, NO_VIEW) else st.tile,
        // The ghost: a clear of buttons that show another request.
        taken: if (clear and st.tile.get(e.pane).req != e.req)
          st.taken.union(Set(st.tile.get(e.pane).req))
        else st.taken,
        n: st.n + 1
      }
    }
  }

  // hub.rs:input, first part: `send_bytes` into the tmux pane. An error if the
  // pane is not in the list.
  action HubInput(i: Input): bool = all {
    s.inputs.contains(i),
    val st = { ...s, inputs: s.inputs.exclude(Set(i)) }
    s' = if (s.hubPanes.contains(i.pane)) { ...st,
      keys: st.keys.set(i.pane, st.keys.get(i.pane).append({ key: i.key, meant: i.meant })),
      clears: st.clears.union(Set({ pane: i.pane, meant: i.meant, n: i.n }))
    } else st
  }

  // hub.rs:input, second part: if `permits` has an entry for the pane, then
  // `answer_permits(pane, None)`.
  action HubInputClear(c: Clear): bool = all {
    s.clears.contains(c),
    val st = { ...s, clears: s.clears.exclude(Set(c)) }
    s' = if (s.permits.exists(e => e.pane == c.pane)) answerPermits(st, c.pane, "input", c.meant) else st
  }

  // hub.rs:tick. It sees that the process of the pane exited, sets `Pane.dead`,
  // and then calls `answer_permits(pane, None)`.
  action HubTickExit(p: str): bool = all {
    s.hubPanes.contains(p),
    not(s.alive.get(p)),
    not(s.dead.get(p)),
    s' = answerPermits({ ...s, dead: s.dead.set(p, true) }, p, "exit", 0)
  }

  // hub.rs:resume and respawn_claude. First `answer_permits(pane, None)`, then
  // a new Claude starts in the same pane, with the same pane id.
  action HubResume(p: str): bool = all {
    s.hubPanes.contains(p),
    not(s.alive.get(p)),
    s.gen.get(p) < MAX_GEN,
    s.n < MAX_EVENTS,
    val st = answerPermits(s, p, "resume", 0)
    s' = { ...st,
      alive: st.alive.set(p, true),
      dead: st.dead.set(p, false),
      gen: st.gen.set(p, st.gen.get(p) + 1),
      n: st.n + 1
    }
  }

  // hub.rs:close. It removes the pane from the list, kills the tmux pane, then
  // calls `answer_permits(id, None)`. Pane ids are not used again.
  action HubClose(p: str): bool = all {
    s.hubPanes.contains(p),
    s.n < MAX_EVENTS,
    val r = s.dialog.get(p)
    val st = { ...s,
      hubPanes: s.hubPanes.exclude(Set(p)),
      alive: s.alive.set(p, false),
      dialog: s.dialog.set(p, 0),
      reqs: if (r == 0) s.reqs else s.reqs.set(r, { ...s.reqs.get(r), status: "gone" }),
      keys: s.keys.set(p, List())
    }
    s' = answerPermits(st, p, "close", 0)
  }

  // ---------- the window and the user ----------

  // The window gets a `pane` event and draws the tile again.
  action WindowSync(p: str): bool = all {
    s.shown.get(p) != (if (s.hubPanes.contains(p)) s.tile.get(p) else NO_VIEW),
    s' = { ...s, shown: s.shown.set(p, if (s.hubPanes.contains(p)) s.tile.get(p) else NO_VIEW) }
  }

  // A click on Allow or Deny calls `permit_answer(id, allow)` (main.rs of the app).
  // The id is the one that the window shows, and it can be old.
  action UserClick(p: str, a: str): bool = all {
    s.shown.get(p).t != 0,
    s.n < MAX_EVENTS,
    val v = s.shown.get(p)
    s' = { ...s,
      clicks: s.clicks.union(Set({ pane: p, t: v.t, answer: a, meant: v.req, n: s.n })),
      n: s.n + 1
    }
  }

  // The user types a key in the live pane. They answer the dialog that they
  // see now. The window sends the key with `input`.
  action UserKey(p: str, k: str): bool = all {
    s.hubPanes.contains(p),
    s.n < MAX_EVENTS,
    s' = { ...s,
      inputs: s.inputs.union(Set({ pane: p, key: k, meant: s.dialog.get(p), n: s.n })),
      n: s.n + 1
    }
  }

  // ---------- init and step ----------

  action init = s' = {
    alive: PANES.mapBy(_ => true),
    gen: PANES.mapBy(_ => 1),
    dialog: PANES.mapBy(_ => 0),
    keys: PANES.mapBy(_ => List()),
    reqs: REQ_IDS.mapBy(_ => { pane: "", gen: 0, status: "none" }),
    nextReq: 1,
    hookAlive: REQ_IDS.mapBy(_ => false),
    ran: Set(),
    applied: REQ_IDS.mapBy(_ => List()),
    conns: Set(),
    msgs: Set(),
    replies: Set(),
    clicks: Set(),
    inputs: Set(),
    clears: Set(),
    n: 0,
    hubPanes: PANES,
    dead: PANES.mapBy(_ => false),
    permits: Set(),
    tile: PANES.mapBy(_ => NO_VIEW),
    clock: 0,
    shown: PANES.mapBy(_ => NO_VIEW),
    sent: Set(),
    spurious: Set(),
    taken: Set()
  }

  // With `crash` false, Claude never exits and the user never closes a pane:
  // more runs reach the tile answers.
  // With `prompt` true, the hub reads each `Permit` line before its request
  // ends: before Claude reads an answer key, times out the hook, or exits.
  // `sb permit` connects in a few ms, and you answer in no less than 100 ms.
  // Without it, a hub thread can wait for any time, also past the 600 s timeout.
  action stepWith(crash: bool, prompt: bool): bool = {
    nondet p = PANES.oneOf()
    nondet a = ANSWERS.oneOf()
    nondet k = KEYS.oneOf()
    nondet ev = Set("working", "turn").oneOf()
    nondet r = REQ_IDS.oneOf()
    // `crash == true`: a bare `crash` in `all` fails the effect check of Quint.
    any {
      ClaudeAsk(p),
      ClaudeEvent(p, ev),
      all { prompt == false or not(s.conns.contains(s.dialog.get(p))), ClaudeReadKey(p) },
      all { crash == true, prompt == false or not(s.conns.contains(s.dialog.get(p))), ClaudeDies(p) },
      all { prompt == false or not(s.conns.contains(r)), HookTimeout(r) },
      HookKilled(r),
      all { s.replies != Set(), nondet x = s.replies.oneOf() ClaudeReadReply(x) },
      HubPermit(r),
      HubTickExit(p),
      all { s.msgs != Set(), nondet x = s.msgs.oneOf() HubState(x) },
      all { s.clicks != Set(), nondet x = s.clicks.oneOf() HubClick(x) },
      all { s.inputs != Set(), nondet x = s.inputs.oneOf() HubInput(x) },
      all { s.clears != Set(), nondet x = s.clears.oneOf() HubInputClear(x) },
      all { crash == true, HubResume(p) },
      all { crash == true, HubClose(p) },
      WindowSync(p),
      UserClick(p, a),
      UserKey(p, k),
    }
  }

  action step = stepWith(true, false)
  action stepNoCrash = stepWith(false, false)
  action stepPrompt = stepWith(true, true)
  action stepPromptNoCrash = stepWith(false, true)

  // ---------- properties that must hold ----------

  // P1. A tool runs only after an Allow for that request.
  val ToolRunsOnlyAfterAllow = s.ran.forall(r =>
    s.applied.get(r).length() > 0 and s.applied.get(r).head().answer == "allow")

  // P2. An answer from the tile applies only to the request that the clicked
  // tile showed. So an old Allow never applies to a newer request.
  val TileAnswerMatchesRequest = REQ_IDS.forall(r =>
    s.applied.get(r).select(x => x.src == "tile").foldl(true, (ok, x) => ok and x.meant == r))

  // P2, hub side. The hub writes "allow" or "deny" only on the socket of the
  // request that the clicked tile showed. This holds even when Claude would
  // drop the line.
  val HubAnswerMatchesRequest = s.sent.forall(x => ANSWERS.contains(x.answer) implies x.meant == x.req)

  // P3. At most one answer applies to a request, and the hub writes at most
  // one line on each `sb permit` socket.
  val OneAnswerEach = REQ_IDS.forall(r =>
    s.applied.get(r).length() <= 1 and s.sent.filter(x => x.req == r).size() <= 1)

  // P4. A Deny never leads to the tool running.
  val DenyNeverRuns = s.ran.forall(r =>
    s.applied.get(r).foldl(true, (ok, x) => ok and x.answer != "deny"))

  // P5. An answer meant for a request of pane A never answers a request of pane B.
  val AnswerStaysInPane = REQ_IDS.forall(r =>
    s.applied.get(r).foldl(true, (ok, x) =>
      ok and (x.meant == 0 or s.reqs.get(x.meant).pane == s.reqs.get(r).pane)))

  // P6. A tile answer never reaches a request of a later process start of the
  // pane (after a resume). A key in the pane can: see F3.
  val AnswerStaysInProcess = REQ_IDS.forall(r =>
    s.applied.get(r).select(x => x.src == "tile").foldl(true, (ok, x) =>
      ok and s.reqs.get(x.meant).gen == s.reqs.get(r).gen))

  // Nothing is in flight, the window shows the tiles as they are, and `tick`
  // saw every process exit.
  val quiet = and {
    s.conns == Set(), s.msgs == Set(), s.replies == Set(),
    s.clicks == Set(), s.inputs == Set(), s.clears == Set(),
    PANES.forall(p => s.keys.get(p).length() == 0),
    PANES.forall(p => s.shown.get(p) == (if (s.hubPanes.contains(p)) s.tile.get(p) else NO_VIEW)),
    s.hubPanes.forall(p => s.alive.get(p) or s.dead.get(p))
  }

  // T1 (was F1). When all is quiet, an open dialog whose `sb permit` waits in
  // `permits` has its buttons on the tile.
  val ButtonsForWaitingRequest = quiet implies s.hubPanes.forall(p =>
    val r = s.dialog.get(p)
    (s.alive.get(p) and r != 0 and s.permits.exists(e => e.req == r)) implies s.tile.get(p).req == r)

  // T2 (was F2). When all is quiet, the tile shows buttons only for a request
  // that waits.
  val NoButtonsForEndedRequest = quiet implies s.hubPanes.forall(p =>
    s.tile.get(p).t != 0 implies s.reqs.get(s.tile.get(p).req).status == "pending")

  // T3 (was F6). A click never takes the buttons of another request.
  val ClickTakesOnlyItsButtons = s.taken == Set()

  // T4 (was F5). Each entry in `permits` has its own id.
  val IdsAreUnique = s.permits.forall(a => s.permits.forall(b =>
    (a.pane == b.pane and a.t == b.t) implies a == b))

  val Safety = and {
    ToolRunsOnlyAfterAllow,
    TileAnswerMatchesRequest,
    HubAnswerMatchesRequest,
    OneAnswerEach,
    DenyNeverRuns,
    AnswerStaysInPane,
    AnswerStaysInProcess
  }

  // These hold in every run, with or without `prompt`.
  val Always = and { Safety, ClickTakesOnlyItsButtons, IdsAreUnique }

  // The tile properties need `prompt` (see `stepWith`). Without it, a `Permit`
  // line that a hub thread reads after its request ended shows buttons for
  // that ended request, until the next hook event of the pane.
  val Tiles = and {
    ButtonsForWaitingRequest,
    NoButtonsForEndedRequest,
    ClickTakesOnlyItsButtons,
    IdsAreUnique
  }

  // ---------- open findings: these fail (SPEC 25) ----------

  // F3. A key typed in the pane answers the dialog that the user saw.
  val PaneAnswerMatchesRequest = REQ_IDS.forall(r =>
    s.applied.get(r).select(x => x.src == "pane").foldl(true, (ok, x) => ok and x.meant == r))

  // F4. The buttons of an open dialog go only because of a key typed in that
  // dialog, or a close.
  val NoSpuriousClear = s.spurious == Set()
}
