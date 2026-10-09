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
const call = (cmd, args) => invoke(cmd, args).catch((e) => { log("warn", cmd, "failed:", e); toast(e); throw e; });

// ---------- tabs ----------

function setTab(tab) {
  if (tab !== "board") window.SB?.term?.blur();
  S.tab = tab;
  $("#view-board").hidden = tab !== "board";
  $("#view-review").hidden = tab === "board";
  drawTabs();
  if (tab === "board") requestAnimationFrame(fitLive);
  else window.SBReview?.draw(tab);
}

function drawTabs() {
  const tabs = [`<button class="tab" role="tab" data-tab="board" aria-selected="${S.tab === "board"}">Board</button>`];
  for (const r of S.reviews.values()) {
    const busy = ["fetching", "writing", "updating"].includes(r.status) ? '<i class="busy"></i>' : "";
    tabs.push(`<button class="tab" role="tab" data-tab="${esc(r.id)}" aria-selected="${S.tab === r.id}">#${esc(r.number)}${busy}</button>`);
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

function meta(p) {
  const parts = [p.repo];
  if (p.branch) parts.push(p.branch);
  if (p.connection && S.conn.list.length > 1) parts.push(p.connection);
  if (p.rss) parts.push(`ra ${(p.rss / 1073741824).toFixed(1)} GB`);
  return parts.join(" · ");
}

function tile(p) {
  const nvimBtn = p.kind !== "nvim" && S.settings.nvim ? `<button class="tool" data-nvim>nvim</button>` : "";
  const state = p.lamp === "none" ? p.kind : LABEL[p.lamp] || p.lamp;
  const cls = ["pane", p.unseen && "unseen", S.focus === p.id && "focused", p.id === S.live && "is-live"].filter(Boolean).join(" ");
  return `<div class="${cls}" data-id="${esc(p.id)}" data-lamp="${esc(p.lamp)}" tabindex="0">
    <div class="ptitle"><span class="name">${esc(paneName(p))}</span><span class="state">${esc(state)}</span>${nvimBtn}<button class="tool x" data-close aria-label="Close">×</button></div>
    <div class="psum"><span class="meta">${esc(meta(p))}</span><span class="text">${esc(p.summary || "")}</span>${actions(p)}</div>
    <pre class="term-tail">${esc(p.tail.map(strip).join("\n"))}</pre>
  </div>`;
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
  const shown = S.panes.filter((p) => p.id !== S.live);
  $("#board").innerHTML = shown.map(tile).join("");
  $("#view-board").classList.toggle("has-live", !!S.live);
  drawTally();
  drawLiveTitle();
}

function drawTally() {
  const n = {};
  for (const p of S.panes) if (p.unseen && COLOR[p.lamp]) n[p.lamp] = (n[p.lamp] || 0) + 1;
  $("#tally").innerHTML = URGENT.filter((s) => n[s])
    .map((s) => `<b style="color:${COLOR[s]}" data-jump="${s}"><i class="dot"></i>${n[s]} ${LABEL[s].toLowerCase()}</b>`)
    .join("");
}
$("#tally").addEventListener("click", () => nextUnseen(1));

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
  const ids = S.panes.filter((p) => p.id !== S.live).map((p) => p.id);
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
  if (!b) return expand(id);
  e.stopPropagation();
  if (b.dataset.close !== undefined) return call("pane_close", { id });
  if (b.dataset.nvim !== undefined) return openNvim(paneById(id).tree);
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
term.attachCustomKeyEventHandler((e) => !(e.ctrlKey && e.code === "Space"));
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
  if (!S.live || S.tab !== "board") return;
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
listen("pane_output", (e) => {
  if (e.payload.pane !== S.live) return;
  const b = bytes(e.payload.data);
  drawn ? term.write(b) : held.push(b);
});

function fitLive() {
  if (!S.live || $("#live").hidden) return;
  fit.fit();
  invoke("pane_resize", { cols: term.cols, rows: term.rows }).catch(() => {});
}
let fitTimer;
new ResizeObserver(() => { clearTimeout(fitTimer); fitTimer = setTimeout(fitLive, 60); }).observe($("#live"));

async function expand(id) {
  log("info", "expand", id);
  if (!paneById(id)) return;
  if (S.tab !== "board") setTab("board");
  S.live = id;
  S.focus = id;
  drawn = false;
  held = [];
  $("#live").hidden = false;
  drawBoardNow();
  // Let the layout settle so the fit sees the real size.
  await new Promise((r) => setTimeout(r, 0));
  fit.fit();
  term.reset();
  try {
    await invoke("pane_resize", { cols: term.cols, rows: term.rows });
    const screen = await invoke("pane_expand", { id });
    if (S.live !== id) return;
    term.write(bytes(screen));
    held.splice(0).forEach((b) => term.write(b));
    drawn = true;
    term.focus();
    log("info", "expanded", id, "cols", term.cols, "rows", term.rows, "focus in terminal:", document.activeElement === term.textarea);
  } catch (e) {
    toast(e);
    collapse();
  }
}

function collapse() {
  const was = S.live;
  S.live = null;
  $("#live").hidden = true;
  invoke("pane_collapse").catch(() => {});
  drawBoardNow();
  if (was) $(`.pane[data-id="${CSS.escape(was)}"]`)?.focus();
}

let renaming = false;
function drawLiveTitle() {
  const p = S.live && paneById(S.live);
  if (!p || renaming) return;
  const state = p.lamp === "none" ? p.kind : LABEL[p.lamp] || p.lamp;
  const nvimBtn = p.kind !== "nvim" && S.settings.nvim ? `<button class="tool" data-nvim>nvim</button>` : "";
  $("#livetitle").innerHTML = `<span class="name">${esc(paneName(p))}</span><span class="state" style="color:var(--muted)">${esc(state)}</span>
    <span class="psum meta" style="border:0;padding:0;font:500 11px/1 var(--mono);color:var(--muted)">${esc(meta(p))}</span>${actions(p).replace('class="tact"', 'class="tact" style="margin:0"')}
    ${nvimBtn}<button class="tool" data-collapse aria-label="Collapse">⤡</button>`;
}
$("#livetitle").addEventListener("click", (e) => {
  const b = e.target.closest("button");
  const p = paneById(S.live);
  if (!b || !p) return;
  if (b.dataset.collapse !== undefined) return collapse();
  if (b.dataset.nvim !== undefined) return openNvim(p.tree);
  if (b.dataset.allow) return call("permit_answer", { permit: b.dataset.allow, allow: true });
  if (b.dataset.deny) return call("permit_answer", { permit: b.dataset.deny, allow: false });
  if (b.dataset.trust) return call("trust_answer", { id: p.id, yes: b.dataset.trust === "1" });
  if (b.dataset.switch !== undefined) return call("connection_resume", { id: p.id });
  if (b.dataset.resume !== undefined) return call("pane_resume", { id: p.id });
});

async function openNvim(tree, file, line) {
  const id = await call("nvim_open", { tree, file: file ?? null, line: line ?? null });
  await refreshPanes();
  expand(id);
}

function nextUnseen(dir) {
  const list = S.panes.filter((p) => p.unseen && p.id !== S.live).sort((a, b) => URGENT.indexOf(a.lamp) - URGENT.indexOf(b.lamp));
  if (!list.length) return;
  expand((dir > 0 ? list[0] : list[list.length - 1]).id);
}

function moveFocus(d) {
  const ids = S.panes.filter((p) => p.id !== S.live).map((p) => p.id);
  if (!ids.length) return;
  const i = Math.max(0, ids.indexOf(S.focus));
  S.focus = ids[(i + d + ids.length) % ids.length];
  drawBoard();
  $(`.pane[data-id="${CSS.escape(S.focus)}"]`)?.focus();
}

// ---------- keys (SPEC 7) ----------

let leader = 0;
window.addEventListener("keydown", (e) => {
  if (e.ctrlKey && e.code === "Space") {
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
    case "x": return S.live && (!focused || focused === S.live) ? collapse() : focused && expand(focused);
    case "j": return nextUnseen(1);
    case "k": return nextUnseen(-1);
    case "J": case "L": return moveFocus(1);
    case "K": case "H": return moveFocus(-1);
    case "e": { const p = paneById(focused); return p && openNvim(p.tree); }
    case "n": return openLauncher();
    case "b": return setTab("board");
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

const L = { list: [], sel: 0, where: "main", run: "claude", pr: null };
const fuzzy = (q, s) => { let j = 0; for (const c of s.toLowerCase()) if (c === q[j]) j++; return j === q.length; };
const setSeg = (id, v) => document.querySelectorAll(`#${id} button`).forEach((b) => b.setAttribute("aria-pressed", b.dataset.v === v));
const agentIn = (tree) => S.panes.some((p) => p.kind === "claude" && p.tree === tree && p.lamp !== "ended");

function filterRepos() {
  const q = $("#lq").value.trim();
  L.pr = /github\.com\/[^/]+\/[^/]+\/pull\/\d+/.test(q) ? q : null;
  L.list = L.pr ? [] : S.repos.filter((r) => fuzzy(q.toLowerCase(), r.name));
  L.sel = Math.min(L.sel, Math.max(0, L.list.length - 1));
  return q;
}

function drawLauncher() {
  const q = filterRepos();
  const open = (r) => S.panes.filter((p) => p.tree === r.path || r.worktrees.some((w) => w.path === p.tree)).length;
  $("#lrepos").innerHTML = L.pr
    ? `<li aria-selected="true"><span class="rn">Review ${esc(q.split("/pull/")[1]?.split(/[/#?]/)[0] || "")}</span><span class="rp">${esc(q)}</span></li>`
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
  $("#lsave").hidden = !!L.pr;
}

function pickRepo(i) {
  filterRepos();
  L.sel = Math.max(0, Math.min(i, L.list.length - 1));
  const r = L.list[L.sel];
  L.where = r && (r.dirty || agentIn(r.path)) ? "new" : "main";
  $("#lerr").textContent = r && agentIn(r.path) ? "An agent already runs in the main checkout." : "";
  drawLauncher();
}

async function openLauncher() {
  if (S.tab !== "board") setTab("board");
  $("#launcher").hidden = false;
  $("#lq").value = "";
  $("#lbranch").value = "";
  $("#lerr").textContent = "";
  L.sel = 0;
  drawLauncher();
  pickRepo(0);
  $("#lq").focus();
  drawLayouts();
  S.repos = await invoke("repos").catch((e) => { $("#lerr").textContent = String(e); return S.repos; });
  if (!$("#launcher").hidden) pickRepo(Math.min(L.sel, S.repos.length - 1));
}

async function drawLayouts() {
  const names = await invoke("layouts").catch(() => []);
  $("#llayouts").innerHTML = names.map((n) => `<button class="chip" data-layout="${esc(n)}">${esc(n)}</button>`).join("");
}

function closeLauncher() {
  $("#launcher").hidden = true;
  (S.live ? term : $("#newpane")).focus();
}

async function launch() {
  if (L.pr) {
    closeLauncher();
    const id = await call("review_open", { url: L.pr });
    await refreshReviews();
    setTab(id);
    return;
  }
  const r = L.list[L.sel];
  if (!r) return;
  const req = { repo: r.path, place: L.where, branch: L.where === "new" ? $("#lbranch").value.trim() : null, tree: L.where === "tree" ? $("#ltree").value : null, run: L.run, title: null };
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

$("#newpane").addEventListener("click", openLauncher);
$("#lcancel").addEventListener("click", closeLauncher);
$("#lgo").addEventListener("click", launch);
$("#launcher").addEventListener("click", (e) => { if (e.target.id === "launcher") closeLauncher(); });
$("#lq").addEventListener("input", () => { L.sel = 0; pickRepo(0); });
$("#lrepos").addEventListener("click", (e) => { const li = e.target.closest("li[data-i]"); if (li) pickRepo(+li.dataset.i); });
$("#lrepos").addEventListener("dblclick", (e) => { if (e.target.closest("li[data-i]")) launch(); });
$("#lwhere").addEventListener("click", (e) => { const b = e.target.closest("button"); if (b) { L.where = b.dataset.v; drawLauncher(); if (L.where === "new") $("#lbranch").focus(); } });
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

async function refreshReviews() {
  const list = await invoke("reviews");
  S.reviews = new Map(list.map((r) => [r.id, r]));
  drawTabs();
}

listen("panes", (e) => {
  S.panes = e.payload;
  if (S.live && !paneById(S.live)) collapse();
  else drawBoard();
});
listen("pane", (e) => {
  upsert(e.payload);
  drawBoard();
});
listen("connections", (e) => { S.conn = e.payload; drawConn(); drawBoard(); });
listen("focus", (e) => refreshPanes().then(() => expand(e.payload)));
listen("tmux_exit", () => toast("The Switchboard tmux server ended. Restart the app to start it again."));
listen("error", (e) => toast(e.payload.message || e.payload));
listen("review", (e) => {
  const was = S.reviews.has(e.payload.id);
  S.reviews.set(e.payload.id, e.payload);
  drawTabs();
  window.SBReview?.update(e.payload);
  if (!was) drawTabs();
});
listen("review_closed", (e) => {
  S.reviews.delete(e.payload);
  if (S.tab === e.payload) setTab("board");
  else drawTabs();
});
listen("review_stream", (e) => window.SBReview?.stream(e.payload));

window.SB = { S, invoke, call, listen, esc, toast, setTab, openNvim, expand, refreshPanes, term, $ };

(async function start() {
  try {
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
