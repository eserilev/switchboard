// Switchboard window: the board, the live pane, the launcher, keys and connections.
// review.js draws the review tabs. Both share the `SB` object.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (s, r = document) => r.querySelector(s);
const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const ANSI = /\x1b\[[0-9;?]*[ -\/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()][0-9A-B]|\x1b[=>]/g;
const strip = (s) => s.replace(ANSI, "");
const bytes = (b64) => Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));

const LABEL = { idle: "Idle", working: "Working", needs: "Needs you", turn: "Your turn", limit: "Limit", error: "Error", ended: "Ended" };
const COLOR = { needs: "var(--needs)", limit: "var(--limit)", error: "var(--error)", turn: "var(--done)" };
const URGENT = ["needs", "limit", "error", "turn"];

const S = {
  panes: [],
  live: null,
  focus: null,
  tab: "board",
  // The review tab whose Claude column holds the live terminal.
  termHost: null,
  // The pane open on the board. It opens again when the board shows after a review took the terminal.
  boardLive: null,
  reviews: new Map(),
  conn: { active: "default", list: [] },
  settings: { nvim: true },
  repos: [],
};

// Window logs go to the app log through the `log` command.
const log = (level, ...parts) => invoke("log", { level, message: parts.map(String).join(" ") }).catch(() => {});
window.addEventListener("error", (e) => log("error", "window error:", e.message, e.filename + ":" + e.lineno));
window.addEventListener("unhandledrejection", (e) => log("error", "unhandled rejection:", e.reason));

function toast(msg) {
  log("warn", "toast:", msg);
  const t = $("#toast");
  t.textContent = String(msg);
  t.hidden = false;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => (t.hidden = true), 7000);
}
// With the link to a remote hub down, the banner says so: no toast for each command.
const call = (cmd, args) => invoke(cmd, args).catch((e) => { log("warn", cmd, "failed:", e); if (LINK.up) toast(e); throw e; });

// ---------- the link to a remote hub (SPEC 29) ----------

// `synced`: the window has loaded everything from the hub since the link came up.
const LINK = { remote: false, up: true, host: "", why: null, taken: false, synced: true };
// Once for each time the link comes up, from the event or from Reconnect.
function linkChanged() {
  if (!LINK.up) LINK.synced = false;
  else if (!LINK.synced) { LINK.synced = true; resync(); }
  drawLink();
}
function drawLink() {
  $("#host").hidden = !LINK.remote;
  $("#host").textContent = LINK.host;
  $("#host").classList.toggle("down", !LINK.up);
  $("#link").hidden = LINK.up;
  if (LINK.up) return;
  const connecting = LINK.why === "connecting";
  $("#linktext").textContent = connecting ? `Connecting to ${LINK.host}…`
    : LINK.taken ? `Another window took over the hub on ${LINK.host}. Your agents run on there.`
    : `No link to ${LINK.host}: ${LINK.why || "the link is down"}. Your agents run on there.`;
  $("#linkgo").hidden = connecting;
  $("#linkgo").textContent = LINK.taken ? "Take it back" : "Reconnect";
}
async function reconnect() {
  $("#linkgo").disabled = true;
  Object.assign(LINK, { why: "connecting", taken: false });
  drawLink();
  try {
    Object.assign(LINK, await invoke("link_reconnect"));
  } catch (e) {
    Object.assign(LINK, { up: false, why: String(e) });
  }
  $("#linkgo").disabled = false;
  linkChanged();
}
// Everything again from the hub. The hub dropped the screen of the old link, so
// the live pane and its parts open again.
async function resync() {
  const live = S.live;
  if (live) collapse();
  try {
    S.settings = await invoke("settings");
    S.conn = await invoke("connections");
    drawConn();
    await refreshPanes();
    await refreshReviews();
    window.SBTasks?.refresh();
    if (live && paneById(live)) expand(live);
  } catch (e) {
    log("warn", "resync failed:", e);
  }
}

// ---------- tabs ----------

// `quiet`: the caller opens a pane itself, so the board does not open its own again.
function setTab(tab, quiet) {
  if (tab !== "board") window.SB?.term?.blur();
  if (tab !== "tasks") window.SBTasks?.leave();
  // The terminal lives in a review's Claude column while that review shows.
  if (S.termHost && S.termHost !== tab) {
    S.termHost = null;
    parkLive();
    if (S.live) collapse(true);
  }
  S.tab = tab;
  $("#view-board").hidden = tab !== "board";
  $("#view-tasks").hidden = tab !== "tasks";
  $("#view-review").hidden = tab === "board" || tab === "tasks";
  drawTabs();
  if (tab === "board") {
    const back = S.boardLive;
    if (!quiet && back && !S.live && paneById(back)) expand(back);
    else requestAnimationFrame(fitLive);
  } else if (tab === "tasks") window.SBTasks?.draw();
  else window.SBReview?.draw(tab);
}

function drawTabs() {
  const tabs = [`<button class="tab" role="tab" data-tab="board" aria-selected="${S.tab === "board"}">Board</button>`,
    `<button class="tab" role="tab" data-tab="tasks" aria-selected="${S.tab === "tasks"}">Tasks</button>`];
  for (const r of S.reviews.values()) {
    const busy = ["fetching", "writing", "updating"].includes(r.status) ? '<i class="busy"></i>' : "";
    const label = r.kind === "local" ? `${esc(r.repo)} commits` : `#${esc(r.number)}${r.round > 1 ? ` R${r.round}` : ""}`;
    tabs.push(`<button class="tab" role="tab" data-tab="${esc(r.id)}" aria-selected="${S.tab === r.id}">${label}${busy}</button>`);
  }
  $("#tabs").innerHTML = tabs.join("");
}
$("#tabs").addEventListener("click", (e) => { const t = e.target.closest("[data-tab]"); if (t) setTab(t.dataset.tab); });

// ---------- board ----------

