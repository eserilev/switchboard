// Review tabs (SPEC 11): steps, guide + diff, a Claude Code pane and drafts.

(() => {
  const { S, call, invoke, esc, toast, $ } = window.SB;
  const R = new Map(); // review id -> { cur, anchor, term (the Claude pane), diff: Map(step -> view) }
  // Your place in each review survives a restart: the step, Guide or Files, the file.
  const placeKey = (id) => `sb.review.place.${id}`;
  const loadPlace = (id) => { try { return JSON.parse(localStorage.getItem(placeKey(id)) || "{}"); } catch (_) { return {}; } };
  const savePlace = (id) => {
    const l = R.get(id);
    if (!l) return;
    try { localStorage.setItem(placeKey(id), JSON.stringify({ cur: l.cur, mode: l.mode, file: l.file, viewed: [...l.viewed], vhead: l.vhead })); } catch (_) {}
  };
  const local = (id) => {
    if (!R.has(id)) {
      const p = loadPlace(id);
      R.set(id, { cur: p.cur || 0, anchor: null, term: null, diff: new Map(), loading: new Set(),
        // Guide shows the steps; Files shows every changed file, like GitHub's "Files changed".
        full: new Map(), shown: new Map(),
        mode: p.mode === "files" ? "files" : "guide", file: p.file || null, files: null, viewed: new Set(p.viewed || []), vhead: p.vhead || null });
    }
    return R.get(id);
  };
  const view = () => $("#view-review");
  const steps = (r) => r.guide?.steps || [];
  const stepState = (r, sid) => r.steps.find((s) => s.id === sid) || { checked: false, stale: false };
  const short = (h) => (h || "").slice(0, 9);

  // Markdown from the agent. Raw HTML in it is shown as text, never run.
  const mdr = new marked.Renderer();
  mdr.html = (h) => esc(typeof h === "string" ? h : h?.text ?? "");
  const md = (s) => { try { return marked.parse(String(s ?? ""), { renderer: mdr, gfm: true }); } catch (_) { return esc(s); } };
  const mdi = (s) => { try { return marked.parseInline(String(s ?? ""), { renderer: mdr, gfm: true }); } catch (_) { return esc(s); } };
  // Links open in the browser, not in the app window.
  document.addEventListener("click", (e) => {
    const a = e.target.closest?.(".md a, #view-review a[href]");
    if (!a) return;
    e.preventDefault();
    const href = a.getAttribute("href") || "";
    if (/^https?:\/\//.test(href)) invoke("open_url", { url: href }).catch((err) => toast(err));
  });

  function draw(id) {
    const r = S.reviews.get(id);
    if (!r) return;
    const l = local(id);
    // The live terminal can sit in the old markup: take it out first.
    window.SB.parkLive();
    view().innerHTML = `
      <div class="rhead" id="rhead"></div>
      <div class="rcols">
        <div class="col"><ol class="steps" id="rsteps"></ol></div>
        <div class="col center" id="rcenter"><div class="guide" id="rguide"></div><div id="rdiff"></div></div>
        <div class="col">
          <div class="tbar"><span class="tlabel">${r.kind === "local" ? "Agent" : "Claude"}</span>${r.kind === "local" ? "" : `<button class="btn small" type="button" id="rstep" title="Type the current step into Claude's prompt">Send step</button>`}</div>
          <div class="askhost" id="rterm"><div class="askwait" id="raskwait"></div></div>
          <div class="drafts" id="rdrafts"></div>
        </div>
      </div>
      <div class="finish" id="rfinish" hidden></div>`;
    if ($("#rstep")) $("#rstep").onclick = () => sendStep(id);
    $("#raskwait").onclick = (e) => { if (e.target.closest("[data-show]")) showTerm(id); };
    l.term = null;
    update(r, true);
    openTerm(id);
  }

  function update(r, fresh) {
    if (S.tab !== r.id) return;
    if (!fresh && !$("#rsteps")) return draw(r.id);
    const l = local(r.id);
    // A review of local commits has no guide: only Files (SPEC 11.11).
    if (r.kind === "local") l.mode = "files";
    // A new agent pane for the folder: ask for it again.
    if (r.kind === "local" && l.pane !== r.pane) { l.pane = r.pane; l.noTerm = false; if (l.term !== r.pane) l.term = null; }
    l.cur = Math.min(l.cur, Math.max(0, steps(r).length - 1));
    // Viewed marks belong to one head. New commits clear them, as on GitHub.
    if (l.vhead !== r.head) { l.viewed = new Set(); l.vhead = r.head; }
    drawHead(r);
    if (l.mode === "files") {
      // Not while the commits load: the old model would answer.
      if (!l.files && r.head && r.status === "ready") loadFiles(r.id);
      drawFiles(r);
      drawFileHead(r);
    } else {
      drawSteps(r);
      drawGuide(r);
    }
    if (!l.term && !l.opening && !l.noTerm && r.status === "ready") openTerm(r.id);
    drawTermWait(r);
    drawDrafts(r);
    const key = diffKey(r);
    if (key && !l.diff.has(key)) loadDiff(r.id);
    else drawDiff(r);
  }

  // The key of the diff on screen: a step id, or "file:" and a path.
  function diffKey(r) {
    const l = local(r.id);
    if (l.mode === "files") return l.file ? `file:${l.file}` : null;
    return steps(r)[l.cur]?.id || null;
  }

  const curFile = (l) => (l.files || []).find((f) => f.path === l.file);

  async function loadFiles(id) {
    const l = local(id);
    if (l.loading.has("files")) return;
    l.loading.add("files");
    try {
      l.files = await invoke("review_files", { id });
    } catch (e) {
      l.files = [];
      toast(e);
    }
    l.loading.delete("files");
    if (!curFile(l)) l.file = l.files[0]?.path || null;
    const r = S.reviews.get(id);
    if (r && S.tab === id) update(r);
  }

  function setMode(id, mode) {
    const l = local(id);
    if (l.mode === mode || S.reviews.get(id)?.kind === "local") return;
    l.mode = mode;
    l.anchor = null;
    l.compose = null;
    l.placing = null;
    savePlace(id);
    update(S.reviews.get(id));
  }

  function goFile(id, i) {
    const l = local(id);
    const files = l.files || [];
    if (!files.length) return;
    const f = files[Math.max(0, Math.min(files.length - 1, i))];
    if (f.path === l.file) return;
    l.file = f.path;
    l.anchor = null;
    l.compose = null;
    savePlace(id);
    update(S.reviews.get(id));
    $(`#rsteps [data-f="${CSS.escape(f.path)}"]`)?.scrollIntoView({ block: "nearest" });
  }

  const fileIndex = (l) => (l.files || []).findIndex((f) => f.path === l.file);

  function toggleViewed(id, path) {
    const l = local(id);
    if (l.viewed.has(path)) l.viewed.delete(path);
    else l.viewed.add(path);
    savePlace(id);
    update(S.reviews.get(id));
  }

  function drawFiles(r) {
    const l = local(r.id);
    if (!l.files) { $("#rsteps").innerHTML = `<li class="empty-review">Loading the files…</li>`; return; }
    const stepNo = (sid) => steps(r).findIndex((x) => x.id === sid) + 1;
    const head = `<li class="fhead">${l.files.length} file${l.files.length === 1 ? "" : "s"} · ${l.viewed.size} viewed</li>`;
    $("#rsteps").innerHTML = head + l.files.map((f) => {
      const slash = f.path.lastIndexOf("/");
      const dir = slash >= 0 ? f.path.slice(0, slash + 1) : "";
      const name = f.path.slice(slash + 1);
      const nums = f.steps.map(stepNo).filter((n) => n > 0);
      const n = r.threads.filter((t) => t.path === f.path).length;
      return `<li class="fitem${l.viewed.has(f.path) ? " viewed" : ""}" tabindex="0" data-f="${esc(f.path)}" ${f.path === l.file ? 'aria-current="step"' : ""}>
        <span class="vbox" aria-hidden="true">${l.viewed.has(f.path) ? "✓" : ""}</span>
        <div class="fbody" title="${esc(f.path)}"><div class="fname">${esc(name)}</div>${dir ? `<div class="fdir">${esc(dir)}</div>` : ""}
        <div class="fmeta"><span class="plus">+${f.added}</span><span class="minus">−${f.removed}</span>${nums.length ? `<span>step ${nums.join(", ")}</span>` : ""}${n ? `<span class="badge th">${n}</span>` : ""}${f.note ? `<span class="fnote">${esc(f.note)}</span>` : ""}</div></div></li>`;
    }).join("");
    $("#rsteps").onclick = (e) => {
      const it = e.target.closest(".fitem");
      if (it) goFile(r.id, l.files.findIndex((f) => f.path === it.dataset.f));
    };
  }

  function drawFileHead(r) {
    const l = local(r.id);
    const f = curFile(l);
    if (!f) { $("#rguide").innerHTML = ""; return; }
    const i = fileIndex(l);
    const chips = f.steps.map((sid) => {
      const si = steps(r).findIndex((x) => x.id === sid);
      return si >= 0 ? `<button class="chip link" data-step="${si}">Step ${si + 1}: ${mdi(steps(r)[si].title)}</button>` : "";
    }).join("");
    $("#rguide").innerHTML = `
      <div class="eyebrow">File ${i + 1} of ${l.files.length}</div>
      <h4 class="fpath">${esc(f.path)}</h4>
      <div class="fmeta big"><span class="plus">+${f.added}</span><span class="minus">−${f.removed}</span>${f.old_path && f.old_path !== f.path ? `<span>from ${esc(f.old_path)}</span>` : ""}${f.note ? `<span class="fnote">${esc(f.note)}</span>` : ""}</div>
      ${chips ? `<div class="ctx"><span class="clabel">In the guide:</span>${chips}</div>` : ""}
      <div class="gactions">
        <button class="btn" data-act="viewed">${l.viewed.has(f.path) ? "Unmark viewed" : "Mark viewed"}</button>
        ${i < l.files.length - 1 ? `<button class="btn primary" data-act="nextfile">Next file</button>` : ""}
      </div>`;
    $("#rguide").onclick = (e) => {
      const b = e.target.closest("button");
      if (!b) return;
      if (b.dataset.step) { setMode(r.id, "guide"); go(r.id, +b.dataset.step); }
      if (b.dataset.act === "viewed") toggleViewed(r.id, f.path);
      if (b.dataset.act === "nextfile") nextFile(r.id);
    };
  }

  function nextFile(id) {
    const l = local(id);
    if (l.file && !l.viewed.has(l.file)) { l.viewed.add(l.file); savePlace(id); }
    const i = fileIndex(l);
    if (i < (l.files || []).length - 1) goFile(id, i + 1);
    else update(S.reviews.get(id));
  }

  // The result of the verified checker, in one line.
  function coverageLine(r) {
    const c = r.coverage;
    if (!c) return "";
    const what = `${c.changed_lines} changed lines in ${c.files} files`;
    if (!c.accepted) return `<span class="cov">${esc(what)}</span>`;
    const missed = c.missed_lines ? ` · ${c.missed_lines} added by the app` : "";
    const gh = c.github?.length ? `<span class="cov warn" title="${esc(c.github.join("\n"))}">${c.github.length} count${c.github.length > 1 ? "s" : ""} differ from GitHub</span>` : "";
    return `<span class="cov ok" title="Every changed line is in a step. The checker is proved in Lean.">✓ All ${esc(what)} covered${esc(missed)}</span>${gh}`;
  }

  function drawHead(r) {
    if (r.kind === "local") return drawLocalHead(r);
    const status = { fetching: "Fetching the PR", writing: "Writing the guide", updating: "Updating the guide", error: "Error", ready: "" }[r.status] ?? r.status;
    const next = (r.round || 1) + 1;
    const newHead = r.new_head ? `<span class="newhead">New commits: ${esc(short(r.head))} → ${esc(short(r.new_head))}<button class="btn small primary" data-act="round" title="A new review of only the changes since ${esc(short(r.head))}">Start round ${next}</button><button class="btn small" data-act="update" title="Write this guide again for the new head">Update guide</button></span>` : "";
    // A later round covers only the changes since the round before.
    const round = r.round > 1 ? `<span class="roundtag" title="${esc(r.scope_note || "A plain diff from the commit that the round before reviewed.")}">Round ${r.round} · changes since ${esc(short(r.since))}${r.scope_note ? " ⚠" : ""}</span>` : "";
    const retry = r.status === "error" ? `<button class="btn small" data-act="retry">Retry</button>` : "";
    $("#rhead").innerHTML = `<div><div class="t">${esc(r.title)}</div><div class="m">${esc(r.repo)} #${esc(r.number)} · ${esc(short(r.head))}${r.tree ? " · " + esc(r.tree) : ""}</div></div>
      ${round}${status ? `<span class="status${r.status === "error" ? " error" : ""}">${esc(status)}</span>` : ""}${coverageLine(r)}${newHead}
      <div class="right"><div class="seg" id="rmode" title="Switch with f"><button type="button" data-mode="guide" aria-pressed="${local(r.id).mode === "guide"}">Guide</button><button type="button" data-mode="files" aria-pressed="${local(r.id).mode === "files"}">Files</button></div>${retry}<button class="btn small" data-act="close">Close review</button></div>
      ${r.error ? `<div class="rerror" style="flex-basis:100%;padding:0">${esc(r.error)}</div>` : ""}
      ${r.round > 1 && r.scope_note ? `<div class="scopenote">${esc(r.scope_note)}</div>` : ""}`;
    $("#rhead").onclick = async (e) => {
      const mode = e.target.closest("[data-mode]")?.dataset.mode;
      if (mode) return setMode(r.id, mode);
      const act = e.target.closest("[data-act]")?.dataset.act;
      if (act === "update") { local(r.id).diff.clear(); local(r.id).files = null; call("review_update", { id: r.id }); }
      if (act === "retry") call("review_retry", { id: r.id });
      if (act === "round") {
        try {
          const nid = await invoke("review_round", { id: r.id });
          if (!S.reviews.has(nid)) S.reviews.set(nid, await invoke("review_view", { id: nid }));
          window.SB.setTab(nid);
        } catch (err) { toast(err); }
      }
      if (act === "close") closeReview(r.id);
    };
  }

  // The header of a review of local commits: the range, what it compares with, and a
  // Refresh when the agent made new commits.
  function drawLocalHead(r) {
    const status = { fetching: "Reading the commits", error: "Error", ready: "" }[r.status] ?? r.status;
    const range = r.head ? `${esc(short(r.base))}..${esc(short(r.head))}` : "";
    const newHead = r.new_head ? `<span class="newhead">New commits: ${esc(short(r.head))} → ${esc(short(r.new_head))}<button class="btn small primary" data-act="update" title="Read the commits again; your comments move with their code">Refresh</button></span>` : "";
    const retry = r.status === "error" ? `<button class="btn small" data-act="retry">Retry</button>` : "";
    $("#rhead").innerHTML = `<div><div class="t">${esc(r.title || "Local commits")}</div><div class="m">${esc(r.repo)} · ${range}${r.tree ? " · " + esc(r.tree) : ""}</div></div>
      ${status ? `<span class="status${r.status === "error" ? " error" : ""}">${esc(status)}</span>` : ""}${newHead}
      <div class="right">${r.new_head ? "" : `<button class="btn small" data-act="update" title="Read the commits again">Refresh</button>`}${retry}<button class="btn small" data-act="close">Close review</button></div>
      ${r.error ? `<div class="rerror" style="flex-basis:100%;padding:0">${esc(r.error)}</div>` : ""}
      ${r.scope_note ? `<div class="scopenote">${esc(r.scope_note)}</div>` : ""}`;
    $("#rhead").onclick = (e) => {
      const act = e.target.closest("[data-act]")?.dataset.act;
      if (act === "update" || act === "retry") { local(r.id).diff.clear(); local(r.id).files = null; call(act === "update" ? "review_update" : "review_retry", { id: r.id }); }
      if (act === "close") closeReview(r.id);
    };
  }

  function drawSteps(r) {
    const l = local(r.id);
    $("#rsteps").innerHTML = steps(r).map((s, i) => {
      const st = stepState(r, s.id);
      const n = r.threads.filter((t) => t.step === s.id).length;
      const rs = s.ranges || [];
      const loc = rs.length ? `${rs[0].file}:${rs[0].from}-${rs[0].to}${rs.length > 1 ? ` +${rs.length - 1}` : ""}` : (s.files || []).join(", ");
      const badges = [n ? `<span class="badge th">${n} thread${n > 1 ? "s" : ""}</span>` : "", st.stale ? `<span class="badge stale">stale</span>` : "", s.auto ? `<span class="badge stale">added by the app</span>` : ""].join("");
      return `<li class="step${st.checked ? " checked" : ""}" tabindex="0" data-i="${i}" ${i === l.cur ? 'aria-current="step"' : ""}>
        <span class="n">${st.checked && i !== l.cur ? "✓" : i + 1}</span>
        <div><div class="st">${mdi(s.title)}</div>${loc ? `<div class="loc" title="${esc(loc)}">${esc(loc)}</div>` : ""}${badges ? `<div class="badges">${badges}</div>` : ""}</div></li>`;
    }).join("") || `<li class="empty-review">${r.status === "error" ? "" : "The guide shows here when the agent is done."}</li>`;
    $("#rsteps").onclick = (e) => { const s = e.target.closest(".step"); if (s) go(r.id, +s.dataset.i); };
  }

  function drawGuide(r) {
    const l = local(r.id);
    const s = steps(r)[l.cur];
    if (!s) { $("#rguide").innerHTML = ""; return; }
    const st = stepState(r, s.id);
    const pins = r.pins.filter((p) => p.step === s.id);
    const ctx = [...(s.context || []), ...(s.files || []).map((f) => `binary: ${f}`)];
    $("#rguide").innerHTML = `
      <div class="eyebrow">Step ${l.cur + 1} of ${steps(r).length}</div>
      <h4>${mdi(s.title)}</h4>
      ${s.what ? `<div class="what md">${md(s.what)}</div>` : ""}
      ${s.check?.length ? `<div class="check md"><ul>${s.check.map((c) => `<li>${mdi(c)}</li>`).join("")}</ul></div>` : ""}
      ${pins.map((p) => `<div class="pinned" data-pin="${p.id}"><span class="md" contenteditable="true" data-raw="${esc(p.text)}">${mdi(p.text)}</span><button class="tool x" data-unpin="${p.id}" aria-label="Remove">×</button></div>`).join("")}
      ${ctx.length ? `<div class="ctx">${ctx.map((c) => `<span class="chip">${esc(typeof c === "string" ? c : c.label || c.ref || "")}</span>`).join("")}</div>` : ""}
      <div class="gactions">
        <button class="btn" data-act="mark">${st.checked ? "Unmark" : "Mark reviewed"}</button>
        ${l.cur < steps(r).length - 1 ? `<button class="btn primary" data-act="next">Next step</button>` : ""}
      </div>`;
    $("#rguide").onclick = (e) => {
      const b = e.target.closest("button");
      if (!b) return;
      if (b.dataset.act === "mark") call("review_mark", { id: r.id, step: s.id, checked: !st.checked });
      if (b.dataset.act === "next") next(r.id);
      if (b.dataset.unpin) call("review_pin_edit", { id: r.id, pin: +b.dataset.unpin, text: null });
    };
    $("#rguide").querySelectorAll("[data-pin] span").forEach((el) => {
      // Edit the markdown source, not the rendered text.
      el.addEventListener("focus", () => { el.textContent = el.dataset.raw; });
      el.addEventListener("blur", () => call("review_pin_edit", { id: r.id, pin: +el.parentElement.dataset.pin, text: el.textContent.trim() || null }));
      el.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); el.blur(); } e.stopPropagation(); });
    });
  }

  async function loadDiff(id) {
    const l = local(id);
    const key = diffKey(S.reviews.get(id));
    if (!key || l.diff.has(key) || l.loading.has(key)) return;
    l.loading.add(key);
    try {
      l.diff.set(key, key.startsWith("file:")
        ? await invoke("review_file_diff", { id, path: key.slice(5) })
        : await invoke("review_diff", { id, step: key }));
    } catch (e) {
      l.diff.set(key, { sections: [], error: String(e) });
    }
    l.loading.delete(key);
    const r = S.reviews.get(id);
    if (S.tab === id && diffKey(r) === key) drawDiff(r);
  }

  function drawDiff(r) {
    const l = local(r.id);
    const s = l.mode === "files" ? null : steps(r)[l.cur];
    const key = diffKey(r);
    const d = key && l.diff.get(key);
    // A view of an older head never takes a click: refetch until the server has the
    // review's head. (During an update the server loads the new commit for a while.)
    if (d && d.head && r.head && d.head !== r.head) {
      l.diff.delete(key);
      clearTimeout(l.retry);
      // A failed update keeps the old model: show the error, and stop asking.
      if (r.status === "error") { $("#rdiff").innerHTML = `<div class="rerror">${esc(r.error || "The new commit did not load. Use Retry.")}</div>`; return; }
      $("#rdiff").innerHTML = `<div class="rnote">Loading the new commit…</div>`;
      l.retry = setTimeout(() => loadDiff(r.id), 800);
      return;
    }
    if (!d || !d.sections?.length) { $("#rdiff").innerHTML = d?.error ? `<div class="rerror">${esc(d.error)}</div>` : ""; return; }
    const html = d.sections.map((sec, si) => {
      const inRange = (side, num) => sec.ranges.some((g) => g.side === side && num >= g.from && num <= g.to);
      const anchored = new Set(r.threads.filter((t) => (!s || t.step === s.id) && t.line && t.path === sec.path).map((t) => `${t.side || "new"}:${t.line}`));
      const pathOf = (side) => (side === "old" ? sec.old_path || sec.path : sec.path);
      const c = l.compose && l.compose.sec === sec.path ? l.compose : null;
      const rows = items(l, key, sec).map((it) => {
        if (it.gap) return gapRow(si, it);
        const row = it.row;
        if (row.kind === "@") return `<tr class="hunk"><td class="cm"></td><td class="ln"></td><td class="src">${esc(row.text)}</td></tr>`;
        const rowSide = row.kind === "-" ? "old" : "new";
        const num = rowSide === "old" ? row.old : row.new;
        // A context line is in a range on either side.
        const focus = inRange(rowSide, num) || (row.kind === " " && inRange("old", row.old));
        const isAnchor = l.anchor && l.anchor.path === sec.path && l.anchor.side === rowSide && l.anchor.line === num;
        const sel = c && c.side === rowSide && num >= c.start && num <= c.line;
        // The color comes only from the row kind: "-" removed, "+" added, " " context.
        const cls = [KIND_CLASS[row.kind], focus ? "focus" : "", isAnchor ? "anchor" : "", anchored.has(`${rowSide}:${num}`) ? "anchored" : "", sel ? "sel" : ""].join(" ");
        const sign = row.kind === " " ? " " : row.kind;
        // A stale draft has no line at this head: it shows only in the list.
        const mine = r.drafts.filter((x) => !x.stale && x.path === pathOf(rowSide) && (x.side || "new") === rowSide && x.line === num);
        const cards = mine.map((x) => `<tr class="drow"><td colspan="3"><div class="dcard${x.agent ? " agent" : ""}" data-d="${x.id}">
          <div class="dmeta"><span class="who">${x.agent ? "Agent draft" : "Your comment"}</span>${x.start_line ? `<span>lines ${x.start_line}-${x.line}</span>` : ""}<button class="tool x" data-ddel="${x.id}" aria-label="Delete comment">×</button></div>
          <div class="dtext" contenteditable="true">${esc(x.text)}</div></div></td></tr>`).join("");
        const form = c && c.side === rowSide && c.line === num ? `<tr class="crow"><td colspan="3"><form class="cform" id="rcform">
          <div class="cat">Comment on ${esc(c.path)}:${c.start < c.line ? `${c.start}-${c.line}` : c.line}${c.side === "old" ? " (old)" : ""} <span class="hint">Shift-click a + to choose a range</span></div>
          <textarea id="rctext" placeholder="Leave a comment">${esc(c.text || "")}</textarea>
          <div class="cbtns"><button type="button" class="btn small" data-ccancel>Cancel</button><button type="submit" class="btn small primary">Add to review</button></div>
        </form></td></tr>` : "";
        return `<tr class="${cls}"><td class="cm"><button type="button" class="cmb" data-cm data-sec="${si}" data-side="${rowSide}" data-n="${num}" aria-label="Comment on line ${num}">+</button></td><td class="ln" data-sec="${si}" data-side="${rowSide}" data-ln="${num}">${num}</td><td class="src"><span class="sg">${sign}</span>${esc(row.text)}</td></tr>${cards}${form}`;
      }).join("");
      const moved = sec.old_path && sec.old_path !== sec.path ? ` <span class="ctxnote">from ${esc(sec.old_path)}</span>` : "";
      const note = sec.note ? ` <span class="ctxnote">${esc(sec.note)}</span>` : "";
      // A step shows only its part of the file. This bar names the other changes.
      const o = sec.other;
      const links = o ? o.steps.map((sid) => steps(r).findIndex((x) => x.id === sid)).filter((i) => i >= 0)
        .map((i) => `<button class="chip link" data-ostep="${i}">Step ${i + 1}</button>`).join("") : "";
      const bar = o ? `<div class="other"><span>This step shows only its part of the file. ${o.lines} more changed line${o.lines === 1 ? " is" : "s are"} ${links ? "in" : "outside this step."}</span>${links}<button class="btn small" data-whole="${esc(sec.path)}">Show the whole file</button></div>` : "";
      return `<div class="file"><span>${esc(sec.path)}${moved}${note}${sec.context ? ' <span class="ctxnote">not changed by the PR</span>' : ""}</span><button class="tool" data-nvim="${si}">nvim</button></div>
        ${bar}<div class="codewrap"><table class="code">${rows}</table></div>`;
    }).join("");
    $("#rdiff").innerHTML = html;
    wireComments(r, d);
    $("#rdiff").onclick = async (e) => {
      if (e.target.closest(".drow, .crow")) return;
      const os = e.target.closest("[data-ostep]");
      if (os) { go(r.id, +os.dataset.ostep); return; }
      const whole = e.target.closest("[data-whole]");
      if (whole) {
        l.file = whole.dataset.whole;
        setMode(r.id, "files");
        return;
      }
      const xb = e.target.closest("[data-exp]");
      if (xb) { expand(r, d, xb); return; }
      const cm = e.target.closest("[data-cm]");
      if (cm) { compose(r, d, cm, e.shiftKey); return; }
      const nv = e.target.closest("[data-nvim]");
      if (nv) {
        const sec = d.sections[+nv.dataset.nvim];
        const first = sec.ranges.find((g) => g.side === "new") || sec.ranges[0];
        const line = (l.anchor?.path === sec.path && l.anchor.line) || first?.from || 1;
        const pane = await call("review_nvim", { id: r.id, path: sec.path, line });
        await window.SB.refreshPanes();
        window.SB.expand(pane);
        return;
      }
      const td = e.target.closest("td.ln[data-ln]");
      if (!td || td.dataset.ln === "undefined") return;
      const sec = d.sections[+td.dataset.sec];
      const line = +td.dataset.ln;
      const path = td.dataset.side === "old" ? (sec.old_path || sec.path) : sec.path;
      const same = l.anchor && l.anchor.path === path && l.anchor.line === line && l.anchor.side === td.dataset.side;
      l.anchor = same ? null : { path, side: td.dataset.side, line, text: td.nextElementSibling.textContent.slice(1), head: d.head || "" };
      drawDiff(r);
      // The line goes into Claude's prompt, for a question about it.
      if (l.anchor) typeInto(r.id, `${path}:${line}${td.dataset.side === "old" ? " (old side)" : ""} \`${l.anchor.text.trim()}\` `);
    };
    const first = $("#rdiff tr.focus") || $("#rdiff tr.add, #rdiff tr.del");
    if (first && !l.scrolled?.has(key)) {
      (l.scrolled ||= new Set()).add(key);
      first.scrollIntoView({ block: "center" });
    }
  }

  // ---------- expand: hidden rows of a file, like GitHub ----------
  const isHeader = (row) => row.kind === "@" && row.text.startsWith("@@");
  const isNote = (row) => row.kind === "@" && !row.text.startsWith("@@");
  const rowKey = (row) => (row ? `${row.kind}|${row.old ?? ""}|${row.new ?? ""}` : "");
  const STEP = 20;

  // The rows of a file as lines, each with its "No newline" note.
  function fileLines(full) {
    const lines = [];
    for (const row of full) {
      if (isNote(row)) { if (lines.length) lines[lines.length - 1].note = row; }
      else lines.push({ row });
    }
    return lines;
  }

  // What to draw for a section: rows, and gaps of hidden rows with expand buttons.
  // Before the first expand, the app does not have the file, so each header is a gap
  // of unknown size, and one more gap waits at the end.
  function items(l, key, sec) {
    const full = l.full.get(sec.path);
    if (!full) {
      const out = [];
      sec.rows.forEach((row, k) => {
        if (!isHeader(row)) return out.push({ row });
        const prev = [...sec.rows.slice(0, k)].reverse().find((x) => x.kind !== "@");
        const next = sec.rows.slice(k + 1).find((x) => x.kind !== "@");
        out.push({ gap: true, text: row.text, prev: rowKey(prev), next: rowKey(next) });
      });
      const last = [...sec.rows].reverse().find((x) => x.kind !== "@");
      if (last && !sec.note) out.push({ gap: true, text: "", prev: rowKey(last), next: "" });
      return out;
    }
    const lines = fileLines(full);
    const index = new Map(lines.map((x, k) => [rowKey(x.row), k]));
    const shown = new Set(l.shown.get(`${key}|${sec.path}`) || []);
    for (const row of sec.rows) if (!isHeader(row) && !isNote(row) && index.has(rowKey(row))) shown.add(index.get(rowKey(row)));
    const out = [];
    let from = null;
    const gap = (a, b) => out.push({ gap: true, from: a, to: b, count: b - a + 1, prev: rowKey(lines[a - 1]?.row), next: rowKey(lines[b + 1]?.row), nextRow: lines[b + 1]?.row });
    lines.forEach((x, k) => {
      if (!shown.has(k)) { if (from === null) from = k; return; }
      if (from !== null) { gap(from, k - 1); from = null; }
      out.push({ row: x.row });
      if (x.note) out.push({ row: x.note });
    });
    if (from !== null) gap(from, lines.length - 1);
    return out;
  }

  function gapRow(si, it) {
    const at = `data-sec="${si}" data-prev="${esc(it.prev)}" data-next="${esc(it.next)}"`;
    const n = it.count;
    const up = it.prev !== "" && it.next !== "";
    let btns;
    if (n !== undefined && n <= STEP) btns = `<button type="button" class="xb" data-exp="all" ${at}>↕ Show ${n} line${n === 1 ? "" : "s"}</button>`;
    else btns = [
      it.next !== "" ? `<button type="button" class="xb" data-exp="up" ${at} title="Show ${STEP} more lines above">↑</button>` : "",
      it.prev !== "" ? `<button type="button" class="xb" data-exp="down" ${at} title="Show ${STEP} more lines below">↓</button>` : "",
      up || n !== undefined ? `<button type="button" class="xb" data-exp="all" ${at}>Show all${n !== undefined ? ` ${n}` : ""}</button>` : "",
    ].join("");
    const nr = it.nextRow;
    const text = n === undefined ? it.text : nr ? `@@ old ${nr.old ?? "–"} · new ${nr.new ?? "–"} @@` : "";
    const hidden = n !== undefined ? `<span class="hidden">${n} hidden line${n === 1 ? "" : "s"}</span>` : "";
    return `<tr class="hunk gap"><td class="cm"></td><td class="ln"></td><td class="src"><div class="gapbar">${btns}${text ? `<span class="htext">${esc(text)}</span>` : ""}${hidden}</div></td></tr>`;
  }

  async function expand(r, d, b) {
    const l = local(r.id);
    const key = diffKey(r);
    const sec = d.sections[+b.dataset.sec];
    if (!l.full.has(sec.path)) {
      try {
        const got = await invoke("review_file_rows", { id: r.id, path: sec.path });
        if (got.head !== d.head) { toast("The diff on screen is from an older commit. It reloads now."); l.diff.clear(); drawDiff(S.reviews.get(r.id) || r); return; }
        l.full.set(sec.path, got.rows);
      } catch (e) { toast(e); return; }
    }
    reveal(l, key, sec, b.dataset.prev, b.dataset.next, b.dataset.exp);
    drawDiff(S.reviews.get(r.id) || r);
  }

  // Shows hidden rows of the gap between the rows `prev` and `next` (row keys):
  // the last STEP rows for "up", the first STEP rows for "down", or all of them.
  function reveal(l, key, sec, prev, next, dir) {
    const g = items(l, key, sec).find((it) => it.gap && it.prev === prev && it.next === next);
    if (!g) return;
    const set = new Set(l.shown.get(`${key}|${sec.path}`) || []);
    let a = g.from, z = g.to;
    if (dir === "up") a = Math.max(g.from, g.to - STEP + 1);
    if (dir === "down") z = Math.min(g.to, g.from + STEP - 1);
    for (let k = a; k <= z; k++) set.add(k);
    l.shown.set(`${key}|${sec.path}`, set);
  }

  // Kind to class: the only input to the row color.
  const KIND_CLASS = { "+": "add", "-": "del", " ": "" };

  // Open the comment form on a line, or with Shift, stretch it to a range.
  function compose(r, d, b, shift) {
    const l = local(r.id);
    const sec = d.sections[+b.dataset.sec];
    const side = b.dataset.side;
    const n = +b.dataset.n;
    const path = side === "old" ? sec.old_path || sec.path : sec.path;
    const c = l.compose;
    if (shift && c && c.sec === sec.path && c.side === side) {
      const lo = Math.min(c.start, c.line, n);
      const hi = Math.max(c.start, c.line, n);
      l.compose = { ...c, start: lo, line: hi };
    } else {
      l.compose = { sec: sec.path, path, side, start: n, line: n, text: c?.text || l.placing?.text || "" };
    }
    drawDiff(r);
    $("#rctext")?.focus();
  }

  // The text of the last line of a comment, as the diff on screen shows it.
  function lineText(l, d, c) {
    const rows = [...(d.sections.find((x) => x.path === c.sec)?.rows || []), ...(l.full.get(c.sec) || [])];
    const hit = rows.find((x) => x.kind !== "@" && (c.side === "old" ? x.kind !== "+" && x.old === c.line : x.kind !== "-" && x.new === c.line));
    return hit ? hit.text : "";
  }

  function wireComments(r, d) {
    const l = local(r.id);
    const form = $("#rcform");
    if (form) {
      const ta = $("#rctext");
      ta.addEventListener("input", () => { l.compose.text = ta.value; });
      ta.addEventListener("keydown", (e) => {
        e.stopPropagation();
        if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); form.requestSubmit(); }
        if (e.key === "Escape") { l.compose = null; l.placing = null; drawDiff(r); }
      });
      form.querySelector("[data-ccancel]").onclick = () => { l.compose = null; l.placing = null; drawDiff(r); };
      form.addEventListener("submit", async (e) => {
        e.preventDefault();
        const c = l.compose;
        const text = ta.value.trim();
        if (!c || !text) return;
        try {
          await call("review_comment", { id: r.id, comment: { path: c.path, side: c.side, line: c.line, start_line: c.start < c.line ? c.start : null, text, head: d.head || "", line_text: lineText(l, d, c) } });
          // Place again: the old draft goes only when this is the same comment.
          if (l.placing && text === l.placing.text.trim()) await call("review_draft_edit", { id: r.id, draft: l.placing.id, text: null }).catch(() => {});
          l.placing = null;
          l.compose = null;
          drawDiff(S.reviews.get(r.id) || r);
        } catch (_) {}
      });
    }
    $("#rdiff").querySelectorAll(".dcard").forEach((card) => {
      const id = +card.dataset.d;
      const t = card.querySelector(".dtext");
      t.addEventListener("keydown", (e) => e.stopPropagation());
      t.addEventListener("blur", () => call("review_draft_edit", { id: r.id, draft: id, text: t.textContent.trim() || null }));
      card.querySelector("[data-ddel]").onclick = () => call("review_draft_edit", { id: r.id, draft: id, text: null });
    });
  }

  const EVENT_NAME = { COMMENT: "Comment", APPROVE: "Approve", REQUEST_CHANGES: "Request changes" };

  function drawDrafts(r) {
    const n = r.drafts.length;
    const isLocal = r.kind === "local";
    const sent = (r.posted || []).map((p) => p.event === "AGENT"
      ? `<div class="sent">Sent to the agent: ${p.comments} comment${p.comments === 1 ? "" : "s"} on ${esc(short(p.head))}</div>`
      : `<div class="sent">Sent: ${esc(EVENT_NAME[p.event] || p.event)}, ${p.comments} line comment${p.comments === 1 ? "" : "s"} <a href="${esc(p.url)}">open on GitHub</a></div>`).join("");
    const main = isLocal
      ? `<button class="btn small primary" data-agent ${n ? "" : "disabled"} title="Type the comments into the agent's prompt; you press Enter">Send to agent</button>`
      : `<button class="btn small primary" data-finish>Finish review</button>`;
    $("#rdrafts").innerHTML = `<div class="rvhead"><span class="badge">${n ? `${n} pending comment${n > 1 ? "s" : ""}` : "No pending comments"}</span><span class="rvbtns">${n ? `<button class="btn small" data-copy>Copy all</button>` : ""}${main}</span></div>` + sent +
      r.drafts.map((d) => {
        const lines = d.start_line ? `${d.start_line}-${d.line}` : d.line;
        const loc = d.path ? `${d.path}${d.line ? ":" + lines : ""}${d.side === "old" ? " (old)" : ""}` : "";
        const long = d.text.split("\n").length > 2 || d.text.length > 240;
        const stale = d.stale ? `<em class="stalenote">The code changed after you wrote this. Place it again or delete it.</em><span class="stalebtns"><button class="btn small" data-place="${d.id}">Place again</button><button class="btn small" data-drop="${d.id}">Delete</button></span>` : "";
        return `<div class="draft${long ? " long" : ""}${d.agent ? " agent" : ""}${d.stale ? " stale" : ""}" data-d="${d.id}"><b>${esc(loc)}${d.agent ? ' <i>agent</i>' : ""}</b>${stale}<span contenteditable="true">${esc(d.text)}</span></div>`;
      }).join("");
    if ($("#rdrafts [data-finish]")) $("#rdrafts [data-finish]").onclick = () => openFinish(r.id);
    if ($("#rdrafts [data-agent]")) $("#rdrafts [data-agent]").onclick = async () => {
      try {
        const n = await invoke("review_send_agent", { id: r.id, note: "" });
        toast(`${n} comment${n === 1 ? "" : "s"} in the agent's prompt. Press Enter there to send.`);
        if (local(r.id).term) window.SB.term.focus();
      } catch (e) {
        toast(e);
      }
    };
    // Place again: the next + opens the form with this text; the add removes the old draft.
    $("#rdrafts").querySelectorAll("[data-place]").forEach((b) => (b.onclick = () => {
      const d = r.drafts.find((x) => x.id === +b.dataset.place);
      local(r.id).placing = d ? { id: d.id, text: d.text } : null;
      toast("Click + on the line for this comment.");
    }));
    $("#rdrafts").querySelectorAll("[data-drop]").forEach((b) => (b.onclick = () => call("review_draft_edit", { id: r.id, draft: +b.dataset.drop, text: null })));
    $("#rdrafts").querySelectorAll("[data-d] span").forEach((el) => {
      el.addEventListener("blur", () => call("review_draft_edit", { id: r.id, draft: +el.parentElement.dataset.d, text: el.textContent.trim() || null }));
      el.addEventListener("keydown", (e) => e.stopPropagation());
    });
    if ($("#rdrafts [data-copy]")) $("#rdrafts [data-copy]").onclick = async (e) => {
      const text = await call("review_drafts_text", { id: r.id });
      copy(text);
      e.target.textContent = "Copied";
      setTimeout(() => (e.target.textContent = "Copy all"), 1500);
    };
  }

  // Finish review: you pick the type and write the summary. The app checks every
  // comment against GitHub's diff, shows you the result, and sends only on your click.
  async function openFinish(id) {
    const r = S.reviews.get(id);
    const l = local(id);
    l.finish = { event: l.finish?.event || "COMMENT", preview: null, error: null, sending: false };
    const box = $("#rfinish");
    box.hidden = false;
    box.innerHTML = `<div class="fbox" role="dialog" aria-label="Finish your review">
      <h4>Finish your review</h4>
      <label class="flabel" for="rsummary">Summary</label>
      <textarea id="rsummary" placeholder="Leave a comment on the whole PR">${esc(r.summary || "")}</textarea>
      <div class="fevents" role="radiogroup">
        ${Object.entries(EVENT_NAME).map(([k, v]) => `<label><input type="radio" name="revent" value="${k}"${k === l.finish.event ? " checked" : ""}> <b>${v}</b> <span>${{ COMMENT: "General feedback, no approval.", APPROVE: "Give your approval to merge.", REQUEST_CHANGES: "Feedback that must be fixed before a merge." }[k]}</span></label>`).join("")}
      </div>
      <div class="fpreview" id="rpreview">Checking every comment against GitHub's diff…</div>
      <div class="fbtns"><button class="btn" data-fclose>Cancel</button><button class="btn primary" data-fsend disabled>Submit review</button></div>
    </div>`;
    let timer;
    $("#rsummary").addEventListener("input", (e) => {
      clearTimeout(timer);
      const text = e.target.value;
      timer = setTimeout(() => call("review_summary", { id, text }).catch(() => {}), 400);
    });
    $("#rsummary").addEventListener("keydown", (e) => e.stopPropagation());
    box.querySelectorAll("[name=revent]").forEach((x) => x.addEventListener("change", () => { l.finish.event = x.value; }));
    box.querySelector("[data-fclose]").onclick = () => closeFinish(id);
    box.querySelector("[data-fsend]").onclick = () => send(id);
    box.onkeydown = (e) => { if (e.key === "Escape") closeFinish(id); };
    try {
      l.finish.preview = await invoke("review_post_preview", { id });
    } catch (e) {
      l.finish.error = String(e);
    }
    drawPreview(id);
  }

  function closeFinish(id) {
    const l = local(id);
    if (l.finish?.sending) return;
    const text = $("#rsummary")?.value;
    if (text !== undefined) call("review_summary", { id, text }).catch(() => {});
    l.finish = null;
    $("#rfinish").hidden = true;
  }

  function drawPreview(id) {
    const l = local(id);
    const f = l.finish;
    const box = $("#rpreview");
    if (!f || !box) return;
    const btn = $("#rfinish [data-fsend]");
    if (f.error) {
      box.innerHTML = `<div class="ferr">${esc(f.error)}</div><div><button class="btn small" data-recheck>Check again</button></div>`;
      btn.disabled = true;
      box.querySelector("[data-recheck]").onclick = async () => {
        f.error = null;
        f.preview = null;
        box.textContent = "Checking every comment against GitHub's diff…";
        try { f.preview = await invoke("review_post_preview", { id }); } catch (e) { f.error = String(e); }
        drawPreview(id);
      };
      return;
    }
    const p = f.preview.plan;
    const at = (c) => `${c.path}:${c.start_line ? `${c.start_line}-${c.line}` : c.line}${c.side === "LEFT" ? " (old)" : ""}`;
    const agent = new Set((S.reviews.get(id)?.drafts || []).filter((d) => d.agent).map((d) => d.id));
    const tag = (c) => (agent.has(c.draft) ? ' <i class="agent">agent draft</i>' : "");
    box.innerHTML = `<div class="fwho">Posts as <b>@${esc(f.preview.login)}</b> on commit <code>${esc(short(p.head))}</code>.</div>
      ${p.errors.length ? `<div class="ferr"><b>Nothing can be sent:</b><ul>${p.errors.map((x) => `<li>${esc(x)}</li>`).join("")}</ul></div>` : ""}
      ${p.inline.length ? `<div class="fsec"><b>${p.inline.length} comment${p.inline.length > 1 ? "s" : ""} on lines</b> <span class="ok">✓ same text as GitHub's diff</span><ul>${p.inline.map((c) => `<li><code>${esc(at(c))}</code>${tag(c)} ${esc(c.text)}</li>`).join("")}</ul></div>` : ""}
      ${p.outside.length ? `<div class="fsec warn"><b>${p.outside.length} comment${p.outside.length > 1 ? "s go" : " goes"} into the summary</b><ul>${p.outside.map((c) => `<li><code>${esc(c.at || "general")}</code>${tag(c)} ${esc(c.text)} <span class="why">${esc(c.reason)}</span></li>`).join("")}</ul></div>` : ""}`;
    btn.disabled = p.errors.length > 0 || f.sending;
  }

  async function send(id) {
    const l = local(id);
    const f = l.finish;
    if (!f?.preview || f.sending) return;
    f.sending = true;
    const btn = $("#rfinish [data-fsend]");
    btn.disabled = true;
    btn.textContent = "Sending…";
    try {
      const url = await invoke("review_post", { id, event: f.event, summary: $("#rsummary").value, token: f.preview.token });
      f.sending = false;
      closeFinish(id);
      // The author often pushes a fix: a later round can start from this one, also after a close.
      showBox(id, `<h4>Review sent</h4><p><a href="${esc(url)}">${esc(url)}</a></p>
        <p>Close the review now? Its worktree and Claude pane go. When the author pushes again, open the PR and pick the next round.</p>`,
        [["Keep open", false], ["Close review", true]], (v) => v && closeReview(id));
    } catch (e) {
      f.sending = false;
      f.error = String(e);
      btn.textContent = "Submit review";
      drawPreview(id);
    }
  }

  // A small dialog in the finish layer. `buttons` is [label, value]; the last one is
  // the main one. `on` gets the value of the clicked button, or null on Escape.
  function showBox(id, html, buttons, on) {
    const box = $("#rfinish");
    if (!box || S.tab !== id) return;
    box.innerHTML = `<div class="fbox" role="dialog" aria-modal="true">${html}
      <div class="fbtns">${buttons.map(([label], i) => `<button class="btn${i === buttons.length - 1 ? " primary" : ""}" data-b="${i}">${esc(label)}</button>`).join("")}</div></div>`;
    box.hidden = false;
    const done = (v) => { box.hidden = true; box.innerHTML = ""; on(v); };
    box.querySelectorAll("[data-b]").forEach((b) => (b.onclick = () => done(buttons[+b.dataset.b][1])));
    box.onkeydown = (e) => { e.stopPropagation(); if (e.key === "Escape") done(null); };
    box.querySelector(".btn.primary")?.focus();
  }

  // Closes a review. When its worktree has changes, you choose first (SPEC 11.10).
  // "Discard" sends the list you saw: a change that came after it is not dropped.
  async function closeReview(id, keep = false, discard = null) {
    try {
      const res = await invoke("review_close", { id, keep, discard });
      if (res.closed) return;
      const list = res.changes.slice(0, 30).map((c) => `<li><code>${esc(c)}</code></li>`).join("") + (res.changes.length > 30 ? `<li>and ${res.changes.length - 30} more</li>` : "");
      const again = discard ? "<p>New changes came after you looked.</p>" : "";
      showBox(id, `<h4>Close review</h4>${again}<p>The review's worktree has changes. A removal loses them:</p><ul class="fchanges">${list}</ul>`,
        [["Cancel", null], ["Keep the folder", "keep"], ["Discard the changes", "discard"]],
        (v) => v === "keep" ? closeReview(id, true) : v === "discard" && closeReview(id, false, res.changes));
    } catch (e) {
      toast(e);
    }
  }

  function copy(text) {
    const ta = document.createElement("textarea");
    ta.value = text;
    document.body.appendChild(ta);
    ta.select();
    try { document.execCommand("copy"); } catch (_) {}
    navigator.clipboard?.writeText(text).catch(() => {});
    ta.remove();
  }

  // The step that the Claude pane hears about. In Files, the first step that covers the file.
  function curStep(r, l) {
    return l.mode === "files" ? steps(r).find((x) => x.id === curFile(l)?.steps[0]) || steps(r)[l.cur] : steps(r)[l.cur];
  }

  // Finds the review's Claude Code pane, or starts it when the guide is ready. The
  // pane forks the guide session, so it knows the review (SPEC 11.5).
  async function openTerm(id) {
    const r = S.reviews.get(id);
    const l = local(id);
    if (!r || l.opening) return;
    l.opening = true;
    const ready = r.status === "ready" && steps(r).length > 0;
    try {
      const pane = await invoke("review_term", { id, step: curStep(r, l)?.id ?? null, start: ready });
      // A local review with its agent pane closed: ask once, not on every draw.
      if (!pane && r.kind === "local") l.noTerm = true;
      if (pane) {
        l.term = pane;
        if (S.tab === id) await showTerm(id);
      }
    } catch (e) {
      l.termError = String(e);
    }
    l.opening = false;
    if (S.tab === id) drawTermWait(S.reviews.get(id));
  }

  async function showTerm(id) {
    const l = local(id);
    if (!l.term || S.tab !== id || !$("#rterm")) return;
    if (!window.SB.paneById(l.term)) await window.SB.refreshPanes();
    // The tab can change during the wait: take the host only now.
    const host = $("#rterm");
    if (S.tab !== id || !host) return;
    // The pane is gone, for example closed on the board: start a new one.
    if (!window.SB.paneById(l.term)) { l.term = null; return openTerm(id); }
    await window.SB.expand(l.term, host);
    drawTermWait(S.reviews.get(id));
  }

  function drawTermWait(r) {
    const el = $("#raskwait");
    if (!el || !r) return;
    const l = local(r.id);
    const shown = l.term && S.live === l.term && S.termHost === r.id;
    el.hidden = !!shown;
    el.innerHTML = l.term ? `<button class="btn small" data-show>Show ${r.kind === "local" ? "the agent" : "Claude"}</button>`
      : l.termError ? esc(l.termError)
      : r.kind === "local" ? "The agent pane is closed. Copy all still works."
      : r.status === "ready" ? "Starting Claude…" : "Claude opens here when the guide is ready.";
  }

  // Types text into Claude's prompt, without Enter. The text comes from the PR, so
  // control characters go, and it goes in as one paste. Not while Claude asks
  // something: a "1" in the text would pick an answer.
  async function typeInto(id, text) {
    const l = local(id);
    if (!l.term) return toast("Claude is not open yet.");
    const p = window.SB.paneById(l.term);
    if (p && (p.permit || p.trust || p.lamp === "needs")) return toast("Claude is asking you something. Answer it first.");
    if (p && p.lamp === "ended") return toast("Claude has ended. Use Resume on its title bar.");
    if (S.live !== l.term) await showTerm(id);
    if (S.tab !== id) return;
    const clean = text.replace(/[\u0000-\u001f\u007f-\u009f]/g, " ");
    await invoke("pane_input", { id: l.term, data: `\x1b[200~${clean}\x1b[201~`, binary: false }).catch((e) => toast(e));
    window.SB.term.focus();
  }

  async function sendStep(id) {
    const r = S.reviews.get(id);
    const s = curStep(r, local(id));
    if (!s) return;
    try {
      typeInto(id, (await invoke("review_step_note", { id, step: s.id })) + " ");
    } catch (e) {
      toast(e);
    }
  }

  function go(id, i) {
    const r = S.reviews.get(id);
    const l = local(id);
    l.cur = Math.max(0, Math.min(steps(r).length - 1, i));
    l.anchor = null;
    savePlace(id);
    update(r);
    $(`#rsteps [data-i="${l.cur}"]`)?.scrollIntoView({ block: "nearest" });
  }

  function next(id) {
    const r = S.reviews.get(id);
    const s = steps(r)[local(id).cur];
    if (s && !stepState(r, s.id).checked) call("review_mark", { id, step: s.id, checked: true });
    go(id, local(id).cur + 1);
  }

  // Review keys work only when no text field has focus (SPEC 7).
  document.addEventListener("keydown", (e) => {
    if (S.tab === "board" || e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.target.closest?.("textarea, input, select, [contenteditable='true']")) return;
    const id = S.tab;
    const l = local(id);
    const files = l.mode === "files";
    if (e.key === "f" && S.reviews.get(id)?.kind !== "local") setMode(id, files ? "guide" : "files");
    else if (e.key === "j") files ? goFile(id, fileIndex(l) + 1) : go(id, l.cur + 1);
    else if (e.key === "k") files ? goFile(id, fileIndex(l) - 1) : go(id, l.cur - 1);
    else if (e.key === "n") files ? nextFile(id) : next(id);
    else return;
    e.preventDefault();
  });

  window.SBReview = {
    draw,
    update: (r) => {
      const l = local(r.id);
      // A Refresh can move only the base (after a push): that is a new diff too.
      if (l.head !== r.head || l.base !== r.base || l.guideLen !== JSON.stringify(r.guide || "").length) {
        l.diff.clear();
        l.files = null;
        l.full.clear();
        l.shown.clear();
        l.head = r.head;
        l.base = r.base;
        l.guideLen = JSON.stringify(r.guide || "").length;
      }
      update(r);
    },
    termWait: (id) => drawTermWait(S.reviews.get(id)),
    // For the window checks only.
    _test: { items, reveal, STEP },
  };
})();
