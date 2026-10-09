// Clicks through the UI and writes the results into #__results as JSON.
(async () => {
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const $ = (s) => document.querySelector(s);
  const $$ = (s) => [...document.querySelectorAll(s)];
  const results = [];
  const check = (name, ok, info) => results.push({ name, ok: !!ok, info: ok ? undefined : info });
  const called = (cmd) => window.__calls.filter((c) => c[0] === cmd);
  const key = (k, o = {}) => window.dispatchEvent(new KeyboardEvent("keydown", { key: k, code: o.code || (k === " " ? "Space" : "Key" + k.toUpperCase()), ctrlKey: !!o.ctrl, shiftKey: !!o.shift, bubbles: true }));
  const screen = () => { const b = window.SB.term.buffer.active; let s = ""; for (let i = 0; i < b.length; i++) s += b.getLine(i).translateToString(true) + "\n"; return s; };
  const docKey = (k) => document.body.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true }));

  await wait(400);

  // Board
  check("three tiles", $$("#board .pane").length === 3, $$("#board .pane").length);
  check("unseen tiles get the class", $$(".pane.unseen").length === 2);
  check("tally lists needs before turn", /1 needs you.*1 your turn/.test($("#tally").textContent), $("#tally").textContent);
  check("ANSI is stripped from tails", !$("#board").textContent.includes("\x1b") && $("#board").textContent.includes("test result: ok"));
  check("rss shows in GB", $("#board").textContent.includes("ra 2.4 GB"));
  check("permit buttons on the tile", !!$('[data-allow="p2-1"]') && !!$('[data-deny="p2-1"]'));
  check("nvim tile has no nvim button", !$('.pane[data-id="p3"] [data-nvim]'));
  check("connection button shows a", $("#conn").textContent === "a");
  check("worktree tile is named by branch", $('.pane[data-id="p2"] .name').textContent === "il", $('.pane[data-id="p2"] .name').textContent);

  // Drag p3 onto the left half of p1: p3 goes first.
  {
    const from = $('.pane[data-id="p3"]').getBoundingClientRect();
    const to = $('.pane[data-id="p1"]').getBoundingClientRect();
    const pt = (x, y) => ({ clientX: x, clientY: y, bubbles: true, button: 0, pointerId: 1 });
    $('.pane[data-id="p3"] .term-tail').dispatchEvent(new PointerEvent("pointerdown", pt(from.left + 40, from.top + 150)));
    window.dispatchEvent(new PointerEvent("pointermove", pt(from.left + 30, from.top + 150)));
    window.dispatchEvent(new PointerEvent("pointermove", pt(to.left + 20, to.top + 150)));
    check("drag marks the drop side", $('.pane[data-id="p1"]').classList.contains("drop-before"));
    window.dispatchEvent(new PointerEvent("pointerup", pt(to.left + 20, to.top + 150)));
    $('.pane[data-id="p3"] .term-tail')?.click();
    await wait(50);
    const order = called("pane_reorder")[0]?.[1].ids;
    check("drop saves the new order", JSON.stringify(order) === '["p3","p1","p2"]', JSON.stringify(order));
    check("board shows the new order", $$("#board .pane").map((p) => p.dataset.id).join() === "p3,p1,p2");
    check("a drag is not a click", !called("pane_expand").length);
    window.__emit("panes", structuredClone(window.__data.panes));
    await wait(50);
  }

  $('[data-allow="p2-1"]').click();
  await wait(50);
  check("allow calls permit_answer", called("permit_answer").some((c) => c[1].permit === "p2-1" && c[1].allow === true));

  // Expand and the live pane
  $('.pane[data-id="p1"] .term-tail').click();
  await wait(300);
  check("click expands", called("pane_expand").some((c) => c[1].id === "p1"));
  check("live section shows", !$("#live").hidden && $("#view-board").classList.contains("has-live"));
  check("live pane is not in the strip", !$('#board .pane[data-id="p1"]'));
  // Many tiles with a live pane: they wrap in rows and stay inside the window.
  {
    const extra = Array.from({ length: 14 }, (_, i) => Object.assign(structuredClone(window.__data.panes[2]), { id: `px${i}`, tree: `/c/extra-${i}` }));
    window.__emit("panes", [...window.__data.panes, ...extra]);
    await wait(150);
    const tiles = $$("#board .pane");
    const right = Math.max(...tiles.map((t) => t.getBoundingClientRect().right));
    const rows = new Set(tiles.map((t) => Math.round(t.getBoundingClientRect().top))).size;
    check("tiles wrap into rows and stay inside the window", right <= window.innerWidth + 1 && rows > 1 && document.documentElement.scrollWidth <= window.innerWidth, `right=${right} width=${window.innerWidth} rows=${rows}`);
    check("the tile rows scroll under the live pane", $("#board").scrollHeight > $("#board").clientHeight && $("#live").getBoundingClientRect().height >= 228, `${$("#board").scrollHeight} ${$("#board").clientHeight} live=${$("#live").getBoundingClientRect().height}`);
    window.__emit("panes", window.__data.panes);
    await wait(100);
  }
  check("pane_resize before expand", window.__calls.findIndex((c) => c[0] === "pane_resize") < window.__calls.findIndex((c) => c[0] === "pane_expand"));
  await wait(100);
  check("terminal drew the screen", screen().includes("hello from the live pane"), screen().slice(0, 80));
  window.__emit("pane_output", { pane: "p1", data: btoa("streamed bytes") });
  await wait(100);
  check("stream writes to the terminal", screen().includes("streamed bytes"), screen().slice(0, 120));

  // Rename on the live title survives a pane update.
  $("#livetitle .name").dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  window.__emit("pane", Object.assign({}, window.__data.panes[0], { tail: ["new line"] }));
  await wait(50);
  check("rename survives a pane event", $("#livetitle .name")?.contentEditable === "true");
  $("#livetitle .name").textContent = "my title";
  $("#livetitle .name").dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  await wait(50);
  check("rename saves", called("pane_rename").some((c) => c[1].id === "p1" && c[1].title === "my title"));

  // Leader keys: Ctrl+Space, then x collapses.
  key(" ", { ctrl: true, code: "Space" });
  check("leader shows", !$("#leader").hidden);
  key("x");
  await wait(100);
  check("leader x collapses", $("#live").hidden && called("pane_collapse").length === 1);
  key(" ", { ctrl: true, code: "Space" });
  key("j");
  await wait(300);
  check("leader j opens the most urgent unseen tile", called("pane_expand").slice(-1)[0]?.[1].id === "p2", JSON.stringify(called("pane_expand")));

  // A pane event updates one tile.
  window.__emit("pane", Object.assign({}, window.__data.panes[0], { lamp: "limit", unseen: true, summary: "API Error: Rate limit reached", alert: true }));
  await wait(50);
  check("limit tile offers a switch", $('.pane[data-id="p1"] [data-switch]')?.textContent === "Switch to b", "live=" + window.SB.S.live + " board=" + $$("#board .pane").map((p) => p.dataset.id + ":" + p.dataset.lamp).join() + " title=" + $("#livetitle").innerHTML.slice(0, 300));

  // Launcher
  $("#newpane").click();
  await wait(100);
  check("launcher opens", !$("#launcher").hidden);
  const q = $("#lq");
  q.value = "time";
  q.dispatchEvent(new Event("input"));
  await wait(50);
  check("fuzzy filter", $$("#lrepos li[data-i]").length === 1 && $("#lrepos").textContent.includes("timeways"));
  q.value = "light";
  q.dispatchEvent(new Event("input"));
  await wait(50);
  check("dirty repo defaults to new worktree", $('#lwhere [data-v="new"]').getAttribute("aria-pressed") === "true");
  check("branch input shows for new worktree", !$("#lbranch").hidden);
  q.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  await wait(50);
  check("no branch: an error, no open", $("#lerr").textContent.includes("branch") && !called("pane_open").length, $("#lerr").textContent);
  $("#lbranch").value = "feat-x";
  $("#lgo").click();
  await wait(400);
  const open = called("pane_open")[0]?.[1].req;
  check("open sends the request", open && open.repo === "/c/lighthouse" && open.place === "new" && open.branch === "feat-x" && open.run === "claude", JSON.stringify(open));
  check("launcher closes after open", $("#launcher").hidden);

  $("#newpane").click();
  await wait(50);
  q.value = "https://github.com/sigp/lighthouse/pull/10071";
  q.dispatchEvent(new Event("input"));
  await wait(50);
  check("PR URL shows a review item", $("#lrepos").textContent.includes("Review 10071") && $("#lrows").hidden);
  $("#lgo").click();
  await wait(400);
  check("PR URL opens the review tab", called("review_open").length === 1 && !$("#view-review").hidden);

  // Review
  check("review tab button", $$('#tabs [data-tab="r1"]').length === 1);
  check("steps render", $$("#rsteps .step").length === 2);
  docKey("j");
  await wait(300);
  check("j goes to step 2", $('#rsteps [aria-current="step"]')?.dataset.i === "1");
  check("answers render markdown", !!$("#rthreads .msg.md pre code") && !!$("#rthreads .msg.md strong") && $$("#rthreads .msg.md li").length === 2);
  check("HTML in an answer is text, not code", !window.__xss && $("#rthreads").textContent.includes("<script>"));
  check("guide text renders markdown", !!$("#rguide .what strong") && !!$("#rguide .check code"));
  check("pin shows in the guide", $("#rguide").textContent.includes("Use safe_sub, not -."));
  check("diff renders with focus lines", $$("#rdiff tr.focus").length === 3, $$("#rdiff tr.focus").length);
  check("coverage shows in the header", $("#rhead .cov.ok")?.textContent.includes("All 3 changed lines in 1 files covered"), $("#rhead").textContent);
  check("step shows its first range", $('#rsteps [data-i="1"] .loc')?.textContent === "beacon_node/gloas.rs:11-12 +1", $('#rsteps [data-i="1"] .loc')?.textContent);
  check("removed line has the old number", [...$$("#rdiff tr.del td.ln")].map((t) => t.textContent).join() === "11");
  check("thread anchor dot on line 11", !!$('#rdiff tr.anchored td.ln[data-ln="11"][data-side="new"]'));
  check("thread renders with pin and draft actions", $$("#rthreads .thread").length === 1 && !!$('#rthreads [data-pin="t1"]'));
  $('#rdiff td.ln[data-ln="12"]').click();
  await wait(50);
  check("click a line sets the anchor", $("#ranchor").textContent.includes("gloas.rs:12"));
  $("#rq").value = "is 12 needed?";
  $("#rask").dispatchEvent(new Event("submit", { cancelable: true }));
  await wait(200);
  const ask = called("review_ask")[0]?.[1];
  check("ask sends the anchor", ask && ask.step === "s2" && ask.anchor?.line === 12 && ask.anchor.side === "new" && ask.thread === null && ask.anchor.text.includes("let b = 2"), JSON.stringify(ask));
  window.__emit("review_stream", { review: "r1", thread: "t2", delta: "partial" });
  // A long answer: the thread list scrolls inside the column.
  {
    const long = Object.assign({}, window.__data.reviews[0]);
    long.threads = [Object.assign({}, long.threads[0], { messages: [{ id: 1, me: true, text: "long?" }, { id: 2, me: false, text: Array.from({ length: 120 }, (_, i) => `Line ${i} of a long answer.`).join("\n\n") }] })];
    window.__emit("review", long);
    await wait(100);
    const box = $("#rthreads");
    const col = box.parentElement.getBoundingClientRect();
    check("long thread list scrolls inside its column", box.scrollHeight > box.clientHeight && box.getBoundingClientRect().bottom <= col.bottom + 1, `${box.scrollHeight} ${box.clientHeight} ${box.getBoundingClientRect().bottom} ${col.bottom} text=${box.textContent.length} tab=${window.SB.S.tab} step=${document.querySelector("#rsteps [aria-current=step]")?.dataset.i}`);
    check("the ask box stays on screen", $("#rq").getBoundingClientRect().bottom <= window.innerHeight + 1, `${$("#rq").getBoundingClientRect().bottom} ${window.innerHeight}`);
    // A long guide: the steps list scrolls too.
    const many = Object.assign({}, window.__data.reviews[0]);
    many.guide = { steps: Array.from({ length: 60 }, (_, i) => ({ id: `s${i + 1}`, title: `Step number ${i + 1} with a title`, ranges: [] })) };
    window.__emit("review", many);
    await wait(100);
    const list = $("#rsteps");
    check("a long steps list scrolls inside its column", list.scrollHeight > list.clientHeight && list.getBoundingClientRect().bottom <= window.innerHeight + 1, `${list.scrollHeight} ${list.clientHeight} ${list.getBoundingClientRect().bottom}`);
    window.__emit("review", window.__data.reviews[0]);
    await wait(100);
  }
  // Step / All: the thread of step 2 shows from step 1 under All, and a click jumps to it.
  docKey("k");
  await wait(100);
  check("Step view hides threads of other steps", !$$("#rthreads .thread").length);
  $('#rscope [data-v="all"]').click();
  await wait(50);
  check("All shows every thread with its step", $$("#rthreads .thread").length === 1 && $("#rthreads .anchor").textContent.startsWith("Step 2"), $("#rthreads").textContent.slice(0, 80));
  $("#rthreads .thread").click();
  await wait(100);
  check("a click jumps to the thread's step and makes it active", $('#rsteps [aria-current="step"]')?.dataset.i === "1" && $("#rthreads .thread.active"));
  check("the place is saved for a restart", JSON.parse(localStorage.getItem("sb.review.place.r1") || "{}").cur === 1);
  $('#rscope [data-v="step"]').click();
  await wait(50);
  $("#rthreads .thread.active")?.click();
  await wait(50);
  check("drafts list with copy", $("#rdrafts").textContent.includes("nit: use safe_sub") && !!$("#rdrafts [data-copy]"));
  // Diff colors come from the row kind, and the step bar does not hide them.
  {
    const bg = (sel) => { const el = $(sel); return el ? getComputedStyle(el).backgroundColor : "none"; };
    check("removed rows are red", bg("#rdiff tr.del td.src").includes("201, 122, 116"), bg("#rdiff tr.del td.src"));
    check("added rows are green, also in the step range", bg("#rdiff tr.add.focus td.src").includes("90, 168, 128"), bg("#rdiff tr.add.focus td.src"));
    check("signs are in their own span", $("#rdiff tr.add .sg")?.textContent === "+" && $("#rdiff tr.del .sg")?.textContent === "-");
  }
  // Your own comments: a + on a line, Shift for a range, then Add to review.
  check("agent draft shows on its line in the diff", $('#rdiff .dcard[data-d="1"]')?.textContent.includes("Agent draft"));
  $('#rdiff [data-cm][data-side="new"][data-n="12"]').click();
  await wait(50);
  check("+ opens the comment form", !!$("#rcform") && $("#rcform .cat").textContent.includes("gloas.rs:12"));
  $('#rdiff [data-cm][data-side="new"][data-n="11"]').dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true }));
  await wait(50);
  check("shift-click makes a range", $("#rcform .cat").textContent.includes("gloas.rs:11-12") && $$("#rdiff tr.sel").length === 2, $("#rcform")?.textContent);
  $("#rctext").value = "Both lines need a test.";
  $("#rcform").requestSubmit();
  await wait(100);
  const cm = called("review_comment")[0]?.[1];
  check("Add to review sends path, side and range", cm && cm.path === "beacon_node/gloas.rs" && cm.side === "new" && cm.line === 12 && cm.startLine === 11 && cm.text === "Both lines need a test.", JSON.stringify(cm));
  check("the form closes after the add", !$("#rcform"));
  // Finish review: preview from GitHub, pick a type, send only on the click.
  $("#rdrafts [data-finish]").click();
  await wait(150);
  check("finish shows the GitHub check", !$("#rfinish").hidden && $("#rpreview").textContent.includes("@eserilev") && $("#rpreview").textContent.includes("1 comment on lines") && $("#rpreview").textContent.includes("goes into the summary"), $("#rpreview")?.textContent);
  check("agent drafts are marked in the preview", $("#rpreview").textContent.includes("agent draft"));
  check("nothing is sent before the click", !called("review_post").length);
  $('#rfinish input[value="APPROVE"]').click();
  $("#rsummary").value = "LGTM";
  $("#rfinish [data-fsend]").click();
  await wait(150);
  const post = called("review_post")[0]?.[1];
  check("submit sends the type, summary and preview token", post && post.event === "APPROVE" && post.summary === "LGTM" && post.token === "tok1", JSON.stringify(post));
  check("the dialog closes after the send", $("#rfinish").hidden);
  $(".thread[data-t='t1']").click();
  await wait(50);
  check("click a thread makes it active", $(".thread[data-t='t1']").classList.contains("active") && $("#ranchor").textContent === "");
  docKey("n");
  await wait(100);
  check("n marks the step", called("review_mark").some((c) => c[1].step === "s2" && c[1].checked));

  // Expand: hidden rows above and below a part of the file, like GitHub.
  check("each gap has expand buttons", !!$('#rdiff [data-exp="up"]') && !!$('#rdiff [data-exp="down"]'));
  $('#rdiff [data-exp="up"]').click();
  await wait(150);
  check("up shows the hidden lines above", called("review_file_rows").length === 1 && !!$('#rdiff td.ln[data-side="new"][data-ln="1"]') && $$("#rdiff .gapbar").length === 1 && $("#rdiff .gapbar").textContent.includes("49 hidden lines"), $$("#rdiff .gapbar").map((x) => x.textContent).join(" | "));
  $('#rdiff [data-exp="down"]').click();
  await wait(150);
  check("down shows 20 lines below", !!$('#rdiff td.ln[data-side="new"][data-ln="32"]') && !$('#rdiff td.ln[data-side="new"][data-ln="33"]') && $("#rdiff .gapbar")?.textContent.includes("29 hidden lines"), $$("#rdiff .gapbar").map((x) => x.textContent).join(" | "));
  check("revealed rows keep their numbers and colors", $$("#rdiff tr.add").length === 2 && $$("#rdiff tr.del").length === 1);
  $('#rdiff [data-exp="all"]').click();
  await wait(150);
  check("show all ends the gap", !!$('#rdiff td.ln[data-side="new"][data-ln="61"]') && !$("#rdiff .gapbar"));

  // A step shows only its part of a file; a bar names the other changes.
  check("the bar names the other changes and their steps", $("#rdiff .other")?.textContent.includes("4 more changed lines are in") && !!$('#rdiff .other [data-ostep="0"]'), $("#rdiff .other")?.textContent);
  $("#rdiff [data-whole]").click();
  await wait(150);
  check("Show the whole file opens it in Files", $('#rmode [data-mode="files"]').getAttribute("aria-pressed") === "true" && $('#rsteps [aria-current="step"]')?.dataset.f === "beacon_node/gloas.rs" && !$("#rdiff .other"));

  // Files: every changed file, like GitHub's "Files changed".
  $('#rmode [data-mode="files"]').click();
  await wait(150);
  check("Files lists every changed file", $$("#rsteps .fitem").length === 2 && $("#rsteps").textContent.includes("gloas.rs") && $("#rsteps").textContent.includes("binary file"), $("#rsteps").textContent.slice(0, 120));
  check("a file diff loads from the model", called("review_file_diff").some((c) => c[1].path === "beacon_node/gloas.rs") && !!$("#rdiff tr.add"));
  check("the file head links to its step", !!$('#rguide [data-step="1"]') && $("#rguide .fpath").textContent === "beacon_node/gloas.rs");
  check("the + comment works in Files", !!$('#rdiff [data-cm][data-n="12"]'));
  docKey("n");
  await wait(150);
  check("n marks the file viewed and opens the next one", $('#rsteps .fitem.viewed[data-f="beacon_node/gloas.rs"]') && $('#rsteps [aria-current="step"]')?.dataset.f === "docs/img.png");
  check("a binary file shows its note", $("#rdiff").textContent.includes("binary file"));
  docKey("k");
  await wait(150);
  check("k goes to the previous file", $('#rsteps [aria-current="step"]')?.dataset.f === "beacon_node/gloas.rs");
  check("the mode and file are saved for a restart", (() => { const p = JSON.parse(localStorage.getItem("sb.review.place.r1") || "{}"); return p.mode === "files" && p.file === "beacon_node/gloas.rs" && p.viewed.includes("beacon_node/gloas.rs"); })());
  $('#rguide [data-step="1"]').click();
  await wait(150);
  check("a step link goes back to the guide at that step", $('#rmode [data-mode="guide"]').getAttribute("aria-pressed") === "true" && $('#rsteps [aria-current="step"]')?.dataset.i === "1");
  docKey("f");
  await wait(150);
  check("f switches to Files", $('#rmode [data-mode="files"]').getAttribute("aria-pressed") === "true" && $$("#rsteps .fitem").length === 2);
  docKey("f");
  await wait(150);

  // Round 2: after new commits, a new review of only the changes since this head.
  window.__emit("review", Object.assign({}, window.__data.reviews[0], { new_head: "fff0000bbbb" }));
  await wait(100);
  check("new commits offer the next round", $('#rhead [data-act="round"]')?.textContent === "Start round 2" && !!$('#rhead [data-act="update"]'));
  $('#rhead [data-act="round"]').click();
  await wait(200);
  check("the round opens in its own tab", $('#tabs [data-tab="r2"]')?.textContent.includes("#10071 R2") && $('#tabs [data-tab="r2"]').getAttribute("aria-selected") === "true", $("#tabs").textContent);
  check("the round header names its scope", $("#rhead .roundtag")?.textContent.includes("Round 2 · changes since ac92ae9aa") && $("#rhead .scopenote")?.textContent.includes("leaves out the changes from the base branch"), $("#rhead").textContent);
  window.SB.setTab("r1");
  await wait(100);

  // Property test of the expand logic: random files, random first rows, random clicks.
  {
    const { items, reveal, STEP } = window.SBReview._test;
    let seed = 12345;
    const rnd = (n) => { seed = (seed * 1103515245 + 12345) % 2147483648; return seed % n; };
    const key = (r) => `${r.kind}|${r.old ?? ""}|${r.new ?? ""}`;
    let bad = null, cases = 0, clicks = 0, big = 0;
    for (let c = 0; c < 400 && !bad; c++) {
      // A file as the numbering makes it: removed, added and same rows in order.
      const full = [];
      let o = 0, n = 0;
      const len = 1 + rnd(400);
      for (let i = 0; i < len; i++) {
        const k = rnd(10);
        if (k === 0) full.push({ kind: "-", old: ++o, new: null, text: "r" });
        else if (k === 1) full.push({ kind: "+", old: null, new: ++n, text: "a" });
        else full.push({ kind: " ", old: ++o, new: ++n, text: "s" });
        if (rnd(40) === 0) full.push({ kind: "@", old: null, new: null, text: "\\ No newline at end of file" });
      }
      const lines = full.filter((r) => !(r.kind === "@"));
      // The first rows: a random set, with a header before each run, as the server cuts.
      const sparse = 2 + rnd(40);
      const first = new Set(lines.map((_, k) => k).filter(() => rnd(sparse) === 0));
      if (!first.size) first.add(rnd(lines.length));
      const rows = [];
      lines.forEach((r, k) => { if (first.has(k)) { if (!first.has(k - 1)) rows.push({ kind: "@", old: null, new: null, text: "@@ old 1 · new 1 @@" }); rows.push(r); } });
      const sec = { path: "f.rs", rows, note: null };
      const l = { full: new Map([["f.rs", full]]), shown: new Map() };
      cases++;
      for (let step = 0; step < 12 && !bad; step++) {
        const it = items(l, "k", sec);
        const shownRows = it.filter((x) => !x.gap && x.row.kind !== "@").map((x) => x.row);
        const idx = shownRows.map((r) => lines.findIndex((y) => key(y) === key(r)));
        const gaps = it.filter((x) => x.gap);
        const hidden = gaps.reduce((a, g) => a + g.count, 0);
        if (idx.some((v, i) => i > 0 && v <= idx[i - 1])) bad = `rows out of order or twice in case ${c}`;
        else if (idx.length + hidden !== lines.length) bad = `shown ${idx.length} + hidden ${hidden} != ${lines.length} in case ${c}`;
        else if ([...first].some((k) => !idx.includes(k))) bad = `a first row went away in case ${c}`;
        else if (it.some((x, i) => x.gap && (x.count < 1 || it[i + 1]?.gap))) bad = `an empty or double gap in case ${c}`;
        if (bad || !gaps.length) break;
        const g = gaps[rnd(gaps.length)];
        if (g.count > STEP) big++;
        const dir = ["up", "down", "all"][rnd(3)];
        const before = idx.length;
        reveal(l, "k", sec, g.prev, g.next, dir);
        clicks++;
        const after = items(l, "k", sec).filter((x) => !x.gap && x.row.kind !== "@").length;
        const want = dir === "all" ? g.count : Math.min(STEP, g.count);
        if (after - before !== want) bad = `${dir} showed ${after - before}, not ${want}, in case ${c}`;
      }
    }
    check("expand: rows stay in order, once each, and every click shows the right count", !bad && big > 100, `${bad} (${cases} files, ${clicks} clicks, ${big} clicks on gaps over ${STEP})`);
  }

  // The leader comes from the config: cmd+; on a Mac, for example.
  {
    const was = window.SB.S.settings.leader;
    window.SB.S.settings.leader = "cmd+;";
    window.dispatchEvent(new KeyboardEvent("keydown", { key: " ", code: "Space", ctrlKey: true, bubbles: true }));
    const oldKey = !$("#leader").hidden;
    window.dispatchEvent(new KeyboardEvent("keydown", { key: ";", code: "Semicolon", metaKey: true, bubbles: true }));
    const newKey = !$("#leader").hidden;
    key("Escape");
    window.SB.S.settings.leader = was;
    check("the leader key comes from the config", !oldKey && newKey, `old=${oldKey} new=${newKey}`);
    $("#leader").hidden = true;
  }

  // A stale draft: the code changed after an update. It shows in the list with a note,
  // and not in the diff.
  {
    const r = structuredClone(window.__data.reviews[0]);
    r.drafts = [Object.assign({}, r.drafts[0], { stale: true })];
    window.__emit("review", r);
    await wait(100);
    check("a stale draft shows a note in the list and not in the diff", $("#rdrafts .draft.stale .stalenote")?.textContent.includes("code changed") && !$('#rdiff .dcard[data-d="1"]'), $("#rdrafts").textContent);
    window.__emit("review", window.__data.reviews[0]);
    await wait(100);
  }

  // Back to the board with the leader.
  key(" ", { ctrl: true, code: "Space" });
  key("b");
  await wait(50);
  check("leader b shows the board", !$("#view-board").hidden);

  check("no script errors", window.__errors.length === 0, window.__errors);
  const pre = document.createElement("pre");
  pre.id = "__results";
  pre.textContent = JSON.stringify(results);
  document.body.appendChild(pre);
})().catch((e) => {
  const pre = document.createElement("pre");
  pre.id = "__results";
  pre.textContent = JSON.stringify([{ name: "checks crashed", ok: false, info: String(e && e.stack || e) }]);
  document.body.appendChild(pre);
});
