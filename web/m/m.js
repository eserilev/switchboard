// The phone screens (SPEC 42). The phone app mounts them after pairing:
//
//   SBM.mount(el, { call, listen, hub })
//
// `call(cmd, args)` runs one hub command and gives a promise of its answer, `listen(event, fn)`
// gets the hub's events, and `hub` is { name, linked } for the bar at the top. The screens hold no
// secret and no state of their own that matters: the hub has it, so the desktop and the phone agree.
(function () {
  "use strict";
  const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
  // A small markdown: paragraphs, **bold**, `code`. Everything else stays text.
  const mdi = (s) => esc(s).replace(/`([^`]+)`/g, "<code>$1</code>").replace(/\*\*([^*]+)\*\*/g, "<b>$1</b>");
  const md = (s) => String(s ?? "").split(/\n\s*\n/).map((p) => `<p>${mdi(p.trim()).replace(/\n/g, "<br>")}</p>`).join("");
  const SEVS = ["must", "should", "nit", "question"];
  const noteKey = (sid, n) => `${sid}|${n.severity}|${n.file || ""}:${n.line || ""}|${n.text}`;
  const LATER = [["Tomorrow", 1], ["Monday", "mon"], ["In a week", 7]];
  const ICON = {
    today: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
    reviews: '<circle cx="6" cy="6" r="2.5"/><circle cx="18" cy="18" r="2.5"/><path d="M6 8.5v9.5M18 15.5V9a3 3 0 0 0-3-3h-4"/>',
    agents: '<rect x="3" y="4" width="18" height="14" rx="2"/><path d="M7 9l3 3-3 3M12 15h5"/>',
    you: '<circle cx="12" cy="8" r="4"/><path d="M4 21c1.5-4 4.5-6 8-6s6.5 2 8 6"/>',
    back: '<path d="M15 5l-7 7 7 7"/>',
  };
  const svg = (k, n = 22) => `<svg width="${n}" height="${n}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">${ICON[k]}</svg>`;

  // One copy per page: a second mount (phone.js and m.js both try) gets the first one,
  // with its hub state updated.
  let mounted = null;
  function mount(root, api) {
    if (mounted) {
      if (api?.hub) mounted.setHub(api.hub);
      if (root !== mounted.root && !root.childElementCount) root.remove();
      return mounted;
    }
    mounted = mountOne(root, api);
    return mounted;
  }
  function mountOne(root, api) {
    const S = { tab: "today", job: null, today: null, reviews: [], panes: [], view: null, diff: new Map(), sel: null, sheet: null, undo: null, hub: api.hub || { name: "hub", linked: true }, open: new Set() };
    root.classList.add("m");
    const call = (cmd, args) => api.call(cmd, args || {});
    const toast = (text, undo) => {
      S.undo = { text, undo };
      draw();
      clearTimeout(S.undoTimer);
      S.undoTimer = setTimeout(() => { S.undo = null; draw(); }, 5000);
    };
    const fail = (e) => toast(String(e?.message || e));

    // ---------- data ----------
    async function loadToday() {
      try { S.today = await call("today", { job: S.job }); } catch (e) { S.today = { error: String(e) }; }
      draw();
    }
    async function loadReviews() {
      try { S.reviews = (await call("reviews")).filter((r) => r.kind !== "local"); } catch (e) { fail(e); }
      draw();
    }
    async function loadPanes() {
      try { S.panes = await call("panes"); } catch (e) { fail(e); }
      draw();
    }
    const review = (id) => S.reviews.find((r) => r.id === id);
    const steps = (r) => r?.guide?.steps || [];
    const checked = (r, sid) => !!(r.steps || []).find((s) => s.id === sid)?.checked;
    const notesOf = (r, sid, take) => {
      const gone = new Set(r.dismissed || []);
      return (take?.notes || []).map((n) => ({ ...n, side: n.side || "new", key: noteKey(sid, n), dismissed: gone.has(noteKey(sid, n)) }));
    };
    async function loadDiff(r, sid) {
      const k = `${r.id}|${sid}|${r.head}`;
      if (!S.diff.has(k)) {
        S.diff.set(k, null);
        try { S.diff.set(k, await call("review_diff", { id: r.id, step: sid })); } catch (e) { S.diff.set(k, { error: String(e), sections: [] }); }
        draw();
      }
      return S.diff.get(k);
    }

    // ---------- the frame ----------
    function hubBar(title, back) {
      const h = S.hub;
      return `<div class="top">${back ? `<button type="button" class="back" data-go="${esc(back)}" aria-label="Back">${svg("back")}</button>` : ""}${title}
        <span class="hub" aria-label="${esc(h.linked ? `Linked to ${h.name}` : `Not linked to ${h.name}`)}"><span class="dot${h.linked ? " on" : ""}"></span>${esc(h.name)}${h.linked ? "" : esc(h.seen ? ` · last seen ${h.seen}` : " · not linked")}</span></div>`;
    }
    function tabs() {
      const waiting = (S.today?.jobs || []).find((j) => j.name === (S.today?.job || "Everything"))?.count || 0;
      const needs = S.panes.filter((p) => p.permit || p.lamp === "needs").length;
      const t = (k, label, badge) => `<button type="button" data-tab="${k}"${S.tab === k ? ' aria-current="page"' : ""}>${svg(k)}${label}${badge ? `<span class="badge">${badge}</span>` : ""}</button>`;
      return `<nav class="tabs" aria-label="Tabs">${t("today", "Today", waiting)}${t("reviews", "Reviews")}${t("agents", "Agents", needs)}${t("you", "You")}</nav>`;
    }

    // ---------- Today (42.3) ----------
    function todayScreen() {
      const v = S.today;
      if (!v) return hubBar("<h1>Today</h1>") + `<div class="body"><div class="empty">Loading…</div></div>`;
      if (v.error) return hubBar("<h1>Today</h1>") + `<div class="body"><div class="empty">${esc(v.error)}</div></div>`;
      const jobs = (v.jobs || []).length > 1 ? `<div class="chips" role="tablist" aria-label="Jobs" style="padding:0 6px 6px">${v.jobs.map((j) => `<button type="button" class="chip" role="tab" data-job="${esc(j.name)}" aria-selected="${j.name === v.job}">${esc(j.name)} ${j.count}</button>`).join("")}</div>` : "";
      const secs = (v.sections || []).filter((s) => s.items.length);
      const rows = secs.map((s) => `<section aria-label="${esc(s.title)}"><h2>${esc(s.title)} · ${s.items.length}</h2>${s.items.map((i) => {
        const key = `${i.repo}#${i.number}#${i.section === "changed" ? "c" : ""}`;
        const tags = (i.tags || []).map((t) => `<span class="tag">${esc(t)}</span>`).join(" ");
        return `<div class="swipe" data-swipe="today" data-key="${esc(key)}"><div class="under"><span class="r">✓ Done</span><span class="l">Later</span></div>
          <button type="button" class="card" data-item="${esc(key)}"><span class="k"><span>${esc(i.repo.split("/").pop())}#${i.number}</span>${tags}</span><span class="t1">${esc(i.line)}</span><span class="t2">${esc(i.title)}</span></button></div>`;
      }).join("")}</section>`).join("");
      const body = rows || `<div class="empty">All done for today.</div>`;
      return hubBar("<h1>Today</h1>") + `<div class="body">${jobs}${body}<p class="t2" style="padding:0 8px">Swipe right: Done · Swipe left: Later</p></div>`;
    }
    const todayItem = (key) => (S.today?.sections || []).flatMap((s) => s.items).find((i) => `${i.repo}#${i.number}#${i.section === "changed" ? "c" : ""}` === key);
    async function done(key) {
      const i = todayItem(key);
      if (!i) return;
      const args = { repo: i.repo, number: i.number, changed: i.section === "changed" };
      try {
        await call("today_done", { ...args, on: true });
        await loadToday();
        haptic();
        toast(`Done: #${i.number}`, () => call("today_done", { ...args, on: false }).then(loadToday));
      } catch (e) { fail(e); }
    }
    function laterSheet(key) {
      const i = todayItem(key);
      if (!i) return;
      S.sheet = { kind: "later", key, html: `<h3>Later: #${i.number}</h3>${LATER.map(([l, v], n) => `<button type="button" class="btn" data-later="${n}">${l}</button>`).join("")}<button type="button" class="btn" data-close>Cancel</button>` };
      draw();
    }
    async function later(key, n) {
      const i = todayItem(key);
      if (!i) return;
      const d = new Date();
      d.setHours(9, 0, 0, 0);
      const v = LATER[n][1];
      if (v === "mon") d.setDate(d.getDate() + (((8 - d.getDay()) % 7) || 7));
      else d.setDate(d.getDate() + v);
      const args = { repo: i.repo, number: i.number, changed: i.section === "changed" };
      S.sheet = null;
      try {
        await call("today_later", { ...args, until: Math.floor(d / 1000) });
        await loadToday();
        toast(`Later: #${i.number}`, () => call("today_later", { ...args, until: null }).then(loadToday));
      } catch (e) { fail(e); }
    }
    function itemSheet(key) {
      const i = todayItem(key);
      if (!i) return;
      const open = S.reviews.find((r) => r.repo === i.repo && r.number === i.number);
      S.sheet = { kind: "item", html: `<h3>${esc(i.repo)}#${i.number}</h3><div class="t1">${esc(i.title)}</div><div class="what">${esc(i.line)}</div>
        ${(i.detail || []).map((d) => `<div class="t2">${esc(d)}</div>`).join("")}${i.why ? `<div class="t2">${esc(i.why)}</div>` : ""}
        <div class="row2">${open ? `<button type="button" class="btn gold" data-openrev="${esc(open.id)}">Open review</button>` : `<button type="button" class="btn gold" data-startrev="${esc(i.url)}">Start review</button>`}<button type="button" class="btn" data-close>Close</button></div>` };
      draw();
    }

    // ---------- Reviews (42.4) ----------
    function reviewsScreen() {
      const list = S.reviews.map((r) => {
        const n = steps(r).length;
        const ok = steps(r).filter((s) => checked(r, s.id)).length;
        const st = r.status === "ready" ? `${n} steps · ${ok} reviewed` : esc(r.status);
        return `<button type="button" class="card" data-openrev="${esc(r.id)}"><span class="k"><span>${esc(r.repo.split("/").pop())}#${r.number}</span><span>${r.drafts.length ? `${r.drafts.length} draft${r.drafts.length > 1 ? "s" : ""}` : ""}</span></span><span class="t1">${esc(r.title)}</span><span class="t2">${st}</span></button>`;
      }).join("");
      return hubBar("<h1>Reviews</h1>") + `<div class="body">${list || `<div class="empty">No open review. Start one from Today.</div>`}</div>`;
    }
    function progress(r, cur) {
      const ss = steps(r);
      return `<div class="prog" aria-hidden="true" style="grid-template-columns:repeat(${Math.max(ss.length, 1)},minmax(0,1fr))">${ss.map((s, i) => `<span class="${i === cur ? "cur" : checked(r, s.id) ? "ok" : ""}"></span>`).join("")}</div>`;
    }
    function overview(r) {
      const ss = steps(r);
      const ok = ss.filter((s) => checked(r, s.id)).length;
      const at = Math.max(0, ss.findIndex((s) => !checked(r, s.id)));
      const take = r.guide?.take;
      const all = [...ss.flatMap((s) => notesOf(r, s.id, s.take)), ...notesOf(r, "pr", take)].filter((n) => !n.dismissed);
      const counts = SEVS.map((v) => [v, all.filter((n) => n.severity === v).length]).filter(([, c]) => c).map(([v, c]) => `<span class="sev sev-${v}">${c} ${v}</span>`).join("");
      const verdict = { good: "looks good", change: "needs a change", question: "has a question" }[take?.verdict] || "";
      const body = r.status !== "ready"
        ? `<div class="empty">${esc(r.status === "error" ? r.error || "The guide failed." : "Claude is writing the guide…")}</div>`
        : `<div class="t1" style="font-size:13px;display:flex;justify-content:space-between"><span>${ss.length} steps · ${ok} reviewed</span><span class="t2">${r.drafts.length} draft${r.drafts.length === 1 ? "" : "s"}</span></div>${progress(r, ok === ss.length ? -1 : at)}
          ${take ? `<section class="take" aria-label="Claude on the PR"><div class="lbl">Claude on the PR${verdict ? ` · ${verdict}` : ""}</div><div class="what">${mdi(take.summary || "")}</div><div class="chips">${counts}</div></section>` : ""}
          <div class="steps">${ss.map((s, i) => `<button type="button" data-step="${i}" class="${checked(r, s.id) ? "done" : ""}${i === at && ok < ss.length ? " cur" : ""}"><span class="n">${checked(r, s.id) ? "✓" : i + 1}</span><span>${mdi(s.title)}</span></button>`).join("")}</div>`;
      const go = r.status !== "ready" ? "" : ok === ss.length ? `<button type="button" class="btn gold" data-finish>Finish review</button>` : `<button type="button" class="btn" data-finish>Finish</button><button type="button" class="btn gold" data-step="${at}" style="flex:2">${at ? `Go on at step ${at + 1}` : "Start at step 1"}</button>`;
      return hubBar(`<span class="mono t2">${esc(r.repo.split("/").pop())}#${r.number} · round ${r.round || 1}</span>`, "reviews")
        + `<div class="body" style="padding:0 18px 16px"><h1>${esc(r.title)}</h1>${body}</div>${go ? `<div class="bar">${go}</div>` : ""}`;
    }
    function stepScreen(r, i) {
      const ss = steps(r);
      const s = ss[i];
      if (!s) return overview(r);
      const d = S.diff.get(`${r.id}|${s.id}|${r.head}`);
      if (d === undefined) loadDiff(r, s.id);
      const long = (s.what || "").length > 420 && !S.open.has(`${r.id}|${s.id}`);
      const qs = (s.check || []).map((q, n) => {
        const a = s.check_answers?.[n];
        return `<div class="q">${a ? "•" : "?"} ${mdi(q)}${a ? `<div class="t2">${mdi(a)}</div>` : ""}</div>`;
      }).join("");
      const qlabel = (s.check || []).length ? `<h2 style="margin:10px 0 2px">${(s.check_answers || []).length ? "What Claude looked into" : "For you to decide"}</h2>` : "";
      const notes = notesOf(r, s.id, s.take);
      const cards = notes.map((n) => noteCard(r, n)).join("");
      const drafts = r.drafts.filter((x) => (d?.sections || []).some((sec) => sec.path === x.path));
      return hubBar(`<span class="t2" style="font-weight:600">Step ${i + 1} of ${ss.length}</span>`, `ov:${r.id}`)
        + `<div class="body" style="padding:0 16px 16px" data-swipe="step">${progress(r, i)}<h1 style="font-size:21px;margin-top:8px">${mdi(s.title)}</h1>
          <div class="what${long ? " short" : ""}">${md(s.what || "")}${s.why ? `<p><b>${mdi(s.why)}</b></p>` : ""}</div>${long ? `<button type="button" class="more" data-more="${esc(s.id)}">More</button>` : ""}
          ${qlabel}${qs}${codeBox(r, s, d)}${drafts.map((x) => `<div class="draft"><div class="t2 mono">${esc(x.path.split("/").pop())}:${x.line}</div>${esc(x.text)}</div>`).join("")}
          ${cards ? `<h2 style="margin:14px 0 6px">Claude's notes</h2>${cards}` : ""}</div>
          <div class="bar"><button type="button" class="btn" style="flex:0 0 52px" data-prev aria-label="Previous step">${svg("back", 20)}</button><button type="button" class="btn" data-ask>Ask</button><button type="button" class="btn" data-comment ${S.sel ? "" : "disabled"}>Comment</button><button type="button" class="btn gold" style="flex:1.5" data-reviewed>${checked(r, s.id) ? "Next ›" : "Reviewed ›"}</button></div>`;
    }
    // The narrow diff (42.4): one column, wrapped lines, a + on each line, folded context.
    function codeBox(r, s, d) {
      if (d === undefined || d === null) return `<div class="code"><div class="fh">Loading the lines…</div></div>`;
      if (d.error) return `<div class="code"><div class="fh">${esc(d.error)}</div></div>`;
      return (d.sections || []).map((sec, si) => {
        if (sec.note) return `<div class="code"><div class="fh">${esc(sec.path)} · ${esc(sec.note)}</div></div>`;
        const add = sec.rows.filter((x) => x.kind === "+").length;
        const del = sec.rows.filter((x) => x.kind === "-").length;
        const keep = sec.rows.map((x, n) => x.kind === "+" || x.kind === "-" || sec.rows.slice(Math.max(0, n - 3), n + 4).some((y) => y.kind === "+" || y.kind === "-"));
        const unfold = S.open.has(`${r.id}|${s.id}|${si}`);
        let out = "";
        let hidden = 0;
        const flush = () => { if (hidden) out += `<button type="button" class="fold" data-unfold="${si}">Show ${hidden} more line${hidden > 1 ? "s" : ""}</button>`; hidden = 0; };
        sec.rows.forEach((x, n) => {
          if (x.kind === "@") return;
          if (!keep[n] && !unfold) { hidden++; return; }
          flush();
          const side = x.kind === "-" ? "old" : "new";
          const ln = side === "old" ? x.old : x.new;
          const sel = S.sel && S.sel.path === sec.path && S.sel.side === side && ln >= S.sel.start && ln <= S.sel.line;
          const dim = S.sel && S.sel.path === sec.path && S.sel.side !== side && x.kind !== " ";
          const cls = { "+": "add", "-": "del", " ": "" }[x.kind] || "";
          out += `<div class="row ${cls}${sel ? " sel" : ""}${dim ? " dim" : ""}"><button type="button" class="plus" data-line="${si}|${side}|${ln}" aria-label="Select line ${ln}">+</button><span class="ln">${ln ?? ""}</span><span class="tx">${esc(x.text)}</span></div>`;
          if (S.sel && S.sel.path === sec.path && S.sel.side === side && ln === S.sel.line) {
            out += `<div class="selbar"><span>${S.sel.start === S.sel.line ? `Line ${ln}` : `Lines ${S.sel.start}–${S.sel.line}`}</span><button type="button" class="btn sm" data-ask>Ask</button><button type="button" class="btn sm gold" data-comment>Comment</button><button type="button" class="btn sm" data-unsel aria-label="Clear the selection">✕</button></div>`;
          }
        });
        flush();
        return `<div class="code"><div class="fh"><span>${esc(sec.path)}</span><span style="margin-left:auto;white-space:nowrap">+${add} −${del}</span></div>${out}</div>`;
      }).join("");
    }
    function noteCard(r, n) {
      if (n.dismissed) return `<div class="note" data-note="${esc(n.key)}"><span class="t2">Dismissed${n.line ? ` · line ${n.line}` : ""}</span><div class="acts"><button type="button" class="btn sm" data-undismiss>Undo</button></div></div>`;
      const added = S.open.has(`added|${n.key}`);
      return `<div class="swipe" data-swipe="note" data-key="${esc(n.key)}"><div class="under"><span class="r">Add as comment</span><span class="l">Dismiss</span></div>
        <div class="note card"><div style="display:flex;gap:8px;align-items:center"><span class="sev sev-${esc(n.severity)}">${esc(n.severity)}</span>${n.file ? `<span class="mono t2">${esc(n.file.split("/").pop())}${n.line ? `:${n.line}` : ""}</span>` : ""}${added ? `<span class="tag">✓ added</span>` : ""}</div>
        <div>${mdi(n.text)}</div><div class="acts">${n.line && !added ? `<button type="button" class="btn sm" data-addnote>Add as comment</button>` : ""}<button type="button" class="btn sm" data-asknote>Ask</button><button type="button" class="btn sm" data-dismiss>Dismiss</button></div></div></div>`;
    }
    const lineText = (d, sel) => {
      const sec = (d?.sections || []).find((x) => x.path === sel.path);
      const row = sec?.rows.find((x) => (sel.side === "old" ? x.old : x.new) === sel.line && (sel.side === "old" ? x.kind === "-" || x.kind === " " : x.kind !== "-"));
      return row?.text || "";
    };
    async function addNote(r, s, key) {
      const n = notesOf(r, s.id, s.take).find((x) => x.key === key);
      if (!n?.line || !n.file) return fail("This note has no line.");
      const d = S.diff.get(`${r.id}|${s.id}|${r.head}`);
      try {
        await call("review_comment", { id: r.id, comment: { path: n.file, side: n.side, line: n.line, start_line: null, text: n.text, head: d?.head || r.head, line_text: lineText(d, { path: n.file, side: n.side, line: n.line }), agent: true } });
        S.open.add(`added|${key}`);
        haptic();
        await loadReviews();
        toast("Added as a draft comment.");
      } catch (e) { fail(e); }
    }
    async function dismiss(r, key, on) {
      try {
        await call("review_dismiss", { id: r.id, note: key, on });
        await loadReviews();
        if (on) toast("Dismissed.", () => call("review_dismiss", { id: r.id, note: key, on: false }).then(loadReviews));
      } catch (e) { fail(e); }
    }
    function commentSheet(r, s) {
      const sel = S.sel;
      if (!sel) return;
      S.sheet = { kind: "comment", html: `<h3>Comment on ${sel.start === sel.line ? `line ${sel.line}` : `lines ${sel.start}–${sel.line}`}</h3><div class="t2 mono">${esc(sel.path)}</div>
        <textarea data-ctext aria-label="Your comment" placeholder="Your comment. It stays a draft until Finish review."></textarea>
        <div class="row2"><button type="button" class="btn" data-close>Cancel</button><button type="button" class="btn gold" data-savecomment>Save draft</button></div>` };
      draw();
      root.querySelector("[data-ctext]")?.focus();
      S.sheet.save = async () => {
        const text = root.querySelector("[data-ctext]")?.value.trim();
        if (!text) return;
        const d = S.diff.get(`${r.id}|${s.id}|${r.head}`);
        try {
          await call("review_comment", { id: r.id, comment: { path: sel.path, side: sel.side, line: sel.line, start_line: sel.start < sel.line ? sel.start : null, text, head: d?.head || r.head, line_text: lineText(d, sel), agent: false } });
          S.sheet = null;
          S.sel = null;
          await loadReviews();
          toast("Saved as a draft.");
        } catch (e) { fail(e); }
      };
    }
    function askSheet(r, s, about) {
      const sel = S.sel;
      const d = S.diff.get(`${r.id}|${s.id}|${r.head}`);
      const anchor = sel ? { path: sel.path, side: sel.side, line: sel.line, text: lineText(d, sel), head: d?.head || r.head } : null;
      S.sheet = { kind: "ask", rid: r.id, thread: null, html: "" };
      const paint = () => {
        const rr = review(r.id);
        const t = S.sheet?.thread && rr?.threads.find((x) => x.id === S.sheet.thread);
        const last = t?.messages.filter((m) => !m.me).at(-1);
        S.sheet.html = `<h3>Ask about ${sel ? `line ${sel.line}` : "this step"}</h3>${t ? `<div class="answer">${last ? mdi(last.text) : "Claude is thinking…"}</div>` : ""}
          <textarea data-qtext aria-label="Your question" placeholder="Ask Claude about this step.">${esc(about || "")}</textarea>
          <div class="row2"><button type="button" class="btn" data-close>Close</button><button type="button" class="btn gold" data-sendask ${t?.busy ? "disabled" : ""}>Ask</button></div>`;
      };
      S.sheet.paint = paint;
      S.sheet.send = async () => {
        const text = root.querySelector("[data-qtext]")?.value.trim();
        if (!text) return;
        try {
          S.sheet.thread = await call("review_ask", { id: r.id, step: s.id, anchor: S.sheet.thread ? null : anchor, thread: S.sheet.thread, text });
          S.sheet.paint();
          draw();
        } catch (e) { fail(e); }
      };
      paint();
      draw();
    }
    async function reviewed(r, i) {
      const s = steps(r)[i];
      try {
        if (!checked(r, s.id)) {
          await call("review_mark", { id: r.id, step: s.id, checked: true });
          haptic();
        }
        await loadReviews();
        S.sel = null;
        const next = i + 1 < steps(r).length ? i + 1 : null;
        S.view = next === null ? { kind: "ov", id: r.id } : { kind: "step", id: r.id, i: next };
        seen(review(r.id), S.view.i);
        draw();
        root.querySelector(".body")?.scrollTo(0, 0);
      } catch (e) { fail(e); }
    }
    // Your place follows you: the desktop opens at the step you were on.
    const seen = (r, i) => { const s = r && steps(r)[i]; if (s) call("review_step_seen", { id: r.id, step: s.id }).catch(() => {}); };
    async function finishSheet(r) {
      S.sheet = { kind: "finish", html: `<h3>Finish review</h3><div class="t2">Loading what goes to GitHub…</div>` };
      draw();
      let p;
      try { p = await call("review_post_preview", { id: r.id }); } catch (e) { S.sheet = null; return fail(e); }
      S.sheet.preview = p;
      S.sheet.event = "COMMENT";
      const paint = () => {
        const drafts = review(r.id)?.drafts || [];
        S.sheet.html = `<h3>Finish review</h3>${p.newer ? `<div class="t2">The PR has newer commits than the reviewed one.</div>` : ""}
          <div class="t2">${drafts.length} comment${drafts.length === 1 ? "" : "s"} as ${esc(p.login)}</div>${drafts.map((x) => `<div class="draft"><div class="t2 mono">${esc((x.path || "").split("/").pop())}${x.line ? `:${x.line}` : ""}</div>${esc(x.text)}</div>`).join("")}
          <div class="verdicts">${[["COMMENT", "Comment"], ["APPROVE", "Approve"], ["REQUEST_CHANGES", "Request changes"]].map(([k, l]) => `<button type="button" class="btn" data-event="${k}" aria-pressed="${S.sheet.event === k}">${l}</button>`).join("")}</div>
          <textarea data-summary aria-label="Summary" placeholder="A summary (optional)">${esc(review(r.id)?.summary || "")}</textarea>
          <div class="row2"><button type="button" class="btn" data-close>Cancel</button><button type="button" class="btn gold" data-send>Send to GitHub</button></div>`;
      };
      S.sheet.paint = paint;
      S.sheet.send = async () => {
        const btn = root.querySelector("[data-send]");
        if (btn) { btn.disabled = true; btn.textContent = "Sending…"; }
        try {
          await call("review_post", { id: r.id, event: S.sheet.event, summary: root.querySelector("[data-summary]")?.value || "", token: p.token });
          S.sheet = null;
          await loadReviews();
          toast("Review sent to GitHub.");
        } catch (e) { if (btn) { btn.disabled = false; btn.textContent = "Send to GitHub"; } fail(e); }
      };
      paint();
      draw();
    }

    // ---------- Agents (42.5) ----------
    function agentsScreen() {
      const claude = S.panes.filter((p) => p.kind === "claude");
      const name = (p) => p.title || p.repo;
      const needs = claude.filter((p) => p.permit || p.lamp === "needs");
      const rest = claude.filter((p) => !needs.includes(p));
      const group = (title, list) => list.length ? `<h2>${title} · ${list.length}</h2>${list.map((p) => `<div class="card"><span class="k"><span>${esc(p.id)} · ${esc(p.repo)}</span><span class="st-${esc(p.lamp)}">${esc(p.lamp === "turn" ? "your turn" : p.lamp)}</span></span><span class="t1">${esc(name(p))}</span>${p.summary ? `<span class="t2">${esc(p.summary)}</span>` : ""}</div>`).join("")}` : "";
      const permits = needs.map((p) => `<article class="permit"><div class="k"><span>${esc(p.id)} · ${esc(p.branch || p.repo)}</span></div><div class="t1">${esc(name(p))} ${p.permit ? "wants to run" : "needs you"}</div>${p.permit ? `<pre>${esc(p.permit.summary)}</pre><div class="row2"><button type="button" class="btn" data-permit="${esc(p.permit.id)}" data-allow="0">Deny</button><button type="button" class="btn gold" data-permit="${esc(p.permit.id)}" data-allow="1">Allow</button></div>` : `<div class="t2">${esc(p.summary || "It waits for you in its pane.")}</div>`}</article>`).join("");
      const working = rest.filter((p) => p.lamp === "working");
      const other = rest.filter((p) => p.lamp !== "working");
      return hubBar("<h1>Agents</h1>") + `<div class="body">${needs.length ? `<h2 style="color:var(--gold)">Needs you · ${needs.length}</h2>${permits}` : ""}${group("Working", working)}${group("Other", other)}${claude.length ? "" : `<div class="empty">No agents on this hub.</div>`}</div>`;
    }
    // ---------- You (42.6) ----------
    function youScreen() {
      const h = S.hub;
      return hubBar("<h1>You</h1>") + `<div class="body"><div class="card"><span class="k"><span>Hub</span></span><span class="t1">${esc(h.name)}</span><span class="t2">${h.linked ? "Linked" : "Not linked"}${h.version ? ` · ${esc(h.version)}` : ""}</span></div>
        <div class="card"><span class="k"><span>This phone</span></span><span class="t2">You manage devices on the computer: <span class="mono">sb devices</span>.</span></div>
        ${api.unpair ? `<button type="button" class="btn" data-unpair>Remove this phone</button>` : ""}</div>`;
    }

    // ---------- drawing and input ----------
    function screen() {
      if (S.view?.kind === "ov") { const r = review(S.view.id); if (r) return overview(r); }
      if (S.view?.kind === "step") { const r = review(S.view.id); if (r) return stepScreen(r, S.view.i); }
      S.view = null;
      return { today: todayScreen, reviews: reviewsScreen, agents: agentsScreen, you: youScreen }[S.tab]() + tabs();
    }
    function draw() {
      if (S.sheet?.paint) S.sheet.paint();
      const y = root.querySelector(".body")?.scrollTop || 0;
      const typed = root.querySelector(".sheet textarea")?.value;
      root.innerHTML = screen()
        + (S.sheet ? `<div class="scrim" data-scrim><div class="sheet" role="dialog" aria-modal="true">${S.sheet.html}</div></div>` : "")
        + (S.undo ? `<div class="undo" role="status"><span>${esc(S.undo.text)}</span>${S.undo.undo ? `<button type="button" class="btn sm" data-undo>Undo</button>` : ""}</div>` : "");
      const b = root.querySelector(".body");
      if (b) b.scrollTop = y;
      const ta = root.querySelector(".sheet textarea");
      if (ta && typed !== undefined) ta.value = typed;
    }
    const haptic = () => { try { navigator.vibrate?.(10); } catch (_) {} };
    const cur = () => {
      const r = S.view && review(S.view.id);
      return { r, s: r && S.view.kind === "step" ? steps(r)[S.view.i] : null };
    };

    root.addEventListener("click", (e) => {
      const t = e.target.closest("button, [data-go]");
      if (!t || t.disabled) {
        if (e.target.matches("[data-scrim]")) { S.sheet = null; draw(); }
        return;
      }
      const d = t.dataset;
      const { r, s } = cur();
      if (d.tab) { S.tab = d.tab; S.view = null; S.sel = null; if (d.tab === "today") loadToday(); if (d.tab === "reviews") loadReviews(); if (d.tab === "agents") loadPanes(); draw(); return; }
      if (d.go) { S.sel = null; if (d.go.startsWith("ov:")) S.view = { kind: "ov", id: d.go.slice(3) }; else { S.view = null; S.tab = d.go; } draw(); return; }
      if (d.job !== undefined) { S.job = d.job; loadToday(); return; }
      if (d.item) return itemSheet(d.item);
      if (d.close !== undefined) { S.sheet = null; draw(); return; }
      if (d.later) return later(S.sheet.key, +d.later);
      if (d.undo !== undefined) { const u = S.undo?.undo; S.undo = null; draw(); u?.(); return; }
      if (d.openrev) { S.sheet = null; S.view = { kind: "ov", id: d.openrev }; S.tab = "reviews"; draw(); return; }
      if (d.startrev) { S.sheet = null; call("review_open", { url: d.startrev, round_of: null }).then((id) => loadReviews().then(() => { S.view = { kind: "ov", id }; S.tab = "reviews"; draw(); })).catch(fail); return; }
      if (d.step !== undefined && r) { S.view = { kind: "step", id: r.id, i: +d.step }; S.sel = null; seen(r, +d.step); draw(); root.querySelector(".body")?.scrollTo(0, 0); return; }
      if (d.finish !== undefined && r) return finishSheet(r);
      if (d.event && S.sheet?.kind === "finish") { S.sheet.event = d.event; draw(); return; }
      if (d.send !== undefined && S.sheet?.send) return S.sheet.send();
      if (d.permit) { call("permit_answer", { permit: d.permit, allow: d.allow === "1" }).then(() => { haptic(); loadPanes(); }).catch(fail); return; }
      if (d.unpair !== undefined) { api.unpair?.(); return; }
      if (!r || !s) return;
      const i = S.view.i;
      if (d.more) { S.open.add(`${r.id}|${s.id}`); draw(); return; }
      if (d.unfold) { S.open.add(`${r.id}|${s.id}|${d.unfold}`); draw(); return; }
      if (d.line) {
        const [si, side, ln] = d.line.split("|");
        const dd = S.diff.get(`${r.id}|${s.id}|${r.head}`);
        const path = dd.sections[+si].path;
        const n = +ln;
        // A second tap on the same side grows the selection to that line.
        if (S.sel && S.sel.path === path && S.sel.side === side && n !== S.sel.start) S.sel = { path, side, start: Math.min(S.sel.start, n), line: Math.max(S.sel.start, n) };
        else S.sel = S.sel && S.sel.path === path && S.sel.start === n && S.sel.line === n ? null : { path, side, start: n, line: n };
        draw();
        return;
      }
      if (d.unsel !== undefined) { S.sel = null; draw(); return; }
      if (d.comment !== undefined) return commentSheet(r, s);
      if (d.savecomment !== undefined) return S.sheet?.save?.();
      if (d.ask !== undefined) return askSheet(r, s);
      if (d.sendask !== undefined) return S.sheet?.send?.();
      if (d.prev !== undefined) { S.sel = null; S.view = i ? { kind: "step", id: r.id, i: i - 1 } : { kind: "ov", id: r.id }; draw(); return; }
      if (d.reviewed !== undefined) return reviewed(r, i);
      const key = t.closest("[data-key], [data-note]")?.dataset;
      const nk = key?.key || key?.note;
      if (d.addnote !== undefined) return addNote(r, s, nk);
      if (d.dismiss !== undefined) return dismiss(r, nk, true);
      if (d.undismiss !== undefined) return dismiss(r, nk, false);
      if (d.asknote !== undefined) { const n = notesOf(r, s.id, s.take).find((x) => x.key === nk); S.sel = n?.line && n.file ? { path: n.file, side: n.side, start: n.line, line: n.line } : S.sel; return askSheet(r, s, n ? `About your note "${n.text}": ` : ""); }
    });

    // Swipes: a row follows the finger; past 80 px it acts. Every swipe also has a button.
    let drag = null;
    root.addEventListener("pointerdown", (e) => {
      const row = e.target.closest("[data-swipe]");
      if (!row || e.target.closest("textarea")) return;
      drag = { row, x: e.clientX, y: e.clientY, dx: 0, live: false };
    });
    root.addEventListener("pointermove", (e) => {
      if (!drag) return;
      drag.dx = e.clientX - drag.x;
      if (!drag.live && Math.abs(drag.dx) > 12 && Math.abs(drag.dx) > Math.abs(e.clientY - drag.y)) { drag.live = true; drag.row.classList.add("drag"); }
      const card = drag.row.querySelector(":scope > .card");
      if (drag.live && card) card.style.transform = `translateX(${drag.dx}px)`;
    });
    const end = () => {
      if (!drag) return;
      const { row, dx, live } = drag;
      drag = null;
      row.classList.remove("drag");
      const card = row.querySelector(":scope > .card");
      if (card) card.style.transform = "";
      if (!live || Math.abs(dx) < 80) return;
      const kind = row.dataset.swipe;
      const { r, s } = cur();
      if (kind === "today") return dx > 0 ? done(row.dataset.key) : laterSheet(row.dataset.key);
      if (kind === "note" && r && s) return dx > 0 ? addNote(r, s, row.dataset.key) : dismiss(r, row.dataset.key, true);
      if (kind === "step" && r && dx < 0) return reviewed(r, S.view.i);
    };
    root.addEventListener("pointerup", end);
    root.addEventListener("pointercancel", () => { if (drag) { drag.row.classList.remove("drag"); const c = drag.row.querySelector(":scope > .card"); if (c) c.style.transform = ""; } drag = null; });
    // A tap on a swiped row is no tap.
    root.addEventListener("click", (e) => { if (e.target.closest(".drag")) e.stopPropagation(); }, true);

    // ---------- the hub's events ----------
    api.listen?.("review", (e) => {
      const v = e.payload ?? e;
      if (!v?.id) return;
      const at = S.reviews.findIndex((r) => r.id === v.id);
      if (v.kind === "local") return;
      if (at < 0) S.reviews.push(v); else S.reviews[at] = v;
      draw();
    });
    api.listen?.("review_closed", (e) => { const id = e.payload ?? e; S.reviews = S.reviews.filter((r) => r.id !== (id.id || id)); draw(); });
    api.listen?.("panes", (e) => { const v = e.payload ?? e; if (Array.isArray(v)) { S.panes = v; draw(); } else loadPanes(); });
    api.listen?.("today", () => loadToday());

    loadToday();
    loadReviews();
    loadPanes();
    draw();
    return {
      root,
      setHub(h) { S.hub = { ...S.hub, ...h }; draw(); },
      state: S,
    };
  }
  window.SBM = { mount };

  // On a phone the screens come up with the link, over the desktop page. The mount is
  // here (web/phone.js does pairing and the link only). A page that has them already
  // gets no second copy.
  let ui = null;
  const linkOf = (l) => ({ name: l?.host || l?.hub || "hub", linked: l?.up !== false, version: l?.version });
  function onPhone(link) {
    if (ui) return ui.setHub(linkOf(link));
    const T = window.__TAURI__;
    if (!T) return;
    const el = document.createElement("div");
    el.id = "sbm";
    document.body.appendChild(el);
    document.body.classList.add("sbm-on");
    ui = mount(el, {
      call: (cmd, args) => (window.SBPhone?.call || T.core.invoke)(cmd, args),
      listen: (ev, fn) => T.event.listen(ev, fn),
      hub: linkOf(link),
      unpair: () => T.core.invoke("phone_unpair").then(() => location.reload()),
    });
  }
  if (window.SBPhone?.ready) window.SBPhone.ready(onPhone);
  else document.addEventListener("sb:phone-ready", (e) => onPhone(e.detail));
})();
