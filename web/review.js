// Review tabs (SPEC 11): steps, guide + diff, a Claude Code pane, and the
// inspector (SPEC 33) for what you use less often.

(() => {
  const { S, call, invoke, esc, toast, $, setHTML } = window.SB;
  const $$ = (q) => [...document.querySelectorAll(q)];
  // Writes the HTML only when it changed, and says so. A review sends many updates
  // while it works: a new element under the mouse drops a click.
  const put = (el, html) => {
    if (!el || el._html === html) return false;
    setHTML(el, html);
    return true;
  };
  const R = new Map(); // review id -> { cur, anchor, term (the Claude pane), diff: Map(step -> view) }
  // Your place in each review survives a restart: the step, Guide or Files, the file.
  const placeKey = (id) => `sb.review.place.${id}`;
  const loadPlace = (id) => { try { return JSON.parse(localStorage.getItem(placeKey(id)) || "{}"); } catch (_) { return {}; } };
  const savePlace = (id) => {
    const l = R.get(id);
    if (!l) return;
    try { localStorage.setItem(placeKey(id), JSON.stringify({ cur: l.cur, mode: l.mode, file: l.file, viewed: [...l.viewed], vhead: l.vhead, showAt: l.showAt || 0 })); } catch (_) {}
  };
  const local = (id) => {
    if (!R.has(id)) {
      const p = loadPlace(id);
      R.set(id, { cur: p.cur || 0, anchor: null, term: null, diff: new Map(), loading: new Set(),
        // Guide shows the steps; Files shows every changed file, like GitHub's "Files changed".
        full: new Map(), shown: new Map(),
        mode: p.mode === "files" ? "files" : "guide", file: p.file || null, files: null, viewed: new Set(p.viewed || []), vhead: p.vhead || null,
        // Claude points to the code (SPEC 11.15): the tour of its places, where you were
        // before a jump, the place to scroll to, and a view of a file the PR does not change.
        tour: null, hist: [], target: null, ctx: null, showAt: p.showAt || 0 });
    }
    return R.get(id);
  };
  const view = () => $("#view-review");
  const steps = (r) => r.guide?.steps || [];
  const stepState = (r, sid) => r.steps.find((s) => s.id === sid) || { checked: false, stale: false };
  const short = (h) => (h || "").slice(0, 9);
  const plural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;
  const BUSY = ["fetching", "writing", "updating"];

  // Markdown from the agent. Raw HTML in it is shown as text, never run. An image
  // shows as its text: a remote image would load at once, and its URL could carry
  // the code out. Only http(s) links are links, and a click opens the browser.
  const mdr = new marked.Renderer();
  mdr.html = (h) => esc(typeof h === "string" ? h : h?.text ?? "");
  mdr.image = (href, title, text) => esc(text || href || "");
  mdr.link = (href, title, text) => (/^https?:\/\//.test(href || "") ? `<a href="${esc(href)}">${text}</a>` : text);
  const md = (s) => { try { return marked.parse(String(s ?? ""), { renderer: mdr, gfm: true }); } catch (_) { return esc(s); } };
  const mdi = (s) => { try { return marked.parseInline(String(s ?? ""), { renderer: mdr, gfm: true }); } catch (_) { return esc(s); } };
  // Links open in the browser, not in the app window.
  document.addEventListener("click", (e) => {
    const a = e.target.closest?.(".md a, #view-review a[href], #insp a[href]");
    if (!a) return;
    e.preventDefault();
    const href = a.getAttribute("href") || "";
    if (/^https?:\/\//.test(href)) invoke("open_url", { url: href }).catch((err) => toast(err));
  });

  // The screen of a review: a 52 px header, then the steps (or files), the step
  // with its diff and a footer, and the Claude column. The parts change in place.
  function draw(id) {
    const r = S.reviews.get(id);
    if (!r) return;
    const l = local(id);
    const isLocal = r.kind === "local";
    // The live terminal can sit in the old markup: take it out first.
    window.SB.parkLive();
    l.finish = null;
    view().innerHTML = `
      <header class="head rhead" id="rhead">
        <h1 class="htitle" id="rtitle"></h1><span class="hmeta" id="rmeta"></span><span class="rstat" id="rstat"></span>
        <span class="sp"></span><span class="hacts" id="racts"></span>
      </header>
      <div class="rcols" id="rcols">
        <div class="col rside"><div class="rshead" id="rshead"></div><ol class="steps" id="rsteps"></ol></div>
        <div class="gutter gx" id="rgutl" title="Drag to size. Double-click to reset."></div>
        <div class="col rmid"><div class="center" id="rcenter"><div class="guide" id="rguide"></div><div id="rdiff"></div></div><div class="stepfoot" id="rfoot" hidden></div></div>
        <div class="gutter gx" id="rgutr" title="Drag to size. Double-click to reset."></div>
        <div class="col rclaude">
          <div class="tbar">${isLocal ? '<span class="tname" id="rtname"></span>' : ""}<span class="seg mini" role="group" aria-label="Show in this column (Leader e)"><button type="button" class="segb" data-rview="agent" title="Show ${isLocal ? "the agent" : "Claude"} (Leader e)">${isLocal ? "Agent" : "Claude"}</button><button type="button" class="segb" data-rview="nvim" title="Open nvim at the file you are on (Leader e)">nvim</button></span>${isLocal ? "" : `<span class="sp"></span><button class="btn sm" type="button" id="rstep" title="Type the current step into Claude's prompt">Send step</button>`}</div>
          <div class="chatlog" id="rchat" hidden></div>
          <div class="askhost" id="rterm"><div class="askwait" id="raskwait"></div></div>
          <div class="mentions" id="rment" hidden></div>
        </div>
      </div>
      <div class="fpop" id="rfinpop" role="dialog" aria-label="Finish review" hidden></div>
      <div class="finish" id="rfinish" hidden></div>`;
    if ($("#rstep")) $("#rstep").onclick = () => sendStep(id);
    $$("[data-rview]").forEach((b) => { b.onclick = () => (b.dataset.rview === "nvim" ? showNvim(id) : showAgent(id)); });
    $("#rhead").onclick = (e) => {
      const act = e.target.closest("[data-act]")?.dataset.act;
      if (act === "finish") return local(id).finish ? closeFinish(id) : openFinish(id);
      if (act === "agent") return sendAgent(id);
      // New commits, New changes and comments to place: the inspector says what to do.
      if (act === "newcommits" || act === "stale") return window.SBInsp.open();
    };
    $("#rshead").onclick = (e) => { const m = e.target.closest("[data-mode]")?.dataset.mode; if (m) setMode(id, m); };
    $("#rfoot").onclick = (e) => footAct(id, e.target.closest("[data-foot]"));
    drawView(id);
    bindColumns();
    $("#raskwait").onclick = (e) => { if (e.target.closest("[data-show]")) showAgent(id); };
    l.term = null;
    update(r, true);
    openTerm(id);
    seen(id);
    loadChat(id);
  }

  function update(r, fresh) {
    if (S.tab !== r.id) return;
    if (!fresh && !$("#rsteps")) return draw(r.id);
    const l = local(r.id);
    // A new review_show call: the window jumps to its first place, once.
    if (r.show?.points?.length && r.show.at > (l.showAt || 0) && r.status === "ready") {
      l.showAt = r.show.at;
      savePlace(r.id);
      if (startTour(r.id, 0)) return;
    }
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
    if (l.ctx) drawCtxHead(r);
    drawFoot(r);
    drawMentions(r);
    if (r.kind === "local") {
      const p = r.pane && window.SB.paneById(r.pane);
      put($("#rtname"), p ? esc(window.SB.paneName(p)) : "");
    }
    if (!l.term && !l.opening && !l.noTerm && r.status === "ready") openTerm(r.id);
    drawTermWait(r);
    window.SBInsp.refresh();
    const key = diffKey(r);
    if (key && !l.diff.has(key)) loadDiff(r.id);
    else drawDiff(r);
  }

  // The key of the diff on screen: a step id, or "file:" and a path.
  function diffKey(r) {
    const l = local(r.id);
    if (l.ctx) return `ctx:${l.ctx.line}:${l.ctx.path}`;
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
    if (l.ctx) { l.ctx = null; if (l.mode === mode) return update(S.reviews.get(id)); }
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
    const ctx = l.ctx;
    l.ctx = null;
    if (f.path === l.file) { if (ctx) update(S.reviews.get(id)); return; }
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

  // The head of the left column: the Guide and Files switch (f) and how far you are.
  function drawSideHead(r) {
    const l = local(r.id);
    let count = "";
    if (l.mode === "files") {
      if (l.files) count = `${l.files.filter((f) => l.viewed.has(f.path)).length} / ${l.files.length} seen`;
    } else if (steps(r).length) {
      count = `${steps(r).filter((s) => stepState(r, s.id).checked).length} / ${steps(r).length} reviewed`;
    }
    const left = r.kind === "local"
      ? `<span class="rslabel">${l.files ? plural(l.files.length, "file") : "Files"}</span>`
      : `<span class="seg mini" id="rmode" role="group" aria-label="View (f)" title="Switch with f"><button type="button" data-mode="guide" aria-pressed="${l.mode !== "files"}">Guide</button><button type="button" data-mode="files" aria-pressed="${l.mode === "files"}">Files</button></span>`;
    put($("#rshead"), `${left}<span class="rcount">${esc(count)}</span>`);
  }

  // A file row: a tick when seen, the name, and your comments on it. The path, the
  // counts and the steps are in the tooltip.
  function drawFiles(r) {
    const l = local(r.id);
    drawSideHead(r);
    if (!l.files) { put($("#rsteps"), `<li class="empty-review">Loading the files…</li>`); return; }
    const stepNo = (sid) => steps(r).findIndex((x) => x.id === sid) + 1;
    const names = l.files.map((f) => f.path.split("/").pop());
    put($("#rsteps"), l.files.map((f, k) => {
      const parts = f.path.split("/");
      // Two files with one name show their folder too.
      const label = names.filter((x) => x === names[k]).length > 1 ? parts.slice(-2).join("/") : names[k];
      const nums = f.steps.map(stepNo).filter((n) => n > 0);
      const t = r.threads.filter((x) => x.path === f.path).length;
      const d = r.drafts.filter((x) => x.path === f.path).length;
      const tip = [f.path, `+${f.added} −${f.removed}`, f.old_path && f.old_path !== f.path ? `from ${f.old_path}` : "", nums.length ? `step ${nums.join(", ")}` : "", d ? plural(d, "comment") : "", t ? plural(t, "thread") : "", f.note || "", l.viewed.has(f.path) ? "seen" : ""].filter(Boolean).join(" · ");
      return `<li class="fitem${l.viewed.has(f.path) ? " viewed" : ""}" tabindex="0" data-f="${esc(f.path)}" title="${esc(tip)}" ${f.path === l.file ? 'aria-current="step"' : ""}>
        <span class="vbox" aria-hidden="true">${l.viewed.has(f.path) ? "✓" : ""}</span><span class="fname">${esc(label)}</span><span class="fcount" title="${d ? esc(plural(d, "comment")) : ""}">${d || ""}</span>${f.note ? `<span class="fnote">${esc(f.note)}</span>` : ""}</li>`;
    }).join(""));
    $("#rsteps").onclick = (e) => {
      const it = e.target.closest(".fitem");
      if (it) goFile(r.id, l.files.findIndex((f) => f.path === it.dataset.f));
    };
  }

  // Above the diff of a file in Files: where you are, and the steps that hold it.
  function drawFileHead(r) {
    const l = local(r.id);
    const f = curFile(l);
    if (!f || r.kind === "local") { put($("#rguide"), ""); return; }
    const i = fileIndex(l);
    const chips = f.steps.map((sid) => {
      const si = steps(r).findIndex((x) => x.id === sid);
      return si >= 0 ? `<button class="chip link" data-step="${si}">Step ${si + 1}: ${mdi(steps(r)[si].title)}</button>` : "";
    }).join("");
    put($("#rguide"), `<div class="eyebrow">File ${i + 1} of ${l.files.length}</div>${chips ? `<div class="ctx"><span class="clabel">In the guide:</span>${chips}</div>` : ""}`);
    $("#rguide").onclick = (e) => {
      const b = e.target.closest("[data-step]");
      if (b) { setMode(r.id, "guide"); go(r.id, +b.dataset.step); }
    };
  }

  function nextFile(id) {
    const l = local(id);
    if (l.file && !l.viewed.has(l.file)) { l.viewed.add(l.file); savePlace(id); }
    const i = fileIndex(l);
    if (i < (l.files || []).length - 1) goFile(id, i + 1);
    else update(S.reviews.get(id));
  }

  // The footer of the middle column: Prev (k), and Mark reviewed & next (n). During
  // a tour, n goes to the next place, so the button shows Shift+N, which always works.
  function drawFoot(r) {
    const l = local(r.id);
    const el = $("#rfoot");
    let html = "";
    if (l.mode === "files") {
      const f = curFile(l);
      if (f) {
        const i = fileIndex(l);
        const seen = l.viewed.has(f.path);
        const last = i >= l.files.length - 1;
        html = `<button type="button" class="btn sm ghost" data-foot="prev"${i <= 0 ? " disabled" : ""}>Prev <kbd>k</kbd></button><span class="sp"></span>`
          + (seen ? `<button type="button" class="btn sm ghost" data-foot="unmark">Unmark seen</button>` : "")
          + `<button type="button" class="btn" data-foot="next" title="Mark the file seen and open the next one"${last && seen ? " disabled" : ""}>${last ? (seen ? "Seen" : "Mark seen") : "Mark seen & next"} <kbd>n</kbd></button>`;
      }
    } else if (steps(r)[l.cur] && !l.ctx) {
      const st = stepState(r, steps(r)[l.cur].id);
      const last = l.cur >= steps(r).length - 1;
      const tour = !!l.tour;
      html = `<button type="button" class="btn sm ghost" data-foot="prev"${l.cur <= 0 ? " disabled" : ""}>Prev <kbd>k</kbd></button><span class="sp"></span>`
        + (st.checked ? `<button type="button" class="btn sm ghost" data-foot="unmark">Unmark</button>` : "")
        + `<button type="button" class="btn" data-foot="next" title="${tour ? "During a tour, n goes to the next place, so this takes Shift+N" : "Mark the step reviewed and go to the next one (n or Shift+N)"}"${last && st.checked ? " disabled" : ""}>${last ? (st.checked ? "Reviewed" : "Mark reviewed") : "Mark reviewed &amp; next"} <kbd>${tour ? "⇧N" : "n"}</kbd></button>`;
    }
    put(el, html);
    el.hidden = !html;
  }

  function footAct(id, b) {
    if (!b || b.disabled) return;
    const r = S.reviews.get(id);
    const l = local(id);
    const files = l.mode === "files";
    const a = b.dataset.foot;
    if (a === "prev") files ? goFile(id, fileIndex(l) - 1) : go(id, l.cur - 1);
    if (a === "next") files ? nextFile(id) : next(id);
    if (a === "unmark") {
      if (files) { if (l.file) toggleViewed(id, l.file); }
      else { const s = steps(r)[l.cur]; if (s) call("review_mark", { id, step: s.id, checked: false }); }
    }
  }

  // The result of the verified checker, in one line.
  function coverageText(r) {
    const c = r.coverage;
    if (!c) return null;
    const what = `${c.changed_lines} changed lines in ${c.files} files`;
    if (!c.accepted) return what;
    const missed = c.missed_lines ? ` · ${c.missed_lines} added by the app` : "";
    const gh = c.github?.length ? ` · ${plural(c.github.length, "count")} differ from GitHub: ${c.github.join("; ")}` : "";
    return `✓ All ${what} covered${missed}${gh}`;
  }

  const STATUS = { fetching: "fetching", writing: "writing", updating: "updating", ready: "guide ready", error: "error" };

  // The header: the title, the facts in grey, the guide's state, at most one gold
  // button, and room at the right for the inspector toggle.
  function drawHead(r) {
    const l = local(r.id);
    put($("#rtitle"), esc(r.title || (r.kind === "local" ? "Local commits" : "")));
    $("#rtitle").title = r.title || "";
    const stale = r.drafts.filter((d) => d.stale).length;
    const place = stale ? `<button type="button" class="btn sm ghost" data-act="stale" title="The code changed under ${stale === 1 ? "a comment" : "these comments"}: place ${stale === 1 ? "it" : "them"} again or delete ${stale === 1 ? "it" : "them"} in the details">${plural(stale, "comment")} to place</button>` : "";
    if (r.kind === "local") return drawLocalHead(r, place);
    put($("#rmeta"), `#${esc(r.number)} · round ${esc(r.round || 1)}`);
    $("#rmeta").title = `${r.repo} #${r.number} · head ${short(r.head)}`;
    // A guide that is ready can still name pages that did not load (SPEC 11.2).
    const note = r.status === "ready" && r.error ? r.error : "";
    put($("#rstat"), `<i class="sdot s-${esc(r.status)}"></i><span>${esc(STATUS[r.status] ?? r.status)}</span>${note ? `<span class="rwarn" title="${esc(note)}">⚠ ${esc(note.length > 60 ? note.slice(0, 57) + "…" : note)}</span>` : ""}`);
    $("#rstat").title = r.status === "error" ? r.error || "" : note;
    const n = r.drafts.length;
    const next = (r.round || 1) + 1;
    put($("#racts"), (r.new_head ? `<button type="button" class="btn sm ghost" data-act="newcommits" title="Commits the guide does not cover: Start round ${next} or Update guide">New commits</button>` : "") + place
      + `<button type="button" class="btn big${l.finish ? "" : " primary"}" data-act="finish" aria-haspopup="dialog" aria-expanded="${!!l.finish}" title="Send your comments to GitHub as one review">Finish review · ${n}</button>`);
  }

  // The header of a review of an agent's work (SPEC 11.11, 11.14): the range, New
  // or All, and Send to agent. The rest is in the inspector.
  function drawLocalHead(r, place) {
    const range = r.head ? `${short(r.base)}..${short(r.head)}` : "";
    put($("#rmeta"), esc([r.repo, r.base_ref ? `on ${r.base_ref}` : "", range, r.range === "all" ? "All" : "New"].filter(Boolean).join(" · ")));
    $("#rmeta").title = r.scope_note || "";
    const st = { fetching: "reading the work", error: "error" }[r.status];
    put($("#rstat"), st ? `<i class="sdot s-${esc(r.status)}"></i><span>${esc(st)}</span>` : "");
    $("#rstat").title = r.status === "error" ? r.error || "" : "";
    const n = r.drafts.filter((d) => !d.stale).length;
    put($("#racts"), (r.new_head ? `<button type="button" class="btn sm ghost" data-act="newcommits" title="The agent changed more: Refresh in the details">New changes</button>` : "") + place
      + `<button type="button" class="btn big primary" data-act="agent"${n ? "" : " disabled"} title="Type the comments into the agent's prompt; you press Enter">Send ${plural(n, "comment")} to agent</button>`);
  }

  // The side columns of a review: drag the handles, or use the arrow keys on them.
  // One size for every review tab; the middle column keeps at least MID px.
  const SIDE_MIN = 160, MID = 320, COLS_KEY = "sb.split.review";
  const colDefaults = () => (window.innerWidth <= 1100 ? [200, 260] : [232, 290]);
  function colSizes() {
    const row = $("#rcols");
    const saved = SBSplit.load(COLS_KEY) || colDefaults();
    return SBSplit.fit(row.getBoundingClientRect().width - 20, saved[0], saved[1], SIDE_MIN, MID);
  }
  function setCols([l, r]) {
    const row = $("#rcols");
    if (!row) return;
    row.style.setProperty("--rl", `${l}px`);
    row.style.setProperty("--rr", `${r}px`);
  }
  function bindColumns() {
    const row = $("#rcols");
    setCols(colSizes());
    const total = () => row.getBoundingClientRect().width - 20;
    const cur = () => [parseFloat(row.style.getPropertyValue("--rl")), parseFloat(row.style.getPropertyValue("--rr"))];
    const done = () => { SBSplit.save(COLS_KEY, cur()); window.dispatchEvent(new Event("resize")); };
    // A reset puts back only the column of that handle.
    const resetSide = (i) => () => { const d = colDefaults(); const c = cur(); c[i] = d[i]; setCols(SBSplit.fit(total(), c[0], c[1], SIDE_MIN, MID)); };
    const max = (other) => total() - other - MID;
    SBSplit.bind($("#rgutl"), { axis: "x", dir: 1, size: () => cur()[0], reset: resetSide(0), done, controls: "rsteps",
      limits: () => [SIDE_MIN, max(cur()[1])],
      apply: (v) => { const [, r] = cur(); setCols([SBSplit.side(total(), v, r, SIDE_MIN, MID), r]); } });
    SBSplit.bind($("#rgutr"), { axis: "x", dir: -1, size: () => cur()[1], reset: resetSide(1), done, controls: "rterm",
      limits: () => [SIDE_MIN, max(cur()[0])],
      apply: (v) => { const [l] = cur(); setCols([l, SBSplit.side(total(), v, l, SIDE_MIN, MID)]); } });
  }
  // A smaller window shrinks the saved sizes so the middle keeps its room.
  window.addEventListener("resize", () => { if ($("#rcols")) setCols(colSizes()); });

  // How long ago a time in seconds since the epoch was: "just now", "5 min ago", "2 h ago".
  function ago(t) {
    const s = Math.max(0, Date.now() / 1000 - t);
    if (s < 90) return "just now";
    if (s < 90 * 60) return `${Math.round(s / 60)} min ago`;
    if (s < 36 * 3600) return `${Math.round(s / 3600)} h ago`;
    const d = Math.round(s / 86400);
    return `${d} day${d > 1 ? "s" : ""} ago`;
  }

  // The numbers of Claude's places in a step, during a tour.
  function placesOf(r, i) {
    const l = local(r.id);
    return tourPts(l).map((p, k) => [p, k]).filter(([p]) => stepOf(r, p) === i).map(([, k]) => k + 1);
  }

  // Does a draft sit on a line of this step?
  const inStep = (s, d) => (s.ranges || []).some((g) => g.file === d.path && g.side === (d.side || "new") && d.line >= g.from && d.line <= g.to);

  // A step row: a tick or its number, the title, and at most one mark: ~ when the
  // code changed, else a dot for Claude's worst note. The rest is in the tooltip.
  function drawSteps(r) {
    const l = local(r.id);
    drawSideHead(r);
    put($("#rsteps"), steps(r).map((s, i) => {
      const st = stepState(r, s.id);
      const rs = s.ranges || [];
      const live = takeNotes(r, s.id, s.take).filter((x) => !x.dismissed);
      const w = worst(live);
      const counts = SEVS.map((v) => [v, live.filter((x) => x.severity === v).length]).filter(([, c]) => c).map(([v, c]) => `${c} ${SEV[v]}`).join(", ");
      const t = r.threads.filter((x) => x.step === s.id).length;
      const d = r.drafts.filter((x) => !x.stale && inStep(s, x)).length;
      const pl = placesOf(r, i);
      const tip = [
        rs.length ? `${rs[0].file}:${rs[0].from}-${rs[0].to}${rs.length > 1 ? ` +${rs.length - 1}` : ""}` : (s.files || []).join(", "),
        counts ? `Claude: ${counts}` : "",
        t ? plural(t, "thread") : "",
        d ? plural(d, "comment") : "",
        pl.length ? `Claude's place${pl.length > 1 ? "s" : ""} ${pl.join(", ")}` : "",
        st.stale ? "stale: the code changed" : "",
        s.auto ? "added by the app" : "",
        st.checked ? "reviewed" : "",
      ].filter(Boolean).join(" · ");
      const mark = st.stale ? `<span class="smark" aria-label="stale">~</span>`
        : w ? `<i class="sevdot sev-${w}" aria-label="${esc(SEV[w])}"></i>` : `<span class="smark"></span>`;
      return `<li class="step${st.checked ? " checked" : ""}" tabindex="0" data-i="${i}" title="${esc(tip)}" ${i === l.cur ? 'aria-current="step"' : ""}>
        <span class="n">${st.checked && i !== l.cur ? "✓" : i + 1}</span><span class="st">${mdi(s.title)}</span>${mark}</li>`;
    }).join("") || `<li class="empty-review">${r.status === "error" ? "" : "The guide shows here when the agent is done."}</li>`);
    $("#rsteps").onclick = (e) => { const s = e.target.closest(".step"); if (s) go(r.id, +s.dataset.i); };
  }

  // ---------- Claude's take (SPEC 11.12) ----------
  // Severity colors are their own (pink, teal, grey), never the gold of "needs you".
  const SEV = { must: "must fix", should: "should fix", nit: "nit" };
  const SEVS = ["must", "should", "nit"];
  const VERDICT = { good: "looks good", change: "needs a change", question: "has a question" };
  // A note's key names it for Dismiss: the step, the severity, the line and the text.
  const noteKey = (sid, n) => `${sid}|${n.severity}|${n.file || ""}:${n.line || ""}|${n.text}`;
  function takeNotes(r, sid, take) {
    const gone = new Set(r.dismissed || []);
    return (take?.notes || []).map((n) => ({ ...n, side: n.side || "new", key: noteKey(sid, n), dismissed: gone.has(noteKey(sid, n)) }));
  }
  const worst = (notes) => SEVS.find((v) => notes.some((n) => n.severity === v));
  const sevChip = (v, text) => `<span class="sevchip sev-${esc(v)}">${esc(text ?? v)}</span>`;
  const countChips = (notes) => SEVS.map((v) => [v, notes.filter((n) => n.severity === v).length]).filter(([, c]) => c).map(([v, c]) => sevChip(v, `${c} ${v}`)).join("");
  function noteCard(n, extra) {
    if (n.dismissed && !extra?.show) return `<div class="tdismissed" data-tn="${esc(n.key)}">A dismissed note${n.line ? ` on line ${esc(n.line)}` : ""} <button type="button" data-tshow>Show</button><button type="button" data-tundo>Undo</button></div>`;
    const at = extra?.at && n.file ? `<span class="tat">${esc(n.file.split("/").pop())}${n.line ? `:${esc(n.line)}` : ""}</span>` : "";
    const added = extra?.added ? `<span class="tadded">✓ Added</span>` : "";
    return `<div class="ncard tnote" data-tn="${esc(n.key)}"><div class="dmeta"><span class="who">Claude's note</span>${sevChip(n.severity)}${at}${added}</div>
      <div class="dtext md">${mdi(n.text)}</div>
      <div class="nacts">${n.line ? `<button type="button" class="btn sm" data-tadd>Add as comment</button>` : ""}<button type="button" class="btn sm" data-task>Ask</button>${n.dismissed ? `<button type="button" class="btn sm ghost" data-tundo>Undo</button>` : `<button type="button" class="btn sm ghost" data-tdis>Dismiss</button>`}</div></div>`;
  }

  // The context links of a step (spec pages, binary files), as small chips.
  function ctxChips(s) {
    const ctx = [...(s?.context || []), ...(s?.files || []).map((f) => `binary: ${f}`)];
    return ctx.map((c) => {
      const t = typeof c === "string" ? c : c.label || c.ref || "";
      return /^https?:\/\//.test(t) ? `<a class="chip link" href="${esc(t)}">${esc(t)}</a>` : `<span class="chip">${esc(t)}</span>`;
    }).join("");
  }

  // The take line under the checks: the verdict, the counts, the links of the step,
  // the notes with no line on screen, and a menu with Re-run.
  function drawTake(r) {
    const el = $("#rtake");
    if (!el) return;
    const l = local(r.id);
    const s = steps(r)[l.cur];
    const take = s?.take;
    const label = `<span class="tk-label" title="Claude's opinion, not verified">Claude's take${take ? ":" : ""}</span>`;
    const links = ctxChips(s);
    const more = `<button type="button" class="icon tk-more" data-tk="menu" aria-haspopup="menu" aria-expanded="${!!l.takeMenu}" aria-label="More: Re-run Claude's take" title="More"><svg width="14" height="14" viewBox="0 0 14 14" fill="currentColor" aria-hidden="true"><circle cx="3" cy="7" r="1.3"/><circle cx="7" cy="7" r="1.3"/><circle cx="11" cy="7" r="1.3"/></svg></button>`;
    const menu = l.takeMenu ? `<div class="tk-menu" role="menu"><button type="button" role="menuitem" data-tk="rerun">Re-run Claude's take on this step</button><span class="tk-info">Claude's opinion, not verified${take ? ` · on ${esc(short(r.head))}` : ""}</span></div>` : "";
    let html;
    if (r.status === "updating") html = `${label}<span class="tk-dim">Claude is reading this step…</span>${links}`;
    else if (!take) html = `${label}<span class="tk-dim">none for this step</span>${links}${more}${menu}`;
    else {
      const notes = takeNotes(r, s.id, take);
      const live = notes.filter((n) => !n.dismissed);
      const stale = r.new_head ? `<span class="tk-stale" title="The take is from ${esc(short(r.head))}; the head is now ${esc(short(r.new_head))}.">stale</span>` : "";
      const rest = notes.filter((n) => !(l.placed || new Set()).has(n.key));
      const gone = notes.filter((n) => n.dismissed).length;
      const list = l.takeOpen && rest.length ? `<div class="tk-list">${rest.map((n) => noteCard(n, { at: true, show: l.shownNotes?.has?.(n.key) })).join("")}</div>` : "";
      html = `${label}<span class="verdict v-${esc(take.verdict)}">${esc(VERDICT[take.verdict] || take.verdict)}</span>${stale}${countChips(live)}${gone ? `<span class="tk-dim">${gone} dismissed</span>` : ""}${links}`
        + `${rest.length ? `<button type="button" class="tk-btn" data-tk="list">${l.takeOpen ? "Hide list" : `Show list (${rest.length})`}</button>` : ""}${more}`
        + `${take.verdict === "good" && take.checked ? `<div class="tk-checked">Checked: ${mdi(take.checked)}</div>` : ""}${menu}${list}`;
    }
    put(el, html);
    el.onclick = (e) => {
      const tk = e.target.closest("[data-tk]")?.dataset.tk;
      if (tk === "list") { l.takeOpen = !l.takeOpen; drawTake(r); return; }
      if (tk === "menu") { l.takeMenu = !l.takeMenu; drawTake(r); return; }
      if (tk === "rerun") { l.takeMenu = false; drawTake(r); call("review_retake", { id: r.id, step: s?.id ?? null }).catch(() => {}); return; }
      noteAction(r, e);
    };
  }

  // The actions of a note, in the diff, in the take list or in the inspector.
  function noteAction(r, e) {
    const card = e.target.closest("[data-tn]");
    const b = e.target.closest("button");
    if (!card || !b) return false;
    const l = local(r.id);
    const key = card.dataset.tn;
    const all = [...steps(r).flatMap((s) => takeNotes(r, s.id, s.take)), ...takeNotes(r, "pr", r.guide?.take)];
    const n = all.find((x) => x.key === key);
    if (!n) return true;
    if (b.dataset.tdis !== undefined) call("review_dismiss", { id: r.id, note: key, on: true }).catch(() => {});
    else if (b.dataset.tundo !== undefined) call("review_dismiss", { id: r.id, note: key, on: false }).catch(() => {});
    else if (b.dataset.tshow !== undefined) { (l.shownNotes ||= new Set()).add(key); drawDiff(r); drawTake(r); window.SBInsp.refresh(true); }
    else if (b.dataset.task !== undefined) typeInto(r.id, `${n.file ? `${n.file}${n.line ? `:${n.line}` : ""} ` : ""}Your note: "${n.text}" `);
    else if (b.dataset.tadd !== undefined) addNote(r, n);
    return true;
  }

  // Add as comment: the comment form on the note's line, with its text, so you edit
  // it first. It saves as an agent draft.
  function addNote(r, n) {
    const l = local(r.id);
    const d = l.diff.get(diffKey(r));
    const sec = d?.sections?.find((x) => (n.side === "old" ? x.old_path || x.path : x.path) === n.file);
    const drawn = sec && $(`#rdiff [data-cm][data-sec="${d.sections.indexOf(sec)}"][data-side="${n.side}"][data-n="${n.line}"]`);
    if (!drawn) { toast("This line is not on screen. Open its file or show the hidden lines, then add the comment."); return; }
    l.compose = { sec: sec.path, path: n.file, side: n.side, start: n.line, line: n.line, text: n.text, fromTake: n.key };
    drawDiff(r);
    $("#rctext")?.focus();
  }

  // "It matters because …": the guide's why as one lead sentence.
  function whyLead(why) {
    let t = String(why).trim().replace(/^(it matters because|because)[:,]?\s*/i, "");
    const w = /^[A-Za-z]+/.exec(t)?.[0] || "";
    // "A wrong slot …" reads "a wrong slot …"; "ILs …" stays as it is.
    if (w && w.slice(1) === w.slice(1).toLowerCase()) t = t[0].toLowerCase() + t.slice(1);
    return t;
  }
  // The mark before Claude's answer to a check: ✓ for a plain yes, ! for a problem,
  // none when the words do not say. A guess from the words: Claude's text is the answer.
  function answerMark(a) {
    const t = String(a).trim().toLowerCase();
    if (/^(no|not|only|partly|partially|unclear|unknown|missing|wrong|maybe|see the note|does not|doesn't|is not|isn't|it does not)\b/.test(t)) return `<b class="g warn" aria-label="a problem">!</b> `;
    if (/^(yes|matches|correct|right|ok|it does|true|both|done)\b/.test(t)) return `<b class="g ok" aria-label="fine">✓</b> `;
    return "";
  }

  function drawGuide(r) {
    const l = local(r.id);
    const s = steps(r)[l.cur];
    const g = $("#rguide");
    if (!s) {
      put(g, r.status === "error" ? `<div class="rerror">${esc(r.error || "The guide failed.")}</div><div><button type="button" class="btn small" data-act="retry">Retry</button></div>` : "");
      g.onclick = (e) => { if (e.target.closest('[data-act="retry"]')) call("review_retry", { id: r.id }); };
      return;
    }
    const pins = r.pins.filter((p) => p.step === s.id);
    const changed = put(g, `
      <div class="eyebrow">Step ${l.cur + 1} of ${steps(r).length}</div>
      <h4>${mdi(s.title)}</h4>
      ${s.what ? `<div class="what md">${md(s.what)}</div>` : ""}
      ${s.why ? `<p class="lead md">It matters because ${mdi(whyLead(s.why))}</p>` : ""}
      ${s.why_split ? `<p class="whysplit"><span class="wk">A step of its own:</span> <span class="md">${mdi(s.why_split)}</span></p>` : ""}
      ${s.check?.length ? `<ul class="checks md">${s.check.map((c, i) => `<li><span class="q">${mdi(c)}</span>${s.check_answers?.[i] ? `<span class="ans">${answerMark(s.check_answers[i])}${mdi(s.check_answers[i])}</span>` : ""}</li>`).join("")}</ul>` : ""}
      <div class="take" id="rtake"></div>
      ${pins.map((p) => `<div class="pinned" data-pin="${p.id}"><span class="md" contenteditable="true" data-raw="${esc(p.text)}">${mdi(p.text)}</span><button type="button" class="icon x" data-unpin="${p.id}" aria-label="Remove the pin">×</button></div>`).join("")}`);
    drawTake(r);
    g.onclick = (e) => {
      if (e.target.closest("#rtake")) return;
      const b = e.target.closest("button");
      if (b?.dataset.unpin) call("review_pin_edit", { id: r.id, pin: +b.dataset.unpin, text: null });
    };
    if (!changed) return;
    g.querySelectorAll("[data-pin] span").forEach((el) => {
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
      const ctx = /^ctx:(\d+):(.*)$/s.exec(key);
      l.diff.set(key, ctx ? await invoke("review_context", { id, path: ctx[2], line: +ctx[1] })
        : key.startsWith("file:")
        ? await invoke("review_file_diff", { id, path: key.slice(5) })
        : await invoke("review_diff", { id, step: key }));
    } catch (e) {
      l.diff.set(key, { sections: [], error: String(e) });
    }
    l.loading.delete(key);
    const r = S.reviews.get(id);
    if (S.tab === id && diffKey(r) === key) drawDiff(r);
  }

  // The tour of Claude's places, in the header of the file that holds the current
  // place: "◆ i/N" and p, n and '.
  function tourGroup(l) {
    const t = l.tour;
    const n = t.points.length;
    const tip = `Claude pointed to ${plural(n, "place")}: ${t.i + 1} of ${n}`;
    return `<span class="tourg" id="rtour" role="group" aria-label="${esc(tip)}" title="${esc(tip)}"><span class="tpos">◆ ${t.i + 1}/${n}</span>`
      + `<button type="button" class="icon" data-tour="prev" aria-label="Previous place (p)" title="Previous place (p)"><kbd>p</kbd></button>`
      + `<button type="button" class="icon" data-tour="next" aria-label="Next place (n)" title="Next place (n)"><kbd>n</kbd></button>`
      + `<button type="button" class="icon" data-tour="back" aria-label="Back to where you were (')" title="Back to where you were (')"><kbd>'</kbd></button></span>`;
  }

  function drawDiff(r) {
    const l = local(r.id);
    const s = l.mode === "files" ? null : steps(r)[l.cur];
    const key = diffKey(r);
    const d = key && l.diff.get(key);
    const box = $("#rdiff");
    // A view of an older head never takes a click: refetch until the server has the
    // review's head. (During an update the server loads the new commit for a while.)
    if (d && d.head && r.head && d.head !== r.head) {
      l.diff.delete(key);
      clearTimeout(l.retry);
      // A failed update keeps the old model: show the error, and stop asking.
      if (r.status === "error") { put(box, `<div class="rerror">${esc(r.error || "The new commit did not load. Use Retry.")}</div>`); return; }
      put(box, `<div class="rnote">Loading the new commit…</div>`);
      l.retry = setTimeout(() => loadDiff(r.id), 800);
      return;
    }
    if (!d || !d.sections?.length) { put(box, d?.error ? `<div class="rerror">${esc(d.error)}</div>` : ""); return; }
    // The notes placed under a line; the take line lists the others.
    const placed = new Set();
    const pts = tourPts(l);
    const curPt = l.tour ? pts[l.tour.i] : null;
    // The file that holds the current place gets the tour; else the first file.
    const tourAt = l.tour ? Math.max(0, d.sections.findIndex((sec) => curPt && (curPt.side === "old" ? sec.old_path || sec.path : sec.path) === curPt.file)) : -1;
    const html = d.sections.map((sec, si) => {
      const inRange = (side, num) => sec.ranges.some((g) => g.side === side && num >= g.from && num <= g.to);
      const anchored = new Set(r.threads.filter((t) => (!s || t.step === s.id) && t.line && t.path === sec.path).map((t) => `${t.side || "new"}:${t.line}`));
      const pathOf = (side) => (side === "old" ? sec.old_path || sec.path : sec.path);
      // Take notes on this file: the step's in Guide, every step's in Files.
      const noted = (l.mode === "files" ? steps(r) : s ? [s] : []).flatMap((x) => takeNotes(r, x.id, x.take)).filter((n) => n.line && n.file === pathOf(n.side));
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
        const here = noted.filter((n) => n.side === rowSide && n.line === num);
        here.forEach((n) => placed.add(n.key));
        const dot = worst(here.filter((n) => !n.dismissed));
        // Claude's places on this row: on its side, or on the old side of a context row.
        const lineOf = (p) => (p.side === "old" && row.kind === " " ? row.old : num);
        const onRow = (p) => covers(p, pathOf(rowSide), rowSide, num) || (row.kind === " " && covers(p, pathOf("old"), "old", row.old));
        const onPts = pts.filter(onRow);
        const starts = pts.map((p, i) => [p, i]).filter(([p]) => onRow(p) && lineOf(p) === p.line);
        const ends = pts.map((p, i) => [p, i]).filter(([p]) => onRow(p) && lineOf(p) === (p.end || p.line));
        const cls = [KIND_CLASS[row.kind], focus ? "focus" : "", isAnchor ? "anchor" : "", anchored.has(`${rowSide}:${num}`) ? "anchored" : "", sel ? "sel" : "", dot ? `noted-${dot}` : "", onPts.length ? "pt" : "", curPt && onPts.includes(curPt) ? "ptcur" : ""].join(" ");
        const pboxes = ends.map(([p, i]) => `<tr class="prow"><td colspan="3">${pointBox(p, i, p === curPt, !sec.context)}</td></tr>`).join("");
        const notes = here.map((n) => `<tr class="drow"><td colspan="3">${noteCard(n, { show: l.shownNotes?.has?.(n.key), added: l.added?.has?.(n.key) })}</td></tr>`).join("");
        const sign = row.kind === " " ? " " : row.kind;
        // A stale draft has no line at this head: it shows only in the list.
        const mine = r.drafts.filter((x) => !x.stale && x.path === pathOf(rowSide) && (x.side || "new") === rowSide && x.line === num);
        const cards = mine.map((x) => `<tr class="drow"><td colspan="3"><div class="dcard${x.agent ? " agent" : ""}" data-d="${x.id}">
          <div class="dmeta"><span class="who">${x.agent ? "Agent draft" : "Your comment"}</span><span class="dim">pending</span>${x.start_line ? `<span>lines ${x.start_line}-${x.line}</span>` : ""}<button type="button" class="icon x" data-ddel="${x.id}" aria-label="Delete comment">×</button></div>
          <div class="dtext" contenteditable="true">${esc(x.text)}</div></div></td></tr>`).join("");
        const form = c && c.side === rowSide && c.line === num ? `<tr class="crow"><td colspan="3"><form class="cform" id="rcform">
          <div class="cat">Comment on ${esc(c.path)}:${c.start < c.line ? `${c.start}-${c.line}` : c.line}${c.side === "old" ? " (old)" : ""} <span class="hint">Shift-click a + to choose a range</span></div>
          <textarea id="rctext" placeholder="Leave a comment">${esc(c.text || "")}</textarea>
          <div class="cbtns"><button type="button" class="btn small" data-ccancel>Cancel</button><button type="submit" class="btn small primary">Add to review</button></div>
        </form></td></tr>` : "";
        const oldNum = row.kind === " " && row.old != null ? ` data-o="${row.old}"` : "";
        // The number of a place where it starts, drawn by CSS: the cell text stays the line number.
        const mk = starts.length ? ` data-mk="${starts.map(([, i]) => i + 1).join(",")}"${starts.some(([p]) => p === curPt) ? " data-mkon" : ""}` : "";
        return `<tr class="${cls}"><td class="cm"><button type="button" class="cmb" data-cm data-sec="${si}" data-side="${rowSide}" data-n="${num}" aria-label="Comment on line ${num}">+</button></td><td class="ln" data-sec="${si}" data-side="${rowSide}" data-ln="${num}"${oldNum}${mk}>${num}</td><td class="src"><span class="sg">${sign}</span>${esc(row.text)}</td></tr>${pboxes}${notes}${cards}${form}`;
      }).join("");
      const moved = sec.old_path && sec.old_path !== sec.path ? ` <span class="ctxnote">from ${esc(sec.old_path)}</span>` : "";
      const note = sec.note ? ` <span class="ctxnote">${esc(sec.note)}</span>` : "";
      // A step shows only its part of the file: one short note says how much is elsewhere.
      const o = sec.other;
      const nums = o ? o.steps.map((sid) => steps(r).findIndex((x) => x.id === sid)).filter((i) => i >= 0).map((i) => i + 1) : [];
      const where = nums.length ? ` in ${nums.length} other step${nums.length === 1 ? "" : "s"}` : " outside this step";
      const more = o ? `<span class="ctxnote other" title="${nums.length ? `Steps ${nums.join(", ")}` : ""}">+${o.lines} line${o.lines === 1 ? "" : "s"}${where}</span>` : "";
      const whole = o ? `<button type="button" class="btn sm" data-whole="${esc(sec.path)}" title="Show the whole file in Files">Whole file</button>` : "";
      const f = l.mode === "files" ? curFile(l) : null;
      const stat = f && f.path === sec.path ? `<span class="fstat"><span class="plus">+${f.added}</span><span class="minus">−${f.removed}</span></span>` : "";
      return `<section class="fsec"><div class="file"><span class="fp">${esc(sec.path)}${moved}${note}${sec.context ? ' <span class="ctxnote">Not changed in this PR</span>' : ""}</span>${stat}${more}${whole}<button type="button" class="btn sm" data-nvim="${si}" title="nvim at this line, next to the old side (o)">nvim <kbd>o</kbd></button>${si === tourAt ? tourGroup(l) : ""}</div>
        <div class="codewrap"><table class="code">${rows}</table></div></section>`;
    }).join("");
    const changed = put(box, html);
    l.placed = placed;
    drawTake(r);
    if (changed) wireComments(r, d);
    // A click on the tour leaves the keys where they are, in the terminal too.
    box.onmousedown = (e) => { if (e.target.closest("[data-tour]")) e.preventDefault(); };
    box.onclick = async (e) => {
      const tb = e.target.closest("[data-tour]")?.dataset.tour;
      if (tb) { if (tb === "prev") tourStep(r.id, -1); if (tb === "next") tourStep(r.id, 1); if (tb === "back") back(r.id); return; }
      const padd = e.target.closest("[data-padd]");
      if (padd) { addPoint(r, d, +padd.dataset.padd); return; }
      if (e.target.closest(".prow")) return;
      if (e.target.closest(".tnote, .tdismissed")) { noteAction(r, e); return; }
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
        showNvim(r.id, { path: sec.path, line });
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
    if (l.target && applyTarget(r, d, key)) return;
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
          await call("review_comment", { id: r.id, comment: { path: c.path, side: c.side, line: c.line, start_line: c.start < c.line ? c.start : null, text, head: d.head || "", line_text: lineText(l, d, c), agent: !!c.fromTake } });
          if (c.fromTake) (l.added ||= new Set()).add(c.fromTake);
          // Place again: the old draft goes only when this is the same comment.
          if (l.placing && text === l.placing.text.trim()) await call("review_draft_edit", { id: r.id, draft: l.placing.id, text: null }).catch(() => {});
          l.placing = null;
          l.compose = null;
          drawDiff(S.reviews.get(r.id) || r);
        } catch (_) {}
      });
    }
    $("#rdiff").querySelectorAll(".dcard[data-d]").forEach((card) => {
      const id = +card.dataset.d;
      const t = card.querySelector(".dtext");
      t.addEventListener("keydown", (e) => e.stopPropagation());
      t.addEventListener("blur", () => call("review_draft_edit", { id: r.id, draft: id, text: t.textContent.trim() || null }));
      card.querySelector("[data-ddel]").onclick = () => call("review_draft_edit", { id: r.id, draft: id, text: null });
    });
  }

  const EVENTS = [["COMMENT", "Comment", "General feedback, no approval."], ["APPROVE", "Approve", "Give your approval to merge."], ["REQUEST_CHANGES", "Request changes", "Feedback that must be fixed before a merge."]];
  const EVENT_NAME = Object.fromEntries(EVENTS.map(([k, v]) => [k, v]));

  // Your comments, in the inspector: what you sent, and each draft with its place.
  // A stale draft has no line in the diff, so it has its note and buttons only here.
  function draftsHtml(r) {
    const n = r.drafts.length;
    const sent = (r.posted || []).map((p) => p.event === "AGENT"
      ? `<div class="sent">Sent to the agent: ${plural(p.comments, "comment")} on ${esc(short(p.head))}</div>`
      : `<div class="sent">Sent: ${esc(EVENT_NAME[p.event] || p.event)}, ${plural(p.comments, "line comment")} <a href="${esc(p.url)}">open on GitHub</a></div>`).join("");
    const rows = r.drafts.map((d) => {
      const lines = d.start_line ? `${d.start_line}-${d.line}` : d.line;
      const loc = d.path ? `${d.path}${d.line ? ":" + lines : ""}${d.side === "old" ? " (old)" : ""}` : "";
      const long = d.text.split("\n").length > 2 || d.text.length > 240;
      const stale = d.stale ? `<em class="stalenote">The code changed after you wrote this. Place it again or delete it.</em><span class="stalebtns"><button type="button" class="btn sm" data-place="${d.id}">Place again</button><button type="button" class="btn sm ghost" data-drop="${d.id}">Delete</button></span>` : "";
      return `<div class="draft${long ? " long" : ""}${d.agent ? " agent" : ""}${d.stale ? " stale" : ""}" data-d="${d.id}"><span class="dloc">${esc(loc)}${d.agent ? ' <i>agent</i>' : ""}</span>${stale}<span class="dedit" contenteditable="true" data-dedit="${d.id}">${esc(d.text)}</span></div>`;
    }).join("");
    return `<div class="isec"><div class="isech">Comments · ${n}</div><div class="drafts" id="rdrafts">${sent}${rows || (sent ? "" : '<p class="inone">No comments yet. Hover a line and click +.</p>')}</div></div>`;
  }

  // Send to agent (SPEC 11.11): the comments go into the agent's prompt; you press Enter.
  async function sendAgent(id) {
    try {
      const n = await invoke("review_send_agent", { id, note: "" });
      toast(`${n} comment${n === 1 ? "" : "s"} in the agent's prompt. Press Enter there to send.`);
      if (local(id).term) window.SB.term.focus();
    } catch (e) {
      toast(e);
    }
  }

  async function copyComments(id) {
    try {
      copy(await call("review_drafts_text", { id }));
      toast("The comments are copied.");
    } catch (_) {}
  }

  // Finish review (SPEC 11.8): a popover under the header. You pick the type and
  // write the summary. The app checks every comment against GitHub's diff, shows
  // what goes where, and sends only on your click. While it is open, Submit is the
  // one gold button.
  async function openFinish(id) {
    const r = S.reviews.get(id);
    const l = local(id);
    const pop = $("#rfinpop");
    if (!r || !pop) return;
    l.finish = { event: l.lastEvent || "COMMENT", preview: null, error: null, sending: false };
    pop.hidden = false;
    pop.innerHTML = `<div class="fphead"><span class="fptitle">Finish review</span><button type="button" class="icon" data-fclose aria-label="Close" title="Close (Escape)">${window.SB.ICON.close}</button></div>
      <div class="fseg" role="radiogroup" aria-label="Type of review">${EVENTS.map(([k, v, tip]) => `<button type="button" role="radio" data-ev="${k}" aria-checked="${k === l.finish.event}" title="${esc(tip)}">${v}</button>`).join("")}</div>
      <label class="flabel" for="rsummary">Summary</label>
      <textarea id="rsummary" rows="3" placeholder="Leave a comment on the whole PR">${esc(r.summary || "")}</textarea>
      <div class="fpreview" id="rpreview">Checking every comment against GitHub's diff…</div>
      <div class="fbtns"><button type="button" class="btn" data-fclose>Cancel</button><button type="button" class="btn primary" data-fsend disabled>Submit review</button></div>`;
    drawHead(r);
    let timer;
    $("#rsummary").addEventListener("input", (e) => {
      clearTimeout(timer);
      const text = e.target.value;
      timer = setTimeout(() => call("review_summary", { id, text }).catch(() => {}), 400);
    });
    $("#rsummary").addEventListener("keydown", (e) => { if (e.key !== "Escape") e.stopPropagation(); });
    pop.onclick = (e) => {
      const ev = e.target.closest("[data-ev]");
      if (ev && l.finish) {
        l.finish.event = l.lastEvent = ev.dataset.ev;
        pop.querySelectorAll("[data-ev]").forEach((b) => b.setAttribute("aria-checked", String(b === ev)));
      }
      if (e.target.closest("[data-fclose]")) closeFinish(id);
      if (e.target.closest("[data-fsend]")) send(id);
    };
    pop.onkeydown = (e) => { if (e.key === "Escape") { e.stopPropagation(); closeFinish(id); } };
    $("#rsummary").focus();
    const f = l.finish;
    try {
      f.preview = await invoke("review_post_preview", { id });
    } catch (e) {
      f.error = String(e);
    }
    drawPreview(id);
  }

  function closeFinish(id) {
    const l = local(id);
    if (l.finish?.sending) return;
    const text = $("#rsummary")?.value;
    if (text !== undefined) call("review_summary", { id, text }).catch(() => {});
    l.finish = null;
    const pop = $("#rfinpop");
    if (pop) { pop.hidden = true; pop.innerHTML = ""; }
    const r = S.reviews.get(id);
    if (r && S.tab === id) drawHead(r);
  }

  // What the send does: who posts, on which commit, the comments on lines, and the
  // comments that go into the summary because GitHub's diff has no such line.
  function drawPreview(id) {
    const l = local(id);
    const f = l.finish;
    const box = $("#rpreview");
    if (!f || !box) return;
    const btn = $("#rfinpop [data-fsend]");
    if (f.error) {
      box.innerHTML = `<div class="ferr">${esc(f.error)}</div><div><button type="button" class="btn sm" data-recheck>Check again</button></div>`;
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
    if (!f.preview) return;
    const p = f.preview.plan;
    const at = (c) => `${c.path}:${c.start_line ? `${c.start_line}-${c.line}` : c.line}${c.side === "LEFT" ? " (old)" : ""}`;
    const agent = new Set((S.reviews.get(id)?.drafts || []).filter((d) => d.agent).map((d) => d.id));
    const tag = (c) => (agent.has(c.draft) ? ' <i class="agent">agent draft</i>' : "");
    const nAgent = p.inline.filter((c) => agent.has(c.draft)).length;
    box.innerHTML = `<dl class="fplan">
        <dt>As</dt><dd class="mono">@${esc(f.preview.login)}</dd>
        <dt>On commit</dt><dd class="mono">${esc(short(p.head))}</dd>
        <dt>On lines</dt><dd>${plural(p.inline.length, "comment")}${nAgent ? `, ${plural(nAgent, "agent draft")}` : ""}${p.inline.length ? ' <span class="ok">✓ same text as GitHub\'s diff</span>' : ""}</dd>
        ${p.outside.length ? `<dt>In the summary</dt><dd>${plural(p.outside.length, "comment")}, outside GitHub's diff</dd>` : ""}
      </dl>
      ${p.errors.length ? `<div class="ferr">Nothing can be sent:<ul>${p.errors.map((x) => `<li>${esc(x)}</li>`).join("")}</ul></div>` : ""}
      ${p.inline.length ? `<ul class="flist">${p.inline.map((c) => `<li><code>${esc(at(c))}</code>${tag(c)} ${esc(c.text)}</li>`).join("")}</ul>` : ""}
      ${p.outside.length ? `<ul class="flist warn">${p.outside.map((c) => `<li><code>${esc(c.at || "general")}</code>${tag(c)} ${esc(c.text)} <span class="why">${esc(c.reason)}</span></li>`).join("")}</ul>` : ""}`;
    btn.disabled = p.errors.length > 0 || f.sending;
  }

  async function send(id) {
    const l = local(id);
    const f = l.finish;
    if (!f?.preview || f.sending) return;
    f.sending = true;
    const btn = $("#rfinpop [data-fsend]");
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

  // Starts the review again from the PR, against its base branch as it is now
  // (SPEC 11.13). You confirm first: the guide, the check marks and Claude start fresh.
  function restartReview(id) {
    const r = S.reviews.get(id);
    const base = r?.base_ref ? `<code>${esc(r.base_ref)}</code>` : "its base branch";
    showBox(id, `<h4>Restart the review</h4><p>The app fetches the PR again, compares it with ${base} as it is now, and writes a new guide.</p>
      <p>Your comments stay and move with their code. The check marks and the Claude pane start fresh.</p>`,
      [["Cancel", null], ["Restart", "go"]], async (v) => {
        if (v !== "go") return;
        const l = local(id);
        l.diff.clear();
        l.files = null;
        l.cur = 0;
        l.anchor = null;
        l.term = null;
        l.view = "agent";
        l.chat = null;
        try {
          await invoke("review_restart", { id });
        } catch (e) {
          toast(e);
        }
        const now = S.reviews.get(id);
        if (now) update(now);
      });
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

  // The column shows the agent or nvim (`l.view`). nvim opens at the file and the
  // line that you are on: the line you clicked, else the first new line in view.
  function spot(id) {
    const l = local(id);
    const r = S.reviews.get(id);
    const d = r && l.diff.get(diffKey(r));
    if (!d?.sections?.length) return null;
    if (l.anchor) {
      const sec = d.sections.find((x) => x.path === l.anchor.path || x.old_path === l.anchor.path);
      if (sec && l.anchor.side !== "old") return { path: sec.path, line: l.anchor.line };
      if (sec) return { path: sec.path, line: (sec.ranges.find((g) => g.side === "new") || sec.ranges[0])?.from || 1 };
    }
    const box = $("#rcenter")?.getBoundingClientRect();
    const seen = box && [...$$('#rdiff td.ln[data-side="new"]')].find((td) => {
      const b = td.getBoundingClientRect();
      return b.height > 0 && b.top >= box.top && b.bottom <= box.bottom;
    });
    if (seen) return { path: d.sections[+seen.dataset.sec].path, line: +seen.dataset.ln };
    const sec = d.sections[0];
    return { path: sec.path, line: (sec.ranges.find((g) => g.side === "new") || sec.ranges[0])?.from || 1 };
  }

  async function showNvim(id, at) {
    const l = local(id);
    const where = at || spot(id);
    if (!where) return toast("No file to open yet.");
    try {
      l.nvim = await call("review_nvim", { id, path: where.path, line: where.line });
    } catch (_) {
      return;
    }
    l.view = "nvim";
    if (!window.SB.paneById(l.nvim)) await window.SB.refreshPanes();
    const host = $("#rterm");
    if (S.tab !== id || !host) return;
    await window.SB.expand(l.nvim, host);
    drawView(id);
    drawTermWait(S.reviews.get(id));
  }

  function showAgent(id) {
    local(id).view = "agent";
    drawView(id);
    return showTerm(id);
  }

  // Leader e in a review tab: nvim, or back to the agent.
  function toggleNvim(id) {
    const l = local(id);
    return l.view === "nvim" && S.live === l.nvim ? showAgent(id) : showNvim(id);
  }

  function drawView(id) {
    const v = local(id).view === "nvim" ? "nvim" : "agent";
    $$("[data-rview]").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.rview === v)));
  }

  async function showTerm(id) {
    const l = local(id);
    // A redraw keeps nvim in the column while you look at it.
    if (l.view === "nvim" && l.nvim && window.SB.paneById(l.nvim) && S.tab === id && $("#rterm")) {
      await window.SB.expand(l.nvim, $("#rterm"));
      drawTermWait(S.reviews.get(id));
      return;
    }
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
    const shown = S.termHost === r.id && ((l.term && S.live === l.term) || (l.view === "nvim" && l.nvim && S.live === l.nvim));
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
    if (S.live !== l.term) await showAgent(id);
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

  // ---------- the chat of each step (SPEC 11.5) ----------
  // The review's Claude pane is a full-screen app: its history is not in the terminal
  // scrollback. The app reads it from Claude's session file, by step.
  function seen(id) {
    const r = S.reviews.get(id);
    const s = r && steps(r)[local(id).cur];
    if (s && r.kind !== "local") invoke("review_step_seen", { id, step: s.id }).catch(() => {});
  }
  async function loadChat(id) {
    const r = S.reviews.get(id);
    if (!r || r.kind === "local") return;
    let chat;
    try { chat = await invoke("review_chat", { id }); } catch (_) { return; }
    const l = local(id);
    // The same chat draws nothing: an open answer stays open and in place.
    if (JSON.stringify(chat) === JSON.stringify(l.chat || [])) return;
    l.chat = chat;
    if (S.tab === id) drawChat(S.reviews.get(id));
  }
  function drawChat(r) {
    const el = $("#rchat");
    if (!el || !r) return;
    const l = local(r.id);
    const s = steps(r)[l.cur];
    const all = l.chat || [];
    const mine = l.chatAll ? all : all.filter((t) => s && t.step === s.id);
    el.hidden = !all.length;
    if (!all.length) return;
    // Keep the answers you opened or closed, and the scroll place.
    const opened = new Map([...el.querySelectorAll("details")].map((x) => [x.dataset.k, x.open]));
    const top = el.querySelector(".clist")?.scrollTop || 0;
    const keyOf = (t) => `${t.at}|${t.question}`;
    const name = (sid) => { const i = steps(r).findIndex((x) => x.id === sid); return i >= 0 ? `Step ${i + 1}` : "Before the steps"; };
    el.innerHTML = `<div class="chead"><span>${l.chatAll ? "All steps" : "On this step"} · ${mine.length}</span><button data-chall>${l.chatAll ? "This step" : "All steps"}</button></div>
      <div class="clist">${mine.map((t, i) => `<details data-k="${esc(keyOf(t))}"${(opened.has(keyOf(t)) ? opened.get(keyOf(t)) : i === mine.length - 1) ? " open" : ""}><summary>${l.chatAll ? `<span class="cstep">${esc(name(t.step))}</span>` : ""}${esc(t.question)}</summary><div class="md">${md(t.answer || "…")}</div></details>`).join("") || `<div class="cempty">No questions on this step yet.</div>`}</div>`;
    el.querySelector(".clist").scrollTop = top;
    el.querySelector("[data-chall]").onclick = () => { l.chatAll = !l.chatAll; drawChat(r); };
  }

  function go(id, i) {
    const r = S.reviews.get(id);
    const l = local(id);
    l.cur = Math.max(0, Math.min(steps(r).length - 1, i));
    l.anchor = null;
    l.ctx = null;
    l.takeOpen = false;
    l.takeMenu = false;
    savePlace(id);
    update(r);
    seen(id);
    drawChat(r);
    $(`#rsteps [data-i="${l.cur}"]`)?.scrollIntoView({ block: "nearest" });
  }

  function next(id) {
    const r = S.reviews.get(id);
    const s = steps(r)[local(id).cur];
    if (s && !stepState(r, s.id).checked) call("review_mark", { id, step: s.id, checked: true });
    go(id, local(id).cur + 1);
  }

  // ---------- Claude points to the code (SPEC 11.15) ----------
  // Three ways in: a file:line link in the Claude column's terminal, the "Mentioned"
  // chips of its last answer, and the places of its review_show tool, as a tour.
  // Each only moves the view, and none takes the keys from the terminal.
  const KIND = { must: "must fix", should: "should fix", nit: "nit", question: "question" };
  const covers = (p, file, side, line) => !!p && p.file === file && (p.side || "new") === side && line >= p.line && line <= (p.end || p.line);
  const tourPts = (l) => (l.tour ? l.tour.points : []);

  // The step whose ranges hold the line, or -1.
  function stepOf(r, at) {
    if (!at.line) return -1;
    const side = at.side === "old" ? "old" : "new";
    return steps(r).findIndex((s) => (s.ranges || []).some((g) => g.file === at.file && g.side === side && at.line >= g.from && at.line <= g.to));
  }

  // The changed files, for links and jumps: the Files list, or a copy of it.
  function changedFiles(id) {
    const l = local(id);
    if (l.files) return Promise.resolve(l.files);
    if (l.pathList) return Promise.resolve(l.pathList);
    l.pathLoad ||= invoke("review_files", { id }).then((v) => (l.pathList = v || [])).catch(() => { l.pathLoad = null; return []; });
    return l.pathLoad;
  }

  // The changed file a name means: the full path, a path that ends with it (an
  // absolute path), or the one file whose path ends with "/name". Two: none.
  function resolveFile(files, name) {
    const n = name.replace(/^\.\//, "");
    const exact = files.find((f) => f.path === n || f.old_path === n);
    if (exact) return exact.path;
    const hits = files.filter((f) => n.endsWith("/" + f.path) || f.path.endsWith("/" + n));
    return hits.length === 1 ? hits[0].path : null;
  }

  // Where you were, for Back. A small list.
  function remember(id) {
    const l = local(id);
    l.hist.push({ mode: l.mode, cur: l.cur, file: l.file, ctx: l.ctx, top: $("#rcenter")?.scrollTop || 0 });
    if (l.hist.length > 10) l.hist.shift();
  }

  // Back: where you were before the tour or the jump. It ends the tour.
  function back(id) {
    const l = local(id);
    const h = l.hist.pop();
    l.tour = null;
    l.target = null;
    if (h) Object.assign(l, { mode: h.mode, cur: h.cur, file: h.file, ctx: h.ctx, anchor: null, compose: null });
    savePlace(id);
    update(S.reviews.get(id));
    if (h) { const c = $("#rcenter"); if (c) c.scrollTop = h.top; }
  }

  // Go to a place: the step that holds it, else its file in Files, else the lines
  // of a file the PR does not change. The diff scrolls to the line and flashes it.
  async function goTo(id, at, push = true) {
    const r = S.reviews.get(id);
    if (!r || !at?.file) return;
    const l = local(id);
    const t = { file: at.file, line: +at.line || null, end: +at.end || null, side: at.side === "old" ? "old" : "new" };
    const i = r.kind === "local" ? -1 : stepOf(r, t);
    let files = null;
    if (i < 0) files = await changedFiles(id);
    const f = files && files.find((x) => x.path === t.file || (t.side === "old" && x.old_path === t.file));
    if (i < 0 && !f && !t.line) { toast(`${t.file} is not changed in this PR.`); return; }
    if (push) remember(id);
    Object.assign(l, { anchor: null, compose: null, ctx: null, target: t });
    if (i >= 0) { l.mode = "guide"; l.cur = i; }
    else if (f) { l.mode = "files"; l.file = f.path; l.files ||= files; }
    else l.ctx = { path: t.file, line: t.line };
    savePlace(id);
    const now = S.reviews.get(id);
    if (now && S.tab === id) {
      update(now);
      $(`#rsteps [data-i="${l.cur}"]`)?.scrollIntoView({ block: "nearest" });
    }
  }

  // Scrolls the diff to the place of `l.target` and flashes its lines. A hidden line
  // of a changed file shows first. True when it scrolled.
  function applyTarget(r, d, key) {
    const l = local(r.id);
    const t = l.target;
    const si = d.sections.findIndex((sec) => (t.side === "old" ? sec.old_path || sec.path : sec.path) === t.file || sec.path === t.file);
    if (si < 0) { l.target = null; return false; }
    (l.scrolled ||= new Set()).add(key);
    const sec = d.sections[si];
    const cell = (n) => $(`#rdiff td.ln[data-sec="${si}"][data-side="${t.side}"][data-ln="${n}"]`) || (t.side === "old" ? $(`#rdiff td.ln[data-sec="${si}"][data-o="${n}"]`) : null);
    if (!t.line) {
      l.target = null;
      ($$("#rdiff tr.focus")[0] || $$("#rdiff .file")[si])?.scrollIntoView({ block: "center" });
      return true;
    }
    const td = cell(t.line);
    if (td) {
      l.target = null;
      td.closest("tr").scrollIntoView({ block: "center" });
      for (let n = t.line; n <= (t.end || t.line); n++) {
        const row = cell(n)?.closest("tr");
        if (!row) continue;
        row.classList.remove("flash");
        void row.offsetWidth;
        row.classList.add("flash");
      }
      return true;
    }
    // The line is in a gap of the diff: get the whole file and show it with 3 lines around.
    if (t.tried || sec.context) { l.target = null; toast(`Line ${t.line} of ${t.file} is not in the file.`); return false; }
    t.tried = true;
    (async () => {
      if (!l.full.has(sec.path)) {
        try { l.full.set(sec.path, (await invoke("review_file_rows", { id: r.id, path: sec.path })).rows); } catch (e) { l.target = null; toast(e); return; }
      }
      const lines = fileLines(l.full.get(sec.path));
      const k = lines.findIndex((x) => x.row.kind !== "@" && (t.side === "old" ? x.row.kind !== "+" && x.row.old === t.line : x.row.kind !== "-" && x.row.new === t.line));
      if (k >= 0) {
        const set = new Set(l.shown.get(`${key}|${sec.path}`) || []);
        const z = Math.min(lines.length - 1, k + 3 + ((t.end || t.line) - t.line));
        for (let j = Math.max(0, k - 3); j <= z; j++) set.add(j);
        l.shown.set(`${key}|${sec.path}`, set);
      }
      const now = S.reviews.get(r.id);
      if (now && S.tab === r.id && diffKey(now) === key) drawDiff(now);
    })();
    return true;
  }

  // The tour: the places of the last review_show call, from place i.
  function startTour(id, i) {
    const r = S.reviews.get(id);
    const l = local(id);
    const pts = r?.show?.points || [];
    if (!pts.length) return false;
    if (!l.tour) remember(id);
    if (!l.tour || l.tour.at !== r.show.at) l.tour = { at: r.show.at, points: pts, i: 0 };
    l.tour.i = Math.max(0, Math.min(pts.length - 1, i));
    goTo(id, pts[l.tour.i], false);
    return true;
  }

  function tourStep(id, dir) {
    const l = local(id);
    if (!l.tour) return;
    const n = l.tour.points.length;
    l.tour.i = (l.tour.i + dir + n) % n;
    goTo(id, l.tour.points[l.tour.i], false);
  }

  // A place in the diff: a dashed card with its number, its kind, Claude's note, and
  // Add as comment. The current place has a solid light blue edge.
  function pointBox(p, i, cur, canAdd) {
    const kind = p.kind ? (SEV[p.kind] ? sevChip(p.kind, SEV[p.kind]) : `<span class="pkind k-${esc(p.kind)}">${esc(KIND[p.kind] || p.kind)}</span>`) : "";
    const not = p.changed ? "" : `<span class="pnot">Not changed in this PR</span>`;
    return `<div class="pbox${cur ? " cur" : ""}" data-pt="${i}"><div class="dmeta"><span class="mk${cur ? " on" : ""}">${i + 1}</span><span class="who">Claude pointed here</span>${kind}${not}</div><div class="ptext md">${mdi(p.note)}</div>${canAdd ? `<div class="nacts"><button type="button" class="btn sm" data-padd="${i}">Add as comment</button></div>` : ""}</div>`;
  }

  // Add as comment on a place: the comment form on its lines, with Claude's note.
  // It saves as an agent draft, as a take note does.
  function addPoint(r, d, i) {
    const l = local(r.id);
    const p = tourPts(l)[i];
    if (!p) return;
    const sec = d.sections.find((x) => (p.side === "old" ? x.old_path || x.path : x.path) === p.file);
    if (!sec) return;
    l.compose = { sec: sec.path, path: p.file, side: p.side || "new", start: p.line, line: p.end || p.line, text: p.note, fromTake: `pt|${p.file}|${p.line}|${p.note}` };
    drawDiff(r);
    $("#rctext")?.focus();
  }

  // The head of a file the PR does not change.
  function drawCtxHead(r) {
    const c = local(r.id).ctx;
    put($("#rguide"), `<div class="eyebrow">Outside the diff</div><h4 class="fpath">${esc(c.path)}</h4>
      <div class="ctxlead"><span class="pnot">Not changed in this PR</span> The lines around ${esc(c.line)}, at ${esc(short(r.head))}. Back <kbd>'</kbd> goes to where you were.</div>`);
    $("#rguide").onclick = null;
  }

  // "Mentioned": the places of Claude's last answer, under its column. A chip with a
  // number is a place of review_show; a dashed chip is a line the PR does not change.
  function drawMentions(r) {
    const el = $("#rment");
    if (!el) return;
    const ms = r.mentions || [];
    el.hidden = !ms.length;
    if (!ms.length) { el.innerHTML = ""; return; }
    const pts = r.show?.points || [];
    const l = local(r.id);
    const cur = l.tour?.points[l.tour.i];
    const ptOf = (m) => (m.line ? pts.findIndex((p) => covers(p, m.file, m.side || "new", m.line)) : -1);
    el.innerHTML = `<div class="mhead">Mentioned</div><div class="mchips">${ms.map((m, k) => {
      const pi = ptOf(m);
      const at = m.line ? `:${m.line}${m.end ? `-${m.end}` : ""}` : "";
      const label = m.name ? `${m.name} ${at}` : `${m.file.split("/").pop()}${at}`;
      const on = cur && pi >= 0 && covers(cur, m.file, m.side || "new", m.line);
      const tip = `${m.file}${at}${m.side === "old" ? " (old)" : ""}${m.changed ? "" : " · not changed in this PR"}`;
      return `<button type="button" class="mchip${m.changed ? "" : " unchanged"}${on ? " on" : ""}" data-m="${k}" title="${esc(tip)}"><span class="mk${on ? " on" : ""}">${pi >= 0 ? pi + 1 : "·"}</span>${esc(label)}</button>`;
    }).join("")}</div>`;
    el.onmousedown = (e) => e.preventDefault();
    el.onclick = (e) => {
      const b = e.target.closest("[data-m]");
      if (!b) return;
      const m = ms[+b.dataset.m];
      const pi = ptOf(m);
      if (pi >= 0) startTour(r.id, pi);
      else goTo(r.id, m);
    };
  }

  // Links in the Claude column's terminal: path:line, path:from-to and the name of a
  // changed file. Other text is no link. A click goes to the place.
  const LINK_RE = /(?:[A-Za-z0-9_.-]+\/)*[A-Za-z0-9_-][A-Za-z0-9_.-]*\.[A-Za-z0-9]+(?::\d+(?:-\d+)?)?/g;
  function provideLinks(y, cb) {
    const id = S.termHost;
    const r = id && S.reviews.get(id);
    const l = r && local(id);
    if (!l || !l.term || S.live !== l.term) return cb(undefined);
    const files = l.files || l.pathList;
    if (!files) { changedFiles(id); return cb(undefined); }
    const line = window.SB.term.buffer.active.getLine(y - 1);
    if (!line) return cb(undefined);
    // The text of the row, and the column of each of its chars.
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
    for (const m of text.matchAll(LINK_RE)) {
      const [, name, from, to] = /^(.*?)(?::(\d+)(?:-(\d+))?)?$/.exec(m[0]);
      const file = resolveFile(files, name);
      if (!file) continue;
      const at = { file, line: from ? +from : null, end: to ? +to : null, side: "new" };
      links.push({
        range: { start: { x: cols[m.index], y }, end: { x: cols[m.index + m[0].length - 1], y } },
        text: m[0],
        decorations: { underline: true, pointerCursor: true },
        activate: () => goTo(id, at),
      });
    }
    cb(links.length ? links : undefined);
  }
  window.SB.term.registerLinkProvider({ provideLinks });

  // Review keys work only on a review tab, when no text field has focus (SPEC 7).
  document.addEventListener("keydown", (e) => {
    if (!S.reviews.has(S.tab) || e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.target.closest?.("textarea, input, select, [contenteditable='true']")) return;
    const id = S.tab;
    const l = local(id);
    const files = l.mode === "files";
    if (e.key === "f" && S.reviews.get(id)?.kind !== "local") setMode(id, files ? "guide" : "files");
    else if (e.key === "j") files ? goFile(id, fileIndex(l) + 1) : go(id, l.cur + 1);
    else if (e.key === "k") files ? goFile(id, fileIndex(l) - 1) : go(id, l.cur - 1);
    // In a tour of Claude's places, n and p go through them and ' goes back (SPEC 11.15).
    else if (e.key === "n" && l.tour) tourStep(id, 1);
    else if (e.key === "p" && l.tour) tourStep(id, -1);
    else if (e.key === "'" && (l.tour || l.hist.length)) back(id);
    // n, and Shift+N also in a tour: mark reviewed and go on.
    else if (e.key === "n" || e.key === "N") files ? nextFile(id) : next(id);
    // nvim at the line you are on, next to the old side (SPEC 11.14).
    else if (e.key === "o") showNvim(id);
    else return;
    e.preventDefault();
  });

  // ---------- the inspector of a review (SPEC 33) ----------
  // A PR review: an alert for new commits or an error, the facts, Claude's take on
  // the PR, your comments, and Restart and Close review. A review of local work: its
  // controls (New or All, the base, Mark reviewed), its commits and your comments.

  // The Base menu of a local review (SPEC 11.11): Automatic (the usual rule), the
  // refs the hub found, and Pick a ref… for a ref you type.
  // No ref and no commit has a `*` in its name.
  const PICK = "*pick";
  function baseControl(r, busy) {
    const l = local(r.id);
    const auto = `<option value=""${r.base_pick ? "" : " selected"}>${r.base_pick ? "Automatic" : `Automatic: ${esc(r.base_ref || "no branch")}`}</option>`;
    const list = (r.bases || []).filter((b) => b.ref && (r.base_pick || b.ref !== r.base_ref));
    if (r.base_pick && !list.some((b) => b.ref === r.base_pick)) list.unshift({ ref: r.base_pick, why: "your choice" });
    const opts = list.map((b) => `<option value="${esc(b.ref)}"${r.base_pick === b.ref ? " selected" : ""}>${esc(b.ref)}${b.why ? ` · ${esc(b.why)}` : ""}</option>`).join("");
    const tip = r.base_label ? `Compare with ${r.base_label}` : "What the commits are compared with";
    const typed = l.basePicking ? `<span class="basetype"><input type="text" data-baseref value="${esc(l.baseText || "")}" placeholder="upstream/unstable" spellcheck="false" autocomplete="off" aria-label="A branch or a commit"><button type="button" class="btn sm" data-iact="baseuse">Use</button></span>` : "";
    return `<select id="rbase" data-base title="${esc(tip)}"${busy ? " disabled" : ""}>${auto}${opts}<option value="${PICK}">Pick a ref…</option></select>${typed}`;
  }

  function setBase(id, pick) {
    const r = S.reviews.get(id);
    const l = local(id);
    if (!r) return;
    const typed = l.basePicking;
    l.basePicking = false;
    l.baseText = "";
    const same = pick === (r.base_pick || "");
    if (!same) {
      l.diff.clear();
      l.files = null;
      // A refused ref leaves the menu on the base in use.
      call("review_base", { id, pick }).catch(() => {
        window.SBInsp.refresh(true);
        const sel = $("#insp [data-base]");
        if (sel) sel.value = S.reviews.get(id)?.base_pick || "";
      });
    }
    if (typed || same) window.SBInsp.refresh(true);
  }

  function prInspector(r) {
    const l = local(r.id);
    const busy = BUSY.includes(r.status);
    const n = r.drafts.length;
    const stale = r.drafts.filter((d) => d.stale).length;
    const t = r.guide?.take;
    const prNotes = takeNotes(r, "pr", t);
    const live = prNotes.filter((x) => !x.dismissed);
    const counts = SEVS.map((v) => [v, live.filter((x) => x.severity === v).length]).filter(([, c]) => c).map(([v, c]) => `${c} ${SEV[v]}`);
    const next = (r.round || 1) + 1;
    const alert = r.status === "error" ? { text: r.error || "The guide failed.", buttons: [{ label: "Retry", act: "retry" }] }
      : r.new_head ? { text: `New commits: ${short(r.head)} → ${short(r.new_head)}`, buttons: [
        { label: `Start round ${next}`, act: "round", title: `A new review of only the changes since ${short(r.head)}` },
        { label: "Update guide", act: "update", title: "Write this guide again for the new head" }] }
      : null;
    const take = t ? `<div class="isec"><div class="isech">Claude on the PR</div>${t.summary ? `<p class="isum md">${mdi(t.summary)}</p>` : ""}${prNotes.map((x) => noteCard(x, { at: true, show: l.shownNotes?.has?.(x.key) })).join("")}</div>` : "";
    return {
      title: "Review details",
      alert,
      facts: [
        ["PR", `${r.repo} #${r.number}`],
        ["Round", r.round > 1 ? `${r.round} · changes since ${short(r.since)}` : "1"],
        ["Scope", r.round > 1 ? r.scope_note : null],
        ["Head", short(r.head), { mono: true }],
        ["Base", r.base_ref, { mono: true }],
        ["Coverage", coverageText(r)],
        ["Claude", t ? [VERDICT[t.verdict] || t.verdict, ...counts].join(" · ") : null],
        ["Comments", n ? `${n} pending${stale ? ` · ${stale} to place` : ""}` : "none yet"],
        ["Not fetched", r.status === "ready" ? r.error : null],
        ["State", busy ? r.status : null],
        ["Folder", window.SB.home(r.tree), { mono: true }],
      ],
      actions: [
        { label: "Copy all comments", act: "copy", disabled: !n, title: "Copy every comment as path:line: text" },
        { label: "Re-run Claude's take on the PR", act: "prretake", disabled: busy || !steps(r).length },
        { label: l.mode === "files" ? "Show the guide" : "Show the files", act: "mode", key: "f" },
      ],
      body: take + draftsHtml(r),
      danger: [
        ...(busy ? [] : [{ label: "Restart", act: "restart", title: `Fetch the PR again, compare it with ${r.base_ref || "its base branch"} as it is now, and write a new guide` }]),
        { label: "Close review", act: "close" },
      ],
      onAct: (act) => {
        const id = r.id;
        if (act === "retry") call("review_retry", { id });
        if (act === "update") { l.diff.clear(); l.files = null; call("review_update", { id }); }
        if (act === "round") startRound(id);
        if (act === "copy") copyComments(id);
        if (act === "prretake") call("review_retake", { id, step: null }).catch(() => {});
        if (act === "mode") setMode(id, l.mode === "files" ? "guide" : "files");
        if (act === "restart") restartReview(id);
        if (act === "close") closeReview(id);
      },
    };
  }

  function localInspector(r) {
    const l = local(r.id);
    const busy = r.status === "fetching";
    const n = r.drafts.length;
    const range = r.head ? `${short(r.base)}..${short(r.head)}` : "";
    const seg = `<span class="seg mini" role="group" aria-label="Show">${[["new", "New", r.reviewed ? "The changes since you marked the work reviewed" : "Nothing is marked reviewed yet, so this shows all of it"], ["all", "All", "Every change since the base: the commits and the uncommitted work"]]
      .map(([v, label, tip]) => `<button type="button" data-iact="range-${v}" data-range="${v}" aria-pressed="${r.range === v}" title="${esc(tip)}"${busy ? " disabled" : ""}>${label}</button>`).join("")}</span>`;
    const fact = (k, v, mono) => (v ? `<dt>${k}</dt><dd${mono ? ' class="mono"' : ""}>${v}</dd>` : "");
    const commits = (r.commits || []).length ? `<div class="isec"><div class="isech">Commits · ${r.commits.length}</div><div class="rcommits">${r.commits.map((c) => `<span class="mono">${esc(c.id)}</span><span class="csub" title="${esc(c.subject)}">${esc(c.subject)}</span>`).join("")}</div></div>` : "";
    const body = `<dl class="ifacts rlf">
        <dt>Show</dt><dd>${seg}</dd>
        <dt><label for="rbase">Base</label></dt><dd class="basecell">${baseControl(r, busy)}</dd>
        ${fact("Fetched", r.base_fetched ? esc(ago(r.base_fetched)) : "")}
        ${fact("Reviewed", r.reviewed_at ? esc(short(r.reviewed_at)) : r.reviewed ? "yes" : "", true)}
        ${fact("Range", esc(range), true)}
        ${fact("Folder", esc(window.SB.home(r.tree)), true)}
      </dl>
      ${r.scope_note ? `<p class="scopenote">${esc(r.scope_note)}</p>` : ""}
      ${commits}
      <div class="rlacts"><button type="button" class="btn" data-iact="mark" title="What shows now is reviewed: New shows only later changes. Press o for nvim next to the old side."${busy || !r.head ? " disabled" : ""}>Mark reviewed</button>
        <div class="rlrow">${r.base_ref && !busy ? `<button type="button" class="btn" data-iact="basefetch" title="Fetch only ${esc(r.base_ref)} from its remote, then compare again${r.base_fetched ? `. Fetched ${esc(ago(r.base_fetched))}.` : ""}">Update base</button>` : ""}<button type="button" class="btn" data-iact="copy"${n ? "" : " disabled"} title="Copy every comment as path:line: text">Copy comments</button></div></div>
      ${draftsHtml(r)}`;
    return {
      title: "Review details",
      alert: r.status === "error" ? { text: r.error || "The work did not load.", buttons: [{ label: "Retry", act: "retry" }] }
        : r.new_head ? { text: "New changes", buttons: [{ label: "Refresh", act: "update", title: "Read the work again; your comments move with their code" }] } : null,
      body,
      danger: [
        // No dialog: a fetch of the remotes loses nothing, and your comments move with their code.
        ...(busy ? [] : [{ label: "Restart", act: "restart", title: "Fetch the remotes, then compare the commits with the remote branch as it is now" }]),
        { label: "Close review", act: "close" },
      ],
      onAct: (act) => {
        const id = r.id;
        const now = S.reviews.get(id) || r;
        const reload = () => { l.diff.clear(); l.files = null; };
        if (act === "range-new" || act === "range-all") { const to = act.slice(6); if (to !== now.range) { reload(); call("review_range", { id, range: to }); } }
        if (act === "mark") { reload(); call("review_mark_reviewed", { id }); }
        if (act === "basefetch") { reload(); call("review_base_update", { id }).catch(() => {}); }
        if (act === "baseuse") { const v = $("#insp [data-baseref]")?.value.trim(); if (v) setBase(id, v); }
        if (act === "update" || act === "retry") { reload(); call(act === "update" ? "review_update" : "review_retry", { id }); }
        if (act === "restart") { reload(); call("review_restart", { id }); }
        if (act === "copy") copyComments(id);
        if (act === "close") closeReview(id);
      },
      // A redraw while you type a ref keeps the field and its keys.
      onDraw: (box) => {
        const input = box.querySelector("[data-baseref]");
        if (input && (l.baseFocus || document.activeElement === document.body || !document.activeElement)) {
          l.baseFocus = false;
          input.focus();
          input.setSelectionRange(input.value.length, input.value.length);
        }
      },
    };
  }

  function inspector(tab) {
    const r = S.reviews.get(tab);
    if (!r) return null;
    return r.kind === "local" ? localInspector(r) : prInspector(r);
  }
  window.SBInsp.provide("review", inspector);
  window.SBInsp.provide("local", inspector);

  // The parts of the inspector that its own API does not wire: the note actions,
  // your comments, and the base picker.
  {
    const insp = $("#insp");
    const cur = () => (S.reviews.has(S.tab) ? S.reviews.get(S.tab) : null);
    insp.addEventListener("click", (e) => {
      const r = cur();
      if (!r) return;
      if (e.target.closest("[data-tn]")) { noteAction(r, e); return; }
      const place = e.target.closest("[data-place]");
      if (place) {
        // Place again: the next + opens the form with this text; the add removes the old draft.
        const d = r.drafts.find((x) => x.id === +place.dataset.place);
        local(r.id).placing = d ? { id: d.id, text: d.text } : null;
        toast("Click + on the line for this comment.");
      }
      const drop = e.target.closest("[data-drop]");
      if (drop) call("review_draft_edit", { id: r.id, draft: +drop.dataset.drop, text: null });
    });
    insp.addEventListener("focusout", (e) => {
      const el = e.target.closest?.("[data-dedit]");
      const r = cur();
      if (!el || !r) return;
      const d = r.drafts.find((x) => x.id === +el.dataset.dedit);
      const text = el.textContent.trim();
      if (d && text !== d.text) call("review_draft_edit", { id: r.id, draft: d.id, text: text || null });
    });
    insp.addEventListener("change", (e) => {
      const sel = e.target.closest?.("[data-base]");
      const r = cur();
      if (!sel || !r) return;
      const l = local(r.id);
      if (sel.value === PICK) { l.basePicking = true; l.baseFocus = true; window.SBInsp.refresh(true); return; }
      setBase(r.id, sel.value);
    });
    insp.addEventListener("input", (e) => { const r = cur(); if (r && e.target.matches?.("[data-baseref]")) local(r.id).baseText = e.target.value; });
    insp.addEventListener("keydown", (e) => {
      const r = cur();
      if (!r) return;
      if (e.target.matches?.("[data-baseref]")) {
        const l = local(r.id);
        if (e.key === "Enter") { e.preventDefault(); const v = e.target.value.trim(); if (v) setBase(r.id, v); }
        if (e.key === "Escape") { e.preventDefault(); l.basePicking = false; window.SBInsp.refresh(true); }
        e.stopPropagation();
      } else if (e.target.matches?.("[data-dedit]")) e.stopPropagation();
    });
  }

  async function startRound(id) {
    try {
      const nid = await invoke("review_round", { id });
      if (!S.reviews.has(nid)) S.reviews.set(nid, await invoke("review_view", { id: nid }));
      window.SB.setTab(nid);
    } catch (err) { toast(err); }
  }

  window.SBReview = {
    draw,
    update: (r) => {
      const l = local(r.id);
      // A Refresh can move only the base (after a push): that is a new diff too.
      if (l.head !== r.head || l.base !== r.base || l.guideLen !== JSON.stringify(r.guide || "").length) {
        l.diff.clear();
        l.files = null;
        l.pathList = null;
        l.pathLoad = null;
        l.full.clear();
        l.shown.clear();
        l.head = r.head;
        l.base = r.base;
        l.guideLen = JSON.stringify(r.guide || "").length;
      }
      update(r);
    },
    termWait: (id) => drawTermWait(S.reviews.get(id)),
    toggleNvim,
    // A pane changed. Only a new state of the review's Claude pane (it asked, it
    // answered) reads its chat again; other panes print often and change nothing here.
    panes: () => {
      const id = S.tab;
      const r = S.reviews.get(id);
      const l = r && local(id);
      if (!r || r.kind === "local" || !l.term) return;
      const lamp = window.SB.paneById(l.term)?.lamp;
      if (lamp === l.termLamp) return;
      l.termLamp = lamp;
      clearTimeout(l.chatTimer);
      l.chatTimer = setTimeout(() => loadChat(id), 600);
    },
    // For the window checks only.
    _test: { items, reveal, STEP, provideLinks },
  };
})();