function paneName(p) {
  if (p.title) return p.title;
  const base = p.tree.split("/").filter(Boolean).pop() || p.repo;
  if (p.kind === "nvim") return `${p.repo} · nvim`;
  return base !== p.repo && p.branch ? p.branch : p.repo;
}

function actions(p) {
  const b = (attr, label, primary) => `<button class="btn${primary ? " primary" : ""}" ${attr}>${label}</button>`;
  if (p.trust) return `<div class="tact">${b(`data-trust="1"`, "Trust", true)}${b(`data-trust="0"`, "Exit")}</div>`;
  if (p.permit) return `<div class="tact">${b(`data-allow="${esc(p.permit.id)}"`, "Allow", true)}${b(`data-deny="${esc(p.permit.id)}"`, "Deny")}</div>`;
  if (p.lamp === "limit") {
    const next = nextConnection(p.connection);
    return next ? `<div class="tact">${b("data-switch", `Switch to ${esc(next)}`, true)}</div>` : "";
  }
  if ((p.lamp === "ended" || p.lamp === "error") && p.session && p.kind === "claude") return `<div class="tact">${b("data-resume", "Resume", true)}</div>`;
  return "";
}

function nextConnection(current) {
  const free = S.conn.list.filter((c) => c.name !== current && !(c.limit_until && c.limit_until * 1000 > Date.now()));
  return free[0]?.name;
}

// The review whose Claude pane this is (SPEC 11.5), or none. That pane shows in
// its review tab, not on the board.
const askOf = (p) => p.kind === "claude" && [...S.reviews.values()].find((r) => r.tree === p.tree && p.title === `Ask #${r.number}`);

// The tiles on the board. The nvim and shell of an agent are its parts (SPEC 28).
// A part that is the live pane itself keeps its tile, as every live pane does.
const tiles = () => S.panes.filter((p) => p.id === S.live || (!window.SBParts?.isPart(p) && !askOf(p)));

function meta(p) {
  const parts = [p.repo];
  if (p.branch) parts.push(p.branch);
  if (p.connection && S.conn.list.length > 1) parts.push(p.connection);
  if (p.rss) parts.push(`ra ${(p.rss / 1073741824).toFixed(1)} GB`);
  return parts.join(" · ");
}

function tile(p) {
  const nvimBtn = p.kind === "claude" && window.SBParts ? SBParts.chips(p) : p.kind !== "nvim" && S.settings.nvim ? `<button class="tool" data-nvim>nvim</button>` : "";
  const state = p.lamp === "none" ? p.kind : LABEL[p.lamp] || p.lamp;
  const reviewBtn = p.kind === "claude" ? `<button class="tool" data-lreview title="Review the commits that are not pushed">review</button>` : "";
  const taskBtn = p.kind !== "nvim" ? `<button class="tool" data-ptaskmenu title="Link this pane to a task">task</button>` : "";
  const cls = ["pane", p.unseen && "unseen", S.focus === p.id && "focused", p.id === S.live && "is-live"].filter(Boolean).join(" ");
  return `<div class="${cls}" data-id="${esc(p.id)}" data-lamp="${esc(p.lamp)}" tabindex="0">
    <div class="ptitle"><span class="name" title="${esc(paneName(p))}">${esc(paneName(p))}</span><span class="state">${esc(state)}</span><button class="tool x" data-close aria-label="Close">×</button></div>
    <div class="psum">${taskLine(p)}<div class="pmeta"><span class="meta">${esc(meta(p))}</span><span class="ptools">${taskBtn}${reviewBtn}${nvimBtn}</span></div><span class="text">${esc(p.summary || "")}</span>${actions(p)}</div>
    <pre class="term-tail">${esc(p.tail.map(strip).join("\n"))}</pre>
  </div>`;
}

// The task of a pane, under its title (SPEC 27.6). A click opens the task.
function taskLine(p) {
  const t = window.SBTasks?.forPane(p.id);
  return t ? `<button class="ptask" data-ptask="${esc(t.id)}" title="Open the task">▸ ${esc(t.title)}</button>` : "";
}

// Many pane events can come in one frame. Draw the board once per frame.
let drawQueued = false;
function drawBoard() {
  if (drawQueued) return;
  drawQueued = true;
  // A hidden window gets no frames, so a timer backs the frame up.
  const go = () => { if (drawQueued) { drawQueued = false; drawBoardNow(); } };
  requestAnimationFrame(go);
  setTimeout(go, 50);
}

function drawBoardNow() {
  // A redraw in the middle of a drag throws away the tile under the pointer.
  if (drag?.on) { drawLater = true; return; }
  // The open pane keeps its tile, so the other tiles do not move (SPEC 6).
  $("#board").innerHTML = tiles().map(tile).join("");
  $("#view-board").classList.toggle("has-live", !!S.live);
  $("#bgutter").hidden = !S.live || S.termHost !== null;
  drawTally();
  drawLiveTitle();
}

function drawTally() {
  const n = {};
  for (const p of S.panes) if (p.unseen && COLOR[p.lamp]) n[p.lamp] = (n[p.lamp] || 0) + 1;
  $("#tally").innerHTML = URGENT.filter((s) => n[s])
    .map((s) => `<b style="color:${COLOR[s]}" data-jump="${s}"><i class="dot"></i>${n[s]} ${LABEL[s].toLowerCase()}</b>`)
    .join("");
  const hidden = window.SBParts?.hiddenCount() || 0;
  if (hidden) $("#tally").insertAdjacentHTML("beforeend", `<span class="phidden" title="nvim and shells that run but do not show">${hidden} hidden</span>`);
}
$("#tally").addEventListener("click", (e) => { if (e.target.closest("b")) nextUnseen(1); });

function upsert(p) {
  const i = S.panes.findIndex((x) => x.id === p.id);
  if (i < 0) S.panes.push(p);
  else S.panes[i] = p;
}

