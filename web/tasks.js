// The Tasks tab (SPEC 27): tasks and sprints, as a board of status columns or as a
// list of sprints. The hub keeps the rules; this file draws and sends changes.

(() => {
  const { S, call, invoke, listen, esc, toast, $ } = window.SB;
  const STATUS = [["todo", "To do"], ["doing", "In progress"], ["blocked", "Blocked"], ["review", "In review"], ["done", "Done"]];
  const LABEL = Object.fromEntries(STATUS);
  const PRIO = [["high", "High"], ["normal", "Normal"], ["low", "Low"]];
  const save = (k, v) => { try { localStorage.setItem(`sb.tasks.${k}`, v); } catch (_) {} };
  const load = (k) => { try { return localStorage.getItem(`sb.tasks.${k}`); } catch (_) { return null; } };

  // `sel`: the task in the panel. `focus`: the task the keys act on. `adding`: the
  // place of the new-task field, { status, sprint }. `drag`: the task in a drag.
  const T = { sprints: [], tasks: [], loaded: false, view: load("view") === "list" ? "list" : "board", sel: null, focus: null, adding: null, cancel: null, links: [], missing: [], linking: false, hush: new Set(), drag: null, allDone: false, openClosed: new Set(), notes: "edit", history: [] };

  // Notes are Markdown. Raw HTML and images show as text; only http(s) links are links,
  // and they open in the browser (the click handler in review.js).
  const mdr = new marked.Renderer();
  mdr.html = (h) => esc(typeof h === "string" ? h : h?.text ?? "");
  mdr.image = (href, title, text) => esc(text || href || "");
  mdr.link = (href, title, text) => (/^https?:\/\//.test(href || "") ? `<a href="${esc(href)}">${text}</a>` : text);
  const md = (s) => { try { return marked.parse(String(s ?? ""), { renderer: mdr, gfm: true }); } catch (_) { return esc(s); } };

  const view = () => $("#view-tasks");
  // Panes linked to a task (SPEC 27.6). A pane that is not on the board has ended.
  const linksOf = (id) => T.links.filter((l) => l.task === id);
  const paneOf = (l) => window.SB.paneById(l.pane);
  const lampOf = (l) => paneOf(l)?.lamp || "ended";
  const linkName = (l) => { const p = paneOf(l); return p ? window.SB.paneName(p) : l.title || (l.tree || "").split("/").pop() || l.pane; };
  const missing = (t) => !!t.repo && T.missing.includes(t.repo);
  // "Agent finished": a linked pane is at Your turn and the task is In progress.
  // Dismiss hides it until that pane leaves Your turn.
  const hinted = (t) => t.status === "doing" && linksOf(t.id).some((l) => lampOf(l) === "turn" && !T.hush.has(`${t.id}|${l.pane}`));
  const openSprint = () => T.sprints.find((s) => s.state === "open");
  const sprintOf = (t) => T.sprints.find((s) => s.id === t.sprint);
  const frozen = (t) => sprintOf(t)?.state === "closed";
  const task = (id) => T.tasks.find((t) => t.id === id);
  const listOf = (sprint) => T.tasks.filter((t) => (t.sprint || null) === (sprint || null)).sort((a, b) => a.rank - b.rank);
  const repoName = (path) => (path ? path.split("/").filter(Boolean).pop() : "");
  const pad = (n) => String(n).padStart(2, "0");
  const day = (d) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  const addDays = (s, n) => { const d = new Date(`${s}T00:00`); d.setDate(d.getDate() + n); return day(d); };
  // Days left, with the end date as a sprint day. Local midnight (SPEC 27.3).
  const daysLeft = (s) => Math.round((new Date(`${s.end}T00:00`) - new Date(`${day(new Date())}T00:00`)) / 86400000) + 1;

  // The tasks the board shows in a column, in order.
  function column(status) {
    const sp = openSprint();
    const list = listOf(sp?.id).filter((t) => t.status === status);
    if (status !== "done" || T.allDone) return list;
    return list.slice().sort((a, b) => (b.done || 0) - (a.done || 0)).slice(0, 10).sort((a, b) => a.rank - b.rank);
  }

  // ---------- drawing ----------

  function draw() {
    const v = view();
    if (!v || S.tab !== "tasks") return;
    if (!T.loaded) { v.innerHTML = `<div class="tempty">Loading…</div>`; return; }
    // A select changes at once, so only a text field keeps the panel as it is.
    const keepPanel = T.sel && v.querySelector(".tpanel")?.contains(document.activeElement) && document.activeElement.matches("input, textarea");
    const typing = document.activeElement?.matches?.(".tadd");
    const main = `<div class="tmain">${head()}${T.view === "board" ? board() : list()}</div>`;
    if (keepPanel) {
      // You type in the panel: keep it, so the field keeps its text and focus.
      v.querySelector(".tmain").outerHTML = main;
      drawHistory();
    } else {
      v.innerHTML = main + (T.sel && task(T.sel) ? panel(task(T.sel)) : "");
    }
    v.classList.toggle("has-panel", !!(T.sel && task(T.sel)));
    // The new-task field takes focus once, when it opens, and keeps it while you type.
    const add = v.querySelector(".tadd");
    if (add && (T.adding.focus || typing)) { add.focus(); T.adding.focus = false; }
    v.querySelector(`[data-task="${CSS.escape(T.focus || "")}"]`)?.scrollIntoView({ block: "nearest" });
  }

  function head() {
    const sp = openSprint();
    const seg = `<div class="seg" id="tview">${[["board", "Board"], ["list", "List"]].map(([v, l]) => `<button type="button" data-v="${v}" aria-pressed="${T.view === v}">${l}</button>`).join("")}</div>`;
    let info = `<span class="tsname">No sprint is open</span>`;
    let acts = `<button class="btn small" data-act="sprint-new">New sprint</button>`;
    if (sp) {
      const all = listOf(sp.id);
      const done = all.filter((t) => t.status === "done").length;
      const left = daysLeft(sp);
      const when = left > 0 ? `${left} day${left > 1 ? "s" : ""} left` : `ended ${1 - left} day${left < 0 ? "s" : ""} ago`;
      info = `<span class="tsname">${esc(sp.name)}</span>${sp.goal ? `<span class="tsgoal">${esc(sp.goal)}</span>` : ""}
        <span class="tsdays${left < 1 ? " late" : ""}">${when}</span>
        <span class="tsbar" title="${done} of ${all.length} done"><i style="width:${all.length ? Math.round((100 * done) / all.length) : 0}%"></i></span><span class="tscount">${done}/${all.length}</span>`;
      acts = `<button class="btn small" data-act="sprint-edit">Edit sprint</button><button class="btn small" data-act="sprint-close">Close sprint</button>`;
    }
    return `<div class="thead">${seg}<div class="tsprint">${info}</div><div class="tacts">${acts}<button class="btn small primary" data-act="new">+ Task</button></div></div>`;
  }

  function card(t) {
    const links = linksOf(t.id);
    const needs = links.some((l) => lampOf(l) === "needs");
    const cls = ["tcard", T.focus === t.id && "focused", T.sel === t.id && "open", needs && "needs"].filter(Boolean).join(" ");
    const meta = [t.priority !== "normal" && `<span class="tprio" data-p="${t.priority}">${t.priority === "high" ? "▲ High" : "▼ Low"}</span>`,
      t.repo && `<span class="trepo">${esc(repoName(t.repo))}</span>`,
      missing(t) && `<span class="tmiss">repo missing</span>`,
      t.blocked_by && `<span class="tblock">Blocked by ${esc(t.blocked_by)}</span>`,
      links.length && `<span class="tdots">${links.map(dot).join("")}</span>`].filter(Boolean).join("");
    const hint = hinted(t) ? `<div class="thint">Agent finished<button class="tool" data-hint="review" data-id="${esc(t.id)}" title="Set the task to In review">→ In review</button><button class="tool x" data-hint="hush" data-id="${esc(t.id)}" aria-label="Dismiss">×</button></div>` : "";
    return `<div class="${cls}" data-task="${esc(t.id)}" draggable="true" tabindex="-1"><div class="tt">${esc(t.title)}</div>${meta ? `<div class="tm">${meta}</div>` : ""}${hint}</div>`;
  }

  function dot(l) {
    const lamp = lampOf(l);
    return `<i class="tdot" data-lamp="${esc(lamp)}" title="${esc(linkName(l))} · ${esc(window.SB.LABEL[lamp] || lamp)}"></i>`;
  }

  function adder(status, sprint) {
    const a = T.adding;
    if (!a || a.status !== status || (a.sprint || null) !== (sprint || null)) return "";
    return `<input class="tadd" data-status="${esc(status || "")}" data-sprint="${esc(sprint || "")}" value="${esc(a.text || "")}" placeholder="Task title, then Enter" maxlength="200" aria-label="New task title">`;
  }

  function board() {
    const sp = openSprint();
    if (!sp) {
      return `<div class="tempty">No sprint is open. <button class="btn small primary" data-act="sprint-new">New sprint</button></div>${section(null)}`;
    }
    const cols = STATUS.map(([s, label]) => {
      const tasks = column(s);
      const all = listOf(sp.id).filter((t) => t.status === s).length;
      const more = s === "done" && all > tasks.length ? `<button class="tmore" data-act="all-done">Show all ${all}</button>` : "";
      return `<section class="tcol" data-status="${s}"><header><span class="tchip" data-s="${s}">${label}</span><span class="tn">${all}</span><button class="tool" data-new="${s}" aria-label="New task in ${label}">+</button></header>
        <div class="tdrop" data-status="${s}" data-sprint="${esc(sp.id)}">${tasks.map(card).join("")}${adder(s, sp.id)}${more}</div></section>`;
    });
    return `<div class="tboard">${cols.join("")}</div>`;
  }

  function row(t, closed) {
    const cls = ["trow", T.focus === t.id && "focused", T.sel === t.id && "open"].filter(Boolean).join(" ");
    return `<div class="${cls}" data-task="${esc(t.id)}" draggable="${!closed}" tabindex="-1"><span class="tchip" data-s="${t.status}">${LABEL[t.status]}</span>
      <span class="tt">${esc(t.title)}</span>${t.priority !== "normal" ? `<span class="tprio" data-p="${t.priority}">${t.priority === "high" ? "▲" : "▼"}</span>` : ""}<span class="trepo">${missing(t) ? '<span class="tmiss">repo missing</span>' : esc(repoName(t.repo))}${linksOf(t.id).length ? `<span class="tdots">${linksOf(t.id).map(dot).join("")}</span>` : ""}</span></div>`;
  }

  // One list section: a sprint, or the backlog (null).
  function section(sp) {
    const id = sp?.id || null;
    const tasks = listOf(id);
    const closed = sp?.state === "closed";
    const done = tasks.filter((t) => t.status === "done").length;
    const name = sp ? esc(sp.name) : "Backlog";
    const dates = sp ? `<span class="tsdates">${esc(sp.start)} – ${esc(sp.end)}</span>` : "";
    if (closed) {
      const open = T.openClosed.has(id);
      return `<section class="tsec closed"><header><button class="tool" data-closed="${esc(id)}" aria-expanded="${open}">${open ? "▾" : "▸"}</button><span class="tsname">${name}</span>${dates}<span class="tn">${done} done</span></header>
        ${open ? `<div class="tlist">${tasks.map((t) => row(t, true)).join("")}</div>` : ""}</section>`;
    }
    return `<section class="tsec"><header><span class="tsname">${name}</span>${dates}<span class="tn">${tasks.length} task${tasks.length === 1 ? "" : "s"}${sp ? ` · ${done} done` : ""}</span><button class="tool" data-new="" data-sprint="${esc(id || "")}" aria-label="New task in ${name}">+</button></header>
      <div class="tlist tdrop" data-sprint="${esc(id || "")}">${tasks.map((t) => row(t, false)).join("") || `<div class="tnone">Drop tasks here.</div>`}${adder("todo", id)}</div></section>`;
  }

  function list() {
    const sp = openSprint();
    const closed = T.sprints.filter((s) => s.state === "closed").reverse();
    return `<div class="tsecs">${sp ? section(sp) : ""}${section(null)}${closed.map(section).join("")}</div>`;
  }

  function panel(t) {
    const ro = frozen(t) ? " disabled" : "";
    const opts = (list, cur) => list.map(([v, l]) => `<option value="${esc(v)}"${v === cur ? " selected" : ""}>${esc(l)}</option>`).join("");
    const repos = [["", "None"], ...S.repos.map((r) => [r.path, r.name])];
    if (t.repo && !repos.some(([p]) => p === t.repo)) repos.push([t.repo, repoName(t.repo)]);
    const where = sprintOf(t) ? `${esc(sprintOf(t).name)}${frozen(t) ? " (closed)" : ""}` : "Backlog";
    const notes = T.notes === "preview" || frozen(t)
      ? `<div class="md tnotes-md">${t.notes.trim() ? md(t.notes) : `<p class="tnone">No notes.</p>`}</div>`
      : `<textarea id="tp-notes" placeholder="Notes, in Markdown"${ro}>${esc(t.notes)}</textarea>`;
    return `<aside class="tpanel" data-panel="${esc(t.id)}" aria-label="Task">
      <div class="tphead"><span class="tpwhere">${where}</span><button class="tool x" data-act="panel-close" aria-label="Close">×</button></div>
      <input id="tp-title" value="${esc(t.title)}" maxlength="200" aria-label="Title"${ro}>
      <div class="tpfields">
        <label>Status<select id="tp-status"${ro}>${opts(STATUS, t.status)}</select></label>
        <label>Priority<select id="tp-prio"${ro}>${opts(PRIO, t.priority)}</select></label>
        <label class="wide">Repo<select id="tp-repo"${ro}>${opts(repos, t.repo || "")}</select></label>
        <label class="wide" ${t.status === "blocked" ? "" : "hidden"}>Blocked by<input id="tp-blocked" value="${esc(t.blocked_by || "")}"${ro}></label>
      </div>
      <div class="tpnotes"><div class="seg" id="tp-nseg"${frozen(t) ? " hidden" : ""}>${[["edit", "Edit"], ["preview", "Preview"]].map(([v, l]) => `<button type="button" data-v="${v}" aria-pressed="${T.notes === v}">${l}</button>`).join("")}</div>${notes}</div>
      ${agents(t)}
      <div class="tphist" id="tp-hist"></div>
      <div class="tpfoot">${frozen(t) ? `<button class="btn small" data-act="copy">Copy to backlog</button>` : `<button class="btn small" data-act="delete">Delete</button>`}</div>
    </aside>`;
  }

  // The Agents part of the panel: the linked panes, Start agent and Link pane.
  function agents(t) {
    const ro = frozen(t);
    const rows = linksOf(t.id).map((l) => {
      const lamp = lampOf(l);
      const live = !!paneOf(l);
      const canResume = l.kind === "claude" && l.session && (lamp === "ended" || lamp === "error");
      return `<div class="tpa" data-pane="${esc(l.pane)}">${dot(l)}<span class="tpaname">${esc(linkName(l))}</span><span class="tpastate">${esc(window.SB.LABEL[lamp] || lamp)}</span>
        ${live && lamp !== "ended" ? `<button class="tool" data-pact="open">Open</button>` : ""}${canResume ? `<button class="tool" data-pact="resume">Resume</button>` : ""}
        ${ro ? "" : `<button class="tool x" data-pact="unlink" aria-label="Unlink">×</button>`}</div>`;
    }).join("");
    const mine = new Set(linksOf(t.id).map((l) => l.pane));
    const free = window.SB.S.panes.filter((p) => p.kind !== "nvim" && !mine.has(p.id));
    const pick = T.linking && !ro
      ? `<div class="tplink">${free.length ? free.map((p) => `<button class="tool" data-linkpane="${esc(p.id)}">${esc(window.SB.paneName(p))}${T.links.some((l) => l.pane === p.id) ? " (moves)" : ""}</button>`).join("") : `<span class="tnone">No pane to link.</span>`}</div>` : "";
    const btns = ro ? "" : `<div class="tpabtns"><button class="btn small primary" data-act="start"${missing(t) ? ` disabled title="The repo folder is missing"` : ""}>Start agent</button>
      <button class="btn small" data-act="link" aria-expanded="${T.linking}">Link pane</button></div>`;
    return `<div class="tpagents"><h5>Agents</h5>${rows || `<p class="tnone">No agent works on this task.</p>`}${pick}${btns}</div>`;
  }

  async function startAgent(id) {
    const t = task(id);
    if (!t || frozen(t)) return;
    if (missing(t)) return toast(`The repo folder of this task is missing: ${t.repo}`);
    try {
      const plan = await call("task_start_plan", { id });
      window.SB.openLauncher(plan);
    } catch (_) {}
  }

  // The task menu of a tile: link the pane to a task, unlink it, or make a task from it.
  function menu(pane, btn) {
    const m = $("#taskmenu");
    const p = window.SB.paneById(pane);
    if (!p) return;
    if (!m.hidden && m.dataset.pane === pane) { m.hidden = true; return; }
    const cur = T.links.find((l) => l.pane === pane);
    const sp = openSprint();
    const pickable = T.tasks.filter((t) => t.status !== "done" && !frozen(t) && t.id !== cur?.task && (!t.sprint || t.sprint === sp?.id))
      .sort((a, b) => (a.sprint === sp?.id ? 0 : 1) - (b.sprint === sp?.id ? 0 : 1) || a.rank - b.rank).slice(0, 12);
    m.innerHTML = `<button data-tm="new">New task from this pane</button>${cur ? `<button data-tm="unlink">Unlink from “${esc(task(cur.task)?.title || cur.task)}”</button>` : ""}
      ${pickable.length ? `<div class="tmhead">Link to</div>${pickable.map((t) => `<button data-tm="link" data-id="${esc(t.id)}"><span>${esc(t.title)}</span><span class="at">${LABEL[t.status]}</span></button>`).join("")}` : ""}`;
    m.dataset.pane = pane;
    m.hidden = false;
    const r = btn.getBoundingClientRect();
    // Below the button, or above it when the menu does not fit below.
    const below = r.bottom + 4 + m.offsetHeight <= window.innerHeight - 8;
    m.style.top = `${Math.round(below ? r.bottom + 4 : Math.max(8, r.top - 4 - m.offsetHeight))}px`;
    m.style.left = `${Math.round(Math.max(8, Math.min(r.left, window.innerWidth - m.offsetWidth - 8)))}px`;
  }

  async function menuPick(b) {
    const m = $("#taskmenu");
    const pane = m.dataset.pane;
    m.hidden = true;
    const p = window.SB.paneById(pane);
    if (!p) return;
    try {
      if (b.dataset.tm === "unlink") return await call("task_unlink", { pane });
      if (b.dataset.tm === "link") return await call("task_link", { task: b.dataset.id, pane });
      const repo = window.SB.S.repos.find((r) => r.path === p.tree || r.worktrees.some((w) => w.path === p.tree))?.path || null;
      const t = await call("task_add", { task: { title: window.SB.paneName(p), status: "doing", sprint: openSprint()?.id || null, repo } });
      await call("task_link", { task: t.id, pane });
      toast(`New task “${t.title}”, linked to this pane.`);
    } catch (_) {}
  }
  document.addEventListener("click", (e) => {
    const m = $("#taskmenu");
    if (m.hidden) return;
    const b = e.target.closest("#taskmenu button");
    if (b) return menuPick(b);
    if (!e.target.closest("[data-ptaskmenu]")) m.hidden = true;
  });
  document.addEventListener("keydown", (e) => { if (e.key === "Escape" && !$("#taskmenu").hidden) $("#taskmenu").hidden = true; });

  function drawHistory() {
    const h = $("#tp-hist");
    if (!h) return;
    const when = (s) => new Date(s * 1000).toLocaleString([], { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
    const what = (x) => x.kind === "created" ? `Created as ${LABEL[x.value] || x.value}`
      : x.kind === "status" ? `Status: ${LABEL[x.value] || x.value}`
      : x.kind === "sprint" ? `Moved to ${esc(T.sprints.find((s) => s.id === x.value)?.name || "the backlog")}`
      : x.kind === "copied" ? `Copied from a closed sprint`
      : x.kind === "linked" ? `Linked pane ${esc(x.value)}`
      : x.kind === "unlinked" ? `Unlinked pane ${esc(x.value)}`
      : x.kind === "note" ? `Note: ${esc(x.value)}` : "Edited";
    h.innerHTML = `<h5>History</h5><ol>${T.history.slice().reverse().map((x) => `<li><span class="when">${when(x.at)}</span>${what(x)}${x.by !== "you" ? ` <span class="by">· ${esc(x.by)}</span>` : ""}</li>`).join("")}</ol>`;
  }

  async function loadHistory() {
    const id = T.sel;
    if (!id) return;
    try {
      const h = await invoke("task_history", { id });
      if (T.sel === id) { T.history = h; drawHistory(); }
    } catch (_) {}
  }

  // ---------- changes ----------

  async function refresh() {
    try {
      const v = await invoke("tasks");
      setData(v);
    } catch (e) {
      toast(e);
    }
  }

  function setData(v) {
    T.sprints = v.sprints;
    T.tasks = v.tasks;
    T.links = v.links || [];
    T.missing = v.missing || [];
    T.loaded = true;
    window.SB.drawBoard();
    if (T.sel && !task(T.sel)) T.sel = null;
    if (T.focus && !task(T.focus)) T.focus = null;
    draw();
    loadHistory();
  }

  function openPanel(id) {
    if (T.sel !== id) T.linking = false;
    T.sel = id;
    T.focus = id;
    T.history = [];
    draw();
    loadHistory();
  }

  function closePanel() {
    T.sel = null;
    draw();
  }

  // A move to Blocked asks who or what blocks the task first.
  async function reason(t) {
    const v = await ask(`<h4>Blocked by</h4><p>Who or what blocks “${esc(t.title)}”?</p><input id="tb-reason" maxlength="200" placeholder="For example: a review from Sean">`,
      [["Cancel", null], ["Set blocked", "ok"]], () => $("#tb-reason").value.trim(), (v) => !v && "Type who or what blocks it.");
    return v || null;
  }

  async function setStatus(id, status) {
    const t = task(id);
    if (!t || frozen(t) || t.status === status) return;
    let blockedBy = null;
    if (status === "blocked") {
      blockedBy = await reason(t);
      if (!blockedBy) return draw();
    }
    await call("task_status", { id, status, blockedBy }).catch(() => draw());
  }

  async function move(id, target, before) {
    const t = task(id);
    if (!t || frozen(t) || before === id) return;
    const to = { sprint: target.sprint || null, status: target.status || null, blocked_by: null, before: before || null };
    if (to.status === "blocked" && t.status !== "blocked") {
      to.blocked_by = await reason(t);
      if (!to.blocked_by) return;
    }
    await call("task_move", { id, to }).catch(() => {});
  }

  async function addTask(input) {
    const title = input.value.trim();
    if (!title) { T.adding = null; return draw(); }
    const task = { title, status: input.dataset.status || "todo", sprint: input.dataset.sprint || null, blocked_by: null };
    if (task.status === "blocked") {
      task.blocked_by = await reason({ title });
      if (!task.blocked_by) return;
    }
    input.value = "";
    if (T.adding) T.adding.text = "";
    try {
      const t = await call("task_add", { task });
      T.focus = t.id;
    } catch (_) {
      input.value = title;
    }
  }

  function editFields() {
    const t = task(T.sel);
    if (!t || frozen(t)) return;
    const fields = { title: $("#tp-title").value, notes: $("#tp-notes") ? $("#tp-notes").value : t.notes, priority: $("#tp-prio").value, repo: $("#tp-repo").value || null };
    if (fields.title.trim() === t.title && fields.notes === t.notes && fields.priority === t.priority && fields.repo === (t.repo || null)) return;
    if (!fields.title.trim()) { $("#tp-title").value = t.title; return toast("A task needs a title."); }
    call("task_edit", { id: t.id, fields }).catch(() => {});
  }

  // ---------- dialogs ----------

  // A dialog in the #tbox layer. `buttons` is [label, value]; the last one is the main
  // one. `read` gives the value for the main button, and `bad` an error text or false.
  function ask(html, buttons, read = () => true, bad = () => false) {
    return new Promise((resolve) => {
      const box = $("#tbox");
      box.innerHTML = `<div class="fbox" role="dialog" aria-modal="true">${html}<p class="terr" id="tb-err"></p>
        <div class="fbtns">${buttons.map(([label], i) => `<button class="btn${i === buttons.length - 1 ? " primary" : ""}" data-b="${i}">${esc(label)}</button>`).join("")}</div></div>`;
      box.hidden = false;
      const done = (v) => { T.cancel = null; box.hidden = true; box.innerHTML = ""; resolve(v); };
      T.cancel = () => done(null);
      const main = () => {
        const v = read();
        const err = bad(v);
        if (err) { $("#tb-err").textContent = err; return; }
        done(v);
      };
      box.querySelectorAll("[data-b]").forEach((b) => (b.onclick = () => (buttons[+b.dataset.b][1] === null ? done(null) : main())));
      box.onkeydown = (e) => {
        e.stopPropagation();
        if (e.key === "Escape") done(null);
        else if (e.key === "Enter" && e.target.matches("input")) { e.preventDefault(); main(); }
      };
      (box.querySelector("input, select") || box.querySelector(".btn.primary"))?.focus();
    });
  }

  function sprintFields(s) {
    return `<div class="tform">
      <label class="wide">Name<input id="ts-name" value="${esc(s.name)}" maxlength="80"></label>
      <label class="wide">Goal<input id="ts-goal" value="${esc(s.goal)}" maxlength="200" placeholder="Optional"></label>
      <label>Start<input id="ts-start" type="date" value="${esc(s.start)}"></label>
      <label>End<input id="ts-end" type="date" value="${esc(s.end)}"></label></div>`;
  }
  const readSprint = () => ({ name: $("#ts-name").value.trim(), goal: $("#ts-goal").value.trim(), start: $("#ts-start").value, end: $("#ts-end").value });
  const badSprint = (s) => !s.name ? "The sprint needs a name." : !s.start || !s.end ? "The sprint needs its dates." : s.end < s.start ? "The sprint ends before it starts." : false;
  const nextName = () => `Sprint ${T.sprints.length + 1}`;
  const fresh = (start) => ({ name: nextName(), goal: "", start, end: addDays(start, Math.max(1, S.settings?.sprint_days || 14) - 1) });

  async function newSprint() {
    const sprint = await ask(`<h4>New sprint</h4>${sprintFields(fresh(day(new Date())))}`, [["Cancel", null], ["Start sprint", "ok"]], readSprint, badSprint);
    if (sprint) await call("sprint_new", { sprint }).catch(() => {});
  }

  async function editSprint() {
    const sp = openSprint();
    if (!sp) return;
    const sprint = await ask(`<h4>Edit sprint</h4>${sprintFields(sp)}`, [["Cancel", null], ["Save", "ok"]], readSprint, badSprint);
    if (sprint) await call("sprint_edit", { id: sp.id, sprint }).catch(() => {});
  }

  // Close: a place for each task that is not done, and an optional next sprint.
  async function closeSprint() {
    const sp = openSprint();
    if (!sp) return;
    const open = listOf(sp.id).filter((t) => t.status !== "done");
    const choice = (id) => `<select data-place="${esc(id)}"><option value="next">Next sprint</option><option value="backlog">Backlog</option><option value="done">Done</option></select>`;
    const rows = open.map((t) => `<li><span class="tchip" data-s="${t.status}">${LABEL[t.status]}</span><span class="tt">${esc(t.title)}</span>${choice(t.id)}</li>`).join("");
    const html = `<h4>Close ${esc(sp.name)}</h4>
      ${open.length ? `<p>${open.length} task${open.length > 1 ? "s are" : " is"} not done. Choose where each one goes.</p>
        <label class="tall">All tasks <select id="tc-all"><option value="">…</option><option value="next">Next sprint</option><option value="backlog">Backlog</option><option value="done">Done</option></select></label>
        <ul class="tclose">${rows}</ul>` : "<p>Every task is done.</p>"}
      <label class="tall"><input type="checkbox" id="tc-next" checked> Start the next sprint</label>
      <div id="tc-nextf">${sprintFields(fresh(addDays(sp.end, 1) > day(new Date()) ? addDays(sp.end, 1) : day(new Date())))}</div>`;
    const read = () => ({
      places: [...document.querySelectorAll("#tbox [data-place]")].map((s) => [s.dataset.place, s.value]),
      next: $("#tc-next").checked ? readSprint() : null,
    });
    const bad = (v) => v.next ? badSprint(v.next) : v.places.some(([, p]) => p === "next") && "A task goes to the next sprint. Start the next sprint, or choose another place.";
    const p = ask(html, [["Cancel", null], ["Close sprint", "ok"]], read, bad);
    $("#tc-all")?.addEventListener("change", (e) => { if (e.target.value) document.querySelectorAll("#tbox [data-place]").forEach((s) => (s.value = e.target.value)); });
    $("#tc-next").addEventListener("change", (e) => ($("#tc-nextf").hidden = !e.target.checked));
    const v = await p;
    if (v) await call("sprint_close", { id: sp.id, places: v.places, next: v.next }).catch(() => {});
  }

  async function del(id) {
    const t = task(id);
    if (!t) return;
    const ok = await ask(`<h4>Delete task</h4><p>Delete “${esc(t.title)}” and its history?</p>`, [["Cancel", null], ["Delete", "ok"]]);
    if (!ok) return;
    if (T.sel === id) T.sel = null;
    await call("task_delete", { id }).catch(() => {});
  }

  // ---------- events ----------

  function startAdd(status, sprint) {
    T.adding = { status: status || "todo", sprint: sprint || null, text: "", focus: true };
    draw();
  }

  // The place for a new task near the focused one.
  function addHere() {
    const t = task(T.focus);
    if (T.view === "board") {
      const sp = openSprint();
      return startAdd(t && sp && t.sprint === sp.id ? t.status : "todo", sp ? sp.id : null);
    }
    startAdd("todo", t && !frozen(t) ? t.sprint : openSprint()?.id || null);
  }

  document.addEventListener("click", (e) => {
    const v = view();
    if (!v || !v.contains(e.target)) return;
    const hint = e.target.closest("[data-hint]");
    if (hint) {
      const t = task(hint.dataset.id);
      if (!t) return;
      if (hint.dataset.hint === "review") return setStatus(t.id, "review");
      linksOf(t.id).filter((l) => lampOf(l) === "turn").forEach((l) => T.hush.add(`${t.id}|${l.pane}`));
      return draw();
    }
    const pact = e.target.closest("[data-pact]");
    if (pact) {
      const pane = pact.closest("[data-pane]").dataset.pane;
      if (pact.dataset.pact === "open") return window.SB.expand(pane);
      if (pact.dataset.pact === "unlink") return call("task_unlink", { pane }).catch(() => {});
      pact.disabled = true;
      return call("task_resume", { pane }).then((id) => window.SB.refreshPanes().then(() => window.SB.expand(id))).catch(() => { pact.disabled = false; });
    }
    const lp = e.target.closest("[data-linkpane]");
    if (lp) { T.linking = false; return call("task_link", { task: T.sel, pane: lp.dataset.linkpane }).catch(() => {}); }
    const el = e.target.closest("[data-act], [data-new], [data-closed], #tview button, #tp-nseg button, [data-task]");
    if (!el) return;
    if (el.closest("#tview")) { T.view = el.dataset.v; save("view", T.view); return draw(); }
    if (el.closest("#tp-nseg")) { editFields(); T.notes = el.dataset.v; return draw(); }
    if (el.dataset.closed !== undefined) {
      const id = el.dataset.closed;
      T.openClosed.has(id) ? T.openClosed.delete(id) : T.openClosed.add(id);
      return draw();
    }
    if (el.dataset.new !== undefined) return startAdd(el.dataset.new || "todo", el.dataset.sprint !== undefined ? el.dataset.sprint : openSprint()?.id);
    if (el.dataset.task) return openPanel(el.dataset.task);
    switch (el.dataset.act) {
      case "new": return addHere();
      case "sprint-new": return newSprint();
      case "sprint-edit": return editSprint();
      case "sprint-close": return closeSprint();
      case "all-done": T.allDone = true; return draw();
      case "panel-close": return closePanel();
      case "delete": return del(T.sel);
      case "copy": return call("task_copy", { id: T.sel }).then((t) => openPanel(t.id)).catch(() => {});
      case "start": return startAgent(T.sel);
      case "link": T.linking = !T.linking; return draw();
    }
  });

  document.addEventListener("change", (e) => {
    if (!view()?.contains(e.target)) return;
    const id = e.target.id;
    if (id === "tp-status") {
      const want = e.target.value;
      if (want === "blocked") {
        // The reason field shows; Enter or leaving it sets Blocked.
        const f = $("#tp-blocked");
        f.closest("label").hidden = false;
        f.focus();
        return;
      }
      return setStatus(T.sel, want);
    }
    if (id === "tp-blocked") {
      const t = task(T.sel);
      const why = e.target.value.trim();
      if (!t) return;
      if (!why) return t.status === "blocked" ? (e.target.value = t.blocked_by) : draw();
      return call("task_status", { id: t.id, status: "blocked", blockedBy: why }).catch(() => {});
    }
    if (["tp-title", "tp-prio", "tp-repo", "tp-notes"].includes(id)) editFields();
  });

  document.addEventListener("input", (e) => { if (e.target.matches?.(".tadd") && T.adding) T.adding.text = e.target.value; });
  document.addEventListener("keydown", (e) => {
    if (!view()?.contains(e.target)) return;
    if (e.target.matches(".tadd")) {
      if (e.key === "Enter") { e.preventDefault(); addTask(e.target); }
      else if (e.key === "Escape") { e.preventDefault(); T.adding = null; draw(); }
      return;
    }
    if (e.target.id === "tp-title" && e.key === "Enter") { e.preventDefault(); e.target.blur(); }
    if (e.target.id === "tp-blocked" && e.key === "Enter") { e.preventDefault(); e.target.blur(); }
    if (e.key === "Escape" && e.target.closest(".tpanel")) {
      e.preventDefault();
      // Esc cancels: the old reason goes back first, so the blur changes nothing.
      if (e.target.id === "tp-blocked") e.target.value = task(T.sel)?.blocked_by || "";
      e.target.blur?.();
      closePanel();
    }
  });
  document.addEventListener("focusout", (e) => {
    const t = e.target;
    // You left the reason field empty: the select goes back to the status.
    if (t.id === "tp-blocked" && !t.value.trim()) setTimeout(() => { if (document.activeElement?.id !== "tp-blocked") draw(); }, 0);
    if (t.matches?.(".tadd") && !t.value.trim() && T.adding) {
      // Leaving an empty field drops it. A click on a card goes on as a click.
      setTimeout(() => { if (T.adding && document.activeElement !== $(".tadd")) { T.adding = null; draw(); } }, 150);
    }
  });

  // Drag a card or a row: a column sets the status, a section sets the sprint. The
  // drop goes before the card under the pointer, or at the end.
  document.addEventListener("dragstart", (e) => {
    const el = e.target.closest?.("#view-tasks [data-task][draggable='true']");
    if (!el) return;
    T.drag = el.dataset.task;
    e.dataTransfer?.setData("text/plain", T.drag);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
    el.classList.add("dragging");
  });
  const clearMarks = () => document.querySelectorAll("#view-tasks .drop-before, #view-tasks .drop-in").forEach((x) => x.classList.remove("drop-before", "drop-in"));
  document.addEventListener("dragover", (e) => {
    const zone = T.drag && e.target.closest?.("#view-tasks .tdrop");
    if (!zone) return;
    e.preventDefault();
    clearMarks();
    const over = e.target.closest("[data-task]");
    if (over && over.dataset.task !== T.drag) over.classList.add("drop-before");
    else zone.classList.add("drop-in");
  });
  document.addEventListener("drop", (e) => {
    const zone = T.drag && e.target.closest?.("#view-tasks .tdrop");
    if (!zone) return;
    e.preventDefault();
    const id = T.drag;
    const before = e.target.closest("[data-task]")?.dataset.task || null;
    // A drop on itself is a short drag: the card stays where it is.
    if (before === id) { clearMarks(); T.drag = null; return; }
    clearMarks();
    T.drag = null;
    const t = task(id);
    // In the list view a drop keeps the status.
    move(id, { sprint: zone.dataset.sprint || null, status: zone.dataset.status || t?.status }, before);
  });
  document.addEventListener("dragend", () => { T.drag = null; clearMarks(); document.querySelectorAll("#view-tasks .dragging").forEach((x) => x.classList.remove("dragging")); });

  // ---------- keys (SPEC 27.5) ----------

  // The tasks in key order: column by column on the board, top to bottom in the list.
  function order() {
    if (T.view === "board" && openSprint()) return STATUS.map(([s]) => column(s).map((t) => t.id));
    return [[...document.querySelectorAll("#view-tasks [data-task]")].map((x) => x.dataset.task)];
  }

  function focusMove(dy, dx) {
    const cols = order();
    let c = cols.findIndex((ids) => ids.includes(T.focus));
    let i = c < 0 ? -1 : cols[c].indexOf(T.focus);
    if (c < 0) { c = Math.max(0, cols.findIndex((ids) => ids.length)); i = dy < 0 ? cols[c].length : -1; }
    if (dx) {
      let n = c;
      do n += dx; while (n >= 0 && n < cols.length && !cols[n].length);
      if (n < 0 || n >= cols.length) return;
      c = n;
      i = Math.min(Math.max(i, 0), cols[c].length - 1);
    } else {
      i = Math.min(Math.max(i + dy, 0), cols[c].length - 1);
    }
    const id = cols[c]?.[i];
    if (!id) return;
    T.focus = id;
    if (T.sel) T.sel = id;
    draw();
    if (T.sel) loadHistory();
  }

  document.addEventListener("keydown", (e) => {
    if (S.tab !== "tasks" || e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.target.closest?.("textarea, input, select, [contenteditable='true'], .xterm")) return;
    if (!$("#tbox").hidden || !$("#launcher").hidden) return;
    const k = e.key;
    if (k === "j" || k === "ArrowDown") focusMove(1, 0);
    else if (k === "k" || k === "ArrowUp") focusMove(-1, 0);
    else if (k === "h" || k === "ArrowLeft") focusMove(0, -1);
    else if (k === "l" || k === "ArrowRight") focusMove(0, 1);
    else if (k === "o") addHere();
    else if (k === "Enter" && T.focus) openPanel(T.focus);
    else if (k === "Escape" && T.sel) closePanel();
    else if (k === "s" && T.focus) startAgent(T.focus);
    else if (/^[1-5]$/.test(k) && T.focus) setStatus(T.focus, STATUS[+k - 1][0]);
    else return;
    e.preventDefault();
  });

  listen("tasks", (e) => setData(e.payload));

  window.SBTasks = {
    draw: () => {
      // The repo list loads with the launcher too; the panel needs it first here.
      if (!S.repos.length) invoke("repos").then((r) => { S.repos = r; if (T.sel) draw(); }).catch(() => {});
      return T.loaded ? draw() : refresh();
    },
    // Leaving the tab closes an open dialog.
    leave: () => T.cancel?.(),
    // A text to copy, in a dialog with a Copy button.
    showText: async (title, why, text) => {
      const p = ask(`<h4>${esc(title)}</h4><p>${esc(why)}</p><textarea id="tb-text" readonly rows="6">${esc(text)}</textarea>`, [["Close", null], ["Copy", "ok"]],
        () => window.SB.copyText($("#tb-text").value));
      // A tab change does not close this one: the text is not anywhere else.
      T.cancel = null;
      const ok = await p;
      if (ok) toast("Copied. Paste it into the pane.");
      else if (ok === false) toast("The copy did not work. Select the text and copy it yourself.");
    },
    // For the board: the task of a pane, the task menu of a tile, and a task to open.
    forPane: (pane) => { const l = T.links.find((x) => x.pane === pane); return l ? task(l.task) : null; },
    menu,
    open: (id) => { if (!task(id)) return; window.SB.setTab("tasks"); openPanel(id); },
    // A pane changed: a hint ends when its pane leaves Your turn; dots follow lamps.
    panes: () => {
      for (const k of [...T.hush]) {
        const pane = k.split("|")[1];
        if (window.SB.paneById(pane)?.lamp !== "turn") T.hush.delete(k);
      }
      // Pane events come often: redraw only when a linked lamp changes, never in a drag.
      const sig = T.links.map((l) => `${l.pane}:${lampOf(l)}`).join() + `|${S.panes.length}`;
      if (sig === T.lampSig) return;
      T.lampSig = sig;
      if (S.tab === "tasks" && T.loaded && !T.drag) draw();
    },
    refresh,
    _test: { T, daysLeft, md },
  };
  // The board shows task titles on tiles, so the list loads at start.
  refresh();
})();
