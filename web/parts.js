// The parts of the live pane (SPEC 28): nvim and a shell next to an agent, in its
// worktree. While an agent runs in a worktree, its nvim and its shell have no tile.
// app.js calls in here through `SBParts`.
(() => {
  const { S, invoke, call, esc, setHTML, paneById, toast, bytes } = window.SB;
  const $ = (s) => document.querySelector(s);
  const NARROW = 1100;
  const KINDS = ["nvim", "term"];
  const PANE_KIND = { nvim: "nvim", term: "shell" };
  const KEY = { nvim: "e", term: "s" };
  // Parts show on the board, and in the Work environment tab (SPEC 30); a review
  // tab's live pane has none.
  const parted = () => !S.termHost || S.termHost === "work";
  const work = () => S.termHost === "work";

  // The live pane's layout. `shown` is what the window shows; `zoom` is the one part
  // that fills the pane, or null.
  const P = { tree: null, shown: { nvim: false, term: false }, focus: "agent", zoom: null, reloaded: 0, stopAsk: null };

  // One terminal for each part. Keys go in order, as for the live pane.
  const css = getComputedStyle(document.documentElement);
  function makeTerm(kind) {
    const term = new Terminal({
      fontFamily: css.getPropertyValue("--mono").trim(),
      fontSize: 13,
      cursorBlink: true,
      scrollback: 10000,
      allowProposedApi: true,
      theme: { background: css.getPropertyValue("--bg").trim(), foreground: css.getPropertyValue("--fg").trim(), cursor: css.getPropertyValue("--accent").trim(), selectionBackground: "#d9b56c44" },
    });
    const fit = new FitAddon.FitAddon();
    term.loadAddon(fit);
    term.open($(`#t${kind}`));
    window.SB.remeasure(term);
    term.attachCustomKeyEventHandler((e) => !window.SB.isLeader(e));
    const t = { term, fit, id: null, drawn: false, held: [], keys: "", sending: false };
    const flush = async () => {
      if (t.sending || !t.keys || !t.id) return;
      t.sending = true;
      const data = t.keys;
      t.keys = "";
      await invoke("pane_input", { id: t.id, data, binary: false }).catch(() => {});
      t.sending = false;
      flush();
    };
    term.onData((data) => { if (t.id) { t.keys += data; flush(); } });
    term.onBinary((data) => { if (t.id) invoke("pane_input", { id: t.id, data, binary: true }).catch(() => {}); });
    term.textarea?.addEventListener("focus", () => { P.focus = kind; heads(); });
    let timer;
    new ResizeObserver(() => { clearTimeout(timer); timer = setTimeout(() => fitPart(kind), 60); }).observe($(`#t${kind}`));
    return t;
  }
  const T = { nvim: makeTerm("nvim"), term: makeTerm("term") };
  window.SB.term.textarea?.addEventListener("focus", () => { P.focus = "agent"; heads(); });

  function fitPart(kind) {
    const t = T[kind];
    if (!t.id || !$(`#t${kind}`).offsetParent) return;
    t.fit.fit();
    invoke("part_resize", { id: t.id, cols: t.term.cols, rows: t.term.rows }).catch(() => {});
  }

  // The nvim or the shell of a worktree: the first pane of that kind there.
  const companion = (tree, kind) => S.panes.find((p) => p.kind === PANE_KIND[kind] && p.tree === tree);
  const hasAgent = (tree) => S.panes.some((p) => p.kind === "claude" && p.tree === tree && p.lamp !== "ended");
  // True for the nvim or shell of a worktree with an agent: it has no tile.
  function isPart(p) {
    if (p.kind !== "nvim" && p.kind !== "shell") return false;
    return hasAgent(p.tree) && companion(p.tree, p.kind === "nvim" ? "nvim" : "term")?.id === p.id;
  }
  // The parts that run but do not show.
  const hiddenCount = () => S.panes.filter((p) => isPart(p) && T.nvim.id !== p.id && T.term.id !== p.id).length;

  // The layout of each worktree, for the next open. A convenience of this window only.
  // The work tab keeps its own, so it never changes the board's.
  const key = (tree) => `sb.parts.${work() ? "work." : ""}${tree}`;
  const saved = (tree) => SBSplit.load(key(tree)) || {};
  const remember = (patch) => { if (P.tree) SBSplit.save(key(P.tree), { ...saved(P.tree), ...patch }); };
  const narrow = () => window.innerWidth < NARROW;

  function sizes() {
    const m = P.tree ? saved(P.tree) : {};
    const parts = $("#lparts");
    if (typeof m.nvimW === "number") parts.style.setProperty("--nvim-w", `${Math.round(m.nvimW * 1000) / 10}%`);
    else parts.style.removeProperty("--nvim-w");
    if (typeof m.termH === "number") parts.style.setProperty("--term-h", `${m.termH}px`);
    else parts.style.removeProperty("--term-h");
  }

  function layout() {
    const z = P.zoom;
    const showA = !z || z === "agent";
    const showN = P.shown.nvim && (!z || z === "nvim");
    const showT = P.shown.term && (!z || z === "term");
    $("#pagent").hidden = !showA;
    $("#pnvim").hidden = !showN;
    $("#pterm").hidden = !showT;
    $("#lleft").hidden = !showA && !showT;
    $("#gnvim").hidden = !(showN && (showA || showT));
    $("#gterm").hidden = !(showA && showT);
    $("#pterm").classList.toggle("fill", !showA);
    $("#pnvim").classList.toggle("fill", !showA && !showT);
    $("#lparts").classList.toggle("split", P.shown.nvim || P.shown.term);
    heads();
  }

  // A slim header for each part: its name and its state. Zoom shows on the part
  // with the keys. Hide and Stop are in the inspector (SPEC 33).
  const ZOOM = '<svg width="12" height="12" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><path d="M2 5V2h3M9 2h3v3M12 9v3H9M5 12H2V9"/></svg>';
  function heads() {
    const zoom = (part) => `<button class="icon" type="button" data-zoom="${part}" aria-label="${P.zoom ? "Take the zoom back" : "Zoom"} (Leader z)" title="${P.zoom ? "Take the zoom back" : "Zoom"} (Leader z)">${ZOOM}</button>`;
    const head = (part, name, extra) => `<span class="pn">${name}</span>${extra}<span class="sp"></span>${zoom(part)}`;
    const nv = paneById(T.nvim.id);
    const mod = nv?.modified ? `<span class="pmod" title="A buffer is not saved">[+]</span>` : "";
    const rel = Date.now() - P.reloaded < 6000 ? `<span class="prel">changed on disk, reloaded</span>` : "";
    const sh = paneById(T.term.id);
    const busy = sh?.busy ? `<span class="pmod" title="A command runs">▸</span>` : "";
    // Only a change is written: a pane event comes about once a second, and a new
    // button under the mouse drops the click.
    const set = (part, html) => {
      const h = $(`#p${part} .parthead`);
      setHTML(h, html);
      h.classList.toggle("on", P.focus === part);
    };
    set("agent", head("agent", "claude", ""));
    set("nvim", head("nvim", "nvim", mod + rel));
    set("term", head("term", "term", busy));
    window.SBInsp?.refresh();
  }

  // The parts in the inspector of the board and of the work tab: show or hide each
  // part, reset the sizes, and stop a part. Stop asks first when nvim has a buffer
  // that is not saved.
  function inspector(p) {
    if (!p || S.live !== p.id || !parted()) return null;
    const kinds = KINDS.filter((k) => k !== "nvim" || S.settings.nvim);
    const name = (k) => (k === "nvim" ? "nvim" : "the terminal");
    const actions = kinds.map((k) => ({ label: `${P.shown[k] ? "Hide" : "Show"} ${name(k)}`, act: `${P.shown[k] ? "hide" : "show"}-${k}`, key: KEY[k],
      title: P.shown[k] ? "It keeps running" : `Start or show ${name(k)} in this worktree` }));
    actions.push({ label: "Reset the sizes", act: "reset", title: "The dividers go back to half the width and 220 px" });
    const danger = kinds.filter((k) => T[k].id).map((k) => ({ label: `Stop ${k}`, act: `stop-${k}`, title: `End ${k === "nvim" ? "nvim" : "the shell"}` }));
    const alert = P.stopAsk ? { text: "A buffer is not saved. Stop nvim anyway?", buttons: [{ label: "Stop", act: `stopyes-${P.stopAsk}` }, { label: "Cancel", act: "stopno" }] } : null;
    return { actions, danger, alert };
  }
  function inspAct(act, p) {
    const [verb, kind] = act.split("-");
    if (verb === "hide" && P.shown[kind]) return hide(kind, false);
    if (verb === "show") return show(kind, p.id);
    if (verb === "reset") return resetSizes();
    if (verb === "stop") return stop(kind, false);
    if (verb === "stopyes") return stop(kind, true);
    if (verb === "stopno") { P.stopAsk = null; heads(); }
  }
  // The dividers of this worktree go back to their first sizes.
  function resetSizes() {
    remember({ nvimW: null, termH: null });
    sizes();
  }

  // Puts a running pane in a part and draws its screen.
  async function attach(kind, id) {
    const t = T[kind];
    t.id = id;
    t.drawn = false;
    t.held = [];
    t.keys = "";
    P.shown[kind] = true;
    P.focus = kind;
    layout();
    window.SB.drawBoard();
    await new Promise((r) => setTimeout(r, 0));
    if (t.id !== id) return;
    t.fit.fit();
    t.term.reset();
    try {
      const screen = await invoke("pane_attach", { id, owner: S.live, cols: t.term.cols, rows: t.term.rows });
      if (t.id !== id) return;
      t.term.write(bytes(screen));
      t.held.splice(0).forEach((b) => t.term.write(b));
      t.drawn = true;
      if (P.focus === kind) t.term.focus();
    } catch (e) {
      toast(e);
      hide(kind, true);
    }
  }

  // Takes a part off the screen. Its process keeps running. With `keep`, the
  // layout memory stays as it is.
  function hide(kind, keep) {
    const t = T[kind];
    if (t.id) invoke("pane_detach", { id: t.id }).catch(() => {});
    t.id = null;
    t.drawn = false;
    P.shown[kind] = false;
    if (P.stopAsk === kind) P.stopAsk = null;
    if (P.zoom === kind) P.zoom = null;
    if (!keep) remember({ [kind]: false });
    const back = P.focus === kind;
    if (back) P.focus = "agent";
    layout();
    window.SB.drawBoard();
    if (back) window.SB.term.focus();
  }

  // Shows the part of an agent pane: expands the agent first, and starts nvim or
  // the shell in its worktree when none runs.
  async function show(kind, agentId) {
    const a = paneById(agentId);
    if (!a || a.kind !== "claude" || (kind === "nvim" && !S.settings.nvim)) return;
    // The work tab shows its own agent; it opens that pane itself first.
    if ((S.live !== agentId || !parted()) && !work()) await window.SB.expand(agentId);
    if (S.live !== agentId || !parted()) return;
    if (P.shown[kind]) return focusPart(kind);
    let id;
    try {
      id = kind === "nvim" ? await invoke("nvim_open", { tree: a.tree, file: null, line: null }) : await invoke("shell_open", { tree: a.tree });
    } catch (e) {
      toast(e);
      return;
    }
    await window.SB.refreshPanes();
    if (S.live !== agentId) return;
    // A narrow window shows one part at a time, in place of the agent.
    if (narrow()) {
      for (const k of KINDS) if (k !== kind && P.shown[k]) hide(k, true);
      P.zoom = kind;
    }
    remember({ [kind]: true });
    await attach(kind, id);
  }

  function toggle(kind, agentId) {
    if (S.live === agentId && parted() && P.shown[kind]) hide(kind, false);
    else show(kind, agentId);
  }

  function stop(kind, sure) {
    const id = T[kind].id;
    const p = paneById(id);
    if (!p) return;
    if (kind === "nvim" && p.modified && !sure) { P.stopAsk = kind; heads(); return; }
    P.stopAsk = null;
    hide(kind, false);
    call("pane_close", { id });
  }

  const visible = (part) => !$(`#p${part}`).hidden && (part === "agent" || P.shown[part]);
  function focusPart(part) {
    if (!visible(part)) return;
    P.focus = part;
    (part === "agent" ? window.SB.term : T[part].term).focus();
    heads();
  }
  // Leader o: the next part, in the order claude, term, nvim.
  function cycle() {
    const list = ["agent", "term", "nvim"].filter(visible);
    if (list.length < 2) return;
    focusPart(list[(list.indexOf(P.focus) + 1) % list.length]);
  }
  // Leader h and l: the part on the left or the right.
  let lastLeft = "agent";
  function side(d) {
    // In the work tab nvim is on the left, so the keys turn around.
    if (work()) d = -d;
    if (d > 0 && P.focus !== "nvim") { lastLeft = P.focus; focusPart("nvim"); }
    else if (d < 0 && P.focus === "nvim") focusPart(visible(lastLeft) ? lastLeft : visible("agent") ? "agent" : "term");
  }
  // Leader z. False when there is nothing to zoom.
  function zoom(part = P.focus) {
    // A narrow window always shows one part, so it has no zoom to change.
    if (narrow() || (!P.shown.nvim && !P.shown.term)) return false;
    P.zoom = P.zoom ? null : part;
    layout();
    focusPart(P.zoom || part);
    return true;
  }
  // Leader x takes a zoom back first. A narrow window has no other layout.
  function unzoom() {
    if (!P.zoom || narrow()) return false;
    P.zoom = null;
    layout();
    focusPart(P.focus);
    return true;
  }

  // All parts off, as after a collapse.
  function reset() {
    for (const k of KINDS) { T[k].id = null; T[k].drawn = false; P.shown[k] = false; }
    P.zoom = null;
    P.focus = "agent";
    P.stopAsk = null;
    P.tree = null;
    layout();
  }

  // After an expand: the hub took the old parts off. An agent gets back the parts
  // that showed the last time, when they still run.
  function afterExpand(id) {
    reset();
    const p = paneById(id);
    if (!p || p.kind !== "claude" || !parted()) return;
    P.tree = p.tree;
    sizes();
    const m = saved(p.tree);
    let first = true;
    for (const k of ["nvim", "term"]) {
      const c = companion(p.tree, k);
      if (!m[k] || !c || (narrow() && !first)) continue;
      if (narrow()) P.zoom = k;
      first = false;
      attach(k, c.id);
    }
  }

  function output(pane, b) {
    for (const k of KINDS) {
      const t = T[k];
      if (t.id === pane) t.drawn ? t.term.write(b) : t.held.push(b);
    }
  }

  // The tile shows a short ✓ after a command in the shell ends.
  const wasBusy = new Map();
  const doneAt = new Map();
  function panes() {
    for (const k of KINDS) if (T[k].id && !paneById(T[k].id)) hide(k, false);
    for (const p of S.panes) {
      if (p.kind !== "shell") continue;
      if (wasBusy.get(p.id) && !p.busy) {
        doneAt.set(p.id, Date.now());
        setTimeout(window.SB.drawBoard, 10100);
      }
      wasBusy.set(p.id, !!p.busy);
    }
    heads();
  }

  // The nvim and term chips of an agent tile. They always show (SPEC 28.3): a part
  // that runs has a dot, and a filled chip.
  function chips(p) {
    return KINDS.filter((k) => k !== "nvim" || S.settings.nvim).map((k) => {
      const c = companion(p.tree, k);
      if (!c) return `<button class="chip pchip off" type="button" data-part="${k}" title="Start ${k === "nvim" ? "nvim" : "a shell"} in this worktree (Leader ${KEY[k]})">${k}</button>`;
      const done = k === "term" && !c.busy && Date.now() - (doneAt.get(c.id) || 0) < 10000;
      const plus = c.modified ? ` <span class="pplus">[+]</span>` : "";
      const what = k === "nvim" ? (c.modified ? "nvim runs. A buffer is not saved" : "nvim runs") : c.busy ? "A command runs in the shell" : done ? "The command ended" : "The shell runs";
      const ra = c.rss ? `. rust-analyzer ${(c.rss / 1073741824).toFixed(1)} GB` : "";
      return `<button class="chip pchip on" type="button" data-part="${k}" title="${esc(what + ra)}"><span class="cdot"></span>${k}${plus}${c.busy ? " ▸" : ""}${done ? " ✓ done" : ""}</button>`;
    }).join("");
  }

  // The nvim and term switches in the live title.
  function toggles(p) {
    if (p.kind !== "claude" || !parted()) return null;
    const b = (k) => `<button class="chip ptog" type="button" data-ptoggle="${k}" aria-pressed="${P.shown[k]}" title="Show or hide ${k === "nvim" ? "nvim" : "the terminal"} (Leader ${KEY[k]})">${P.shown[k] ? '<span class="cdot"></span>' : ""}${k}</button>`;
    return `<span class="ptoggles" role="group" aria-label="Parts of this pane">${S.settings.nvim ? b("nvim") : ""}${b("term")}</span>`;
  }

  $("#lparts").addEventListener("click", (e) => {
    const b = e.target.closest("button");
    if (!b) {
      const part = e.target.closest(".part")?.dataset.part;
      if (part && part !== P.focus) focusPart(part);
      return;
    }
    if (b.dataset.zoom) return zoom(b.dataset.zoom);
  });

  // The divider sizes, for each worktree. A double-click resets them.
  SBSplit.bind($("#gnvim"), {
    // nvim is on the right on the board and on the left in the work tab.
    axis: "x", dir: () => (work() ? 1 : -1), controls: "pnvim",
    size: () => $("#pnvim").getBoundingClientRect().width,
    limits: () => [240, Math.max(240, $("#lparts").getBoundingClientRect().width - 240)],
    apply: (v) => {
      const w = $("#lparts").getBoundingClientRect().width;
      $("#lparts").style.setProperty("--nvim-w", `${Math.max(240, Math.min(v, w - 240))}px`);
    },
    reset: () => { remember({ nvimW: null }); sizes(); },
    done: (reset) => {
      if (!reset) remember({ nvimW: $("#pnvim").getBoundingClientRect().width / $("#lparts").getBoundingClientRect().width });
      sizes();
    },
  });
  SBSplit.bind($("#gterm"), {
    axis: "y", dir: -1, controls: "pterm",
    size: () => $("#pterm").getBoundingClientRect().height,
    limits: () => [100, Math.max(100, $("#lleft").getBoundingClientRect().height - 120)],
    apply: (v) => {
      const h = $("#lleft").getBoundingClientRect().height;
      $("#lparts").style.setProperty("--term-h", `${Math.round(Math.max(100, Math.min(v, h - 120)))}px`);
    },
    reset: () => { remember({ termH: null }); sizes(); },
    done: (reset) => { if (!reset) remember({ termH: Math.round($("#pterm").getBoundingClientRect().height) }); },
  });

  // A window that gets narrow keeps one part.
  window.addEventListener("resize", () => {
    if (!narrow() || P.zoom || !(P.shown.nvim || P.shown.term)) return;
    const k = P.shown.nvim ? "nvim" : "term";
    for (const o of KINDS) if (o !== k && P.shown[o]) hide(o, true);
    P.zoom = k;
    layout();
  });

  window.SB.listen("nvim_reloaded", (e) => {
    if (e.payload !== P.tree || !P.shown.nvim) return;
    P.reloaded = Date.now();
    heads();
    setTimeout(heads, 6100);
  });

  layout();
  window.SBParts = { P, T, isPart, hiddenCount, chips, toggles, show, toggle, hide, stop, cycle, side, zoom, unzoom, reset, afterExpand, output, panes, companion, inspector, inspAct, resetSizes };
})();