const paneById = (id) => S.panes.find((p) => p.id === id);

// ---------- drag to reorder ----------

let drag = null;
let drawLater = false;
let justDragged = false;
const clearMarks = () => document.querySelectorAll(".drop-before, .drop-after").forEach((n) => n.classList.remove("drop-before", "drop-after"));

$("#board").addEventListener("pointerdown", (e) => {
  if (e.button !== 0 || e.target.closest("button, [contenteditable='true']")) return;
  const el = e.target.closest(".pane");
  if (el) drag = { id: el.dataset.id, x: e.clientX, y: e.clientY, on: false, target: null };
});

window.addEventListener("pointermove", (e) => {
  if (!drag) return;
  if (!drag.on) {
    if (Math.hypot(e.clientX - drag.x, e.clientY - drag.y) < 6) return;
    drag.on = true;
    document.body.classList.add("is-dragging");
    $(`#board .pane[data-id="${CSS.escape(drag.id)}"]`)?.classList.add("dragging");
  }
  clearMarks();
  const el = document.elementsFromPoint(e.clientX, e.clientY).map((n) => n.closest?.("#board .pane")).find((p) => p && p.dataset.id !== drag.id);
  if (!el) { drag.target = null; return; }
  const r = el.getBoundingClientRect();
  const after = e.clientX > r.left + r.width / 2;
  el.classList.add(after ? "drop-after" : "drop-before");
  drag.target = { id: el.dataset.id, after };
});

window.addEventListener("pointerup", () => {
  if (!drag) return;
  const d = drag;
  drag = null;
  if (!d.on) return;
  document.body.classList.remove("is-dragging");
  clearMarks();
  justDragged = true;
  setTimeout(() => (justDragged = false), 0);
  if (d.target) moveTile(d.id, d.target.id, d.target.after);
  else if (drawLater) drawBoard();
  drawLater = false;
});

/// Moves a tile before or after another one, and saves the order.
function moveTile(id, target, after) {
  const ids = S.panes.map((p) => p.id).filter((x) => x !== id);
  const at = ids.indexOf(target);
  if (at < 0) return;
  ids.splice(after ? at + 1 : at, 0, id);
  S.panes.sort((a, b) => ids.indexOf(a.id) - ids.indexOf(b.id));
  drawBoardNow();
  $(`#board .pane[data-id="${CSS.escape(id)}"]`)?.focus();
  call("pane_reorder", { ids });
}

/// Moves the focused tile one place left (-1) or right (+1).
function nudgeTile(d) {
  const ids = tiles().map((p) => p.id);
  const i = ids.indexOf(S.focus);
  const j = i + d;
  if (i < 0 || j < 0 || j >= ids.length) return;
  moveTile(S.focus, ids[j], d > 0);
}

$("#board").addEventListener("click", (e) => {
  if (justDragged) return;
  const el = e.target.closest(".pane");
  if (!el) return;
  const id = el.dataset.id;
  const b = e.target.closest("button");
  // A click on the open tile goes to its terminal.
  if (!b && id === S.live && S.termHost === null) return term.focus();
  if (!b) return expand(id);
  e.stopPropagation();
  if (b.dataset.close !== undefined) return call("pane_close", { id });
  if (b.dataset.nvim !== undefined) return openNvim(paneById(id).tree);
  if (b.dataset.part) return window.SBParts?.show(b.dataset.part, id);
  if (b.dataset.lreview !== undefined) return openLocalReview(id);
  if (b.dataset.ptask) return window.SBTasks?.open(b.dataset.ptask);
  if (b.dataset.ptaskmenu !== undefined) return window.SBTasks?.menu(id, b);
  if (b.dataset.allow) return call("permit_answer", { permit: b.dataset.allow, allow: true });
  if (b.dataset.deny) return call("permit_answer", { permit: b.dataset.deny, allow: false });
  if (b.dataset.trust) return call("trust_answer", { id, yes: b.dataset.trust === "1" });
  if (b.dataset.switch !== undefined) return call("connection_resume", { id });
  if (b.dataset.resume !== undefined) return call("pane_resume", { id });
});

// Rename: double-click the title.
document.addEventListener("dblclick", (e) => {
  // Rename works on the live title. A click on a tile expands it first.
  const name = e.target.closest("#livetitle .name");
  if (!name || !S.live) return;
  const id = S.live;
  renaming = true;
  e.preventDefault();
  name.contentEditable = "true";
  name.focus();
  document.getSelection().selectAllChildren(name);
  const done = (save) => {
    if (!renaming) return;
    renaming = false;
    name.contentEditable = "false";
    if (save) call("pane_rename", { id, title: name.textContent.trim() || null });
    else drawBoard();
  };
  name.onkeydown = (k) => {
    if (k.key === "Enter") { k.preventDefault(); done(true); }
    if (k.key === "Escape") { k.preventDefault(); done(false); }
    k.stopPropagation();
  };
  name.onblur = () => done(true);
});

// ---------- the live pane ----------

