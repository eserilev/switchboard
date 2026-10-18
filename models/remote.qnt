// -*- mode: Bluespec; -*-
// A model of remote mode (SPEC 29): windows that attach to `sb serve`, take over
// from each other, lose their link, and lose pane bytes that a fresh screen
// replaces.
//
// The model follows the code, not an ideal design. Each action names the
// function that it models. `scripts/check-models.sh` runs the checks.
//
// The parts:
// - A program in each tmux pane. It writes chunks of output. A chunk has a
//   number: chunk k is the k-th change of the pane. A size change makes some
//   programs draw their whole screen again (a "full" chunk); with `redraws`
//   false, the program never does.
// - tmux. It sends the chunks of a streaming pane to the reader thread, in order.
//   A chunk of a paused pane is lost.
// - The hub (crates/core/src/hub.rs and remote.rs). The reader thread, one
//   thread for each command, a writer and a screen thread for each window.
// - The windows (web/app.js). Each attach is a new window with a new epoch: a
//   second computer, or the same app after a lost link.
//
// The queue of each window is the proved `outq` (proofs/OutQ): here it is a list
// of chunks with the same states and the same budget rule.
//
// The state is one record, so each action changes only the fields it names.
module remote {

  // ---------- sizes ----------

  pure val PANES = Set("A", "B")
  pure val PANE_LIST = List("A", "B")
  // Epochs: each attach takes the next one.
  pure val EPOCHS = 1.to(3)
  // The chunks that a pane holds in a queue before they drop (outq `budget`).
  pure val BUDGET = 2
  // A bound on the chunks that one program writes in a run (not the redraws).
  pure val MAX_VER = 5
  // A bound on the events that the users and the programs start in one run.
  pure val MAX_EVENTS = 14

  // ---------- types ----------

  type Chunk = { pane: str, k: int, full: bool }
  // One line from the hub to a window. `kind` is "bytes" (a `pane_output`
  // event), "screen" (the answer to `pane_expand`, a capture at chunk `k`), "err"
  // (an error answer), or "taken" (the `taken_over` event).
  // `n` names the command of an answer.
  type Line = { kind: str, pane: str, k: int, full: bool, n: int }
  // One pane in a queue: "live", "dropping" or "resync" (outq LIVE, DROPPING, RESYNC).
  type PQ = { state: str, bytes: List[Chunk] }
  // A command of window `e` that a hub thread runs: "expand" or "collapse".
  type Cmd = { e: int, cmd: str, pane: str, n: int }
  // A screen command that holds the owner lock. n = 0: none.
  // `phase` is "forgot" (sink.reset ran) or "captured".
  type Hold = { n: int, e: int, pane: str, k: int, phase: str }
  // The screen thread of a window. `phase` is "idle", "paused", "filled" or "resume".
  // `ok` is the on_screen check of the phase before.
  type Job = { pane: str, phase: str, ok: bool }
  // The xterm of a window: the last chunk it shows, and a hole when a chunk is
  // missing or came twice.
  type Term = { upTo: int, hole: bool }
  type Out = { e: int, line: Line, n: int }

  type State = {
    // The programs and tmux.
    ver: str -> int,
    streaming: str -> bool,
    tmuxOut: List[Chunk],
    // Size changes that the program did not answer yet.
    wiggles: Set[str],
    redraws: bool,
    // The hub.
    // `Hub.live`: the pane on screen. "": none.
    live: str,
    // `RemoteSink.owner` and `RemoteSink.current`. 0: none.
    owner: int,
    current: int,
    nextEpoch: int,
    hold: Hold,
    // The commands of each window, in the order of its lines.
    cmds: int -> List[Cmd],
    // An answer that a command thread made and did not put in `lines` yet.
    outbox: Set[Out],
    // Each window on the hub side (`Conn`).
    attached: int -> bool,
    closed: int -> bool,
    ending: int -> bool,
    exited: int -> bool,
    lines: int -> List[Line],
    outq: int -> (str -> PQ),
    want: int -> List[str],
    job: int -> Job,
    // In flight on the link to each window.
    wire: int -> List[Line],
    // Each window (app.js). `gone`: its link went down or it was taken over.
    gone: int -> bool,
    wlive: int -> str,
    drawn: int -> bool,
    held: int -> List[Chunk],
    // app.js `expandSeq`: the newest expand. With `newest` false, any answer for
    // the pane draws (the first code).
    expN: int -> int,
    newest: bool,
    term: int -> Term,
    n: int,
    // Ghosts: windows whose screen command ran while they did not own the screen,
    // and the fresh screens that filled a pane on screen.
    stale: Set[int],
    filled: int
  }

