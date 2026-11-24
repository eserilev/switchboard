// Today (SPEC 39): what needs you on GitHub, ranked. The hub fetches and ranks;
// this file draws the list and sends Done, Later and Undo.

(() => {
  const { S, call, invoke, listen, esc, toast, setTab, $ } = window.SB;
  const load = (k) => { try { return localStorage.getItem(`sb.today.${k}`); } catch (_) { return null; } };
  const save = (k, v) => { try { localStorage.setItem(`sb.today.${k}`, v); } catch (_) {} };

  // `sel`: the key of the selected row. `later`: the row whose Later menu is open.
  // `undo`: the last Done or Later, to take back. `g`: Leader-free `g` waits for a number.
  const D = { job: load("job") || "Everything", v: null, sel: null, later: null, undo: null, g: false, busy: false, fresh: 0 };
  const view = () => $("#view-today");
  const keyOf = (i) => `${i.repo.toLowerCase()}#${i.number}#${i.section === "changed" ? "c" : ""}`;
  const rows = () => (D.v ? D.v.sections.flatMap((s) => s.items) : []);
  const byKey = (k) => rows().find((i) => keyOf(i) === k) || (D.v ? [...D.v.done, ...D.v.later, ...(D.v.hidden || [])].find((i) => keyOf(i) === k) : null);
  // What Claude made of your notes for one item, in words (39.12).
  const claudeSays = (c) => c ? `${c.act === "hide" ? "Hidden" : c.act === "up" ? "Moved up" : "Moved down"} by Claude, from your notes: ${c.why}` : "";
  // `name#N`, or `owner/name#N` when two repos on the list share a name.
  const short = (r) => r.split("/").pop().toLowerCase();
  const name = (i) => {
    if (!D.v?.many_repos) return `#${i.number}`;
    const all = new Set([...D.v.sections.flatMap((s) => s.items), ...D.v.done, ...D.v.later, ...(D.v.hidden || [])].map((x) => x.repo));
    const shared = [...all].some((r) => r !== i.repo && short(r) === short(i.repo));
    return `${shared ? i.repo : i.repo.split("/").pop()}#${i.number}`;
  };
  const ago = (t) => {
    const s = Math.max(0, Date.now() / 1000 - t);
    return s < 60 ? "just now" : s < 3600 ? `${Math.round(s / 60)} min ago` : s < 86400 ? `${Math.round(s / 3600)} h ago` : `${Math.round(s / 86400)} d ago`;
  };
  const when = (t) => new Date(t * 1000).toLocaleString([], { weekday: "short", hour: "2-digit", minute: "2-digit" });
  const startOfDay = () => { const d = new Date(); d.setHours(0, 0, 0, 0); return d.getTime() / 1000; };

  // The Later times: 9:00 tomorrow, 9:00 next Monday, and a week from now at 9:00.
  function laterTimes() {
    const at9 = (d) => { d.setHours(9, 0, 0, 0); return Math.floor(d.getTime() / 1000); };
    const t = new Date(); t.setDate(t.getDate() + 1);
    const m = new Date(); m.setDate(m.getDate() + (((8 - m.getDay()) % 7) || 7));
    const w = new Date(); w.setDate(w.getDate() + 7);
    return [["Tomorrow morning", at9(t)], ["Monday morning", at9(m)], ["In a week", at9(w)]];
  }

  async function refresh() {
    try {
      D.v = await invoke("today", { job: D.job });
      D.job = D.v.job;
    } catch (e) {
      D.v = D.v || null;
      toast(e);
    }
    if (D.v && !byKey(D.sel)) D.sel = rows()[0] ? keyOf(rows()[0]) : null;
    if (S.tab === "today") draw();
    window.SB.drawTabs();
  }
  let queued = null;
  const soon = () => { clearTimeout(queued); queued = setTimeout(refresh, 150); };
  listen("today", () => { if (S.tab === "today" || D.v) soon(); });

  // ---------- drawing ----------

  function rowHTML(i) {
    const k = keyOf(i);
    const tags = i.tags.map((t) => `<span class="tdtag${t === "came back" ? " back" : ""}">${esc(t)}</span>`).join("");
    return `<div class="tdrow${D.sel === k ? " sel" : ""}" data-k="${esc(k)}" role="option" aria-selected="${D.sel === k}">
      <button class="tdcheck" type="button" data-act="done" title="Done (x)" aria-label="Mark ${esc(name(i))} done"></button>
      <span class="tdname">${esc(name(i))}</span>
      <span class="tdline">${esc(i.line)}<small>${esc(i.title)}</small></span>
      <span class="tdtags">${tags}</span>
    </div>`;
  }

  function panelHTML(i) {
    if (!i) return `<div class="tdempty">Pick a row to see why it is here.</div>`;
    const hidden = i.done_at || i.later_until;
    const back = i.came_back ? `<div class="tdback">${esc(i.came_back)}</div>` : "";
    const detail = i.detail.length ? `<h3>What it found</h3><ul>${i.detail.map((d) => `<li>${esc(d)}</li>`).join("")}</ul>` : "";
    const reasons = i.reasons.length > 1 ? `<ul class="tdreasons">${i.reasons.map((r) => `<li>${esc(r.line)}</li>`).join("")}</ul>` : "";
    const later = D.later === keyOf(i)
      ? `<div class="tdlater" role="menu">${laterTimes().map(([l, t], n) => `<button class="btn" type="button" data-later="${t}"><kbd>${n + 1}</kbd> ${esc(l)}<small>${esc(when(t))}</small></button>`).join("")}</div>` : "";
    const acts = hidden
      ? `<button class="btn" type="button" data-act="back">Bring back now</button>`
      : `<div class="tdacts"><button class="btn" type="button" data-act="done">Done <kbd>x</kbd></button><button class="btn" type="button" data-act="later" aria-expanded="${D.later === keyOf(i)}">Later <kbd>l</kbd></button></div>
         <p class="tdnote">Done hides it until someone else acts on the PR. Later hides it until a time you pick.</p>${later}`;
    return `${back}
      <div class="tdph"><span class="tdname">${esc(i.repo)}#${i.number}</span></div>
      <h2 class="tdtitle">${esc(i.title)}</h2>
      <p class="tdtodo">${esc(i.line)}</p>
      ${detail}
      ${i.claude ? `<p class="tdclaude">${esc(claudeSays(i.claude))}</p>` : ""}
      <h3>Why it is here</h3><p class="tdwhy">${esc(i.why || "")}</p>${reasons}
      ${i.claude?.act === "hide" ? `<p class="tdnote">To show it again, forget the note that hid it, or tell Claude it matters.</p>` : acts}
      <div class="tdlinks">
        <button class="linkish" type="button" data-act="open">Open on GitHub <kbd>o</kbd></button>
        <button class="linkish" type="button" data-act="task">Make a task <kbd>t</kbd></button>
        <button class="linkish" type="button" data-act="review">Start review <kbd>r</kbd></button>
        <button class="linkish" type="button" data-act="about">Tell Claude about this one</button>
      </div>`;
  }

  function draw() {
    const el = view();
    if (!el) return;
    const v = D.v;
    if (!v) {
      el.innerHTML = `<header class="head"><h1 class="htitle">Today</h1></header><div class="tdwait">Loading what needs you on GitHub…</div>`;
      return;
    }
    const tabs = v.jobs.map((j, n) => `<button class="tdjob" type="button" data-job="${esc(j.name)}" aria-current="${j.name === v.job ? "page" : "false"}" title="${esc(j.name)} (g ${n + 1})">${esc(j.name)}<span class="nc">${j.count || ""}</span></button>`).join("");
    const shown = rows();
    const doneToday = v.done.filter((i) => (i.done_at || 0) >= startOfDay()).length;
    const total = shown.length + doneToday;
    const pct = total ? Math.round((doneToday / total) * 100) : 100;
    const meta = [v.login ? `as ${esc(v.login)}` : "", v.refreshing ? "refreshing…" : v.updated ? `updated ${ago(v.updated)}` : ""].filter(Boolean).join(" · ");
    const sections = v.sections.filter((s) => s.items.length).map((s) => `<section class="tdsec"><h2>${esc(s.title)} <span>${s.items.length}</span></h2><div role="listbox" aria-label="${esc(s.title)}">${s.items.map(rowHTML).join("")}</div></section>`).join("");
    const empty = !v.loaded ? `<div class="tdwait">${v.error ? "" : "Loading what needs you on GitHub…"}</div>`
      : shown.length ? "" : `<div class="tdall">All done for today.</div>`;
    const fold = (title, list, hint, back = true) => list.length ? `<details class="tdfold"${load(`open.${title}`) === "1" ? " open" : ""} data-fold="${title}"><summary>${title} (${list.length})</summary>${list.map((i) => `<div class="tdrow small${D.sel === keyOf(i) ? " sel" : ""}" data-k="${esc(keyOf(i))}"><span class="tdname">${esc(name(i))}</span><span class="tdline">${esc(i.line)}<small>${esc(hint(i))}</small></span>${back ? `<button class="linkish" type="button" data-act="back">Bring back</button>` : "<span></span>"}</div>`).join("")}</details>` : "";
    const notes = v.notes || [];
    const status = v.judging ? `<span class="tdbusy">Claude is reading your notes…</span>`
      : v.judge_error ? `<span class="tdbad" role="alert">${esc(v.judge_error)}</span>`
      : v.reply && notes.length ? `<span>Claude: ${esc(v.reply)}</span>` : "";
    const tell = `<form class="tdtell" id="tdtell">
        <input id="tdtellin" type="text" maxlength="1000" autocomplete="off" aria-label="Tell Claude what is wrong with this list" placeholder="Tell Claude what is wrong with this list. For example: FOCIL work does not matter now.">
        <button class="btn" type="submit">Tell Claude</button>
      </form>${status ? `<p class="tdreply" aria-live="polite">${status}</p>` : ""}`;
    const notesFold = notes.length ? `<details class="tdfold"${load("open.Notes") === "1" ? " open" : ""} data-fold="Notes"><summary>What you told Claude (${notes.length})</summary>${notes.map((n) => `<div class="tdnoterow"><span>${esc(n.text)}</span><small>${esc(ago(n.at))}</small><button class="linkish" type="button" data-forget="${n.id}">Forget</button></div>`).join("")}</details>` : "";
    // Keep what you typed and the focus across a redraw.
    const was = el.querySelector("#tdtellin");
    const typed = was ? { value: was.value, focus: document.activeElement === was, at: was.selectionStart } : null;
    el.innerHTML = `<header class="head tdbar"><h1 class="htitle">Today</h1><div class="tdjobs" role="tablist" aria-label="Jobs">${tabs}</div><span class="sp"></span>
        <span class="tdmeta">${meta}</span><button class="btn" type="button" id="tdrefresh" title="Refresh from GitHub"${v.refreshing ? " disabled" : ""}>Refresh</button></header>
      <div class="tdbody">
        <div class="tdmain" tabindex="-1">
          <div class="tdprog"><span>${doneToday} of ${total} done today</span><i style="--p:${pct}%"></i></div>
          ${v.since ? `<p class="tdsince"><b>Since you last looked:</b> ${esc(v.since)}</p>` : ""}
          ${v.error ? `<div class="tderr" role="alert">${esc(v.error)}</div>` : ""}
          ${tell}
          ${sections}${empty}
          ${fold("Done", v.done, (i) => `done ${ago(i.done_at)}`)}${fold("Later", v.later, (i) => `back ${when(i.later_until)}`)}${fold("Hidden by your notes", v.hidden || [], (i) => i.claude?.why || "", false)}${notesFold}
        </div>
        <aside class="tdpanel" aria-label="The selected item">${panelHTML(byKey(D.sel))}</aside>
      </div>
      <footer class="tdkeys">j/k move · x done · l later · u undo · o open on GitHub · r review · t task · c tell Claude · g 1–9 job</footer>`;
    if (typed) {
      const inp = el.querySelector("#tdtellin");
      inp.value = typed.value;
      if (typed.focus) { inp.focus(); inp.setSelectionRange(typed.at, typed.at); }
    }
    el.querySelector(".tdrow.sel")?.scrollIntoView({ block: "nearest" });
  }

  // ---------- actions ----------

  function changedOf(i) { return i.section === "changed"; }

  async function done(i, on = true) {
    if (!i) return;
    D.undo = { kind: "done", repo: i.repo, number: i.number, changed: changedOf(i) };
    await call("today_done", { repo: i.repo, number: i.number, changed: changedOf(i), on });
    if (on) { toast(`Done: ${name(i)}. Press u to undo.`); step(1, true); }
    await refresh();
  }
  async function later(i, until) {
    if (!i) return;
    D.later = null;
    D.undo = { kind: "later", repo: i.repo, number: i.number, changed: changedOf(i) };
    await call("today_later", { repo: i.repo, number: i.number, changed: changedOf(i), until });
    toast(`Later: ${name(i)}, back ${when(until)}. Press u to undo.`);
    step(1, true);
    await refresh();
  }
  async function bringBack(i) {
    if (!i) return;
    if (i.later_until) await call("today_later", { repo: i.repo, number: i.number, changed: changedOf(i), until: null });
    else await call("today_done", { repo: i.repo, number: i.number, changed: changedOf(i), on: false });
    await refresh();
  }
  async function undo() {
    const u = D.undo;
    if (!u) return toast("Nothing to undo.");
    D.undo = null;
    if (u.kind === "later") await call("today_later", { repo: u.repo, number: u.number, changed: u.changed, until: null });
    else await call("today_done", { repo: u.repo, number: u.number, changed: u.changed, on: false });
    D.sel = `${u.repo.toLowerCase()}#${u.number}#${u.changed ? "c" : ""}`;
    await refresh();
  }
  async function makeTask(i) {
    if (!i) return;
    await call("task_add", { task: { title: `${name(i)} ${i.title}`, notes: `${i.line}\n\n${i.url}`, links: [i.url] } });
    toast(`Task added for ${name(i)}.`);
  }
  async function review(i) {
    if (!i) return;
    const id = await call("review_open", { url: i.url, roundOf: null });
    await window.SB.refreshReviews?.();
    setTab(id);
  }
  const open = (i) => i && invoke("open_url", { url: i.url }).catch(toast);

  // Tell Claude (39.12): a note on what is wrong with the list.
  function focusTell(prefix) {
    const inp = view()?.querySelector("#tdtellin");
    if (!inp) return;
    if (prefix && !inp.value.startsWith(prefix)) inp.value = prefix + inp.value;
    inp.focus();
    inp.setSelectionRange(inp.value.length, inp.value.length);
  }
  async function tell() {
    const inp = view()?.querySelector("#tdtellin");
    const text = inp?.value.trim();
    if (!text) return focusTell();
    try {
      await call("today_tell", { text });
      inp.value = "";
      if (D.v) D.v.judging = true;
      draw();
    } catch (e) { toast(e); }
  }
  async function forget(id) {
    try { await call("today_forget", { id }); } catch (e) { toast(e); }
    await refresh();
  }

  // Moves the selection. `after`: the row goes away, so keep the place.
  function step(d, after) {
    const list = rows();
    if (!list.length) return;
    const at = list.findIndex((i) => keyOf(i) === D.sel);
    const n = after ? Math.min(list.length - 1, Math.max(0, at + (at === list.length - 1 ? -1 : d))) : Math.min(list.length - 1, Math.max(0, at + d));
    D.sel = keyOf(list[at < 0 ? 0 : n]);
    D.later = null;
    draw();
  }

  async function pickJob(job) {
    if (D.v && job !== D.v.job) invoke("today_looked", { job: D.v.job }).catch(() => {});
    D.job = job;
    save("job", job);
    D.sel = null;
    await refresh();
  }

  document.addEventListener("click", (e) => {
    const el = view();
    if (!el || !el.contains(e.target)) return;
    if (e.target.closest("#tdrefresh")) { call("today_refresh").catch(() => {}); D.v.refreshing = true; draw(); return; }
    const j = e.target.closest("[data-job]");
    if (j) return pickJob(j.dataset.job);
    const fg = e.target.closest("[data-forget]");
    if (fg) return forget(Number(fg.dataset.forget));
    const lt = e.target.closest("[data-later]");
    if (lt) return later(byKey(D.sel), Number(lt.dataset.later));
    const row = e.target.closest("[data-k]");
    const act = e.target.closest("[data-act]")?.dataset.act;
    if (row && row.dataset.k !== D.sel) { D.sel = row.dataset.k; D.later = null; if (!act) return draw(); }
    const it = byKey(D.sel);
    if (act === "done") return done(it);
    if (act === "later") { D.later = D.later ? null : D.sel; return draw(); }
    if (act === "back") return bringBack(it);
    if (act === "open") return open(it);
    if (act === "task") return makeTask(it).catch(() => {});
    if (act === "review") return review(it).catch(() => {});
    if (act === "about" && it) return focusTell(`About ${it.repo}#${it.number} (${it.title}): `);
  });
  document.addEventListener("submit", (e) => {
    if (e.target.id !== "tdtell") return;
    e.preventDefault();
    tell();
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && e.target.id === "tdtellin") { e.target.blur(); view()?.querySelector(".tdmain")?.focus(); }
  });
  document.addEventListener("toggle", (e) => {
    const f = e.target.closest?.("[data-fold]");
    if (f) save(`open.${f.dataset.fold}`, f.open ? "1" : "0");
  }, true);

  window.addEventListener("keydown", (e) => {
    if (S.tab !== "today" || e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.target.matches?.("input, textarea, select") || e.target.closest?.(".xterm, .modal, #launcher")) return;
    if (window.SB.isLeader?.(e)) return;
    const it = byKey(D.sel);
    if (D.g) {
      D.g = false;
      const n = Number(e.key);
      if (n >= 1 && n <= 9 && D.v?.jobs[n - 1]) { e.preventDefault(); pickJob(D.v.jobs[n - 1].name); }
      return;
    }
    if (D.later && ["1", "2", "3"].includes(e.key)) { e.preventDefault(); return later(it, laterTimes()[Number(e.key) - 1][1]); }
    const k = { j: () => step(1), ArrowDown: () => step(1), k: () => step(-1), ArrowUp: () => step(-1), x: () => done(it), l: () => { D.later = D.later ? null : D.sel; draw(); },
      u: () => undo(), o: () => open(it), r: () => review(it).catch(() => {}), t: () => makeTask(it).catch(() => {}), g: () => { D.g = true; }, c: () => focusTell(),
      Escape: () => { if (D.later) { D.later = null; draw(); } } }[e.key];
    if (!k) return;
    e.preventDefault();
    k();
  });

  window.SBToday = {
    D,
    draw: () => { draw(); refresh(); view()?.querySelector(".tdmain")?.focus(); },
    leave: () => { if (D.v) invoke("today_looked", { job: D.v.job }).catch(() => {}); },
    // Your PRs and Reviews of the job, for the rail.
    count: () => (D.v ? D.v.sections.filter((s) => s.key === "mine" || s.key === "reviews").reduce((a, s) => a + s.items.length, 0) : 0),
    refresh,
  };
})();