const css = getComputedStyle(document.documentElement);
const term = new Terminal({
  fontFamily: css.getPropertyValue("--mono").trim(),
  fontSize: 13,
  cursorBlink: true,
  scrollback: 50000,
  allowProposedApi: true,
  theme: { background: css.getPropertyValue("--bg").trim(), foreground: css.getPropertyValue("--fg").trim(), cursor: css.getPropertyValue("--accent").trim(), selectionBackground: "#d9b56c44" },
});
const fit = new FitAddon.FitAddon();
term.loadAddon(fit);
term.open($("#term"));
term.attachCustomKeyEventHandler((e) => !isLeader(e));
// Keys go in order. While one send is on its way, new keys wait in a buffer,
// and the next send takes them all at once. So fast typing costs one round trip.
let pendingKeys = "";
let pendingPane = null;
let sending = false;
async function flushKeys() {
  if (sending || !pendingKeys) return;
  sending = true;
  const id = pendingPane;
  const data = pendingKeys;
  pendingKeys = "";
  try {
    await invoke("pane_input", { id, data, binary: false });
  } catch (e) {
    log("warn", "pane_input failed:", e);
  }
  sending = false;
  flushKeys();
}
const send = (data, binary) => {
  if (!S.live || (S.tab !== "board" && S.termHost !== S.tab)) return;
  if (binary) {
    // Mouse reports are bytes, not text: they go on their own, after the keys.
    const id = S.live;
    const before = pendingKeys;
    pendingKeys = "";
    if (before) invoke("pane_input", { id, data: before, binary: false }).catch(() => {});
    invoke("pane_input", { id, data, binary: true }).catch(() => {});
    return;
  }
  if (pendingPane !== S.live) { pendingKeys = ""; pendingPane = S.live; }
  pendingKeys += data;
  flushKeys();
};
term.onData((data) => send(data, false));
term.onBinary((data) => send(data, true));

let drawn = false;
let held = [];
let firstOutput = null;
// The newest expand. Only its answer draws: an older answer has an older screen.
let expandSeq = 0;
listen("pane_output", (e) => {
  if (e.payload.pane !== S.live) return window.SBParts?.output(e.payload.pane, bytes(e.payload.data));
  if (firstOutput === S.live) { firstOutput = null; log("info", "first output reached the window for", S.live); }
  const b = bytes(e.payload.data);
  drawn ? term.write(b) : held.push(b);
});

function fitLive() {
  if (!S.live || $("#live").hidden || !$("#term").offsetParent) return;
  fit.fit();
  invoke("pane_resize", { cols: term.cols, rows: term.rows }).catch(() => {});
}
let fitTimer;
const fitSoon = () => { clearTimeout(fitTimer); fitTimer = setTimeout(fitLive, 60); };
new ResizeObserver(fitSoon).observe($("#live"));
// A part that shows or hides changes the size of the agent's terminal only.
new ResizeObserver(fitSoon).observe($("#term"));
// Full screen and back also fires a window resize: fit then too.
window.addEventListener("resize", fitSoon);

// The board handle: the height of the tile strip under the live terminal. The
// terminal keeps at least LIVE_MIN px; the strip at least one row of tiles, and a
// drag ends on whole rows.
const ROW_H = 136, ROW_GAP = 10, STRIP_PAD = 24;
const STRIP_MIN = ROW_H + STRIP_PAD, LIVE_MIN = 240, STRIP_KEY = "sb.split.board";
// The strip height of whole rows nearest to `h`.
const snapRows = (h) => { const rows = Math.max(1, Math.round((h - STRIP_PAD + ROW_GAP) / (ROW_H + ROW_GAP))); return rows * ROW_H + (rows - 1) * ROW_GAP + STRIP_PAD; };
function setStrip(h) {
  const wrap = $("#view-board");
  if (h === null) { wrap.classList.remove("sized"); wrap.style.removeProperty("--strip"); return null; }
  wrap.classList.add("sized");
  wrap.style.setProperty("--strip", `${h}px`);
  return h;
}
const stripRoom = () => $("#view-board").getBoundingClientRect().height - 24;
const stripNow = () => $("#board").getBoundingClientRect().height;
{
  const saved = SBSplit.load(STRIP_KEY);
  if (typeof saved === "number") setStrip(saved);
  const clamp = (v) => SBSplit.side(stripRoom(), v, 0, STRIP_MIN, LIVE_MIN);
  SBSplit.bind($("#bgutter"), {
    axis: "y", dir: -1, size: stripNow, controls: "board",
    limits: () => [STRIP_MIN, Math.max(STRIP_MIN, stripRoom() - LIVE_MIN)],
    apply: (v) => setStrip(clamp(v)),
    reset: () => { try { localStorage.removeItem(STRIP_KEY); } catch (_) {} setStrip(null); },
    done: (reset, drag) => {
      if (!reset && drag && $("#view-board").classList.contains("sized")) {
        // A drag ends on whole rows, unless that takes the terminal's room. Keys
        // move by pixels.
        const h = snapRows(stripNow());
        setStrip(h === clamp(h) ? h : clamp(stripNow()));
      }
      if (!reset && $("#view-board").classList.contains("sized")) SBSplit.save(STRIP_KEY, stripNow());
      fitSoon();
    },
  });
}

// Puts the live section back on the board.
function parkLive() {
  const live = $("#live");
  if (live.parentElement !== $("#view-board")) $("#view-board").prepend(live);
}

// Shows a pane in the live terminal. With `host`, the terminal moves into that
// element of the current review tab; without it, the board shows.
async function expand(id, host) {
  log("info", "expand", id, host ? "in review" : "");
  if (!paneById(id)) return;
  if (host) {
    // A host of a tab that no longer shows: do nothing.
    if (S.tab === "board" || !host.isConnected) return;
    window.SBParts?.reset();
    S.termHost = S.tab;
    if ($("#live").parentElement !== host) host.appendChild($("#live"));
  } else {
    parkLive();
    S.termHost = null;
    S.boardLive = id;
    if (S.tab !== "board") setTab("board", true);
  }
  S.live = id;
  S.focus = id;
  const seq = ++expandSeq;
  drawn = false;
  held = [];
  firstOutput = id;
  $("#live").hidden = false;
  drawBoardNow();
  // Let the layout settle so the fit sees the real size.
  await new Promise((r) => setTimeout(r, 0));
  fit.fit();
  term.reset();
  try {
    await invoke("pane_resize", { cols: term.cols, rows: term.rows });
    const screen = await invoke("pane_expand", { id });
    if (S.live !== id || seq !== expandSeq) return;
    term.write(bytes(screen));
    held.splice(0).forEach((b) => term.write(b));
    drawn = true;
    term.focus();
    if (!host) window.SBParts?.afterExpand(id);
    log("info", "expanded", id, "cols", term.cols, "rows", term.rows, "focus in terminal:", document.activeElement === term.textarea);
  } catch (e) {
    if (seq !== expandSeq) return;
    toast(e);
    collapse();
  }
}