  var s: State

  pure val NO_HOLD = { n: 0, e: 0, pane: "", k: 0, phase: "" }
  pure val IDLE = { pane: "", phase: "idle", ok: false }
  pure val EMPTY_Q = { state: "live", bytes: List() }

  // ---------- helpers ----------

  // A chunk on the xterm. A full chunk shows the whole pane at k. Else the chunk
  // must be the next one; a gap or a repeat leaves a hole.
  pure def apply(t: Term, c: Chunk): Term =
    if (c.full) { upTo: c.k, hole: false }
    else if (c.k == t.upTo + 1) { upTo: c.k, hole: t.hole }
    else { upTo: if (c.k > t.upTo) c.k else t.upTo, hole: true }

  // hub.rs:collapse. Every pane leaves the screen and stops its stream.
  pure def collapse(st: State): State =
    { ...st, live: "", streaming: if (st.live == "") st.streaming else st.streaming.set(st.live, false) }

  // outq.rs:forget on the queue of window `e`.
  pure def forget(st: State, e: int, p: str): State =
    if (e == 0) st else { ...st, outq: st.outq.set(e, st.outq.get(e).set(p, EMPTY_Q)) }

  // outq.rs:push of a chunk for the queue of window `e`.
  pure def push(q: PQ, c: Chunk): PQ =
    if (q.state != "live") q
    else if (q.bytes.length() < BUDGET) { ...q, bytes: q.bytes.append(c) }
    else { state: "dropping", bytes: List() }

  // ---------- the programs and tmux ----------

