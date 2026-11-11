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
const URGENT = ["needs", "limit", "error", "turn"];

// The shape of each lamp, so a state reads without its color: needs a diamond,
// your turn a dot, working a dashed ring, limit a square, error a triangle, idle
// a ring. The color comes from the `data-lamp` around it (style.css).
const LAMP_SHAPE = {
  needs: '<path d="M5 0.5 9.5 5 5 9.5 0.5 5Z" fill="currentColor"/>',
  turn: '<circle cx="5" cy="5" r="4.2" fill="currentColor"/>',
  working: '<circle cx="5" cy="5" r="3.8" fill="none" stroke="currentColor" stroke-width="1.6" stroke-dasharray="4 2"/>',
  limit: '<rect x="1" y="1" width="8" height="8" rx="1" fill="currentColor"/>',
  error: '<path d="M5 0.8 9.5 9H0.5Z" fill="currentColor"/>',
  idle: '<circle cx="5" cy="5" r="3.8" fill="none" stroke="currentColor" stroke-width="1.4"/>',
  ended: '<circle cx="5" cy="5" r="3.8" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M2.6 7.4l4.8-4.8" stroke="currentColor" stroke-width="1.4"/>',
};
const lampIcon = (lamp) => (LAMP_SHAPE[lamp] ? `<svg class="lamp" data-shape="${lamp}" width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">${LAMP_SHAPE[lamp]}</svg>` : "");
const svg = (w, body, sw = 1.5) => `<svg width="${w}" height="${w}" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="${sw}" aria-hidden="true">${body}</svg>`;
const ICON = {
  board: svg(18, '<rect x="2" y="2" width="5" height="5" rx="1"/><rect x="9" y="2" width="5" height="5" rx="1"/><rect x="2" y="9" width="5" height="5" rx="1"/><rect x="9" y="9" width="5" height="5" rx="1"/>'),
  tasks: svg(18, '<path d="M2 4l1.5 1.5L6 3M2 10l1.5 1.5L6 9M8 4.5h6M8 10.5h6"/>'),
  work: svg(18, '<rect x="2" y="2.5" width="12" height="11" rx="1"/><path d="M8 2.5v11M8 8h6"/>'),
  reviews: svg(18, '<circle cx="4" cy="4" r="1.6"/><circle cx="4" cy="12" r="1.6"/><circle cx="12" cy="9" r="1.6"/><path d="M4 5.6v4.8M12 7.4V6a2 2 0 0 0-2-2H7"/>'),
  plug: svg(16, '<path d="M6 2v3M10 2v3M4.5 5h7v2.5a3.5 3.5 0 0 1-7 0zM8 11v3"/>').replace("<svg", '<svg class="plug"'),
  caret: '<svg class="caret" width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><path d="M2 3.5 5 6.5 8 3.5"/></svg>',
  collapse: '<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><path d="M5 2v3H2M9 2v3h3M5 12V9H2M9 12V9h3"/></svg>',
  close: '<svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true"><path d="M2.5 2.5l7 7M9.5 2.5l-7 7"/></svg>',
};
const gb = (b) => `${(b / 1073741824).toFixed(1)} GB`;
const home = (path) => String(path || "").replace(/^\/home\/[^/]+|^\/Users\/[^/]+/, "~");

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
// A pane that a tab hosts (the work agent, a review's Ask pane or nvim) opens in
// that tab again, never on the board, and the tab stays.
async function resync() {
  const live = S.live;
  const host = S.termHost;
  if (live) collapse();
  try {
    S.settings = await invoke("settings");
    S.conn = await invoke("connections");
    drawConn();
    await refreshPanes();
    await refreshReviews();
    window.SBTasks?.refresh();
    if (!live || !paneById(live)) return;
    if (!host) return expand(live);
    if (S.tab !== host) return;
    // The work tab opens its agent again with its parts.
    if (host === "work" && workOf(paneById(live))) return window.SBWork?.go(paneById(live));
    // The tab can be redrawn meanwhile: take its host element only now.
    const el = host === "work" ? $("#whost") : $("#rterm");
    if (el) expand(live, el);
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
  $("#view-work").hidden = tab !== "work";
  $("#view-review").hidden = tab === "board" || tab === "tasks" || tab === "work";
  // The rail has names on the Board, and only icons on the other screens.
  document.body.classList.toggle("rail-wide", tab === "board");
  drawTabs();
  if (tab === "board") {
    // The pane of the tab before (work, Ask) has no tile: it is no focus here.
    if (S.focus && !tiles().some((p) => p.id === S.focus)) S.focus = null;
    const back = S.boardLive;
    if (!quiet && back && !S.live && paneById(back)) expand(back);
    else requestAnimationFrame(fitLive);
  } else if (tab === "tasks") window.SBTasks?.draw();
  else if (tab === "work") window.SBWork?.draw();
  else window.SBReview?.draw(tab);
  drawInspFrame();
}

const reviewBusy = (r) => ["fetching", "writing", "updating"].includes(r.status);
const reviewName = (r) => (r.kind === "local" ? `${r.repo} commits` : `#${r.number}${r.round > 1 ? ` R${r.round}` : ""}`);
const reviewLabel = (r) => esc(reviewName(r));
// Writes the HTML only when it changed. A review sends many updates while it
// works: a full redraw would replace the menu item under the mouse, and a click in
// that moment lands outside the menu and closes it.
function setHTML(el, html) {
  if (el._html === html) return;
  el._html = html;
  el.innerHTML = html;
}
// The rail (SPEC 33): Board, Tasks, Work and Reviews. It has names on the Board
// and only icons elsewhere. Reviews opens a menu of the open reviews and New
// review…; on the wide rail the open reviews also show under it. A count or a
// badge shows what waits, and a dot shows while a review works. The rail is
// built once and then changed in place.
function railItem(tab, label) {
  return `<button class="nav" type="button" data-tab="${tab}">${ICON[tab]}<span class="nl">${label}</span><span class="nc"></span><span class="rbadge"></span></button>`;
}
// The count, the badge and the tooltip of one rail entry.
function setRail(b, { count = "", badge = "", hot = false, tip }) {
  const nc = b.querySelector(".nc"), rb = b.querySelector(".rbadge");
  if (nc.textContent !== String(count)) nc.textContent = String(count);
  if (rb.textContent !== String(badge)) rb.textContent = String(badge);
  rb.classList.toggle("hot", hot);
  if (b.title !== tip) { b.title = tip; b.setAttribute("aria-label", tip); }
}
// The panes that wait on you, for each urgent state: unseen tiles.
function waiting() {
  const n = {};
  for (const p of S.panes) if (p.unseen && URGENT.includes(p.lamp)) n[p.lamp] = (n[p.lamp] || 0) + 1;
  return n;
}
function drawTabs() {
  if (!$("#revtab")) {
    $("#tabs").innerHTML = railItem("board", "Board") + railItem("tasks", "Tasks") + railItem("work", "Work") +
      `<div class="revtab"><button class="nav" type="button" id="revtab" aria-haspopup="menu" aria-expanded="false">${ICON.reviews}<span class="nl">Reviews</span><i class="busy" hidden></i><span class="nc"></span><span class="rbadge"></span></button>` +
      '<div class="menu revmenu" id="revmenu" role="menu" hidden></div><div class="revsubs" id="revsubs"></div></div>';
  }
  for (const t of document.querySelectorAll("#tabs .nav[data-tab]")) {
    if (S.tab === t.dataset.tab) t.setAttribute("aria-current", "page");
    else t.removeAttribute("aria-current");
  }
  const n = waiting();
  const wait = URGENT.reduce((a, s) => a + (n[s] || 0), 0);
  const hidden = window.SBParts?.hiddenCount() || 0;
  setRail($('#tabs [data-tab="board"]'), { count: tiles().length, badge: wait || "", hot: wait > 0,
    tip: `Board${wait ? ` · ${wait} wait${wait > 1 ? "" : "s"} on you` : ""}${hidden ? ` · ${hidden} part${hidden > 1 ? "s run" : " runs"} hidden` : ""} (Leader b)` });
  const tasks = window.SBTasks?.count?.() || 0;
  setRail($('#tabs [data-tab="tasks"]'), { count: tasks || "", badge: tasks || "", tip: `Tasks${tasks ? ` · ${tasks} open in the sprint` : ""} (Leader t)` });
  const repo = window.SBWork?.W.repo ? window.SBWork.W.repo.split("/").filter(Boolean).pop() : "";
  setRail($('#tabs [data-tab="work"]'), { count: repo, tip: `Work${repo ? ` · ${repo}` : ""}: nvim, Claude and a terminal in one repo (Leader w)` });
  const list = [...S.reviews.values()];
  const cur = S.reviews.get(S.tab);
  const tab = $("#revtab");
  if (cur) tab.setAttribute("aria-current", "page");
  else tab.removeAttribute("aria-current");
  setRail(tab, { count: list.length || "", badge: list.length || "", tip: `Reviews${list.length ? ` · ${list.map((r) => reviewName(r)).join(", ")}` : ""} (Leader r)` });
  tab.querySelector(".busy").hidden = !list.some(reviewBusy);
  const menu = $("#revmenu"), subs = $("#revsubs");
  const ids = list.map((r) => r.id).join("\n");
  // Another set of reviews: new items. The same set: each item changes in place.
  if (menu._ids !== ids) {
    menu._ids = ids;
    menu.innerHTML = list.map((r) => `<button role="menuitemradio" data-tab="${esc(r.id)}"></button>`).join("") +
      (list.length ? "" : '<div class="mnone">No open reviews</div>') +
      '<button role="menuitem" data-newreview>New review…</button>';
    subs.innerHTML = list.map((r) => `<button class="sub" type="button" data-tab="${esc(r.id)}"></button>`).join("");
  }
  const items = menu.querySelectorAll("[data-tab]"), rows = subs.querySelectorAll("[data-tab]");
  list.forEach((r, i) => {
    const b = items[i];
    b.setAttribute("aria-selected", String(S.tab === r.id));
    b.setAttribute("aria-checked", String(S.tab === r.id));
    setHTML(b, `<span class="rname">${reviewLabel(r)}${reviewBusy(r) ? '<i class="busy"></i>' : ""}<small>${esc(r.title || "")}</small></span><span class="at">${esc(r.status === "ready" ? "" : r.status || "")}</span>`);
    const s = rows[i];
    if (S.tab === r.id) s.setAttribute("aria-current", "page");
    else s.removeAttribute("aria-current");
    if (s.title !== (r.title || "")) s.title = r.title || "";
    setHTML(s, `<span class="sn">${reviewLabel(r)}</span>${r.status && r.status !== "ready" ? `<span class="ss">${esc(r.status)}</span>` : ""}`);
  });
}
$("#tabs").addEventListener("click", (e) => {
  if (e.target.closest("#revtab")) { $("#revmenu").hidden = !$("#revmenu").hidden; $("#revtab").setAttribute("aria-expanded", String(!$("#revmenu").hidden)); return; }
  const shut = () => { $("#revmenu").hidden = true; $("#revtab").setAttribute("aria-expanded", "false"); };
  if (e.target.closest("[data-newreview]")) { shut(); return openLauncher(); }
  const t = e.target.closest("[data-tab]");
  if (!t) return;
  shut();
  setTab(t.dataset.tab);
});
document.addEventListener("click", (e) => { if (!e.target.closest(".revtab") && $("#revmenu") && !$("#revmenu").hidden) { $("#revmenu").hidden = true; $("#revtab").setAttribute("aria-expanded", "false"); } });
document.addEventListener("keydown", (e) => { if (e.key === "Escape" && $("#revmenu") && !$("#revmenu").hidden) { $("#revmenu").hidden = true; $("#revtab").focus(); } });

// ---------- board ----------

function paneName(p) {
  if (p.title) return p.title;
  const base = p.tree.split("/").filter(Boolean).pop() || p.repo;
  if (p.kind === "nvim") return `${p.repo} · nvim`;
  return base !== p.repo && p.branch ? p.branch : p.repo;
}

// The buttons of a state. They are neutral, but Allow is the gold primary where
// the focus is (`primary`): one gold button on the screen at most.
function actions(p, primary) {
  const b = (attr, label, gold) => `<button class="btn sm${gold ? " primary" : ""}" type="button" ${attr}>${label}</button>`;
  if (p.trust) return `<div class="tact">${b(`data-trust="1"`, "Trust")}${b(`data-trust="0"`, "Exit")}</div>`;
  if (p.permit) return `<div class="tact">${b(`data-allow="${esc(p.permit.id)}"`, "Allow", primary)}${b(`data-deny="${esc(p.permit.id)}"`, "Deny")}</div>`;
  if (p.lamp === "limit") {
    const next = nextConnection(p.connection);
    return next ? `<div class="tact">${b("data-switch", `Switch to ${esc(next)}`)}</div>` : "";
  }
  if ((p.lamp === "ended" || p.lamp === "error") && p.session && p.kind === "claude") return `<div class="tact">${b("data-resume", "Resume")}</div>`;
  return "";
}

function nextConnection(current) {
  const free = S.conn.list.filter((c) => c.name !== current && !(c.limit_until && c.limit_until * 1000 > Date.now()));
  return free[0]?.name;
}

// The review whose Claude pane this is (SPEC 11.5), or none. That pane shows in
// its review tab, not on the board.
const askOf = (p) => p.kind === "claude" && [...S.reviews.values()].find((r) => r.tree === p.tree && p.title === `Ask #${r.number}`);

// The Claude pane of the Work environment tab (SPEC 30). It has no tile: the board
// and the work tab never share an agent.
const WORK_TITLE = "Work environment";
const workOf = (p) => p.kind === "claude" && p.title === WORK_TITLE;

// The tiles on the board. The nvim and shell of an agent are its parts (SPEC 28).
// A part that is the live pane itself keeps its tile, as every live pane on the
// board does. A pane that shows in another tab (a review, the work tab) has none.
const tiles = () => S.panes.filter((p) => (p.id === S.live && !S.termHost) || (!window.SBParts?.isPart(p) && !askOf(p) && !workOf(p)));

// The facts of a pane, for the live title: repo and branch. The inspector has the rest.
const metaShort = (p) => [p.repo, p.branch].filter(Boolean).join(" · ");
const stateWord = (p) => (p.lamp === "none" ? p.kind : LABEL[p.lamp] || p.lamp);
// A summary that names a tool or an error reads as code.
const CODEISH = /^[A-Z][A-Za-z]+(\(|: )|^API Error/;

// A tile is built once and then changed in place (see drawBoardNow). A tile shows
// the name, the state, one summary line, the last two lines of the pane, the nvim
// and term chips, the task chip and the buttons of the state. The rest is in the
// inspector.
function tile(p) {
  const live = p.id === S.live && !S.termHost;
  const chips = p.kind === "claude" && window.SBParts ? SBParts.chips(p) : p.kind !== "nvim" && S.settings.nvim ? `<button class="chip" type="button" data-nvim title="Open nvim in this worktree (Leader e)">nvim</button>` : "";
  return {
    cls: ["pane", p.unseen && "unseen", S.focus === p.id && "focused", p.id === S.live && "is-live"].filter(Boolean).join(" "),
    title: `<i class="udot" title="Unseen"></i><span class="name" title="${esc(paneName(p))}">${esc(paneName(p))}</span><span class="state">${live ? "live" : lampIcon(p.lamp) + esc(stateWord(p))}</span>`,
    foot: `${chips}${taskLine(p)}<span class="sp"></span>${actions(p, S.focus === p.id && !live)}`,
    text: p.summary || "",
    tail: p.tail.map((l) => strip(l).trimEnd()).filter(Boolean).slice(-2).join("\n"),
  };
}

// Changes a tile element to show `t`, the parts of `tile(p)`. The summary and the
// tail change as text, so a button under the mouse stays.
function fillTile(el, p, t) {
  if (el.className !== t.cls) el.className = t.cls;
  if (el.dataset.lamp !== p.lamp) el.dataset.lamp = p.lamp;
  const [head, sum, tail, foot] = el.children;
  setHTML(head, t.title);
  if (sum.textContent !== t.text) sum.textContent = t.text;
  sum.classList.toggle("mono", CODEISH.test(t.text));
  if (tail.textContent !== t.tail) tail.textContent = t.tail;
  setHTML(foot, t.foot);
}

// The task of a pane, as a chip (SPEC 27.6). A click opens the task.
function taskLine(p) {
  const t = window.SBTasks?.forPane(p.id);
  return t ? `<button class="chip ptask" type="button" data-ptask="${esc(t.id)}" title="Open the task: ${esc(t.title)}">▸ ${esc(t.title)}</button>` : "";
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
  // Each tile element stays, and only what changed is written: a pane event comes
  // about once a second, and a new element under the mouse drops the click.
  const board = $("#board");
  const list = tiles();
  const old = new Map([...board.children].map((el) => [el.dataset.id, el]));
  const want = new Set(list.map((p) => p.id));
  for (const [id, el] of old) if (!want.has(id)) el.remove();
  list.forEach((p, i) => {
    let el = old.get(p.id);
    if (!el) {
      el = document.createElement("div");
      el.dataset.id = p.id;
      el.tabIndex = 0;
      el.innerHTML = '<div class="ptitle"></div><div class="psum"></div><pre class="term-tail"></pre><div class="pfoot"></div>';
    }
    fillTile(el, p, tile(p));
    if (board.children[i] !== el) board.insertBefore(el, board.children[i] || null);
  });
  $("#view-board").classList.toggle("has-live", !!S.live);
  $("#bgutter").hidden = !S.live || S.termHost !== null;
  drawTally();
  drawLiveTitle();
  drawTabs();
  drawInspNow();
}

// The board header: a chip for each state that waits on you, with its count. A
// click goes to the next tile in that state.
const WAIT_WORD = { needs: (k) => (k > 1 ? "need you" : "needs you"), limit: () => "limit", error: (k) => (k > 1 ? "errors" : "error"), turn: () => "your turn" };
function drawTally() {
  const n = waiting();
  setHTML($("#tally"), URGENT.filter((s) => n[s])
    .map((s) => `<button class="schip" type="button" data-lamp="${s}" data-jump="${s}" title="The next tile at ${LABEL[s]}">${lampIcon(s)}${n[s]} ${WAIT_WORD[s](n[s])}</button>`)
    .join(""));
  $("#bnext").disabled = !S.panes.some((p) => p.unseen && p.id !== S.live);
}
$("#tally").addEventListener("click", (e) => { const b = e.target.closest("[data-jump]"); if (b) nextOf(b.dataset.jump); });
$("#bnext").addEventListener("click", () => nextUnseen(1));

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
  $("#board .dragging")?.classList.remove("dragging");
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

// Keys on a focused tile: Enter opens it, Delete closes its pane.
$("#board").addEventListener("keydown", (e) => {
  const el = e.target.closest?.(".pane");
  if (!el || e.target !== el) return;
  if (e.key === "Enter") { e.preventDefault(); el.dataset.id === S.live && S.termHost === null ? term.focus() : expand(el.dataset.id); }
  else if (e.key === "Delete") { e.preventDefault(); call("pane_close", { id: el.dataset.id }); }
});
// A tile that gets the focus (Tab, a click) is the focus of the board and of the
// inspector. Not a button in it: the redraw would take the button from under the
// mouse before the click.
$("#board").addEventListener("focusin", (e) => {
  const el = e.target.closest(".pane");
  if (!el || e.target !== el || S.focus === el.dataset.id) return;
  S.focus = el.dataset.id;
  drawBoard();
});

// Rename: double-click the live title, or Rename in the inspector. `el` shows the
// name and becomes a text field until Enter, Escape or a click away.
function startRename(el, id) {
  if (!el || !paneById(id)) return;
  // The Work environment tab finds its agent by its title (SPEC 30): it keeps it.
  if (workOf(paneById(id))) return toast(`The ${WORK_TITLE} agent keeps its name.`);
  renaming = true;
  el.textContent = paneName(paneById(id));
  el.contentEditable = "true";
  el.focus();
  document.getSelection().selectAllChildren(el);
  const done = (save) => {
    if (!renaming) return;
    renaming = false;
    el.contentEditable = "false";
    // The name was edited by hand: the next draw writes the titles again.
    $("#livetitle")._html = null;
    INSP.drawn = null;
    const title = el.textContent.trim();
    if (save && title === WORK_TITLE) {
      toast(`"${WORK_TITLE}" is the name of the agent of the ${WORK_TITLE} tab. Pick another name.`);
      drawBoard();
    } else if (save) call("pane_rename", { id, title: title || null });
    else drawBoard();
  };
  el.onkeydown = (k) => {
    if (k.key === "Enter") { k.preventDefault(); done(true); }
    if (k.key === "Escape") { k.preventDefault(); done(false); }
    k.stopPropagation();
  };
  el.onblur = () => done(true);
}
document.addEventListener("dblclick", (e) => {
  // Rename works on the live title. A click on a tile expands it first.
  const name = e.target.closest("#livetitle .name");
  if (!name || !S.live) return;
  e.preventDefault();
  startRename(name, S.live);
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
// The vendored mono font may load after the terminal measured its cell: measure
// again then. A change of the font family makes xterm.js measure.
const remeasure = (t) => document.fonts?.load('13px "JetBrains Mono"').then(() => {
  const f = t.options.fontFamily;
  t.options.fontFamily = "monospace";
  t.options.fontFamily = f;
}).catch(() => {});
remeasure(term)?.then(() => fitSoon());
term.attachCustomKeyEventHandler((e) => !isLeader(e));
// Web links in a terminal open in your browser (`open_url`): a plain http(s) URL
// in the text, and an OSC 8 link that a program marks. xterm.js would call
// window.open for an OSC 8 link, and the window drops that.
const URL_RE = /https?:\/\/[^\s"'<>`]+/g;
function openWeb(url) {
  invoke("open_url", { url }).catch((e) => toast(e));
}
function webLinks(t) {
  t.options.linkHandler = { activate: (_e, uri) => { if (/^https?:\/\//.test(uri)) openWeb(uri); }, allowNonHttpProtocols: false };
  const provider = {
    provideLinks(y, cb) {
      const line = t.buffer.active.getLine(y - 1);
      if (!line) return cb(undefined);
      // The text of the row, and the column of each of its chars (wide chars take two).
      let text = "";
      const cols = [];
      for (let x = 0; x < line.length; x++) {
        const c = line.getCell(x);
        if (!c || c.getWidth() === 0) continue;
        const ch = c.getChars() || " ";
        for (let k = 0; k < ch.length; k++) cols.push(x + 1);
        text += ch;
      }
      const links = [];
      for (const m of text.matchAll(URL_RE)) {
        // A dot or a bracket at the end is the sentence's, not the URL's.
        const url = m[0].replace(/[.,;:!?)\]}>]+$/, "");
        if (url.length < 10) continue;
        links.push({
          range: { start: { x: cols[m.index], y }, end: { x: cols[m.index + url.length - 1], y } },
          text: url,
          decorations: { underline: true, pointerCursor: true },
          activate: () => openWeb(url),
        });
      }
      cb(links.length ? links : undefined);
    },
  };
  t.registerLinkProvider(provider);
  return provider;
}
webLinks(term);
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
// The room under the board header: the live pane and the strip share it.
const stripRoom = () => $("#view-board").getBoundingClientRect().height - $("#bhead").getBoundingClientRect().height - 24;
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

// Puts the live section back on the board, under the board's header: the grid
// gives the second row to the live pane.
function parkLive() {
  const live = $("#live");
  live.classList.remove("work");
  if (live.parentElement !== $("#view-board")) $("#bhead").after(live);
}

// Shows a pane in the live terminal. With `host`, the terminal moves into that
// element of the current review tab; without it, the board shows.
async function expand(id, host) {
  log("info", "expand", id, host ? "in review" : "");
  if (!paneById(id)) return;
  // A work tab agent shows in its own tab, never on the board.
  if (!host && workOf(paneById(id))) return window.SBWork?.go(paneById(id));
  if (host) {
    // A host of a tab that no longer shows: do nothing.
    if (S.tab === "board" || !host.isConnected) return;
    window.SBParts?.reset();
    S.termHost = S.tab;
    if ($("#live").parentElement !== host) host.appendChild($("#live"));
    // The work tab turns the parts around: nvim on the left (SPEC 30).
    $("#live").classList.toggle("work", S.tab === "work");
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
    if (!host || S.termHost === "work") window.SBParts?.afterExpand(id);
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
  if (S.termHost === "work") window.SBWork?.collapsed();
  else if (S.termHost) window.SBReview?.termWait(S.termHost);
  if (was) $(`.pane[data-id="${CSS.escape(was)}"]`)?.focus();
}

let renaming = false;
// The title line of the live pane: the name, the state, the repo and branch, the
// task chip, the buttons of the state, Review, the nvim and term switches, Collapse.
function drawLiveTitle() {
  const p = S.live && paneById(S.live);
  if (!p || renaming) return;
  const lt = $("#livetitle");
  if (lt.dataset.lamp !== p.lamp) lt.dataset.lamp = p.lamp;
  const nvimBtn = window.SBParts?.toggles(p) ?? (p.kind !== "nvim" && S.settings.nvim ? `<button class="chip" type="button" data-nvim title="Open nvim in this worktree (Leader e)">nvim</button>` : "");
  const reviewBtn = p.kind === "claude" && !S.termHost ? `<button class="btn sm" type="button" data-lreview title="Review the commits that are not pushed">Review</button>` : "";
  setHTML(lt, `<span class="name" title="Double-click to rename">${esc(paneName(p))}</span><span class="state">${lampIcon(p.lamp)}${esc(stateWord(p))}</span>`
    + `<span class="lmeta">${esc(metaShort(p))}</span>${taskLine(p)}${actions(p, S.focus === p.id)}<span class="sp"></span>`
    + `${reviewBtn}${nvimBtn}<button class="icon" type="button" data-collapse aria-label="Collapse (Leader x)" title="Collapse (Leader x)">${ICON.collapse}</button>`);
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
  stateAct(b, p);
});
// A button of `actions(p)`: true when `b` was one.
function stateAct(b, p) {
  if (b.dataset.allow) call("permit_answer", { permit: b.dataset.allow, allow: true });
  else if (b.dataset.deny) call("permit_answer", { permit: b.dataset.deny, allow: false });
  else if (b.dataset.trust) call("trust_answer", { id: p.id, yes: b.dataset.trust === "1" });
  else if (b.dataset.switch !== undefined) call("connection_resume", { id: p.id });
  else if (b.dataset.resume !== undefined) call("pane_resume", { id: p.id });
  else return false;
  return true;
}

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
  // A board agent that runs: not the work agent, a review's Ask pane or an ended one.
  const agent = S.panes.find((p) => p.kind === "claude" && p.tree === tree && p.lamp !== "ended" && !workOf(p) && !askOf(p));
  if (agent && window.SBParts) return SBParts.show("nvim", agent.id);
  expand(id);
}

// The most urgent state first. A state that is not urgent (an ended pane) comes last.
const urgency = (p) => { const i = URGENT.indexOf(p.lamp); return i < 0 ? URGENT.length : i; };
function nextUnseen(dir) {
  const list = S.panes.filter((p) => p.unseen && p.id !== S.live).sort((a, b) => urgency(a) - urgency(b));
  if (!list.length) return;
  goPane(dir > 0 ? list[0] : list[list.length - 1]);
}
// A review's Claude pane has no tile: go to its review tab.
function goPane(p) {
  const r = askOf(p);
  if (r) return setTab(r.id);
  expand(p.id);
}
// A chip of the board header: the next unseen tile in that state.
function nextOf(state) {
  const p = S.panes.find((x) => x.unseen && x.lamp === state && x.id !== S.live);
  if (p) goPane(p);
}

function moveFocus(d) {
  const ids = tiles().map((p) => p.id);
  if (!ids.length) return;
  const i = Math.max(0, ids.indexOf(S.focus));
  S.focus = ids[(i + d + ids.length) % ids.length];
  drawBoard();
  $(`.pane[data-id="${CSS.escape(S.focus)}"]`)?.focus();
}

// ---------- the inspector (SPEC 33) ----------

// A right panel of 280 px. Leader i or the toggle at the right of the header opens
// and closes it. Each screen (board, tasks, work, review, local) remembers open or
// closed in the window's local storage, a convenience only.
// A screen gives the content with `SBInsp.provide(screen, fn)`: `fn(tab)` returns
//   { title, display, empty, alert: { text, buttons }, facts: [[label, value, opts]],
//     actions: [{ label, act, key, title, disabled, primary, attr }], body, danger: [...],
//     onAct(act, el), onDraw(box) }
// and the panel shows, in order: the alert, the facts, the repeat actions, the
// body, a spacer, and the destructive actions. A value in `facts` is text; with
// opts.act it is a link-like button. `body` is HTML that the screen escapes itself;
// a button in it with `data-iact` calls `onAct` too. `onDraw` runs after each draw.
const INSP = { providers: {}, open: {}, spec: null, drawn: null, queued: false };
// A review of local work is a screen of its own: its controls live in the
// inspector, so it starts open there (SPEC 11.11). Every other screen starts closed.
const screenOf = (tab) => (tab === "board" || tab === "tasks" || tab === "work" ? tab : S.reviews.get(tab)?.kind === "local" ? "local" : "review");
function inspIsOpen(screen = screenOf(S.tab)) {
  if (!(screen in INSP.open)) {
    try {
      const v = localStorage.getItem(`sb.insp.${screen}`);
      INSP.open[screen] = v === null ? screen === "local" : v === "1";
    } catch (_) { INSP.open[screen] = screen === "local"; }
  }
  return INSP.open[screen];
}
function setInsp(open) {
  const screen = screenOf(S.tab);
  if (!INSP.providers[screen]) return;
  INSP.open[screen] = !!open;
  try { localStorage.setItem(`sb.insp.${screen}`, open ? "1" : "0"); } catch (_) { /* a convenience only */ }
  drawInspFrame();
}
const toggleInsp = () => setInsp(!inspIsOpen());
// The panel and the toggle for the screen that shows.
function drawInspFrame() {
  const screen = screenOf(S.tab);
  const has = !!INSP.providers[screen];
  const open = has && inspIsOpen(screen);
  $("#insp").hidden = !open;
  $("#itoggle").hidden = !has;
  $("#itoggle").setAttribute("aria-pressed", String(open));
  if (open) drawInspNow();
}
// Many events can come in one frame: draw once per frame, as the board does.
function drawInsp() {
  if (INSP.queued) return;
  INSP.queued = true;
  const go = () => { if (INSP.queued) { INSP.queued = false; drawInspNow(); } };
  requestAnimationFrame(go);
  setTimeout(go, 50);
}
function drawInspNow() {
  const box = $("#insp");
  const fn = INSP.providers[screenOf(S.tab)];
  // A name that you type stays as it is until you end the rename.
  if (box.hidden || !fn || renaming) return;
  let spec = null;
  try { spec = fn(S.tab); } catch (e) { log("warn", "inspector:", e); }
  spec = spec || { title: "Details", empty: "Nothing to show here." };
  INSP.spec = spec;
  if (!box.querySelector(".ihead")) {
    box.innerHTML = `<div class="ihead"><span class="ititle"></span><button class="icon" type="button" data-iclose aria-label="Close the details (Leader i)" title="Close the details (Leader i)">${ICON.close}</button></div>`
      + '<div class="ialert" role="status"></div><dl class="ifacts"></dl><div class="iacts"></div><div class="iextra"></div><span class="isp"></span><div class="idanger"></div>';
  }
  // Each part changes only when its HTML changes, so a button under the mouse stays.
  const title = box.querySelector(".ititle");
  const head = `${spec.display ? 1 : 0}|${spec.title || ""}`;
  if (INSP.drawn !== head) { INSP.drawn = head; title.textContent = spec.title || ""; title.classList.toggle("disp", !!spec.display); }
  const btn = (a, cls = "") => `<button class="btn${cls}${a.primary ? " primary" : ""}" type="button" data-iact="${esc(a.act)}"${a.title ? ` title="${esc(a.title)}"` : ""}${a.disabled ? " disabled" : ""}${a.attr ? ` ${a.attr}` : ""}><span>${esc(a.label)}</span>${a.key ? `<kbd>${esc(a.key)}</kbd>` : ""}</button>`;
  const al = spec.alert;
  setHTML(box.querySelector(".ialert"), al ? `<span>${esc(al.text)}</span>${al.buttons?.length ? `<div class="ibtns">${al.buttons.map((a) => btn(a, " sm")).join("")}</div>` : ""}` : "");
  setHTML(box.querySelector(".ifacts"), (spec.facts || []).filter((f) => f && f[1] != null && f[1] !== "").map(([k, v, o = {}]) =>
    `<dt>${esc(k)}</dt><dd${o.mono ? ' class="mono"' : ""}>${o.act ? `<button class="ilink" type="button" data-iact="${esc(o.act)}">${esc(v)}</button>` : esc(v)}</dd>`).join(""));
  setHTML(box.querySelector(".iacts"), (spec.actions || []).map((a) => btn(a)).join(""));
  setHTML(box.querySelector(".iextra"), spec.empty ? `<p class="iempty">${esc(spec.empty)}</p>` : spec.body || "");
  setHTML(box.querySelector(".idanger"), (spec.danger || []).map((a) => btn(a)).join(""));
  spec.onDraw?.(box);
}
$("#insp").addEventListener("click", (e) => {
  if (e.target.closest("[data-iclose]")) return setInsp(false);
  const b = e.target.closest("[data-iact]");
  if (b && !b.disabled) INSP.spec?.onAct?.(b.dataset.iact, b);
});
$("#itoggle").addEventListener("click", toggleInsp);
window.SBInsp = {
  provide: (screen, fn) => { INSP.providers[screen] = fn; if (screenOf(S.tab) === screen) drawInspFrame(); },
  // `now`: draw at once, for a change that you made in the panel.
  refresh: (now) => (now ? drawInspNow() : drawInsp()),
  open: () => setInsp(true),
  close: () => setInsp(false),
  toggle: toggleInsp,
  isOpen: () => !$("#insp").hidden,
  screen: () => screenOf(S.tab),
};

// The board's inspector: the focused tile, or the live pane.
function boardTarget() {
  const own = S.focus && (S.focus === S.live || tiles().some((p) => p.id === S.focus)) ? S.focus : null;
  return paneById(own || S.live || "") || null;
}
SBInsp.provide("board", () => {
  const p = boardTarget();
  if (!p) return { title: "Details", empty: "Focus a tile to see its details here." };
  const task = window.SBTasks?.forPane(p.id);
  const live = p.id === S.live && !S.termHost;
  // The parts of the live agent (SPEC 28): show, hide, sizes, stop.
  const parts = live && p.kind === "claude" ? window.SBParts?.inspector(p) : null;
  const actions = [];
  if (p.kind === "claude") actions.push({ label: "Review the commits not pushed", act: "lreview", title: "A review of this agent's commits that no remote has (SPEC 11.11)" });
  if (p.kind !== "nvim") actions.push({ label: task ? "Link to another task" : "Link to a task", act: "taskmenu", attr: "data-ptaskmenu" });
  actions.push({ label: "Rename", act: "rename" }, { label: "Move left", act: "left", key: "<" }, { label: "Move right", act: "right", key: ">" });
  if (parts) actions.push(...parts.actions);
  return {
    title: paneName(p),
    display: true,
    alert: parts?.alert || null,
    facts: [
      ["State", stateWord(p)],
      ["Repo", p.repo],
      ["Branch", p.branch, { mono: true }],
      ["Folder", home(p.tree), { mono: true }],
      ["Connection", p.connection && S.conn.list.length > 1 ? p.connection : null],
      ["rust-analyzer", p.rss ? gb(p.rss) : null],
      // Bash of this agent runs in Claude Code's sandbox (SPEC 31).
      ["Bash", p.sandboxed ? "sandbox" : null],
      ["Task", task ? task.title : null, { act: "task" }],
    ],
    actions,
    danger: [...(parts?.danger || []), { label: "Close pane", act: "close", title: "Close this pane (Delete on a focused tile). An agent with a session can resume later." }],
    onAct: (act, el) => {
      if (act === "lreview") return openLocalReview(p.id);
      if (act === "taskmenu") return window.SBTasks?.menu(p.id, el);
      if (act === "task") return task && window.SBTasks?.open(task.id);
      if (act === "rename") return startRename($("#insp .ititle"), p.id);
      if (act === "left" || act === "right") { S.focus = p.id; return nudgeTile(act === "left" ? -1 : 1); }
      if (act === "close") return call("pane_close", { id: p.id });
      window.SBParts?.inspAct(act, p);
    },
  };
});

// ---------- keys (SPEC 7) ----------

let leader = 0;
let leaderTimer = null;
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
    // The leader shows only while it waits for its key.
    $("#leader").hidden = false;
    clearTimeout(leaderTimer);
    leaderTimer = setTimeout(() => { leader = 0; $("#leader").hidden = true; }, 1500);
    return;
  }
  if (!leader) return;
  if (["Shift", "Control", "Alt", "Meta"].includes(e.key)) return;
  const fresh = Date.now() - leader < 1500;
  leader = 0;
  clearTimeout(leaderTimer);
  $("#leader").hidden = true;
  if (!fresh) return;
  e.preventDefault();
  e.stopPropagation();
  // S.focus counts only for a tile or the pane that shows: a work or Ask pane stays
  // in S.focus after its tab is gone.
  const own = S.focus && (S.focus === S.live || tiles().some((p) => p.id === S.focus)) ? S.focus : null;
  const focused = document.activeElement?.closest?.(".pane")?.dataset.id || own || S.live;
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
    case "i": return toggleInsp();
    case "b": return setTab("board");
    case "t": return setTab("tasks");
    case "w": return setTab("work");
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
  // An icon on the narrow rail; the name and a caret on the wide one.
  setHTML(btn, `${ICON.plug}<span class="nl">${esc(S.conn.active)}</span>${ICON.caret}`);
  const tip = `Connection · ${S.conn.active}${atLimit ? " · at its limit" : ""} (Leader a)`;
  if (btn.title !== tip) { btn.title = tip; btn.setAttribute("aria-label", tip); }
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

// ---------- settings (SPEC 14.1) ----------

// The lists of the Settings window: the key, the title, the help line, and an example.
const SET_LISTS = [
  ["scan", "Repo folders", "Switchboard looks for git repos in these folders, 3 levels deep. Repos where you ran claude show too.", "~/code"],
  ["spec_clones", "Spec clones", "Folders that the review agent reads first, such as a consensus-specs clone.", "~/code/consensus-specs"],
  ["web_domains", "Sites the review agent can fetch", "Host names only, such as github.com. Other sites are refused.", "github.com"],
];
// The lists of the sandbox part (SPEC 31). They show when the mode is not Off.
const SET_SANDBOX_LISTS = [
  ["sandbox_domains", "Sites that commands in the sandbox can reach", "Host names, such as github.com. *.example.com takes every host under example.com.", "crates.io"],
  ["sandbox_write", "More folders that commands can write", "Full paths. The tree of the agent and its git folder can always be written.", "~/.cache/build"],
  ["sandbox_unsandboxed", "Commands that run outside the sandbox", "Patterns like Bash rules, such as gh *. They ask as usual.", "docker *"],
];
const SANDBOX_MODES = [["off", "Off"], ["review", "Review panes"], ["all", "All agents"]];
const SET = { draft: null, missing: new Set(), config: "", sandboxMissing: [] };

async function openSettings() {
  const s = await invoke("settings").catch((e) => { toast(e); return null; });
  if (!s) return;
  const sb = s.sandbox || { mode: "review", domains: [], write: [], unsandboxed: [] };
  SET.draft = { scan: [...s.scan], spec_clones: [...s.spec_clones], web_domains: [...s.web_domains], web_search: !!s.web_search,
    sandbox_mode: sb.mode, sandbox_domains: [...sb.domains], sandbox_write: [...sb.write], sandbox_unsandboxed: [...sb.unsandboxed] };
  SET.hasSandbox = !!s.sandbox;
  SET.sandboxMissing = s.sandbox_missing || [];
  SET.missing = new Set(s.missing || []);
  SET.config = s.config || "";
  $("#serr").textContent = "";
  drawSettings();
  $("#settings").hidden = false;
  $("#sbody input")?.focus();
}

function closeSettings() {
  $("#settings").hidden = true;
  SET.draft = null;
}

function drawSettings() {
  const d = SET.draft;
  const group = ([key, title, help, eg]) => `<div class="sgroup"><h4>${esc(title)}</h4><p>${esc(help)}</p>${d[key].map((v, i) =>
    `<div class="srow"><input data-list="${key}" data-i="${i}" value="${esc(v)}" placeholder="${esc(eg)}" spellcheck="false" autocomplete="off" aria-label="${esc(title)} ${i + 1}">${SET.missing.has(v) ? '<span class="miss">not found</span>' : ""}<button class="btn small" type="button" data-del="${key}" data-i="${i}" aria-label="Remove">×</button></div>`).join("")}<button class="btn small" type="button" data-add="${key}">Add</button></div>`;
  $("#sbody").innerHTML = SET_LISTS.map(group).join("") +
    `<label class="scheck"><input type="checkbox" id="swebsearch"${d.web_search ? " checked" : ""}> Let the review agent search the web</label>` +
    sandboxSettings(d, group) +
    `<div class="sfile">Saved in ${esc(SET.config)}. A change works at once, with no restart.</div>`;
}

// The sandbox part of Settings (SPEC 31): the mode, a warning when Linux lacks
// the tools, and the lists when the mode is not Off.
function sandboxSettings(d, group) {
  if (!SET.hasSandbox) return "";
  const opts = SANDBOX_MODES.map(([v, t]) => `<option value="${v}"${d.sandbox_mode === v ? " selected" : ""}>${t}</option>`).join("");
  const warn = SET.sandboxMissing.length && d.sandbox_mode !== "off"
    ? `<p class="swarn" id="ssandboxwarn">Not on PATH: ${esc(SET.sandboxMissing.join(", "))}. Install bubblewrap and socat, or a sandboxed agent stops at start.</p>` : "";
  return `<div class="sgroup"><h4>Sandbox</h4><p>Bash of these agents runs in Claude Code's sandbox: it writes only its tree, and reaches only the sites below. A new agent gets the change.</p>` +
    `<select id="ssandbox" aria-label="Sandbox mode">${opts}</select>${warn}</div>` +
    (d.sandbox_mode === "off" ? "" : SET_SANDBOX_LISTS.map(group).join(""));
}

// The fields that Save sends: the sandbox lists go in one table.
function settingsToSend(d) {
  const out = { scan: d.scan, spec_clones: d.spec_clones, web_domains: d.web_domains, web_search: d.web_search };
  if (SET.hasSandbox) out.sandbox = { mode: d.sandbox_mode, domains: d.sandbox_domains, write: d.sandbox_write, unsandboxed: d.sandbox_unsandboxed };
  return out;
}

$("#gear").addEventListener("click", openSettings);
$("#scancel").addEventListener("click", closeSettings);
$("#settings").addEventListener("click", (e) => { if (e.target === $("#settings")) closeSettings(); });
$("#settings").addEventListener("keydown", (e) => { if (e.key === "Escape") { e.preventDefault(); closeSettings(); } });
$("#sbody").addEventListener("input", (e) => {
  const t = e.target;
  if (t.dataset.list) SET.draft[t.dataset.list][+t.dataset.i] = t.value;
  else if (t.id === "swebsearch") SET.draft.web_search = t.checked;
});
$("#sbody").addEventListener("change", (e) => {
  if (e.target.id === "swebsearch") SET.draft.web_search = e.target.checked;
  else if (e.target.id === "ssandbox") {
    SET.draft.sandbox_mode = e.target.value;
    drawSettings();
  }
});
$("#sbody").addEventListener("click", (e) => {
  const add = e.target.closest("[data-add]"), del = e.target.closest("[data-del]");
  if (add) {
    SET.draft[add.dataset.add].push("");
    drawSettings();
    const rows = document.querySelectorAll(`#sbody input[data-list="${add.dataset.add}"]`);
    rows[rows.length - 1]?.focus();
  } else if (del) {
    SET.draft[del.dataset.del].splice(+del.dataset.i, 1);
    drawSettings();
  }
});
$("#ssave").addEventListener("click", async () => {
  if (!SET.draft) return;
  $("#serr").textContent = "";
  try {
    const s = await invoke("settings_save", { set: settingsToSend(SET.draft) });
    // The leader comes from the config of this computer (SPEC 29.1): keep it.
    S.settings = { ...S.settings, ...s, leader: S.settings?.leader ?? s.leader };
    closeSettings();
    const missing = s.missing || [];
    toast(missing.length ? `Saved. Not found: ${missing.join(", ")}` : "Saved.");
    // The repo list follows the new folders at once.
    loadRepos();
  } catch (e) {
    $("#serr").textContent = String(e);
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
  loadRepos();
}
function loadRepos() {
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
  window.SBWork?.panes();
});
listen("pane", (e) => {
  upsert(e.payload);
  drawBoard();
  window.SBParts?.panes();
  window.SBReview?.panes?.();
  window.SBTasks?.panes();
  window.SBWork?.panes();
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
  const gone = S.reviews.get(e.payload);
  S.reviews.delete(e.payload);
  if (S.tab !== e.payload) return drawTabs();
  // A closed round goes back to the newest open round of the same PR.
  const back = gone && gone.round > 1 && [...S.reviews.values()]
    .filter((r) => r.repo === gone.repo && r.number === gone.number && r.round < gone.round)
    .sort((a, b) => b.round - a.round)[0];
  setTab(back ? back.id : "board");
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

window.SB = { S, LINK, invoke, call, listen, esc, setHTML, toast, setTab, WORK_TITLE, workOf, openNvim, openLocalReview, expand, collapse, parkLive, paneById, refreshPanes, drawBoard, drawTabs, openLauncher, paneName, copyText, LABEL, term, isLeader, webLinks, bytes, webLinks, $,
  lampIcon, actions, stateAct, stateWord, gb, home, ICON, startRename, remeasure };

(async function start() {
  try {
    Object.assign(LINK, await invoke("link_state"));
    if (LINK.config_error) toast(LINK.config_error);
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