// `keep`: a tab change took the terminal, so the board keeps its pane for later.
function collapse(keep) {
  const was = S.live;
  S.live = null;
  if (!keep && !S.termHost) S.boardLive = null;
  window.SBParts?.reset();
  $("#live").hidden = true;
  invoke("pane_collapse").catch(() => {});
  drawBoardNow();
  if (S.termHost) window.SBReview?.termWait(S.termHost);
  if (was) $(`.pane[data-id="${CSS.escape(was)}"]`)?.focus();
}

let renaming = false;
function drawLiveTitle() {
  const p = S.live && paneById(S.live);
  if (!p || renaming) return;
  const state = p.lamp === "none" ? p.kind : LABEL[p.lamp] || p.lamp;
  const nvimBtn = window.SBParts?.toggles(p) ?? (p.kind !== "nvim" && S.settings.nvim ? `<button class="tool" data-nvim>nvim</button>` : "");
  const reviewBtn = p.kind === "claude" && !S.termHost ? `<button class="tool" data-lreview title="Review the commits that are not pushed">review</button>` : "";
  $("#livetitle").innerHTML = `<span class="name">${esc(paneName(p))}</span><span class="state" style="color:var(--muted)">${esc(state)}</span>
    ${taskLine(p)}<span class="psum meta" style="border:0;padding:0;font:500 11px/1 var(--mono);color:var(--muted)">${esc(meta(p))}</span>${actions(p).replace('class="tact"', 'class="tact" style="margin:0"')}
    ${reviewBtn}${nvimBtn}<button class="tool" data-collapse aria-label="Collapse">⤡</button>`;
}
$("#livetitle").addEventListener("click", (e) => {
  const b = e.target.closest("button");
  const p = paneById(S.live);
  if (!b || !p) return;
  if (b.dataset.collapse !== undefined) return collapse();
  if (b.dataset.nvim !== undefined) return openNvim(p.tree);
  if (b.dataset.ptoggle) return window.SBParts?.toggle(b.dataset.ptoggle, p.id);
  if (b.dataset.lreview !== undefined) return openLocalReview(p.id);
  if (b.dataset.ptask) return window.SBTasks?.open(b.dataset.ptask);
  if (b.dataset.allow) return call("permit_answer", { permit: b.dataset.allow, allow: true });
  if (b.dataset.deny) return call("permit_answer", { permit: b.dataset.deny, allow: false });
  if (b.dataset.trust) return call("trust_answer", { id: p.id, yes: b.dataset.trust === "1" });
  if (b.dataset.switch !== undefined) return call("connection_resume", { id: p.id });
  if (b.dataset.resume !== undefined) return call("pane_resume", { id: p.id });
});

// A review of the pane's unpushed commits, in its own tab (SPEC 11.11).
async function openLocalReview(pane) {
  try {
    const id = await invoke("review_open_local", { pane });
    await refreshReviews();
    setTab(id);
  } catch (e) {
    toast(e);
  }
}

// nvim in a worktree, at a file and line when given. With an agent in the worktree,
// nvim opens next to it (SPEC 28). Else it gets its own tile.
async function openNvim(tree, file, line) {
  const id = await call("nvim_open", { tree, file: file ?? null, line: line ?? null });
  await refreshPanes();
  const agent = S.panes.find((p) => p.kind === "claude" && p.tree === tree);
  if (agent && window.SBParts) return SBParts.show("nvim", agent.id);
  expand(id);
}

function nextUnseen(dir) {
  const list = S.panes.filter((p) => p.unseen && p.id !== S.live).sort((a, b) => URGENT.indexOf(a.lamp) - URGENT.indexOf(b.lamp));
  if (!list.length) return;
  const p = dir > 0 ? list[0] : list[list.length - 1];
  // A review's Claude pane has no tile: go to its review tab.
  const r = askOf(p);
  if (r) return setTab(r.id);
  expand(p.id);
}

function moveFocus(d) {
  const ids = tiles().map((p) => p.id);
  if (!ids.length) return;
  const i = Math.max(0, ids.indexOf(S.focus));
  S.focus = ids[(i + d + ids.length) % ids.length];
  drawBoard();
  $(`.pane[data-id="${CSS.escape(S.focus)}"]`)?.focus();
}

// ---------- keys (SPEC 7) ----------

