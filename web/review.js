// Review tabs (SPEC 11): steps, guide + diff, threads and drafts.

(() => {
  const { S, call, invoke, esc, toast, $ } = window.SB;
  const R = new Map(); // review id -> { cur, anchor, active, pending: Map(thread -> text), diff: Map(step -> view), busy: Set }
  // Your place in each review survives a restart: the step, the active thread, Step or All.
  const placeKey = (id) => `sb.review.place.${id}`;
  const loadPlace = (id) => { try { return JSON.parse(localStorage.getItem(placeKey(id)) || "{}"); } catch (_) { return {}; } };
  const savePlace = (id) => {
    const l = R.get(id);
    if (!l) return;
    try { localStorage.setItem(placeKey(id), JSON.stringify({ cur: l.cur, active: l.active, all: l.all, mode: l.mode, file: l.file, viewed: [...l.viewed], vhead: l.vhead })); } catch (_) {}
  };
  const local = (id) => {
    if (!R.has(id)) {
      const p = loadPlace(id);
      R.set(id, { cur: p.cur || 0, anchor: null, active: p.active || null, all: !!p.all, pending: new Map(), diff: new Map(), busy: new Set(), loading: new Set(),
        // Guide shows the steps; Files shows every changed file, like GitHub's "Files changed".
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
    view().innerHTML = `
      <div class="rhead" id="rhead"></div>
      <div class="rcols">
        <div class="col"><ol class="steps" id="rsteps"></ol></div>
        <div class="col center" id="rcenter"><div class="guide" id="rguide"></div><div id="rdiff"></div></div>
        <div class="col">
          <div class="tbar"><div class="seg" id="rscope"><button type="button" data-v="step">Step</button><button type="button" data-v="all">All</button></div><span class="tcount" id="rtcount"></span></div>
          <div class="threads" id="rthreads"></div>
          <div class="anchorchip" id="ranchor"></div>
          <form class="ask" id="rask"><label for="rq" hidden>Question</label><textarea id="rq"></textarea><button class="btn primary" type="submit">Ask</button></form>
          <div class="drafts" id="rdrafts"></div>
        </div>
      </div>
      <div class="finish" id="rfinish" hidden></div>`;
    $("#rask").addEventListener("submit", (e) => { e.preventDefault(); ask(id); });
    $("#rq").addEventListener("keydown", (e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); ask(id); } });
    update(r, true);
  }

  function update(r, fresh) {
    if (S.tab !== r.id) return;
    if (!fresh && !$("#rsteps")) return draw(r.id);
    const l = local(r.id);
    l.cur = Math.min(l.cur, Math.max(0, steps(r).length - 1));
    // Viewed marks belong to one head. New commits clear them, as on GitHub.
    if (l.vhead !== r.head) { l.viewed = new Set(); l.vhead = r.head; }
    drawHead(r);
    if (l.mode === "files") {
      if (!l.files) loadFiles(r.id);
      drawFiles(r);
      drawFileHead(r);
    } else {
      drawSteps(r);
      drawGuide(r);
    }
    drawThreads(r);
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
    if (l.mode === mode) return;
    l.mode = mode;
    l.anchor = null;
    l.compose = null;
    l.active = null;
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
    l.active = null;
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
    const status = { fetching: "Fetching the PR", writing: "Writing the guide", updating: "Updating the guide", error: "Error", ready: "" }[r.status] ?? r.status;
    const newHead = r.new_head ? `<span class="newhead">New commits: ${esc(short(r.head))} → ${esc(short(r.new_head))}<button class="btn small" data-act="update">Update guide</button></span>` : "";
    const retry = r.status === "error" ? `<button class="btn small" data-act="retry">Retry</button>` : "";
    $("#rhead").innerHTML = `<div><div class="t">${esc(r.title)}</div><div class="m">${esc(r.repo)} #${esc(r.number)} · ${esc(short(r.head))}${r.tree ? " · " + esc(r.tree) : ""}</div></div>
      ${status ? `<span class="status${r.status === "error" ? " error" : ""}">${esc(status)}</span>` : ""}${coverageLine(r)}${newHead}
      <div class="right"><div class="seg" id="rmode" title="Switch with f"><button type="button" data-mode="guide" aria-pressed="${local(r.id).mode === "guide"}">Guide</button><button type="button" data-mode="files" aria-pressed="${local(r.id).mode === "files"}">Files</button></div>${retry}<button class="btn small" data-act="close">Close review</button></div>
      ${r.error ? `<div class="rerror" style="flex-basis:100%;padding:0">${esc(r.error)}</div>` : ""}`;
    $("#rhead").onclick = async (e) => {
      const mode = e.target.closest("[data-mode]")?.dataset.mode;
      if (mode) return setMode(r.id, mode);
      const act = e.target.closest("[data-act]")?.dataset.act;
      if (act === "update") { local(r.id).diff.clear(); local(r.id).files = null; call("review_update", { id: r.id }); }
      if (act === "retry") call("review_retry", { id: r.id });
      if (act === "close") call("review_close", { id: r.id });
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
    if (!d || !d.sections?.length) { $("#rdiff").innerHTML = d?.error ? `<div class="rerror">${esc(d.error)}</div>` : ""; return; }
    const html = d.sections.map((sec, si) => {
      const inRange = (side, num) => sec.ranges.some((g) => g.side === side && num >= g.from && num <= g.to);
      const anchored = new Set(r.threads.filter((t) => (!s || t.step === s.id) && t.line && t.path === sec.path).map((t) => `${t.side || "new"}:${t.line}`));
      const pathOf = (side) => (side === "old" ? sec.old_path || sec.path : sec.path);
      const c = l.compose && l.compose.sec === sec.path ? l.compose : null;
      const rows = sec.rows.map((row) => {
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
        const mine = r.drafts.filter((x) => x.path === pathOf(rowSide) && (x.side || "new") === rowSide && x.line === num);
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
      l.anchor = same ? null : { path, side: td.dataset.side, line, text: td.nextElementSibling.textContent.slice(1) };
      l.active = null;
      drawDiff(r);
      drawThreads(r);
      $("#rq").focus();
    };
    const first = $("#rdiff tr.focus") || $("#rdiff tr.add, #rdiff tr.del");
    if (first && !l.scrolled?.has(key)) {
      (l.scrolled ||= new Set()).add(key);
      first.scrollIntoView({ block: "center" });
    }
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
      l.compose = { sec: sec.path, path, side, start: n, line: n, text: c?.text || "" };
    }
    drawDiff(r);
    $("#rctext")?.focus();
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
        if (e.key === "Escape") { l.compose = null; drawDiff(r); }
      });
      form.querySelector("[data-ccancel]").onclick = () => { l.compose = null; drawDiff(r); };
      form.addEventListener("submit", async (e) => {
        e.preventDefault();
        const c = l.compose;
        const text = ta.value.trim();
        if (!c || !text) return;
        try {
          await call("review_comment", { id: r.id, path: c.path, side: c.side, line: c.line, startLine: c.start < c.line ? c.start : null, text });
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

  function drawThreads(r) {
    const l = local(r.id);
    const s = steps(r)[l.cur];
    if (!s) { $("#rthreads").innerHTML = ""; return; }
    const all = l.all;
    const mine = all ? r.threads : l.mode === "files" ? r.threads.filter((t) => t.path && (t.path === l.file || t.path === curFile(l)?.old_path)) : r.threads.filter((t) => t.step === s.id);
    const stepIndex = (sid) => steps(r).findIndex((x) => x.id === sid);
    document.querySelectorAll("#rscope button").forEach((b) => b.setAttribute("aria-pressed", String((b.dataset.v === "all") === all)));
    const one = $('#rscope [data-v="step"]');
    if (one) one.textContent = l.mode === "files" ? "File" : "Step";
    $("#rtcount").textContent = r.threads.length ? `${r.threads.length} in review` : "";
    // Keep the scroll place; stay at the bottom when you were at the bottom.
    const box = $("#rthreads");
    const atBottom = box.scrollHeight - box.scrollTop - box.clientHeight < 40;
    const top = box.scrollTop;
    box.innerHTML = mine.map((t) => {
      const si = stepIndex(t.step);
      const stepName = si >= 0 ? `Step ${si + 1}` : "Old step";
      const at = t.line ? `${(t.path || "").split("/").pop()}:${t.line}${t.side === "old" ? " (old)" : ""}` : "";
      const where = all ? [stepName, at].filter(Boolean).join(" · ") : at || stepName;
      const busy = t.busy || l.busy.has(t.id);
      const msgs = t.messages.map((m, i) => {
        if (m.me) return `<div class="msg me">${esc(m.text)}</div>`;
        const lastAnswer = i === t.messages.length - 1 && !busy;
        const acts = lastAnswer ? `<div class="macts"><button data-pin="${esc(t.id)}">Pin</button><button data-draft="${esc(t.id)}">Draft</button></div>` : "";
        return `<div class="msg md">${md(m.text)}${acts}</div>`;
      }).join("");
      const text = l.pending.get(t.id);
      const pending = busy ? (text ? `<div class="msg md wait">${md(text)}</div>` : `<div class="msg wait">…</div>`) : "";
      return `<div class="thread${l.active === t.id ? " active" : ""}" data-t="${esc(t.id)}">
        <div class="anchor" title="${esc(t.path || "")}">${esc(where)}${t.removed ? '<span class="gone">line removed</span>' : ""}</div>${msgs}${pending}</div>`;
    }).join("");
    box.scrollTop = atBottom ? box.scrollHeight : top;
    const anchorText = l.active ? "" : l.anchor ? `→ ${l.anchor.path.split("/").pop()}:${l.anchor.line}` : "";
    $("#ranchor").textContent = anchorText;
    $("#rthreads").onclick = async (e) => {
      const b = e.target.closest("button");
      if (b?.dataset.pin || b?.dataset.draft) {
        e.stopPropagation();
        const t = b.dataset.pin || b.dataset.draft;
        b.disabled = true;
        b.textContent = "…";
        await call(b.dataset.pin ? "review_pin" : "review_draft", { id: r.id, thread: t }).catch(() => {});
        return;
      }
      const th = e.target.closest(".thread");
      if (!th) return;
      const t = r.threads.find((x) => x.id === th.dataset.t);
      const si = t ? stepIndex(t.step) : -1;
      if (l.mode !== "files" && si >= 0 && si !== l.cur) {
        // A thread of another step: go there, with the thread active.
        go(r.id, si);
        l.active = t.id;
        savePlace(r.id);
        drawThreads(r);
        $("#rq").focus();
        return;
      }
      l.active = l.active === th.dataset.t ? null : th.dataset.t;
      savePlace(r.id);
      drawThreads(r);
      if (l.active) $("#rq").focus();
    };
    $("#rscope").onclick = (e) => {
      const b = e.target.closest("button");
      if (!b) return;
      l.all = b.dataset.v === "all";
      savePlace(r.id);
      drawThreads(r);
    };
  }

  const EVENT_NAME = { COMMENT: "Comment", APPROVE: "Approve", REQUEST_CHANGES: "Request changes" };

  function drawDrafts(r) {
    const n = r.drafts.length;
    const sent = (r.posted || []).map((p) => `<div class="sent">Sent: ${esc(EVENT_NAME[p.event] || p.event)}, ${p.comments} line comment${p.comments === 1 ? "" : "s"} <a href="${esc(p.url)}">open on GitHub</a></div>`).join("");
    $("#rdrafts").innerHTML = `<div class="rvhead"><span class="badge">${n ? `${n} pending comment${n > 1 ? "s" : ""}` : "No pending comments"}</span><span class="rvbtns">${n ? `<button class="btn small" data-copy>Copy all</button>` : ""}<button class="btn small primary" data-finish>Finish review</button></span></div>` + sent +
      r.drafts.map((d) => {
        const lines = d.start_line ? `${d.start_line}-${d.line}` : d.line;
        const loc = d.path ? `${d.path}${d.line ? ":" + lines : ""}${d.side === "old" ? " (old)" : ""}` : "";
        const long = d.text.split("\n").length > 2 || d.text.length > 240;
        return `<div class="draft${long ? " long" : ""}${d.agent ? " agent" : ""}" data-d="${d.id}"><b>${esc(loc)}${d.agent ? ' <i>agent</i>' : ""}</b><span contenteditable="true">${esc(d.text)}</span></div>`;
      }).join("");
    $("#rdrafts [data-finish]").onclick = () => openFinish(r.id);
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
      toast(`Review sent: ${url}`);
    } catch (e) {
      f.sending = false;
      f.error = String(e);
      btn.textContent = "Submit review";
      drawPreview(id);
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

  async function ask(id) {
    const r = S.reviews.get(id);
    const l = local(id);
    // In Files, a question belongs to the first step that covers the file.
    const s = l.mode === "files" ? steps(r).find((x) => x.id === curFile(l)?.steps[0]) || steps(r)[l.cur] : steps(r)[l.cur];
    const text = $("#rq").value.trim();
    if (!s || !text) return;
    const busy = l.active && (l.busy.has(l.active) || r.threads.find((t) => t.id === l.active)?.busy);
    if (busy) return toast("This thread is still answering. Wait for it, or click the thread to start a new one.");
    $("#rq").value = "";
    try {
      const t = await invoke("review_ask", { id, step: s.id, anchor: l.active ? null : l.anchor, thread: l.active, text });
      l.active = t;
      l.anchor = null;
      savePlace(id);
      l.busy.add(t);
      l.pending.set(t, "");
      const fresh = await invoke("review_view", { id });
      S.reviews.set(id, fresh);
      window.SBReview.update(fresh);
    } catch (e) {
      $("#rq").value = text;
      toast(e);
    }
  }

  function stream({ review, thread, delta }) {
    const l = local(review);
    l.pending.set(thread, (l.pending.get(thread) || "") + delta);
    if (S.tab !== review) return;
    const el = document.querySelector(`.thread[data-t="${CSS.escape(thread)}"] .msg.wait`);
    if (!el) return;
    const box = $("#rthreads");
    const atBottom = box.scrollHeight - box.scrollTop - box.clientHeight < 40;
    el.classList.add("md");
    el.innerHTML = md(l.pending.get(thread));
    if (atBottom) box.scrollTop = box.scrollHeight;
  }

  function go(id, i) {
    const r = S.reviews.get(id);
    const l = local(id);
    l.cur = Math.max(0, Math.min(steps(r).length - 1, i));
    l.anchor = null;
    l.active = null;
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
    if (e.key === "f") setMode(id, files ? "guide" : "files");
    else if (e.key === "j") files ? goFile(id, fileIndex(l) + 1) : go(id, l.cur + 1);
    else if (e.key === "k") files ? goFile(id, fileIndex(l) - 1) : go(id, l.cur - 1);
    else if (e.key === "n") files ? nextFile(id) : next(id);
    else if (e.key === "d" && l.active) call("review_draft", { id, thread: l.active });
    else return;
    e.preventDefault();
  });

  window.SBReview = {
    draw,
    update: (r) => {
      const l = local(r.id);
      if (l.head !== r.head || l.guideLen !== JSON.stringify(r.guide || "").length) {
        l.diff.clear();
        l.files = null;
        l.head = r.head;
        l.guideLen = JSON.stringify(r.guide || "").length;
      }
      for (const t of r.threads) if (!t.busy) { l.busy.delete(t.id); l.pending.delete(t.id); }
      update(r);
    },
    stream,
  };
})();
