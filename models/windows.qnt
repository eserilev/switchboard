// -*- mode: Bluespec; -*-
// A model of several windows on one hub (SPEC 29, 41.8): windows that attach to
// `sb serve` and stay, each with its own screen; panes that stream while a window
// shows them; lost links; and pane bytes that drop and that a fresh screen from
// the pane's terminal emulator replaces.
//
// It replaces models/remote.qnt, the model of one window with takeovers and a
// fresh screen that paused the pane.
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
// - The hub (crates/core/src/hub.rs, screens.rs and remote.rs). The reader
//   thread, the screen lock, a writer and a screen thread for each window.
// - The windows (web/app.js). Each attach is a new window with a new epoch.
//
// The queue of each window is the proved `outq` (proofs/OutQ): here it is a list
// of chunks with the same states and the same budget rule.
//
// The state is one record, so each action changes only the fields it names.
module windows {

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

  // A chunk of a pane. A full chunk draws the whole screen at `k`; `bad` marks a
  // full chunk that draws a wrong screen (a snapshot of an emulator with a hole).
  type Chunk = { pane: str, k: int, full: bool, bad: bool }
  // One line from the hub to a window. `kind` is "bytes" (a `pane_output`
  // event), "screen" (the answer to `pane_expand`, a capture at chunk `k`) or
  // "err" (an error answer). `n` names the command of an answer.
  type Line = { kind: str, pane: str, k: int, full: bool, bad: bool, n: int }
  // One pane in a queue: "live", "dropping" or "resync" (outq LIVE, DROPPING, RESYNC).
  type PQ = { state: str, bytes: List[Chunk] }
  // A command of window `e`: "expand" or "collapse".
  type Cmd = { e: int, cmd: str, pane: str, n: int }
  // An expand that holds the screen lock. n = 0: none.
  // `phase` is "forgot" (sink.reset_for ran) or "captured".
  type Hold = { n: int, e: int, pane: str, k: int, phase: str }
  // An xterm, or the terminal emulator of a pane (screens.rs `Terms`): the last
  // chunk it shows, and a hole when a chunk is missing or came twice.
  type Term = { upTo: int, hole: bool }
  // The emulator of a pane: `on` while it follows the pane.
  type Emu = { on: bool, upTo: int, hole: bool }
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
    // `Hub.live`: the pane on the screen of each window. "": none.
    live: int -> str,
    // `Hub.terms`.
    emu: str -> Emu,
    nextEpoch: int,
    hold: Hold,
    // The streams that a collapse with no screen lock decided to stop and did not
    // stop yet. Always empty with the lock.
    stops: Set[str],
    // A pane that the old fresh screen paused. "": none.
    paused: str,
    // The commands of each window, in the order of its lines.
    cmds: int -> List[Cmd],
    // An answer that a command made and did not put in `lines` yet.
    outbox: Set[Out],
    // Each window on the hub side (`Conn`).
    attached: int -> bool,
    closed: int -> bool,
    exited: int -> bool,
    lines: int -> List[Line],
    outq: int -> (str -> PQ),
    want: int -> List[str],
    // In flight on the link to each window.
    wire: int -> List[Line],
    // Each window (app.js). `gone`: its link went down.
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
    // Ghosts: the fresh screens that filled a pane on screen, and those that
    // filled one while another window showed the same pane.
    filled: int,
    shared: int
  }

  var s: State

  pure val NO_HOLD = { n: 0, e: 0, pane: "", k: 0, phase: "" }
  pure val EMPTY_Q = { state: "live", bytes: List() }
  pure val NO_EMU = { on: false, upTo: 0, hole: false }

  // ---------- helpers ----------

  // A chunk on a terminal. A full chunk shows the whole pane at k. Else the chunk
  // must be the next one; a gap or a repeat leaves a hole.
  pure def apply(t: Term, c: Chunk): Term =
    if (c.full) { upTo: c.k, hole: c.bad }
    else if (c.k == t.upTo + 1) { upTo: c.k, hole: t.hole }
    else { upTo: if (c.k > t.upTo) c.k else t.upTo, hole: true }

  // True when a window other than `e` shows pane `p` (screens.rs `watch_count`).
  pure def watchedBy(st: State, p: str, e: int): bool =
    EPOCHS.exists(x => x != e and st.live.get(x) == p)

  def watched(p: str): bool = EPOCHS.exists(x => s.live.get(x) == p)

  // hub.rs:stop_streams for one pane.
  pure def stop(st: State, p: str): State =
    { ...st, streaming: st.streaming.set(p, false), emu: st.emu.set(p, NO_EMU) }

  // hub.rs:collapse_in, under the screen lock: the window's screen goes, and a
  // pane no other window shows stops its stream. With `locked` false, the stop
  // comes later, in `StopStream`.
  pure def collapse(st: State, e: int, locked: bool): State =
    val p = st.live.get(e)
    val st2 = { ...st, live: st.live.set(e, "") }
    if (p == "" or watchedBy(st, p, e)) st2
    else if (locked) stop(st2, p)
    else { ...st2, stops: st2.stops.union(Set(p)) }

  // outq.rs:forget on the queue of window `e`.
  pure def forget(st: State, e: int, p: str): State =
    { ...st, outq: st.outq.set(e, st.outq.get(e).set(p, EMPTY_Q)) }

  // outq.rs:push of a chunk for one queue.
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
      tmuxOut: if (s.streaming.get(p)) s.tmuxOut.append({ pane: p, k: k, full: false, bad: false }) else s.tmuxOut,
      n: s.n + 1
    }
  }

  // The program answers a size change (hub.rs:show). With `redraws`, it draws
  // its whole screen: a full chunk.
  action Redraw(p: str): bool = all {
    s.wiggles.contains(p),
    val k = s.ver.get(p) + 1
    val full = s.redraws and s.streaming.get(p)
    s' = { ...s,
      wiggles: s.wiggles.exclude(Set(p)),
      ver: if (s.redraws) s.ver.set(p, k) else s.ver,
      tmuxOut: if (full) s.tmuxOut.append({ pane: p, k: k, full: true, bad: false }) else s.tmuxOut
    }
  }

  // The reader thread (hub.rs, the tmux callback): under the `terms` lock it feeds
  // the pane's emulator and pushes the chunk in the queue of each window that
  // shows the pane. No window: it goes nowhere.
  action Read: bool = all {
    s.tmuxOut.length() > 0,
    val c = s.tmuxOut.head()
    val dest = EPOCHS.filter(e => s.live.get(e) == c.pane)
    val em = s.emu.get(c.pane)
    val t = apply({ upTo: em.upTo, hole: em.hole }, c)
    s' = { ...s,
      tmuxOut: s.tmuxOut.tail(),
      emu: if (dest != Set() and em.on) s.emu.set(c.pane, { on: true, upTo: t.upTo, hole: t.hole }) else s.emu,
      outq: EPOCHS.mapBy(e => if (dest.contains(e)) s.outq.get(e).set(c.pane, push(s.outq.get(e).get(c.pane), c)) else s.outq.get(e))
    }
  }

  // ---------- the hub ----------

  // remote.rs:window, the attach of window `e`. No takeover: the other windows stay.
  action Attach: bool = all {
    s.nextEpoch <= 3,
    s.n < MAX_EVENTS,
    val e = s.nextEpoch
    s' = { ...s, nextEpoch: e + 1, attached: s.attached.set(e, true), n: s.n + 1 }
  }

  // remote.rs:Leave, the end of the read loop: the socket is shut, and the
  // window's screen goes (hub.rs:collapse_in, under the screen lock).
  action Exit(e: int, locked: bool): bool = all {
    s.attached.get(e),
    s.closed.get(e),
    not(s.exited.get(e)),
    s.hold.n == 0,
    s' = collapse({ ...s, exited: s.exited.set(e, true) }, e, locked)
  }

  // A collapse with no screen lock stops a stream it decided to stop before.
  action StopStream(p: str): bool = all {
    s.stops.contains(p),
    s' = { ...stop(s, p), stops: s.stops.exclude(Set(p)) }
  }

  // remote.rs:window and dispatch_in, and the start of hub.rs:expand_in or
  // collapse_in under the screen lock. With `ordered`, the read loop runs command
  // `i = 0` and queues its answer before it reads the next line
  // (dispatch::ORDERED); a closed socket ends the loop. Else each command runs in
  // a thread of its own, in any order.
  action CmdStart(e: int, i: int, ordered: bool, locked: bool): bool = all {
    i < s.cmds.get(e).length(),
    s.hold.n == 0,
    not(ordered) or (i == 0 and not(s.closed.get(e)) and not(s.outbox.exists(o => o.e == e))),
    val c = s.cmds.get(e)[i]
    val l = s.cmds.get(e)
    val st = { ...s, cmds: s.cmds.set(e, l.slice(0, i).concat(l.slice(i + 1, l.length()))) }
    if (c.cmd == "collapse")
      s' = collapse(st, c.e, locked)
    else {
      // expand_in: the window's old pane leaves its screen, then hub.rs:show runs
      // `sink.reset_for` on the queue of this window.
      val old = st.live.get(c.e)
      val st2 = if (old != c.pane) collapse(st, c.e, locked) else st
      s' = forget({ ...st2, live: st2.live.set(c.e, c.pane), hold: { n: c.n, e: c.e, pane: c.pane, k: 0, phase: "forgot" } }, c.e, c.pane)
    }
  }

  // hub.rs:show: the capture.
  action Capture: bool = all {
    s.hold.n != 0,
    s.hold.phase == "forgot",
    s' = { ...s, hold: { ...s.hold, k: s.ver.get(s.hold.pane), phase: "captured" } }
  }

  // hub.rs:show: the emulator starts from the capture (a pane that streams for
  // another window keeps its emulator), the stream goes on, a thread changes the
  // size, and the command returns the screen. The screen lock goes.
  action ShowOn: bool = all {
    s.hold.n != 0,
    s.hold.phase == "captured",
    val h = s.hold
    s' = { ...s,
      emu: if (s.emu.get(h.pane).on) s.emu else s.emu.set(h.pane, { on: true, upTo: h.k, hole: false }),
      streaming: s.streaming.set(h.pane, true),
      wiggles: s.wiggles.union(Set(h.pane)),
      outbox: s.outbox.union(Set({ e: h.e, line: { kind: "screen", pane: h.pane, k: h.k, full: false, bad: false, n: h.n }, n: h.n })),
      hold: NO_HOLD
    }
  }

  // remote.rs:answer: `conn.line` with the answer.
  action Answer(o: Out): bool = all {
    s.outbox.contains(o),
    s' = { ...s,
      outbox: s.outbox.exclude(Set(o)),
      lines: s.lines.set(o.e, s.lines.get(o.e).append(o.line))
    }
  }

  // remote.rs:writer, one round: all lines first, then the bytes of each pane.
  // A pane that dropped goes to "resync" and the screen thread hears of it.
  action Write(e: int): bool = all {
    s.attached.get(e),
    not(s.closed.get(e)),
    val q = s.outq.get(e)
    val busy = s.lines.get(e).length() > 0
      or PANES.exists(p => q.get(p).bytes.length() > 0 or q.get(p).state == "dropping")
    all {
      busy,
      val start = { wire: s.wire.get(e).concat(s.lines.get(e)), q: q, want: s.want.get(e) }
      val r = PANE_LIST.foldl(start, (acc, p) => {
        val pq = acc.q.get(p)
        val w = pq.bytes.foldl(acc.wire, (ws, c) => ws.append({ kind: "bytes", pane: c.pane, k: c.k, full: c.full, bad: c.bad, n: 0 }))
        val drop = pq.state == "dropping"
        { wire: w,
          q: acc.q.set(p, { state: if (drop) "resync" else pq.state, bytes: List() }),
          want: if (drop) acc.want.append(p) else acc.want }
      })
      s' = { ...s,
        wire: s.wire.set(e, r.wire),
        lines: s.lines.set(e, List()),
        outq: s.outq.set(e, r.q),
        want: s.want.set(e, r.want)
      }
    }
  }

  // The screen thread of window `e` (remote.rs:window, hub.rs:fresh_screen): one
  // fresh screen, from the pane's emulator, under the `terms` lock, so the reader
  // passes no chunk meanwhile. tmux never pauses. A pane off this window's
  // screen gets an empty screen, so the pane is live again.
  action Resync(e: int): bool = all {
    s.want.get(e).length() > 0,
    val p = s.want.get(e).head()
    val em = s.emu.get(p)
    val ok = s.live.get(e) == p and em.on
    val pq = s.outq.get(e).get(p)
    val screen = if (ok) List({ pane: p, k: em.upTo, full: true, bad: em.hole }) else List()
    val fills = ok and pq.state == "resync"
    s' = { ...s,
      want: s.want.set(e, s.want.get(e).tail()),
      outq: if (pq.state == "resync") s.outq.set(e, s.outq.get(e).set(p, { state: "live", bytes: screen })) else s.outq,
      filled: if (fills) s.filled + 1 else s.filled,
      shared: if (fills and watchedBy(s, p, e)) s.shared + 1 else s.shared
    }
  }

  // ---------- the fresh screen of remote.qnt: a pause of the pane ----------
  // These two steps are the code before SPEC 41.8. The pause stops the bytes of
  // the pane for every window. windows_test.qnt keeps that trace.

  // hub.rs:screen_for_resync: pause, capture, fill. tmux answers the pause after
  // all the chunks before it, and the reader passes them on first.
  action PauseFill(e: int): bool = all {
    s.paused == "",
    s.want.get(e).length() > 0,
    val p = s.want.get(e).head()
    val ok = s.live.get(e) == p
    val pq = s.outq.get(e).get(p)
    val screen = if (ok) List({ pane: p, k: s.ver.get(p), full: true, bad: false }) else List()
    all {
      not(ok) or not(s.tmuxOut.select(c => c.pane == p).length() > 0),
      s' = { ...s,
        want: s.want.set(e, s.want.get(e).tail()),
        streaming: if (ok) s.streaming.set(p, false) else s.streaming,
        paused: if (ok) p else "",
        outq: if (pq.state == "resync") s.outq.set(e, s.outq.get(e).set(p, { state: "live", bytes: screen })) else s.outq,
        filled: if (ok and pq.state == "resync") s.filled + 1 else s.filled
      }
    }
  }

  // hub.rs:resume_stream: the stream goes on, and the size changes.
  action PauseResume: bool = all {
    s.paused != "",
    val p = s.paused
    val on = watched(p)
    s' = { ...s,
      streaming: if (on) s.streaming.set(p, true) else s.streaming,
      wiggles: if (on) s.wiggles.union(Set(p)) else s.wiggles,
      paused: ""
    }
  }

  // ---------- the link and the windows ----------

  // The link of window `e` goes down (ssh, a sleep, a network change), or the
  // same window came back on a new link and the hub ends the old one. What is in
  // flight is lost. The hub's read loop then ends (`Exit`).
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
    val c = { pane: l.pane, k: l.k, full: l.full, bad: l.bad }
    s' = if (l.kind == "bytes") {
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
      live: EPOCHS.mapBy(_ => ""),
      emu: PANES.mapBy(_ => NO_EMU),
      nextEpoch: 1,
      hold: NO_HOLD,
      stops: Set(),
      paused: "",
      cmds: EPOCHS.mapBy(_ => List()),
      outbox: Set(),
      attached: EPOCHS.mapBy(_ => false),
      closed: EPOCHS.mapBy(_ => false),
      exited: EPOCHS.mapBy(_ => false),
      lines: EPOCHS.mapBy(_ => List()),
      outq: EPOCHS.mapBy(_ => PANES.mapBy(_ => EMPTY_Q)),
      want: EPOCHS.mapBy(_ => List()),
      wire: EPOCHS.mapBy(_ => List()),
      gone: EPOCHS.mapBy(_ => false),
      wlive: EPOCHS.mapBy(_ => ""),
      drawn: EPOCHS.mapBy(_ => false),
      held: EPOCHS.mapBy(_ => List()),
      expN: EPOCHS.mapBy(_ => 0),
      newest: newest,
      term: EPOCHS.mapBy(_ => { upTo: 0, hole: false }),
      n: 0,
      filled: 0,
      shared: 0
    }
  }

  action init = initWith(true)
  // The window before `expandSeq`: an older answer draws too.
  action initAnyAnswer = initWith(false)

  // With `ordered` false, each command runs in a thread of its own. With
  // `locked` false, a collapse stops its streams after the screen lock. With
  // `pause` true, a fresh screen pauses the pane. Each is the code before a fix.
  action stepWith(ordered: bool, locked: bool, pause: bool): bool = {
    nondet p = PANES.oneOf()
    nondet e = EPOCHS.oneOf()
    nondet i = 0.to(3).oneOf()
    any {
      Make(p),
      Redraw(p),
      Read,
      Attach,
      Exit(e, locked),
      StopStream(p),
      CmdStart(e, i, ordered, locked),
      Capture,
      ShowOn,
      all { s.outbox != Set(), nondet o = s.outbox.oneOf() Answer(o) },
      Write(e),
      all { not(pause), Resync(e) },
      all { pause, PauseFill(e) },
      all { pause, PauseResume },
      LinkDrop(e),
      WinExpand(e, p),
      WinCollapse(e),
      WinRead(e),
    }
  }

  action step = stepWith(true, true, false)
  action stepAnyOrder = stepWith(false, true, false)
  action stepNoScreenLock = stepWith(true, false, false)
  action stepPause = stepWith(true, true, true)

  // ---------- properties that must hold ----------

  // W1. A live pane in a queue never holds more than the budget (outq Q1).
  val QueueInBudget = EPOCHS.forall(e => PANES.forall(p => s.outq.get(e).get(p).bytes.length() <= BUDGET))

  // W2. A pane that waits for a fresh screen holds no bytes (outq "no gap").
  val NoBytesWhileWaiting = EPOCHS.forall(e => PANES.forall(p =>
    s.outq.get(e).get(p).state != "live" implies s.outq.get(e).get(p).bytes.length() == 0))

  // W3. Watch counts: outside an expand, a pane streams exactly while a window
  // shows it, and its emulator follows it exactly then. So a fresh screen for
  // one window never stops the bytes of another.
  val StreamsWhileWatched = (s.hold.n == 0 and s.stops == Set()) implies PANES.forall(p => and {
    s.streaming.get(p) == watched(p),
    s.emu.get(p).on == s.streaming.get(p)
  })

  // Nothing is in flight, and each window read all its lines.
  val quiet = and {
    s.tmuxOut.length() == 0, s.wiggles == Set(), s.hold.n == 0,
    s.outbox == Set(), s.stops == Set(), s.paused == "",
    EPOCHS.forall(e => and {
      s.lines.get(e).length() == 0,
      s.closed.get(e) or s.cmds.get(e).length() == 0,
      s.want.get(e).length() == 0,
      PANES.forall(p => s.outq.get(e).get(p).bytes.length() == 0 and s.outq.get(e).get(p).state == "live"),
      s.gone.get(e) or s.wire.get(e).length() == 0,
      s.closed.get(e) implies s.exited.get(e)
    })
  }

  // Window `e` is up and has drawn a pane.
  def shows(e: int): bool = and {
    s.attached.get(e), not(s.gone.get(e)), s.wlive.get(e) != "", s.drawn.get(e)
  }

  // W4. When all is quiet, the pane that each window shows is the pane on its
  // screen in the hub.
  val WindowMatchesHub = quiet implies EPOCHS.forall(e => shows(e) implies s.wlive.get(e) == s.live.get(e))

  // W5. When all is quiet, only the panes on some screen stream.
  val OnlyWatchedStreams = quiet implies PANES.forall(p => s.streaming.get(p) implies watched(p))

  // W6. When all is quiet and the program draws its screen again on a size
  // change, each window shows its pane as it is: no hole, no old chunk.
  val Fresh = (quiet and s.redraws) implies EPOCHS.forall(e =>
    (shows(e) and s.wlive.get(e) == s.live.get(e)) implies
      (s.term.get(e).upTo == s.ver.get(s.wlive.get(e)) and not(s.term.get(e).hole)))

  // ---------- witnesses: each must fail, or the runs miss that path ----------

  // A pane dropped its bytes, and a fresh screen filled it.
  val NoResync = s.filled == 0
  // A fresh screen filled a pane that another window showed too.
  val NoSharedResync = s.shared == 0
  // All is quiet with two windows that show the same pane.
  val NoQuietShare = not(quiet and EPOCHS.exists(a => EPOCHS.exists(b =>
    a < b and shows(a) and shows(b) and s.wlive.get(a) == s.wlive.get(b) and s.wlive.get(a) == s.live.get(a) and s.wlive.get(b) == s.live.get(b))))

  // The same, after a fresh screen of a pane that two windows showed: W6 then
  // checks both windows on that path.
  val NoQuietShareAfterResync = not(s.shared > 0) or NoQuietShare

  val Always = and { QueueInBudget, NoBytesWhileWaiting, StreamsWhileWatched, WindowMatchesHub, OnlyWatchedStreams, Fresh }
}