let leader = 0;
// The leader from the config, for example "ctrl+space", "ctrl+a" or "cmd+;".
// On macOS ctrl+space can switch the input language, so set another key there.
function leaderKey() {
  const parts = String(S.settings?.leader || "ctrl+space").toLowerCase().split("+");
  const key = parts.pop();
  const has = (m) => parts.includes(m);
  const code = key === "space" ? "Space" : key === ";" ? "Semicolon" : key === "," ? "Comma" : key === "." ? "Period" : /^[a-z]$/.test(key) ? `Key${key.toUpperCase()}` : /^[0-9]$/.test(key) ? `Digit${key}` : key;
  return { code, ctrl: has("ctrl"), meta: has("cmd") || has("meta") || has("super"), alt: has("alt") || has("option"), shift: has("shift") };
}
function isLeader(e) {
  const k = leaderKey();
  return e.code === k.code && e.ctrlKey === k.ctrl && e.metaKey === k.meta && e.altKey === k.alt && e.shiftKey === k.shift;
}
window.addEventListener("keydown", (e) => {
  if (isLeader(e)) {
    e.preventDefault();
    e.stopPropagation();
    leader = Date.now();
    $("#leader").hidden = false;
    return;
  }
  if (!leader) return;
  if (["Shift", "Control", "Alt", "Meta"].includes(e.key)) return;
  const fresh = Date.now() - leader < 1500;
  leader = 0;
  $("#leader").hidden = true;
  if (!fresh) return;
  e.preventDefault();
  e.stopPropagation();
  const focused = document.activeElement?.closest?.(".pane")?.dataset.id || S.focus || S.live;
  switch (e.key) {
    case "x":
      if (S.live && (!focused || focused === S.live) && window.SBParts?.unzoom()) return;
      return S.live && (!focused || focused === S.live) ? collapse() : focused && expand(focused);
    case "j": return nextUnseen(1);
    case "k": return nextUnseen(-1);
    case "J": case "L": return moveFocus(1);
    case "K": case "H": return moveFocus(-1);
    case "e": case "s": {
      // In a review tab, nvim and the agent share the right column.
      if (e.key === "e" && S.reviews.has(S.tab)) return window.SBReview?.toggleNvim(S.tab);
      // An agent shows nvim or its shell as a part (SPEC 28). Another pane opens nvim.
      const p = paneById(focused);
      const kind = e.key === "e" ? "nvim" : "term";
      if (p?.kind === "claude" && window.SBParts) return SBParts.toggle(kind, p.id);
      return p && kind === "nvim" && openNvim(p.tree);
    }
    case "o": return window.SBParts?.cycle();
    case "h": return window.SBParts?.side(-1);
    case "l": return window.SBParts?.side(1);
    case "z": return window.SBParts?.zoom();
    case "n": return openLauncher();
    case "b": return setTab("board");
    case "t": return setTab("tasks");
    case "r": { const ids = [...S.reviews.keys()]; return ids.length && setTab(ids[(ids.indexOf(S.tab) + 1) % ids.length]); }
    case "a": return cycleConnection();
    case "<": return nudgeTile(-1);
    case ">": return nudgeTile(1);
  }
}, true);

// ---------- connections (SPEC 8) ----------

function drawConn() {
  const active = S.conn.list.find((c) => c.name === S.conn.active);
  const atLimit = active?.limit_until && active.limit_until * 1000 > Date.now();
  const btn = $("#conn");
  btn.textContent = S.conn.active;
  btn.classList.toggle("limit", !!atLimit);
  btn.hidden = S.conn.list.length < 2 && S.conn.active === "default";
  $("#connmenu").innerHTML = S.conn.list
    .map((c) => {
      const at = c.limit_until && c.limit_until * 1000 > Date.now() ? `<span class="at">limit ${new Date(c.limit_until * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</span>` : `<span class="at" style="color:var(--muted)">${esc(c.kind)}</span>`;
      return `<button role="menuitemradio" aria-checked="${c.name === S.conn.active}" data-conn="${esc(c.name)}">${esc(c.name)}${at}</button>`;
    })
    .join("");
}
$("#conn").addEventListener("click", () => ($("#connmenu").hidden = !$("#connmenu").hidden));
$("#connmenu").addEventListener("click", (e) => {
  const b = e.target.closest("[data-conn]");
  if (!b) return;
  $("#connmenu").hidden = true;
  call("connection_set", { name: b.dataset.conn });
});
document.addEventListener("click", (e) => { if (!e.target.closest(".conn")) $("#connmenu").hidden = true; });
function cycleConnection() {
  const names = S.conn.list.map((c) => c.name);
  if (names.length < 2) return;
  call("connection_set", { name: names[(names.indexOf(S.conn.active) + 1) % names.length] });
}

// ---------- launcher (SPEC 9) ----------

// `prior`: the last closed round of the typed PR. `prSel` 0 starts the next round, 1 a full review.
// `task`: the plan of Start agent from a task (SPEC 27.6), or null.
const L = { list: [], sel: 0, where: "main", run: "claude", pr: null, prior: null, priorUrl: null, prSel: 0, task: null };
const fuzzy = (q, s) => { let j = 0; for (const c of s.toLowerCase()) if (c === q[j]) j++; return j === q.length; };
const setSeg = (id, v) => document.querySelectorAll(`#${id} button`).forEach((b) => b.setAttribute("aria-pressed", b.dataset.v === v));
const agentIn = (tree) => S.panes.some((p) => p.kind === "claude" && p.tree === tree && p.lamp !== "ended");

function filterRepos() {
  const q = $("#lq").value.trim();
  L.pr = /github\.com\/[^/]+\/[^/]+\/pull\/\d+/.test(q) ? q : null;
  if (L.pr !== L.priorUrl) {
    L.priorUrl = L.pr;
    L.prior = null;
    L.prSel = 0;
    const url = L.pr;
    if (url) invoke("review_prior", { url }).then((p) => { if (L.priorUrl === url) { L.prior = p; drawLauncher(); } }).catch(() => {});
  }
  L.list = L.pr ? [] : S.repos.filter((r) => fuzzy(q.toLowerCase(), r.name));
  L.sel = Math.min(L.sel, Math.max(0, L.list.length - 1));
  return q;
}