  // The program in a pane writes a chunk. tmux sends it only while the pane streams.
  action Make(p: str): bool = all {
    s.ver.get(p) < MAX_VER,
    s.n < MAX_EVENTS,
    val k = s.ver.get(p) + 1
    s' = { ...s,
      ver: s.ver.set(p, k),
      tmuxOut: if (s.streaming.get(p)) s.tmuxOut.append({ pane: p, k: k, full: false }) else s.tmuxOut,
      n: s.n + 1
    }
  }

  // The program answers a size change (hub.rs:show, resume_stream). With
  // `redraws`, it draws its whole screen: a full chunk.
  action Redraw(p: str): bool = all {
    s.wiggles.contains(p),
    val k = s.ver.get(p) + 1
    val full = s.redraws and s.streaming.get(p)
    s' = { ...s,
      wiggles: s.wiggles.exclude(Set(p)),
      ver: if (s.redraws) s.ver.set(p, k) else s.ver,
      tmuxOut: if (full) s.tmuxOut.append({ pane: p, k: k, full: true }) else s.tmuxOut
    }
  }

  // The reader thread (tmux.rs) passes the next chunk to `RemoteSink::output`,
  // which pushes it in the queue of the current window. No window: it goes nowhere.
  action Read: bool = all {
    s.tmuxOut.length() > 0,
    val c = s.tmuxOut.head()
    val st = { ...s, tmuxOut: s.tmuxOut.tail() }
    s' = if (s.current == 0) st
      else { ...st, outq: st.outq.set(s.current, st.outq.get(s.current).set(c.pane, push(st.outq.get(s.current).get(c.pane), c))) }
  }

  // ---------- the hub ----------

  // remote.rs:window, the attach of window `e`: the hello answer (not modeled),
  // then the takeover under the owner lock. The old window gets `taken_over`,
  // and its writer shuts its socket after it.
  action Attach: bool = all {
    s.nextEpoch <= 3,
    s.hold.n == 0,
    s.n < MAX_EVENTS,
    val e = s.nextEpoch
    val old = s.current
    val st = collapse({ ...s,
      nextEpoch: e + 1,
      owner: e,
      current: e,
      attached: s.attached.set(e, true),
      n: s.n + 1
    })
    s' = if (old == 0) st else { ...st,
      lines: st.lines.set(old, st.lines.get(old).append({ kind: "taken", pane: "", k: 0, full: false, n: 0 })),
      ending: st.ending.set(old, true)
    }
  }

  // remote.rs:window, the end of the read loop: the socket is shut. When this
  // window owns the screen, it is no longer current, and the screen goes.
  action Exit(e: int): bool = all {
    s.attached.get(e),
    s.closed.get(e),
    not(s.exited.get(e)),
    s.hold.n == 0,
    val st = { ...s, exited: s.exited.set(e, true) }
    s' = if (s.owner != e) st
      else collapse({ ...st, current: if (st.current == e) 0 else st.current })
  }

  // remote.rs:window and run, and the start of hub.rs:expand or collapse.
  // With `ordered`, the read loop runs command `i = 0` and queues its answer
  // before it reads the next line (dispatch::ORDERED); a closed socket ends the
  // loop. Else each command runs in a thread of its own, in any order.
  // A screen command takes the owner lock. With `check`, a command of a window
  // that does not own the screen gets an error and changes nothing.
  action CmdStart(e: int, i: int, check: bool, ordered: bool): bool = all {
    i < s.cmds.get(e).length(),
    s.hold.n == 0,
    not(ordered) or (i == 0 and not(s.closed.get(e)) and not(s.outbox.exists(o => o.e == e))),
    val c = s.cmds.get(e)[i]
    val l = s.cmds.get(e)
    val rest = l.slice(0, i).concat(l.slice(i + 1, l.length()))
    val st = { ...s,
      cmds: s.cmds.set(e, rest),
      stale: if (s.owner != c.e) s.stale.union(Set(c.e)) else s.stale
    }
    if (check and s.owner != c.e)
      s' = { ...s,
        cmds: s.cmds.set(e, rest),
        outbox: s.outbox.union(Set({ e: c.e, line: { kind: "err", pane: c.pane, k: 0, full: false, n: c.n }, n: c.n }))
      }
    else if (c.cmd == "collapse")
      s' = collapse(st)
    else {
      // hub.rs:expand: the old live pane stops, then hub.rs:show runs `sink.reset`
      // on the queue of the current window.
      val off = if (st.live != "" and st.live != c.pane) st.streaming.set(st.live, false) else st.streaming
      s' = forget({ ...st, streaming: off, live: c.pane, hold: { n: c.n, e: c.e, pane: c.pane, k: 0, phase: "forgot" } }, st.current, c.pane)
    }
  }

  // hub.rs:show: the capture.
  action Capture: bool = all {
    s.hold.n != 0,
    s.hold.phase == "forgot",
    s' = { ...s, hold: { ...s.hold, k: s.ver.get(s.hold.pane), phase: "captured" } }
  }

  // hub.rs:show: the stream goes on, a thread changes the size, and the command
  // returns the screen. The owner lock goes.
  action ShowOn: bool = all {
    s.hold.n != 0,
    s.hold.phase == "captured",
    val h = s.hold
    s' = { ...s,
      streaming: s.streaming.set(h.pane, true),
      wiggles: s.wiggles.union(Set(h.pane)),
      outbox: s.outbox.union(Set({ e: h.e, line: { kind: "screen", pane: h.pane, k: h.k, full: false, n: h.n }, n: h.n })),
      hold: NO_HOLD
    }
  }

  // remote.rs:window, the command thread: `conn.line` with the answer.
  action Answer(o: Out): bool = all {
    s.outbox.contains(o),
    s' = { ...s,
      outbox: s.outbox.exclude(Set(o)),
      lines: s.lines.set(o.e, s.lines.get(o.e).append(o.line))
    }
  }

  // remote.rs:writer, one round: all lines first, then the bytes of each pane.
  // A pane that dropped goes to "resync" and the screen thread hears of it.
  // After `ending`, the writer shuts the socket.
  action Write(e: int): bool = all {
    s.attached.get(e),
    not(s.closed.get(e)),
    val q = s.outq.get(e)
    val busy = s.lines.get(e).length() > 0 or s.ending.get(e)
      or PANES.exists(p => q.get(p).bytes.length() > 0 or q.get(p).state == "dropping")
    all {
      busy,
      val start = { wire: s.wire.get(e).concat(s.lines.get(e)), q: q, want: s.want.get(e) }
      val r = PANE_LIST.foldl(start, (acc, p) => {
        val pq = acc.q.get(p)
        val w = pq.bytes.foldl(acc.wire, (ws, c) => ws.append({ kind: "bytes", pane: c.pane, k: c.k, full: c.full, n: 0 }))
        val drop = pq.state == "dropping"
        { wire: w,
          q: acc.q.set(p, { state: if (drop) "resync" else pq.state, bytes: List() }),
          want: if (drop) acc.want.append(p) else acc.want }
      })
      s' = { ...s,
        wire: s.wire.set(e, r.wire),
        lines: s.lines.set(e, List()),
        outq: s.outq.set(e, r.q),
        want: s.want.set(e, r.want),
        closed: if (s.ending.get(e)) s.closed.set(e, true) else s.closed
      }
    }
  }

  // The screen thread of window `e` (remote.rs:window): one fresh screen, under
  // the owner lock, and only while this window owns the screen. It pauses the
  // pane on screen (tmux answers the pause after all the chunks before it, and
  // the reader passes them on first), captures it, fills the queue, and lets the
  // stream go on with a size change. A pane off screen, or a window that lost a
  // takeover, gets an empty screen, so the pane is live again.
  action Resync(e: int): bool = all {
    s.job.get(e).phase == "idle",
    s.want.get(e).length() > 0,
    s.hold.n == 0,
    val p = s.want.get(e).head()
    val ok = s.owner == e and s.live == p
    val pq = s.outq.get(e).get(p)
    val screen = if (ok) List({ pane: p, k: s.ver.get(p), full: true }) else List()
    all {
      not(ok) or not(s.tmuxOut.select(c => c.pane == p).length() > 0),
      s' = { ...s,
        want: s.want.set(e, s.want.get(e).tail()),
        outq: if (pq.state == "resync") s.outq.set(e, s.outq.get(e).set(p, { state: "live", bytes: screen })) else s.outq,
        streaming: if (ok) s.streaming.set(p, true) else s.streaming,
        wiggles: if (ok) s.wiggles.union(Set(p)) else s.wiggles,
        filled: if (ok and pq.state == "resync") s.filled + 1 else s.filled
      }
    }
  }

  // ---------- the first screen thread, with no owner lock ----------
  // These four steps are the code before the fix. A collapse between JobCheck
  // and JobResume leaves a pane off screen streaming (R6), and a window that lost
  // a takeover pauses a pane of the new one. remote_test.qnt keeps that trace.

  // First part: hub.rs:screen_for_resync checks
  // that the pane is on screen and pauses it. tmux answers the pause after all
  // the chunks before it, and the reader passes them on first.
  action JobPause(e: int): bool = all {
    s.job.get(e).phase == "idle",
    s.want.get(e).length() > 0,
    val p = s.want.get(e).head()
    val ok = s.live == p
    all {
      not(ok) or not(s.tmuxOut.select(c => c.pane == p).length() > 0),
      s' = { ...s,
        want: s.want.set(e, s.want.get(e).tail()),
        streaming: if (ok) s.streaming.set(p, false) else s.streaming,
        job: s.job.set(e, { pane: p, phase: "paused", ok: ok })
      }
    }
  }

  // Second part: the capture, then outq.rs:fill. A pane off screen gets an empty
  // screen, so it is live again.
  action JobFill(e: int): bool = all {
    s.job.get(e).phase == "paused",
    val j = s.job.get(e)
    val pq = s.outq.get(e).get(j.pane)
    val screen = if (j.ok) List({ pane: j.pane, k: s.ver.get(j.pane), full: true }) else List()
    s' = { ...s,
      outq: if (pq.state == "resync") s.outq.set(e, s.outq.get(e).set(j.pane, { state: "live", bytes: screen })) else s.outq,
      job: s.job.set(e, { ...j, phase: "filled" })
    }
  }

  // Third part: hub.rs:resume_stream checks that the pane is on screen.
  action JobCheck(e: int): bool = all {
    s.job.get(e).phase == "filled",
    s' = { ...s, job: s.job.set(e, { ...s.job.get(e), phase: "resume", ok: s.live == s.job.get(e).pane }) }
  }

  // Last part: the stream goes on, and the size changes.
  action JobResume(e: int): bool = all {
    s.job.get(e).phase == "resume",
    val j = s.job.get(e)
    s' = { ...s,
      streaming: if (j.ok) s.streaming.set(j.pane, true) else s.streaming,
      wiggles: if (j.ok) s.wiggles.union(Set(j.pane)) else s.wiggles,
      job: s.job.set(e, IDLE)
    }
  }

  // ---------- the link and the windows ----------

  // The link of window `e` goes down (ssh, a sleep, a network change). What is
  // in flight is lost. The hub's read loop then ends (`Exit`).
  action LinkDrop(e: int): bool = all {
    s.attached.get(e),
    not(s.gone.get(e)),
    s.n < MAX_EVENTS,
    s' = { ...s,
      closed: s.closed.set(e, true),
      gone: s.gone.set(e, true),
      wire: s.wire.set(e, List()),
      n: s.n + 1
    }
  }

  // app.js:expand: the window shows the pane, holds its bytes, and sends `pane_expand`.
  action WinExpand(e: int, p: str): bool = all {
    s.attached.get(e),
    not(s.gone.get(e)),
    s.n < MAX_EVENTS,
    val st = { ...s,
      wlive: s.wlive.set(e, p),
      drawn: s.drawn.set(e, false),
      held: s.held.set(e, List()),
      expN: s.expN.set(e, s.n + 1),
      n: s.n + 1
    }
    s' = if (s.closed.get(e)) st else { ...st, cmds: st.cmds.set(e, st.cmds.get(e).append({ e: e, cmd: "expand", pane: p, n: s.n + 1 })) }
  }

  // app.js:collapse.
  action WinCollapse(e: int): bool = all {
    s.attached.get(e),
    not(s.gone.get(e)),
    s.wlive.get(e) != "",
    s.n < MAX_EVENTS,
    val st = { ...s, wlive: s.wlive.set(e, ""), drawn: s.drawn.set(e, false), n: s.n + 1 }
    s' = if (s.closed.get(e)) st else { ...st, cmds: st.cmds.set(e, st.cmds.get(e).append({ e: e, cmd: "collapse", pane: "", n: s.n + 1 })) }
  }

  // The window reads one line. Bytes of the live pane go on the xterm, or wait
  // until the screen comes. The screen of the pane that the window still wants
  // shows, then the bytes that waited. An error collapses.
  action WinRead(e: int): bool = all {
    s.wire.get(e).length() > 0,
    not(s.gone.get(e)),
    val l = s.wire.get(e).head()
    val st = { ...s, wire: s.wire.set(e, s.wire.get(e).tail()) }
    val c = { pane: l.pane, k: l.k, full: l.full }
    s' = if (l.kind == "taken") { ...st, gone: st.gone.set(e, true) }
      else if (l.kind == "bytes") {
        if (l.pane != st.wlive.get(e)) st
        else if (st.drawn.get(e)) { ...st, term: st.term.set(e, apply(st.term.get(e), c)) }
        else { ...st, held: st.held.set(e, st.held.get(e).append(c)) }
      }
      else if (l.pane != st.wlive.get(e) or (st.newest and l.n != st.expN.get(e))) st
      else if (l.kind == "screen") { ...st,
        term: st.term.set(e, st.held.get(e).foldl({ upTo: l.k, hole: false }, (t, x) => apply(t, x))),
        held: st.held.set(e, List()),
        drawn: st.drawn.set(e, true)
      }
      else { ...st, wlive: st.wlive.set(e, ""), drawn: st.drawn.set(e, false) }
  }

  // ---------- init and step ----------

  action initWith(newest: bool): bool = {
    nondet r = Set(true, false).oneOf()
    s' = {
      ver: PANES.mapBy(_ => 0),
      streaming: PANES.mapBy(_ => false),
      tmuxOut: List(),
      wiggles: Set(),
      redraws: r,
      live: "",
      owner: 0,
      current: 0,
      nextEpoch: 1,
      hold: NO_HOLD,
      cmds: EPOCHS.mapBy(_ => List()),
      outbox: Set(),
      attached: EPOCHS.mapBy(_ => false),
      closed: EPOCHS.mapBy(_ => false),
      ending: EPOCHS.mapBy(_ => false),
      exited: EPOCHS.mapBy(_ => false),
      lines: EPOCHS.mapBy(_ => List()),
      outq: EPOCHS.mapBy(_ => PANES.mapBy(_ => EMPTY_Q)),
      want: EPOCHS.mapBy(_ => List()),
      job: EPOCHS.mapBy(_ => IDLE),
      wire: EPOCHS.mapBy(_ => List()),
      gone: EPOCHS.mapBy(_ => false),
      wlive: EPOCHS.mapBy(_ => ""),
      drawn: EPOCHS.mapBy(_ => false),
      held: EPOCHS.mapBy(_ => List()),
      expN: EPOCHS.mapBy(_ => 0),
      newest: newest,
      term: EPOCHS.mapBy(_ => { upTo: 0, hole: false }),
      n: 0,
      stale: Set(),
      filled: 0
    }
  }

  action init = initWith(true)
  // The window before `expandSeq`: an older answer draws too.
  action initAnyAnswer = initWith(false)

  // With `check` false, a screen command runs without the owner check. With
  // `ordered` false, each command runs in a thread of its own. With `locked`
  // false, the screen thread runs in four steps with no owner lock. Each false is
  // the first code.
  action stepWith(check: bool, ordered: bool, locked: bool): bool = {
    nondet p = PANES.oneOf()
    nondet e = EPOCHS.oneOf()
    nondet i = 0.to(3).oneOf()
    any {
      Make(p),
      Redraw(p),
      Read,
      Attach,
      Exit(e),
      CmdStart(e, i, check, ordered),
      Capture,
      ShowOn,
      all { s.outbox != Set(), nondet o = s.outbox.oneOf() Answer(o) },
      Write(e),
      all { locked == true, Resync(e) },
      all { locked == false, JobPause(e) },
      all { locked == false, JobFill(e) },
      all { locked == false, JobCheck(e) },
      all { locked == false, JobResume(e) },
      LinkDrop(e),
      WinExpand(e, p),
      WinCollapse(e),
      WinRead(e),
    }
  }

  action step = stepWith(true, true, true)
  action stepNoOwnerCheck = stepWith(false, true, true)
  action stepAnyOrder = stepWith(true, false, true)
  action stepNoJobLock = stepWith(true, true, false)

  // ---------- properties that must hold ----------

  // R1. A screen command runs only while its window owns the screen. So a window
  // that lost a takeover never changes the screen of the new one.
  val StaleNeverRuns = s.stale == Set()

  // R2. The current window is the owner. Only one window gets the news.
  val CurrentOwns = s.current != 0 implies s.current == s.owner

  // R3. A live pane in a queue never holds more than the budget (outq Q1).
  val QueueInBudget = EPOCHS.forall(e => PANES.forall(p => s.outq.get(e).get(p).bytes.length() <= BUDGET))

  // R4. A pane that waits for a fresh screen holds no bytes (outq "no gap").
  val NoBytesWhileWaiting = EPOCHS.forall(e => PANES.forall(p =>
    s.outq.get(e).get(p).state != "live" implies s.outq.get(e).get(p).bytes.length() == 0))

  // Nothing is in flight, and the current window read all its lines.
  val quiet = and {
    s.tmuxOut.length() == 0, s.wiggles == Set(), s.hold.n == 0,
    s.outbox == Set(),
    EPOCHS.forall(e => and {
      s.lines.get(e).length() == 0,
      s.closed.get(e) or s.cmds.get(e).length() == 0,
      s.want.get(e).length() == 0,
      s.job.get(e).phase == "idle",
      PANES.forall(p => s.outq.get(e).get(p).bytes.length() == 0 and s.outq.get(e).get(p).state == "live"),
      s.gone.get(e) or s.wire.get(e).length() == 0,
      s.closed.get(e) implies s.exited.get(e)
    })
  }

  // The current window has drawn a pane.
  def shows(e: int): bool = and {
    e != 0, not(s.gone.get(e)), s.wlive.get(e) != "", s.drawn.get(e)
  }

  // R5. When all is quiet, the pane that the current window shows is the pane
  // on the hub's screen.
  val WindowMatchesHub = quiet implies (shows(s.current) implies s.wlive.get(s.current) == s.live)

  // R6. When all is quiet, only the pane on screen streams. A pane off screen
  // sends no bytes over the link.
  val OnlyLiveStreams = quiet implies PANES.forall(p => s.streaming.get(p) implies p == s.live)

  // R7. When all is quiet and the program draws its screen again on a size
  // change, the current window shows the pane as it is: no hole, no old chunk.
  // Local mode has the same rule: the redraw fills the gap of the capture.
  val Fresh = (quiet and s.redraws) implies ((shows(s.current) and s.wlive.get(s.current) == s.live) implies
    (s.term.get(s.current).upTo == s.ver.get(s.live) and not(s.term.get(s.current).hole)))

  // ---------- witnesses: each must fail, or the runs miss that path ----------

  // A pane dropped its bytes, and a fresh screen filled it.
  val NoResync = s.filled == 0
  // A window that lost a takeover got an error for a screen command.
  val NoStaleError = EPOCHS.forall(e => s.wire.get(e).select(l => l.kind == "err").length() == 0)
  // All is quiet after a resync, and the current window shows the live pane.
  val NoQuietShow = not(quiet and shows(s.current) and s.wlive.get(s.current) == s.live
    and s.ver.get(s.live) > BUDGET and EPOCHS.exists(e => s.closed.get(e)))

  val Always = and { StaleNeverRuns, CurrentOwns, QueueInBudget, NoBytesWhileWaiting, WindowMatchesHub, OnlyLiveStreams, Fresh }
}