function drawLauncher() {
  const q = filterRepos();
  const open = (r) => S.panes.filter((p) => p.tree === r.path || r.worktrees.some((w) => w.path === p.tree)).length;
  const num = esc(q.split("/pull/")[1]?.split(/[/#?]/)[0] || "");
  const p = L.prior;
  $("#lrepos").innerHTML = L.pr && p
    ? `<li data-pr="0" aria-selected="${L.prSel === 0}"><span class="rn">Round ${p.round + 1} of #${num}</span><span class="rp">The changes since ${esc(p.head.slice(0, 9))}, the head of round ${p.round}${p.sent ? ` · you sent ${p.sent} review${p.sent > 1 ? "s" : ""}` : ""}</span></li>
       <li data-pr="1" aria-selected="${L.prSel === 1}"><span class="rn">New full review of #${num}</span><span class="rp">${esc(q)}</span></li>`
    : L.pr
    ? `<li aria-selected="true"><span class="rn">Review ${num}</span><span class="rp">${esc(q)}</span></li>`
    : L.list.length
      ? L.list.map((r, i) => `<li role="option" data-i="${i}" aria-selected="${i === L.sel}"><span class="rn">${esc(r.name)}</span><span class="rp">${esc(r.path)} · ${esc(r.branch)}</span>
          <span class="rb">${r.dirty ? '<span class="badge dirty">dirty</span>' : ""}${open(r) ? `<span class="badge th">${open(r)} open</span>` : ""}</span></li>`).join("")
      : `<li class="empty">${S.repos.length ? "No match" : "Loading"}</li>`;
  const r = L.list[L.sel];
  $("#lrows").hidden = !!L.pr || !r;
  if (r) {
    $('#lwhere [data-v="tree"]').hidden = !r.worktrees.length;
    if (L.where === "tree" && !r.worktrees.length) L.where = "main";
    $("#ltree").innerHTML = r.worktrees.map((w) => `<option value="${esc(w.path)}">${esc(w.path.split("/").pop())} · ${esc(w.branch)}</option>`).join("");
  }
  $("#lbranch").hidden = L.where !== "new";
  $("#ltree").hidden = L.where !== "tree";
  setSeg("lwhere", L.where);
  setSeg("lrun", L.run);
  $("#lsave").hidden = !!L.pr || !!L.task;
  $("#ltask").hidden = !L.task || !!L.pr;
  $("#llayouts").hidden = !!L.task;
  $("#lfirst").hidden = L.run !== "claude";
}

function pickRepo(i) {
  filterRepos();
  L.sel = Math.max(0, Math.min(i, L.list.length - 1));
  const r = L.list[L.sel];
  L.where = r && (r.dirty || agentIn(r.path)) ? "new" : "main";
  $("#lerr").textContent = r && agentIn(r.path) ? "An agent already runs in the main checkout." : "";
  drawLauncher();
}

// With `task` (a Start plan), the launcher opens over the Tasks tab, filled in: the
// task's repo, a new worktree with the plan's branch, Claude Code, the first message.
async function openLauncher(task = null) {
  if (!task && S.tab !== "board") setTab("board");
  L.task = task;
  $("#launcher").hidden = false;
  const repoName = task?.repo ? task.repo.split("/").filter(Boolean).pop() : "";
  $("#lq").value = repoName;
  $("#lbranch").value = task?.branch || "";
  $("#lerr").textContent = "";
  if (task) {
    $("#ltasktitle").textContent = task.title;
    $("#lfirst").value = task.first;
    L.run = "claude";
  }
  L.sel = 0;
  // The task's repo, exactly. A repo that is not in the list is an error, never
  // the first fuzzy match. After the list loads, this runs again only if you
  // changed nothing in the meantime.
  L.touched = false;
  L.bad = null;
  const pickTask = () => {
    if (!task || L.touched) return;
    const i = L.list.findIndex((r) => r.path === task.repo);
    pickRepo(i < 0 ? 0 : i);
    L.where = "new";
    L.bad = task.repo && i < 0 && S.repos.length ? `The task's repo ${task.repo} is not in your repo list. Add its parent folder to scan in the config, or pick a repo and change the task.` : null;
    $("#lerr").textContent = L.bad || "";
    drawLauncher();
  };
  drawLauncher();
  pickRepo(0);
  pickTask();
  $("#lq").focus();
  drawLayouts();
  S.repos = await invoke("repos").catch((e) => { $("#lerr").textContent = String(e); return S.repos; });
  if ($("#launcher").hidden) return;
  if (task) pickTask();
  else pickRepo(Math.min(L.sel, S.repos.length - 1));
}

async function drawLayouts() {
  const names = await invoke("layouts").catch(() => []);
  $("#llayouts").innerHTML = names.map((n) => `<button class="chip" data-layout="${esc(n)}">${esc(n)}</button>`).join("");
}

function closeLauncher() {
  $("#launcher").hidden = true;
  L.task = null;
  (S.live ? term : $("#newpane")).focus();
}

async function launch() {
  if (L.pr) {
    closeLauncher();
    const id = await call("review_open", { url: L.pr, roundOf: L.prior && L.prSel === 0 ? L.prior.id : null });
    await refreshReviews();
    setTab(id);
    return;
  }
  const r = L.list[L.sel];
  if (!r) return;
  const t = L.task;
  if (t && L.bad && !L.touched) { $("#lerr").textContent = L.bad; return; }
  const req = { repo: r.path, place: L.where, branch: L.where === "new" ? $("#lbranch").value.trim() : null, tree: L.where === "tree" ? $("#ltree").value : null, run: L.run,
    title: t ? t.title : null, task: t && L.run !== "nvim" ? t.task : null, first: t && L.run === "claude" ? $("#lfirst").value : null };
  if (req.place === "new" && !req.branch) { $("#lerr").textContent = "Type a branch name."; $("#lbranch").focus(); return; }
  try {
    const v = await invoke("pane_open", { req });
    closeLauncher();
    await refreshPanes();
    expand(v.id);
  } catch (e) {
    $("#lerr").textContent = String(e);
  }
}

$("#newpane").addEventListener("click", () => openLauncher());
$("#lcancel").addEventListener("click", closeLauncher);
$("#lgo").addEventListener("click", launch);
$("#launcher").addEventListener("click", (e) => { if (e.target.id === "launcher") closeLauncher(); });
$("#lq").addEventListener("input", () => { L.touched = true; L.sel = 0; pickRepo(0); });
$("#lbranch").addEventListener("input", () => (L.touched = true));
$("#lrepos").addEventListener("click", (e) => {
  const pr = e.target.closest("li[data-pr]");
  if (pr) { L.prSel = +pr.dataset.pr; drawLauncher(); return; }
  const li = e.target.closest("li[data-i]");
  if (li) { L.touched = true; pickRepo(+li.dataset.i); }
});
$("#lrepos").addEventListener("dblclick", (e) => { if (e.target.closest("li[data-i], li[data-pr]")) launch(); });
$("#lwhere").addEventListener("click", (e) => { const b = e.target.closest("button"); if (b) { L.touched = true; L.where = b.dataset.v; drawLauncher(); if (L.where === "new") $("#lbranch").focus(); } });
$("#lrun").addEventListener("click", (e) => { const b = e.target.closest("button"); if (b) { L.run = b.dataset.v; drawLauncher(); } });
$("#llayouts").addEventListener("click", async (e) => {
  const b = e.target.closest("[data-layout]");
  if (!b) return;
  closeLauncher();
  await call("layout_open", { name: b.dataset.layout });
  refreshPanes();
});
$("#lsave").addEventListener("click", async () => {
  const name = $("#lq").value.trim();
  if (!name) { $("#lerr").textContent = "Type the layout name in the search box."; $("#lq").focus(); return; }
  await call("layout_save", { name });
  $("#lerr").textContent = `Saved ${name}.`;
  drawLayouts();
});
$("#launcher").addEventListener("keydown", (e) => {
  if (e.key === "Escape") { e.preventDefault(); closeLauncher(); }
  else if (e.key === "Enter" && (e.target.matches("input, select") || e.target.id === "lgo")) { e.preventDefault(); launch(); }
  else if (e.target.id === "lq" && L.pr && L.prior && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
    e.preventDefault();
    L.prSel = e.key === "ArrowDown" ? 1 : 0;
    drawLauncher();
  }
  else if (e.target.id === "lq" && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
    e.preventDefault();
    pickRepo(Math.max(0, Math.min(L.list.length - 1, L.sel + (e.key === "ArrowDown" ? 1 : -1))));
  }
});

// ---------- data ----------

async function refreshPanes() {
  S.panes = await invoke("panes");
  if (S.live && !paneById(S.live)) collapse();
  drawBoard();
}

// The repo list again, when the window gets the focus back: a repo made in a
// terminal shows without a restart. At most once in 5 s; a scan takes about 250 ms.
let reposAt = 0;
function refreshRepos() {
  if (Date.now() - reposAt < 5000) return;
  reposAt = Date.now();
  invoke("repos").then((r) => {
    // Only a new or a gone repo redraws: a redraw resets the launcher's choice.
    const paths = (list) => list.map((x) => x.path).join("\n");
    if (paths(r) === paths(S.repos)) return;
    S.repos = r;
    if (!$("#launcher").hidden) pickRepo(L.sel);
    window.SBTasks?.repos?.();
  }).catch(() => {});
}
window.addEventListener("focus", refreshRepos);

async function refreshReviews() {
  const list = await invoke("reviews");
  S.reviews = new Map(list.map((r) => [r.id, r]));
  drawTabs();
  // The Claude panes of the reviews leave the board.
  drawBoard();
}

listen("panes", (e) => {
  S.panes = e.payload;
  if (S.live && !paneById(S.live)) collapse();
  else drawBoard();
  window.SBParts?.panes();
  window.SBReview?.panes?.();
  window.SBTasks?.panes();
});
listen("pane", (e) => {
  upsert(e.payload);
  drawBoard();
  window.SBParts?.panes();
  window.SBReview?.panes?.();
  window.SBTasks?.panes();
});
// The first message of Start agent did not go in: it goes to the clipboard.
// A click on Copy is a user gesture, so the copy works in every webview.
listen("paste_failed", (e) => window.SBTasks?.showText("The first message did not go in", `The agent was not ready: ${e.payload.why}. Copy the message and paste it into the pane yourself.`, e.payload.text));
listen("connections", (e) => { S.conn = e.payload; drawConn(); drawBoard(); });
listen("focus", (e) => refreshPanes().then(() => expand(e.payload)));
listen("tmux_exit", () => toast("The Switchboard tmux server ended. Restart the app to start it again."));
listen("link", (e) => {
  Object.assign(LINK, { remote: true, why: null, taken: false }, e.payload);
  linkChanged();
  if (!LINK.up) $("#toast").hidden = true;
});
$("#linkgo").addEventListener("click", reconnect);
listen("error", (e) => toast(e.payload.message || e.payload));
listen("review", (e) => {
  const was = S.reviews.has(e.payload.id);
  S.reviews.set(e.payload.id, e.payload);
  drawTabs();
  window.SBReview?.update(e.payload);
  if (!was) { drawTabs(); drawBoard(); }
});
listen("review_cleanup", (e) => toast(e.payload.note));
listen("review_closed", (e) => {
  S.reviews.delete(e.payload);
  if (S.tab === e.payload) setTab("board");
  else drawTabs();
});

// Copies with a hidden text field, and gives the focus back. True when it worked.
function copyText(text) {
  const back = document.activeElement;
  const ta = document.createElement("textarea");
  ta.value = text;
  document.body.appendChild(ta);
  ta.select();
  let ok = false;
  try { ok = document.execCommand("copy"); } catch (_) {}
  ta.remove();
  back?.focus?.();
  return ok;
}

window.SB = { S, LINK, invoke, call, listen, esc, toast, setTab, openNvim, openLocalReview, expand, collapse, parkLive, paneById, refreshPanes, drawBoard, openLauncher, paneName, copyText, LABEL, term, isLeader, bytes, $ };

(async function start() {
  try {
    Object.assign(LINK, await invoke("link_state"));
    // A remote hub that is not linked yet: the `link` event loads everything.
    LINK.synced = !LINK.remote || LINK.up;
    drawLink();
    if (LINK.remote && !LINK.up) return;
    S.settings = await invoke("settings");
    S.conn = await invoke("connections");
    drawConn();
    await refreshPanes();
    await refreshReviews();
    setTab("board");
    invoke("repos").then((r) => (S.repos = r)).catch(() => {});
  } catch (e) {
    toast(e);
  }
})();
