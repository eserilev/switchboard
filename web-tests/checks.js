// Clicks through the UI and writes the results into #__results as JSON.
(async () => {
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const $ = (s) => document.querySelector(s);
  const $$ = (s) => [...document.querySelectorAll(s)];
  const results = [];
  const check = (name, ok, info) => results.push({ name, ok: !!ok, info: ok ? undefined : info });
  const called = (cmd) => window.__calls.filter((c) => c[0] === cmd);
  const local_mode = () => $("#rsteps .fileitem, #rsteps [data-f]") ? "files" : "guide";
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
  {
    // The title row has only the name, the state and Close: the tool buttons sit on
    // the info line, so a long name keeps its room.
    const row = $('.pane[data-id="p1"] .ptitle');
    check("the title row holds only the name, the state and Close", [...row.querySelectorAll("button")].map((b) => b.getAttribute("aria-label")).join() === "Close"
      && $$('.pane[data-id="p1"] .pmeta .ptools [data-ptaskmenu], .pane[data-id="p1"] .pmeta .ptools [data-lreview], .pane[data-id="p1"] .pmeta .ptools [data-part="nvim"]').length === 3);
    const name = $('.pane[data-id="p1"] .name');
    check("the name takes most of the title row", name.getBoundingClientRect().width > row.getBoundingClientRect().width * 0.6, `${name.getBoundingClientRect().width} of ${row.getBoundingClientRect().width}`);
    check("the task button shows with no hover", getComputedStyle($('.pane[data-id="p2"] [data-ptaskmenu]')).display !== "none");
  }
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
  check("the live pane keeps its tile, in its place and marked", $$("#board .pane").map((p) => p.dataset.id).join() === "p1,p2,p3" && $('#board .pane[data-id="p1"]').classList.contains("is-live"), $$("#board .pane").map((p) => p.dataset.id + ":" + p.className).join());
  check("the live tile is light blue", getComputedStyle($('#board .pane[data-id="p1"]')).borderTopColor === "rgb(124, 196, 240)", getComputedStyle($('#board .pane[data-id="p1"]')).borderTopColor);
  {
    const before = window.__calls.filter((c) => c[0] === "pane_expand").length;
    $('.pane[data-id="p1"] .term-tail').click();
    await wait(100);
    check("a click on the live tile does not expand it again", window.__calls.filter((c) => c[0] === "pane_expand").length === before);
  }
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

  // A PR with a closed round: the launcher offers the next round, or a full review.
  $("#newpane").click();
  await wait(50);
  q.value = "https://github.com/sigp/lighthouse/pull/999";
  q.dispatchEvent(new Event("input"));
  await wait(100);
  check("a closed round offers the next round first", $$("#lrepos li[data-pr]").length === 2 && $('#lrepos li[data-pr="0"]').textContent.includes("Round 2 of #999") && $('#lrepos li[data-pr="0"]').getAttribute("aria-selected") === "true" && $("#lrepos").textContent.includes("you sent 2 reviews"), $("#lrepos").textContent);
  q.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
  await wait(20);
  check("down picks the full review", $('#lrepos li[data-pr="1"]').getAttribute("aria-selected") === "true");
  q.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true }));
  await wait(20);
  $("#lgo").click();
  await wait(400);
  check("the next round opens from the closed round", called("review_open")[0]?.[1].roundOf === "r0", JSON.stringify(called("review_open")[0]?.[1]));
  window.SB.setTab("board");

  $("#newpane").click();
  await wait(50);
  q.value = "https://github.com/sigp/lighthouse/pull/10071";
  q.dispatchEvent(new Event("input"));
  await wait(50);
  check("PR URL shows a review item", $("#lrepos").textContent.includes("Review 10071") && $("#lrows").hidden);
  $("#lgo").click();
  await wait(400);
  check("PR URL opens the review tab", called("review_open").length === 2 && called("review_open")[1][1].roundOf === null && !$("#view-review").hidden);

  // Review
  check("review tab button", $$('#tabs [data-tab="r1"]').length === 1);
  check("steps render", $$("#rsteps .step").length === 2);
  docKey("j");
  await wait(300);
  check("j goes to step 2", $('#rsteps [aria-current="step"]')?.dataset.i === "1");
  // The Claude column: a real Claude Code pane that forks the guide session.
  {
    const first = called("review_term")[0]?.[1];
    check("the review starts its Claude pane on the current step", first && first.id === "r1" && first.start === true && first.step === "s1", JSON.stringify(first));
    check("the live terminal moves into the Claude column", !!$("#rterm #live") && !$("#live").hidden && window.SB.S.live === "p8" && window.SB.S.termHost === "r1");
    check("the place is saved for a restart", JSON.parse(localStorage.getItem("sb.review.place.r1") || "{}").cur === 1);
  }
  check("guide text renders markdown", !!$("#rguide .what strong") && !!$("#rguide .check code"));
  // Why, Claude's answers to the checks, and Claude's take (SPEC 11.12).
  check("the step shows why it matters and why it is a step", $("#rguide .why").textContent.includes("Why it matters") && $("#rguide .why").textContent.includes("wrong inclusion list") && $("#rguide .why").textContent.includes("Why a separate step"));
  check("each check item shows Claude's answer", [...$$("#rguide .check .ans")].map((x) => x.textContent).join("|") === "Claude: matches the spec|Claude: see the note on line 11");
  check("the take bar shows the opinion, the verdict and the counts", $("#rtake .tk-label")?.textContent.includes("not verified") && $("#rtake .verdict")?.textContent === "needs a change" && $("#rtake").textContent.includes("1 must fix · 1 should fix · 1 nit"), $("#rtake")?.textContent);
  check("a note with a line shows under its line, with a dot", !!$('#rdiff tr.noted-must td.ln[data-ln="11"]') && $("#rdiff .tnote")?.textContent.includes("saturates") && !!$("#rdiff .tnote .sev-must"));
  check("notes with no line on screen wait in the take list", $("#rtake [data-tk=list]")?.textContent === "Show list (2)");
  $("#rtake [data-tk=list]").click();
  await wait(50);
  check("Show list shows the note with no line", $("#rtake .tk-list").textContent.includes("No test covers slot 0") && $("#rtake .tk-list").textContent.includes("A name far below"));
  window.__calls.length = 0;
  [...$$("#rtake .tk-list .tnote")].find((x) => x.textContent.includes("far below")).querySelector("[data-tadd]").click();
  await wait(100);
  check("Add as comment on a line that is not on screen says so and opens no form", !$("#rcform") && $("#toast")?.textContent.includes("not on screen"), $("#toast")?.textContent);
  check("the steps list shows the worst severity and a count", $('#rsteps [data-i="1"] .badge.sev-must')?.textContent === "must fix · 1");
  check("the header shows Claude's take on the PR", $("#rhead .prtake")?.textContent.includes("Claude on the PR: needs a change") && $("#rhead .prtake").textContent.includes("no test for slot 0"));
  window.__calls.length = 0;
  $("#rdiff .tnote [data-task]").click();
  await wait(100);
  check("Ask types the note into Claude's prompt", called("pane_input").some((c) => c[1].data.includes("beacon_node/gloas.rs:11 Your note")), JSON.stringify(called("pane_input")));
  $("#rtake [data-tk=rerun]").click();
  await wait(50);
  check("Re-run asks for the take of this step again", called("review_retake")[0]?.[1].step === "s2");
  $("#rdiff .tnote [data-tdis]").click();
  await wait(50);
  const dkey = called("review_dismiss")[0]?.[1];
  check("Dismiss sends the note's key", dkey?.on === true && dkey.note.startsWith("s2|must|beacon_node/gloas.rs:11|"), JSON.stringify(dkey));
  window.__emit("review", Object.assign({}, window.__data.reviews[0], { dismissed: [dkey.note] }));
  await wait(150);
  check("a dismissed note folds, with Undo, and leaves the counts", !$("#rdiff .tnote .sev-must") && !!$("#rdiff .tdismissed [data-tundo]") && $("#rtake").textContent.includes("1 dismissed") && !$('#rdiff tr.noted-must'), $("#rtake").textContent);
  window.__errors.length = 0;
  $("#rdiff .tdismissed [data-tshow]").click();
  await wait(100);
  check("Show opens a dismissed note again, with Undo", !!$("#rdiff .tnote .sev-must") && !!$("#rdiff .tnote [data-tundo]") && !window.__errors.length, JSON.stringify(window.__errors));
  window.__emit("review", window.__data.reviews[0]);
  await wait(150);
  $("#rdiff .tnote [data-tadd]").click();
  await wait(100);
  check("Add as comment opens the form on the line with the note's text", $("#rcform .cat")?.textContent.includes("gloas.rs:11") && $("#rctext").value.includes("saturates"));
  window.__calls.length = 0;
  $("#rcform").requestSubmit();
  await wait(150);
  check("the comment from a note saves as an agent draft", called("review_comment")[0]?.[1].comment.agent === true, JSON.stringify(called("review_comment")[0]?.[1]));
  // The chat of each step, above the Claude pane.
  check("the chat shows the questions of this step", $("#rchat")?.hidden === false && $("#rchat .chead").textContent.includes("On this step · 1") && $("#rchat summary").textContent.includes("is slot 0 real?"), $("#rchat")?.textContent);
  {
    // An answer you closed stays closed when the chat draws again.
    $("#rchat details").open = false;
    window.__emit("review", window.__data.reviews[0]);
    await wait(150);
    check("a closed answer stays closed after a redraw", $("#rchat details") && !$("#rchat details").open);
    $("#rchat details").open = true;
  }
  $("#rchat [data-chall]").click();
  await wait(50);
  check("All steps shows every question with its step", $$("#rchat summary").length === 2 && $("#rchat .cstep").textContent === "Step 1");
  $("#rchat [data-chall]").click();
  await wait(50);
  check("pin shows in the guide", $("#rguide").textContent.includes("Use safe_sub, not -."));
  check("diff renders with focus lines", $$("#rdiff tr.focus").length === 3, $$("#rdiff tr.focus").length);
  check("coverage shows in the header", $("#rhead .cov.ok")?.textContent.includes("All 3 changed lines in 1 files covered"), $("#rhead").textContent);
  check("step shows its first range", $('#rsteps [data-i="1"] .loc')?.textContent === "beacon_node/gloas.rs:11-12 +1", $('#rsteps [data-i="1"] .loc')?.textContent);
  check("removed line has the old number", [...$$("#rdiff tr.del td.ln")].map((t) => t.textContent).join() === "11");
  check("thread anchor dot on line 11", !!$('#rdiff tr.anchored td.ln[data-ln="11"][data-side="new"]'));
  window.__calls.length = 0;
  $('#rdiff td.ln[data-ln="12"]').click();
  await wait(100);
  {
    const typed = called("pane_input").map((c) => c[1]);
    check("a click on a line number types the line into Claude's prompt", typed.some((t) => t.id === "p8" && t.data === "\x1b[200~beacon_node/gloas.rs:12 `let b = 2;` \x1b[201~"), JSON.stringify(typed));
  }
  window.__calls.length = 0;
  $("#rstep").click();
  await wait(100);
  {
    const typed = called("pane_input").map((c) => c[1]);
    check("Send step types the step into Claude's prompt, without Enter", called("review_step_note")[0]?.[1].step === "s2" && typed.some((t) => t.id === "p8" && t.data.startsWith("\x1b[200~I am on step s2") && !/[\r\n]/.test(t.data)), JSON.stringify(typed));
  }
  // No typing while Claude asks for a permission: a "1" in the text would answer it.
  {
    const p8 = window.SB.paneById("p8");
    p8.permit = { id: "p8-1", summary: "Bash: ls" };
    window.__calls.length = 0;
    $('#rdiff td.ln[data-ln="11"]').click();
    await wait(100);
    check("no typing into a permission prompt", !called("pane_input").length);
    p8.permit = null;
  }
  // A tab switch while the pane list loads: the terminal stays off the hidden review.
  {
    const keep = window.SB.refreshPanes;
    window.SB.refreshPanes = async () => { await wait(150); return keep(); };
    $("#livetitle [data-collapse]").click();
    await wait(50);
    window.SB.S.panes = window.SB.S.panes.filter((x) => x.id !== "p8");
    $("#raskwait [data-show]").click();
    window.SB.setTab("board");
    await wait(400);
    check("a tab switch during the load keeps the terminal on the board", window.SB.S.termHost === null && $("#live").parentElement === $("#view-board"), `${window.SB.S.termHost} ${$("#live").parentElement?.id}`);
    window.SB.refreshPanes = keep;
    await keep();
    window.SB.setTab("r1");
    await wait(300);
  }
  // The chat history stays when you move between steps: one conversation for the review.
  {
    const lines = Array.from({ length: 100 }, (_, i) => `history line ${i}\r\n`).join("");
    window.__emit("pane_output", { pane: "p8", data: btoa(lines) });
    await wait(100);
    const has = (t) => { const b = window.SB.term.buffer.active; for (let i = 0; i < b.length; i++) if (b.getLine(i)?.translateToString(true).includes(t)) return true; return false; };
    const expands = called("pane_expand").length;
    docKey("k");
    await wait(300);
    docKey("j");
    await wait(300);
    check("moving between steps keeps the chat history", has("history line 0") && has("history line 99") && called("pane_expand").length === expands, JSON.stringify({ first: has("history line 0"), last: has("history line 99"), expands: called("pane_expand").length - expands }));
  }
  // A smaller window shrinks the terminal to its box, at once and not one column
  // at a time (full screen to a window). Else the program draws too wide.
  {
    const fits = () => {
      const t = window.SB.term;
      const col = $("#rterm").getBoundingClientRect();
      const screen = $("#term .xterm-screen").getBoundingClientRect();
      return { ok: screen.right <= col.right + 1, cols: t.cols, screen: Math.round(screen.width), col: Math.round(col.width) };
    };
    const shrink = async (px) => { if (px) SBSplit.save("sb.split.review", [280, px]); else localStorage.removeItem("sb.split.review"); window.dispatchEvent(new Event("resize")); await wait(400); };
    await shrink(800);
    const wide = window.SB.term.cols;
    await shrink(400);
    // xterm repaints a moment after the fit: wait for it, up to a second.
    for (let i = 0; i < 20 && !fits().ok; i++) await wait(50);
    const f = fits();
    const sent = called("pane_resize").at(-1)?.[1];
    check("after a smaller window the review terminal fits its box", f.ok && f.cols < wide - 10 && sent?.cols === f.cols, JSON.stringify({ ...f, wide, sent }));
    await shrink("");
  }
  window.SB.setTab("board");
  await wait(100);
  check("the board takes the terminal back", $("#live").parentElement === $("#view-board") && window.SB.S.live === window.SB.S.boardLive && $("#live").hidden === !window.SB.S.boardLive && window.SB.S.live !== "p8" && window.SB.S.termHost === null, `${window.SB.S.live} ${window.SB.S.boardLive}`);
  window.SB.setTab("r1");
  await wait(200);
  check("the review shows its Claude pane again", !!$("#rterm #live") && !$("#live").hidden && window.SB.S.live === "p8" && called("review_term").every((c) => c[1].id === "r1"));
  // A pane open on the board opens again after a visit to a review tab.
  {
    window.SB.setTab("board");
    await wait(100);
    await window.SB.expand("p1");
    await wait(100);
    window.SB.setTab("r1");
    await wait(300);
    check("the review takes the terminal from the board pane", window.SB.S.live === "p8" && window.SB.S.termHost === "r1", `${window.SB.S.live} ${window.SB.S.termHost}`);
    window.SB.setTab("board");
    await wait(300);
    check("the board pane opens again after the review", window.SB.S.live === "p1" && !$("#live").hidden && $("#live").parentElement === $("#view-board") && window.SB.S.termHost === null, `${window.SB.S.live} ${window.SB.S.termHost}`);
    window.SB.setTab("tasks");
    await wait(100);
    window.SB.setTab("board");
    await wait(200);
    check("the board pane stays open across the Tasks tab", window.SB.S.live === "p1" && !$("#live").hidden);
    $("#livetitle [data-collapse]").click();
    await wait(100);
    window.SB.setTab("r1");
    await wait(300);
    window.SB.setTab("board");
    await wait(300);
    check("a pane closed on the board does not open again", window.SB.S.live === null && $("#live").hidden, window.SB.S.live);
    window.SB.setTab("r1");
    await wait(300);
  }
  $("#livetitle [data-collapse]").click();
  await wait(100);
  check("a collapsed Claude pane leaves a Show button", $("#live").hidden && !!$("#raskwait [data-show]") && !$("#raskwait").hidden);
  $("#raskwait [data-show]").click();
  await wait(200);
  check("Show brings the pane back", !$("#live").hidden && window.SB.S.live === "p8");
  // A long guide: the steps list scrolls inside its column.
  {
    const many = Object.assign({}, window.__data.reviews[0]);
    many.guide = { steps: Array.from({ length: 60 }, (_, i) => ({ id: `s${i + 1}`, title: `Step number ${i + 1} with a title`, ranges: [] })) };
    window.__emit("review", many);
    await wait(100);
    const list = $("#rsteps");
    check("a long steps list scrolls inside its column", list.scrollHeight > list.clientHeight && list.getBoundingClientRect().bottom <= window.innerHeight + 1, `${list.scrollHeight} ${list.clientHeight} ${list.getBoundingClientRect().bottom}`);
    window.__emit("review", window.__data.reviews[0]);
    await wait(100);
  }
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
  const cm = called("review_comment")[0]?.[1].comment;
  check("Add to review sends path, side, range, head and line text", cm && cm.path === "beacon_node/gloas.rs" && cm.side === "new" && cm.line === 12 && cm.start_line === 11 && cm.text === "Both lines need a test." && cm.head === "ac92ae9aaaa" && cm.line_text === "let b = 2;", JSON.stringify(cm));
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
  check("after the send, the dialog offers Close review", !$("#rfinish").hidden && $("#rfinish").textContent.includes("Review sent") && [...$$("#rfinish [data-b]")].map((b) => b.textContent).join() === "Keep open,Close review");
  $('#rfinish [data-b="0"]').click();
  await wait(50);
  check("Keep open keeps the review", $("#rfinish").hidden && !called("review_close").length);
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
    $('#rdrafts [data-place="1"]').click();
    await wait(50);
    $('#rdiff [data-cm][data-side="new"][data-n="12"]').click();
    await wait(50);
    check("Place again opens the form with the draft's text", $("#rctext")?.value === "nit: use safe_sub");
    const before = called("review_draft_edit").length;
    $("#rcform").requestSubmit();
    await wait(150);
    const drop = called("review_draft_edit").slice(before).find((c) => c[1].draft === 1 && c[1].text === null);
    check("Place again adds the comment and removes the stale draft", called("review_comment").slice(-1)[0]?.[1].comment.line === 12 && !!drop);
    // Place again, then Cancel, then a different comment: the stale draft stays.
    $('#rdrafts [data-place="1"]').click();
    await wait(50);
    $('#rdiff [data-cm][data-side="new"][data-n="12"]').click();
    await wait(50);
    $("#rcform [data-ccancel]").click();
    await wait(50);
    $('#rdiff [data-cm][data-side="new"][data-n="11"]').click();
    await wait(50);
    const n2 = called("review_draft_edit").length;
    $("#rctext").value = "Another comment.";
    $("#rcform").requestSubmit();
    await wait(150);
    check("Cancel ends Place again, so a new comment keeps the stale draft", $("#rctext") === null && !called("review_draft_edit").slice(n2).some((c) => c[1].draft === 1 && c[1].text === null));
    window.__emit("review", window.__data.reviews[0]);
    await wait(100);
  }

  // A diff of an older head never takes a click: the window refetches it.
  {
    const r = Object.assign({}, window.__data.reviews[0], { head: "fff0000bbbb" });
    window.__emit("review", r);
    await wait(100);
    check("a diff of an older head shows Loading and no + button", $("#rdiff .rnote")?.textContent.includes("Loading the new commit") && !$("#rdiff [data-cm]"), $("#rdiff").textContent.slice(0, 80));
    window.__emit("review", window.__data.reviews[0]);
    await wait(1200);
  }

  // Close: a worktree with changes asks first, and nothing closes before the choice.
  $('#rhead [data-act="close"]').click();
  await wait(150);
  check("a close with changes lists them and asks", !$("#rfinish").hidden && $("#rfinish").textContent.includes("notes.txt") && called("review_close").length === 1 && called("review_close")[0][1].discard === null && called("review_close")[0][1].keep === false, $("#rfinish").textContent);
  [...$$("#rfinish [data-b]")].find((b) => b.textContent === "Discard the changes").click();
  await wait(150);
  check("Discard sends the changes you saw", called("review_close").length === 2 && JSON.stringify(called("review_close")[1][1].discard) === JSON.stringify([" M beacon_node/gloas.rs", "?? notes.txt"]) && $("#rfinish").hidden);
  window.__emit("review_cleanup", { review: "r1", note: "/c/lighthouse-pr10071 stays: in use by nvim." });
  await wait(50);
  check("a kept folder shows a note", $("#toast")?.textContent.includes("stays: in use by nvim"), $("#toast")?.textContent);

  // A review of an agent's unpushed commits (SPEC 11.11).
  window.SB.setTab("board");
  await wait(100);
  check("a Claude tile has a review button, nvim has none", !!$('.pane[data-id="p1"] [data-lreview]') && !$('.pane[data-id="p3"] [data-lreview]'));
  window.__calls.length = 0;
  $('.pane[data-id="p1"] [data-lreview]').click();
  await wait(400);
  check("review opens a local review of that pane", called("review_open_local")[0]?.[1].pane === "p1" && window.SB.S.tab === "rL", JSON.stringify(called("review_open_local")));
  check("the tab says it is local commits", $('#tabs [data-tab="rL"]')?.textContent.includes("lighthouse commits"));
  check("a local review shows Files only, with no Guide switch and no Send step", !$("#rmode") && !$("#rstep") && local_mode() === "files");
  check("the header shows the range and what it compares with", $("#rhead").textContent.includes("2 commits on feat") && $("#rhead").textContent.includes("b0b0b0b0b..ac92ae9aa") && $("#rhead").textContent.includes("not on origin/unstable"), $("#rhead").textContent);
  check("the agent pane is in the right column", window.SB.S.live === "p1" && !!$("#rterm #live"));
  docKey("f");
  await wait(50);
  check("f does not switch a local review to Guide", local_mode() === "files" && !!$("#rsteps [data-f]"));
  check("Send to agent replaces Finish review", !!$("#rdrafts [data-agent]") && !$("#rdrafts [data-finish]"));
  $("#rdrafts [data-agent]").click();
  await wait(100);
  check("Send to agent sends the review's comments", called("review_send_agent")[0]?.[1].id === "rL" && $("#toast")?.textContent.includes("Press Enter"), $("#toast")?.textContent);
  window.__emit("review", Object.assign({}, window.__data.reviews.find((x) => x.id === "rL"), { new_head: "fffeee0000" }));
  await wait(100);
  $('#rhead .newhead [data-act="update"]').click();
  await wait(50);
  check("new commits offer Refresh, which reads the commits again", called("review_update").some((c) => c[1].id === "rL"));
  {
    const rL = window.__data.reviews.find((x) => x.id === "rL");
    const terms = called("review_term").filter((c) => c[1].id === "rL").length;
    window.__emit("review", Object.assign({}, rL, { pane: "p2", new_head: null }));
    await wait(150);
    check("a new agent pane for the folder is found again", called("review_term").filter((c) => c[1].id === "rL").length > terms);
    const files = called("review_files").filter((c) => c[1].id === "rL").length;
    window.__emit("review", Object.assign({}, rL, { pane: "p2", base: "c1c1c1c1c1c1", status: "fetching" }));
    await wait(100);
    check("no file list loads while the commits load", called("review_files").filter((c) => c[1].id === "rL").length === files);
    window.__emit("review", Object.assign({}, rL, { pane: "p2", base: "c1c1c1c1c1c1", status: "ready" }));
    await wait(150);
    check("a new base alone loads the file list again", called("review_files").filter((c) => c[1].id === "rL").length > files);
  }

  // The same on the board: full width, then a smaller window.
  {
    window.SB.setTab("board");
    await wait(100);
    await window.SB.expand("p1");
    await wait(200);
    const shrink = async (px) => { $("#view-board").style.width = px; window.dispatchEvent(new Event("resize")); await wait(400); };
    await shrink("2400px");
    const wide = window.SB.term.cols;
    // The cell width, from the wide screen. The check is on columns: the test browser
    // can skip the frame in which xterm repaints its screen.
    const cell = $("#term .xterm-screen").getBoundingClientRect().width / wide;
    await shrink("500px");
    const area = $("#view-board").getBoundingClientRect();
    const screen = { width: window.SB.term.cols * cell };
    check("after a smaller window the board terminal fits its box", screen.width <= area.width + 1 && window.SB.term.cols < wide - 10, JSON.stringify({ cols: window.SB.term.cols, wide, screen: Math.round(screen.width), area: Math.round(area.width), live: Math.round($("#live").getBoundingClientRect().width), hidden: $("#live").hidden, S: window.SB.S.live, parent: $("#live").parentElement.id, tab: window.SB.S.tab }));
    await shrink("");
  }

  // Draggable sizes (SPEC 6.7). The rules first: random cases against their spec.
  {
    let seed = 12345;
    const rnd = (n) => { seed = (seed * 1103515245 + 12345) % 2147483648; return seed % n; };
    let bad = null;
    for (let i = 0; i < 5000 && !bad; i++) {
      const min = 40 + rnd(200), mid = 100 + rnd(400), total = rnd(3000);
      const want = rnd(4000) - 500, other = min + rnd(1500);
      const v = SBSplit.side(total, want, other, min, mid);
      const room = total - other - mid;
      // A side keeps its minimum; the middle keeps its own when there is room; a
      // value inside the range stays as asked.
      if (v < min) bad = ["side below min", { total, want, other, min, mid, v }];
      else if (room >= min && v > room) bad = ["side takes the middle's room", { total, want, other, min, mid, v }];
      else if (want >= min && want <= room && v !== Math.round(want)) bad = ["side moved a fitting value", { total, want, v }];
      const [l, r] = SBSplit.fit(total, min + rnd(1500), min + rnd(1500), min, mid);
      const [l2, r2] = SBSplit.fit(total, l, r, min, mid);
      if (!bad && (l < min || r < min)) bad = ["fit below min", { total, l, r, min }];
      else if (!bad && total >= 2 * min + mid && l + r + mid > total) bad = ["fit leaves the middle short", { total, l, r, min, mid }];
      else if (!bad && (l2 !== l || r2 !== r)) bad = ["fit is not stable", { total, l, r, l2, r2 }];
      const [a1, b1] = [min + rnd(300), min + rnd(300)];
      if (!bad && a1 + b1 + mid <= total) { const [fa, fb] = SBSplit.fit(total, a1, b1, min, mid); if (fa !== a1 || fb !== b1) bad = ["fit changed sizes that fit", { total, a1, b1, fa, fb }]; }
    }
    check("the size rules hold on 5000 random cases", !bad, JSON.stringify(bad));
  }
  // The review handles: drag, keys, limits, double-click, and the other side stays.
  {
    window.SB.setTab("r1");
    await wait(300);
    localStorage.removeItem("sb.split.review");
    window.dispatchEvent(new Event("resize"));
    await wait(100);
    const widths = () => [...$("#rcols").children].filter((c) => c.classList.contains("col")).map((c) => Math.round(c.getBoundingClientRect().width));
    const drag = async (el, dx) => {
      const r = el.getBoundingClientRect();
      const x = r.left + r.width / 2, y = r.top + 50;
      el.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: x, clientY: y, button: 0 }));
      window.dispatchEvent(new PointerEvent("pointermove", { clientX: x + dx / 2, clientY: y }));
      window.dispatchEvent(new PointerEvent("pointermove", { clientX: x + dx, clientY: y }));
      window.dispatchEvent(new PointerEvent("pointerup", { clientX: x + dx, clientY: y }));
      await wait(100);
    };
    const [l0, , r0] = widths();
    await drag($("#rgutl"), 60);
    const [l1, , r1] = widths();
    check("dragging the left handle widens the steps and leaves the right column", l1 === l0 + 60 && r1 === r0, JSON.stringify({ l0, l1, r0, r1 }));
    check("the size is saved", JSON.stringify(SBSplit.load("sb.split.review")) === JSON.stringify([l1, r1]), localStorage.getItem("sb.split.review"));
    await drag($("#rgutr"), -40);
    const [l2, m2, r2] = widths();
    check("dragging the right handle to the left widens the Claude column", r2 === r1 + 40 && l2 === l1, JSON.stringify({ l1, l2, r1, r2 }));
    await drag($("#rgutl"), 5000);
    const [, m3] = widths();
    check("a drag past the limit stops at the middle column's minimum", Math.abs(m3 - 320) <= 1, JSON.stringify(widths()));
    await drag($("#rgutl"), -5000);
    check("a drag past the other end stops at the side minimum", widths()[0] === 160, JSON.stringify(widths()));
    const before = widths()[0];
    $("#rgutl").dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    await wait(50);
    $("#rgutl").dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", shiftKey: true, bubbles: true }));
    await wait(50);
    check("arrow keys move a handle by 16 and Shift by 64", widths()[0] === before + 80, JSON.stringify({ before, now: widths()[0] }));
    check("a handle is a focusable separator", $("#rgutl").getAttribute("role") === "separator" && $("#rgutl").tabIndex === 0);
    check("a handle tells screen readers its value and limits", $("#rgutl").getAttribute("aria-valuenow") === String(widths()[0]) && $("#rgutl").getAttribute("aria-valuemin") === "160" && $("#rgutl").getAttribute("aria-controls") === "rsteps");
    const key = (k) => { $("#rgutl").dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true })); return wait(50); };
    await key("Home");
    check("Home goes to the smallest size", widths()[0] === 160, JSON.stringify(widths()));
    await key("End");
    check("End goes to the largest size, and the middle keeps its minimum", Math.abs(widths()[1] - 320) <= 1, JSON.stringify(widths()));
    await key("Home");
    $("#rgutl").dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", shiftKey: true, bubbles: true }));
    await wait(50);
    const mid = widths()[0];
    await key("Enter");
    const small = widths()[0];
    await key("Enter");
    check("Enter goes to the smallest size and back", small === 160 && widths()[0] === mid, JSON.stringify({ mid, small, now: widths()[0] }));
    const right = widths()[2];
    $("#rgutl").dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    await wait(50);
    check("double-click resets only the column of that handle", widths()[0] === 280 && widths()[2] === right, JSON.stringify({ right, now: widths() }));
    // A new review tab takes the saved size.
    await drag($("#rgutl"), 30);
    window.SB.setTab("board");
    window.SB.setTab("r1");
    await wait(200);
    check("a redrawn review keeps the dragged size", widths()[0] === 310, JSON.stringify(widths()));
    $("#rgutl").dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    $("#rgutr").dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    await wait(50);
    localStorage.removeItem("sb.split.review");
  }
  // The board handle: the tile strip under the live terminal.
  {
    window.SB.setTab("board");
    await wait(100);
    await window.SB.expand("p1");
    await wait(200);
    const g = $("#bgutter");
    check("the board handle shows with a live terminal", !g.hidden);
    const strip = () => Math.round($("#board").getBoundingClientRect().height);
    const r = g.getBoundingClientRect();
    const y = r.top + r.height / 2;
    const s0 = strip();
    g.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: r.left + 40, clientY: y, button: 0 }));
    window.dispatchEvent(new PointerEvent("pointermove", { clientX: r.left + 40, clientY: y - 120 }));
    window.dispatchEvent(new PointerEvent("pointerup", { clientX: r.left + 40, clientY: y - 120 }));
    await wait(150);
    const rows = (h) => (h - 24 + 10) / 146;
    check("dragging the board handle up makes the tiles taller, in whole rows", strip() > s0 && Number.isInteger(rows(strip())), JSON.stringify({ s0, now: strip(), rows: rows(strip()) }));
    g.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: r.left + 40, clientY: y, button: 0 }));
    window.dispatchEvent(new PointerEvent("pointermove", { clientX: r.left + 40, clientY: y + 5000 }));
    window.dispatchEvent(new PointerEvent("pointerup", { clientX: r.left + 40, clientY: y + 5000 }));
    await wait(150);
    check("the tiles keep one row at least", strip() === 160, JSON.stringify({ now: strip() }));
    g.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true }));
    await wait(100);
    check("an arrow key moves the board handle 16 px, with no snap", strip() === 176, JSON.stringify({ now: strip() }));
    g.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: r.left + 40, clientY: y, button: 0 }));
    window.dispatchEvent(new PointerEvent("pointermove", { clientX: r.left + 40, clientY: y - 5000 }));
    window.dispatchEvent(new PointerEvent("pointerup", { clientX: r.left + 40, clientY: y - 5000 }));
    await wait(150);
    check("the terminal keeps its minimum height", Math.round($("#live").getBoundingClientRect().height) >= 238, JSON.stringify({ live: $("#live").getBoundingClientRect().height }));
    g.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    await wait(100);
    check("double-click resets the board", !$("#view-board").classList.contains("sized") && !localStorage.getItem("sb.split.board"));
    window.SB.collapse();
    await wait(100);
    check("the board handle hides with no live terminal", g.hidden);
  }

  // Back to the board with the leader.
  key(" ", { ctrl: true, code: "Space" });
  key("b");
  await wait(50);
  check("leader b shows the board", !$("#view-board").hidden);

  // Tasks (SPEC 27)
  {
    const reviewMarks = called("review_mark").length;
    key(" ", { ctrl: true, code: "Space" });
    key("t");
    await wait(100);
    const tv = $("#view-tasks");
    check("leader t shows the Tasks tab", !tv.hidden && $("#view-board").hidden && $("#view-review").hidden && $('#tabs [data-tab="tasks"]').getAttribute("aria-selected") === "true");
    check("the board has a column for each status", $$("#view-tasks .tcol").map((c) => c.dataset.status).join() === "todo,doing,blocked,review,done");
    check("cards sit in their status columns", $('.tcol[data-status="doing"] [data-task="t1"]') && $('.tcol[data-status="blocked"] [data-task="t3"]') && !$('#view-tasks .tboard [data-task="t5"]'));
    check("a blocked card shows who blocks it", $('[data-task="t3"]').textContent.includes("Blocked by spec PR 4512"));
    check("the sprint header shows the name, goal, days left and done count", /Sprint 1.*Ship inclusion lists.*11 days left.*0\/3/s.test($(".thead").textContent), $(".thead").textContent);
    // Linked panes: a lamp dot for each, a hint when an agent finished, a task line on the tile.
    window.__emit("pane", Object.assign(structuredClone(window.__data.panes.find((x) => x.id === "p1")), { lamp: "turn" }));
    await wait(80);
    check("a card has a lamp dot for each linked pane", $('[data-task="t1"] .tdot')?.dataset.lamp === "turn" && $('[data-task="t3"] .tdot')?.dataset.lamp === "ended");
    check("an agent at Your turn on a task In progress shows the hint", !!$('[data-task="t1"] [data-hint="review"]'));
    $('[data-task="t1"] [data-hint="hush"]').click();
    await wait(30);
    check("dismiss hides the hint and changes nothing", !$('[data-task="t1"] .thint') && !called("task_status").length);
    const p1 = window.__data.panes.find((x) => x.id === "p1");
    window.__emit("pane", Object.assign(structuredClone(p1), { lamp: "working" }));
    await wait(30);
    window.__emit("pane", Object.assign(structuredClone(p1), { lamp: "turn" }));
    await wait(50);
    check("the hint comes back when the agent finishes again", !!$('[data-task="t1"] [data-hint="review"]'));
    $('[data-task="t1"] [data-hint="review"]').click();
    await wait(80);
    check("the hint sets In review", called("task_status").some((c) => c[1].id === "t1" && c[1].status === "review") && !!$('.tcol[data-status="review"] [data-task="t1"]'));
    await window.SB.call("task_status", { id: "t1", status: "doing" });
    await wait(50);
    window.__calls.splice(0, window.__calls.length, ...window.__calls.filter((c) => c[0] !== "task_status"));
    check("a tile shows the title of its task", ($('.pane[data-id="p1"] .ptask') || $("#livetitle .ptask"))?.textContent.includes("Review PR 10071"));
    docKey("j");
    await wait(50);
    check("j focuses the first task, and review keys stay off", $('[data-task="t2"]').classList.contains("focused") && called("review_mark").length === reviewMarks);
    docKey("l");
    await wait(50);
    check("l moves to the next column", $('[data-task="t1"]').classList.contains("focused"));
    docKey("3");
    await wait(50);
    check("3 asks who blocks the task", !$("#tbox").hidden && !!$("#tb-reason"));
    $("#tb-reason").value = "Sean's review";
    $("#tbox .btn.primary").click();
    await wait(80);
    check("Blocked sends the reason", called("task_status").some((c) => c[1].id === "t1" && c[1].status === "blocked" && c[1].blockedBy === "Sean's review") && $("#tbox").hidden);
    check("the card moves to Blocked", !!$('.tcol[data-status="blocked"] [data-task="t1"]'));
    docKey("Enter");
    await wait(80);
    check("Enter opens the panel with the history", $(".tpanel")?.dataset.panel === "t1" && $("#tp-title").value === "Review PR 10071" && $("#tp-hist").textContent.includes("In progress"), $("#tp-hist")?.textContent);
    $("#tp-title").value = "Review PR 10071 again";
    $("#tp-title").dispatchEvent(new Event("change", { bubbles: true }));
    await wait(80);
    check("a title edit saves all the fields", called("task_edit").some((c) => c[1].id === "t1" && c[1].fields.title === "Review PR 10071 again" && c[1].fields.priority === "high" && c[1].fields.repo === "/c/lighthouse"));
    document.activeElement?.blur();
    docKey("Escape");
    await wait(50);
    check("Esc closes the panel", !$(".tpanel") && !tv.classList.contains("has-panel"));
    // The status select: the panel follows the change, and an empty reason goes back.
    $('[data-task="t1"]').click();
    await wait(50);
    $("#tp-status").focus();
    $("#tp-status").value = "doing";
    $("#tp-status").dispatchEvent(new Event("change", { bubbles: true }));
    await wait(80);
    check("a status change from the select hides Blocked by", $("#tp-blocked").closest("label").hidden && $("#tp-status").value === "doing");
    $("#tp-status").value = "blocked";
    $("#tp-status").dispatchEvent(new Event("change", { bubbles: true }));
    await wait(30);
    check("Blocked in the select asks for the reason", !$("#tp-blocked").closest("label").hidden && document.activeElement === $("#tp-blocked"));
    $("#tp-blocked").blur();
    await wait(50);
    check("an empty reason puts the status back", $("#tp-status").value === "doing" && !called("task_status").some((c) => c[1].id === "t1" && c[1].status === "blocked" && !c[1].blockedBy));
    $("#tp-status").value = "blocked";
    $("#tp-status").dispatchEvent(new Event("change", { bubbles: true }));
    await wait(30);
    const nStatus = called("task_status").length;
    $("#tp-blocked").value = "typed, then Esc";
    $("#tp-blocked").dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await wait(50);
    check("Esc in the reason field cancels", called("task_status").length === nStatus && !$(".tpanel"));
    // Back to Blocked for the checks below.
    await window.SB.call("task_status", { id: "t1", status: "blocked", blockedBy: "Sean's review" });
    await wait(50);
    docKey("h"); docKey("h");
    await wait(50);
    docKey("o");
    await wait(50);
    check("o opens a new-task field in the focused column", $(".tadd")?.dataset.status === "todo" && document.activeElement === $(".tadd"));
    $(".tadd").value = "Write the IL tests";
    $(".tadd").dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await wait(80);
    check("Enter adds the task to the sprint", called("task_add").some((c) => c[1].task.title === "Write the IL tests" && c[1].task.sprint === "s1" && c[1].task.status === "todo") && $('.tcol[data-status="todo"]').textContent.includes("Write the IL tests"));
    // A short drag that ends on the same card changes nothing.
    {
      const n = called("task_move").length;
      const dt = new DataTransfer();
      const c = $('[data-task="t2"]');
      c.dispatchEvent(new DragEvent("dragstart", { bubbles: true, dataTransfer: dt }));
      c.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: dt }));
      c.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: dt }));
      c.dispatchEvent(new DragEvent("dragend", { bubbles: true, dataTransfer: dt }));
      await wait(50);
      check("a drop on itself does not move the card", called("task_move").length === n);
    }
    // Drag t2 onto the Done column.
    {
      const dt = new DataTransfer();
      $('[data-task="t2"]').dispatchEvent(new DragEvent("dragstart", { bubbles: true, dataTransfer: dt }));
      const zone = $('.tdrop[data-status="done"]');
      zone.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: dt }));
      check("a drag marks the drop column", zone.classList.contains("drop-in"));
      zone.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: dt }));
      await wait(80);
      check("a drop on a column sets the status", called("task_move").some((c) => c[1].id === "t2" && c[1].to.status === "done" && c[1].to.sprint === "s1") && !!$('.tcol[data-status="done"] [data-task="t2"]'));
    }
    // The list view: sprint, backlog, and closed sprints.
    $('#tview [data-v="list"]').click();
    await wait(50);
    const secs = $$("#view-tasks .tsec .tsname").map((x) => x.textContent);
    check("the list shows the sprint, the backlog and the closed sprint", secs.join("|") === "Sprint 1|Backlog|Sprint 0", secs.join("|"));
    {
      const dt = new DataTransfer();
      $('.trow[data-task="t5"]').dispatchEvent(new DragEvent("dragstart", { bubbles: true, dataTransfer: dt }));
      const before = $('.trow[data-task="t1"]');
      before.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: dt }));
      check("a drag over a row marks the place", before.classList.contains("drop-before"));
      before.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: dt }));
      await wait(80);
      const mv = called("task_move").slice(-1)[0]?.[1];
      check("a drop in a sprint plans the task there, before the row", mv?.id === "t5" && mv.to.sprint === "s1" && mv.to.status === "todo" && mv.to.before === "t1", JSON.stringify(mv));
    }
    $('[data-closed="s0"]').click();
    await wait(50);
    check("a closed sprint opens read-only", $('.trow[data-task="t4"]')?.getAttribute("draggable") === "false");
    $('.trow[data-task="t4"]').click();
    await wait(50);
    check("a closed task has Copy to backlog and no edits", !!$('[data-act="copy"]') && !$('[data-act="delete"]') && $("#tp-title").disabled && $("#tp-status").disabled);
    $('.trow[data-task="t5"]').click();
    await wait(50);
    $('#tp-nseg [data-v="preview"]').click();
    await wait(50);
    const notes = $(".tnotes-md");
    check("notes render Markdown with no script, image or bad link", !!notes?.querySelector("strong") && notes.textContent.includes("<script>") && !notes.querySelector("img, script") && !notes.querySelector('a[href^="javascript"]') && !!notes.querySelector('a[href="https://example.com"]') && window.__xss === undefined, notes?.innerHTML);
    $('#tp-nseg [data-v="edit"]').click();
    $('[data-act="panel-close"]').click();
    // Close the sprint: every open task to the backlog, no next sprint.
    $('[data-act="sprint-close"]').click();
    await wait(50);
    check("close lists each task that is not done", $$("#tbox [data-place]").length === 4, $$("#tbox [data-place]").length);
    $("#tc-all").value = "backlog";
    $("#tc-all").dispatchEvent(new Event("change", { bubbles: true }));
    $("#tc-next").checked = false;
    $("#tc-next").dispatchEvent(new Event("change", { bubbles: true }));
    $("#tbox .btn.primary").click();
    await wait(80);
    const cl = called("sprint_close")[0]?.[1];
    check("close sends a place for each task", cl?.id === "s1" && cl.next === null && cl.places.length === 4 && cl.places.every(([, p]) => p === "backlog"), JSON.stringify(cl));
    $('#tview [data-v="board"]').click();
    await wait(50);
    check("with no open sprint the board shows the backlog and New sprint", $(".tsprint").textContent.includes("No sprint is open") && !!$('.tempty [data-act="sprint-new"]') && $$("#view-tasks .tsec .trow").length === 4, $$("#view-tasks .tsec .trow").length);
    $('.tempty [data-act="sprint-new"]').click();
    await wait(50);
    $("#ts-name").value = "";
    $("#tbox .btn.primary").click();
    await wait(30);
    check("a sprint with no name is refused in the dialog", $("#tb-err").textContent.includes("name") && !called("sprint_new").length);
    $("#ts-name").value = "Sprint 3";
    $("#tbox .btn.primary").click();
    await wait(80);
    const ns = called("sprint_new")[0]?.[1].sprint;
    check("New sprint sends 14 days from today", ns?.name === "Sprint 3" && /^\d{4}-\d\d-\d\d$/.test(ns.start) && Math.round((new Date(ns.end) - new Date(ns.start)) / 86400000) === 13, JSON.stringify(ns));
    check("the new sprint shows on the board", $(".tsprint").textContent.includes("Sprint 3") && $$("#view-tasks .tcol").length === 5);
    {
      const d = structuredClone(window.__data.tasks);
      const y = new Date(); y.setDate(y.getDate() - 1);
      d.sprints.find((x) => x.state === "open").end = `${y.getFullYear()}-${String(y.getMonth() + 1).padStart(2, "0")}-${String(y.getDate()).padStart(2, "0")}`;
      window.__emit("tasks", d);
      await wait(30);
      check("a sprint that ended yesterday says so, in amber", $(".tsdays").textContent === "ended 1 day ago" && $(".tsdays").classList.contains("late"), $(".tsdays").textContent);
    }
    $('[data-act="sprint-edit"]').click();
    await wait(30);
    key(" ", { ctrl: true, code: "Space" });
    key("b");
    await wait(50);
    check("leaving the tab closes a task dialog", $("#tbox").hidden && !$("#view-board").hidden);

    // Phase 2: agents from a task (SPEC 27.6).
    key(" ", { ctrl: true, code: "Space" });
    key("t");
    await wait(50);
    $('#tview [data-v="list"]').click();
    await wait(30);
    check("a task whose repo folder is gone says so", $('.trow[data-task="t5"] .tmiss')?.textContent === "repo missing");
    $('.trow[data-task="t5"]').click();
    await wait(50);
    check("Start agent is off when the repo is missing", $('[data-act="start"]')?.disabled === true);
    $('.trow[data-task="t3"]').click();
    await wait(50);
    check("the panel lists the linked pane as ended, with Resume", $(".tpa")?.textContent.includes("spec agent") && $(".tpa")?.textContent.includes("Ended") && !!$('.tpa [data-pact="resume"]'));
    $('.tpa [data-pact="resume"]').click();
    await wait(80);
    check("Resume asks the hub to resume that pane", called("task_resume").some((c) => c[1].pane === "p7"));
    window.SB.setTab("tasks");
    await wait(30);
    $('.trow[data-task="t3"]').click();
    await wait(50);
    $('.tpa [data-pact="unlink"]').click();
    await wait(80);
    check("× unlinks the pane", called("task_unlink").some((c) => c[1].pane === "p7") && !$(".tpa"));
    $('[data-act="link"]').click();
    await wait(30);
    check("Link pane lists the open panes", $$("[data-linkpane]").map((b) => b.dataset.linkpane).includes("p2") && !$$("[data-linkpane]").some((b) => b.dataset.linkpane === "p3"));
    $('[data-linkpane="p2"]').click();
    await wait(80);
    check("a pick links the pane to the task", called("task_link").some((c) => c[1].task === "t3" && c[1].pane === "p2") && $(".tpa")?.dataset.pane === "p2");
    $('[data-act="start"]').click();
    await wait(80);
    check("Start agent fills the launcher from the task", !$("#launcher").hidden && !$("#ltask").hidden && $("#ltasktitle").textContent === "Wait for the spec" && $("#lbranch").value === "sb/wait-for-the-spec" && $("#lfirst").value === "Wait for the spec" && $('#lwhere [data-v="new"]').getAttribute("aria-pressed") === "true" && $('#lrun [data-v="claude"]').getAttribute("aria-pressed") === "true", JSON.stringify({ b: $("#lbranch").value, f: $("#lfirst").value }));
    check("the launcher picks the task's repo", $('#lrepos li[aria-selected="true"] .rp')?.textContent.startsWith("/c/lighthouse "), $('#lrepos li[aria-selected="true"]')?.textContent);
    $("#lfirst").value = "Wait for the spec, then rebase.";
    $("#lgo").click();
    await wait(100);
    const req = called("pane_open").slice(-1)[0]?.[1].req;
    check("Open sends the task, the title and the first message", req?.task === "t3" && req.title === "Wait for the spec" && req.first === "Wait for the spec, then rebase." && req.place === "new" && req.branch === "sb/wait-for-the-spec" && req.run === "claude", JSON.stringify(req));
    window.SB.openLauncher();
    await wait(30);
    check("a plain launcher has no task part", $("#ltask").hidden && !$("#lsave").hidden);
    $("#lcancel").click();
    window.__emit("paste_failed", { pane: "p9", text: "hello", why: "the agent did not start in time" });
    await wait(30);
    check("a failed first message shows in a dialog to copy", !$("#tbox").hidden && $("#tb-text").value === "hello" && $("#tbox").textContent.includes("did not start in time"));
    window.SB.setTab("board");
    await wait(30);
    check("a tab change keeps that dialog", !$("#tbox").hidden);
    $("#tbox .btn.primary").click();
    await wait(30);
    check("Copy closes it and says if the copy worked", $("#tbox").hidden && /Copied|did not work/.test($("#toast").textContent), $("#toast").textContent);
    window.SB.setTab("tasks");
    await wait(30);
    // A task whose repo is not in the repo list: an error, never another repo.
    {
      const d = structuredClone(window.__data.tasks);
      d.tasks.find((t) => t.id === "t3").repo = "/c/elsewhere";
      window.__data.tasks.tasks.find((t) => t.id === "t3").repo = "/c/elsewhere";
      window.__emit("tasks", d);
      await wait(30);
      $('.trow[data-task="t3"]').click();
      await wait(50);
      const n = called("pane_open").length;
      $('[data-act="start"]').click();
      await wait(120);
      check("a task repo that is not in the list is an error", $("#lerr").textContent.includes("not in your repo list"), $("#lerr").textContent);
      $("#lgo").click();
      await wait(50);
      check("Open does not start the agent in another repo", called("pane_open").length === n && !$("#launcher").hidden);
      $("#lcancel").click();
      window.__data.tasks.tasks.find((t) => t.id === "t3").repo = "/c/lighthouse";
    }
    // The + button opens a plain launcher: no task, no first message.
    $("#newpane").click();
    await wait(80);
    check("the + button opens a launcher with no task part", !$("#launcher").hidden && $("#ltask").hidden && !$("#view-board").hidden);
    $("#lbranch").value = "plain";
    $('#lwhere [data-v="new"]').click();
    $("#lbranch").value = "plain";
    $("#lgo").click();
    await wait(80);
    const plain = called("pane_open").slice(-1)[0]?.[1].req;
    check("a plain open sends no task and no first message", plain && !plain.task && !plain.first && plain.title === null, JSON.stringify(plain));
    window.SB.setTab("tasks");
    await wait(30);
    // The task menu of a tile.
    window.SB.setTab("board");
    await wait(50);
    check("an nvim tile has no task menu", !$('.pane[data-id="p3"] [data-ptaskmenu]'));
    const tm = $('.pane[data-id="p2"] [data-ptaskmenu]');
    tm.click();
    await wait(30);
    tm.click();
    await wait(30);
    check("a second click on the task button closes the menu", $("#taskmenu").hidden);
    $('.pane[data-id="p2"] [data-ptaskmenu]').click();
    await wait(30);
    check("the tile task menu offers a new task and open tasks", !$("#taskmenu").hidden && $("#taskmenu").textContent.includes("New task from this pane") && $("#taskmenu").textContent.includes("Unlink from"));
    $('#taskmenu [data-tm="link"]').click();
    await wait(80);
    check("a pick in the menu links the pane", called("task_link").slice(-1)[0]?.[1].pane === "p2" && $("#taskmenu").hidden);
    $('.pane[data-id="p2"] [data-ptaskmenu]').click();
    await wait(30);
    $('#taskmenu [data-tm="new"]').click();
    await wait(120);
    const na = called("task_add").slice(-1)[0]?.[1].task;
    check("New task from this pane adds a task and links it", na?.title === "il" && na.repo === "/c/lighthouse" && called("task_link").slice(-1)[0]?.[1].pane === "p2", JSON.stringify(na));
    $('.pane[data-id="p2"] .ptask').click();
    await wait(50);
    check("the task line of a tile opens the task", !$("#view-tasks").hidden && $(".tpanel")?.dataset.panel === called("task_link").slice(-1)[0]?.[1].task);
    document.activeElement?.blur();
    const plans = called("task_start_plan").length;
    docKey("s");
    await wait(80);
    check("s starts an agent for the focused task", called("task_start_plan").length === plans + 1 && !$("#launcher").hidden);
    $("#lcancel").click();
    {
      const noRepo = window.__data.tasks.tasks.find((t) => t.title === "Write the IL tests");
      window.SB.setTab("tasks");
      await wait(30);
      $('#tview [data-v="list"]').click();
      await wait(30);
      $(`.trow[data-task="${noRepo.id}"]`).click();
      await wait(50);
      $('[data-act="start"]').click();
      await wait(120);
      check("Start on a task with no repo lets you pick the repo", !$("#launcher").hidden && !$("#ltask").hidden && $("#lsave").hidden && $$("#lrepos li[data-i]").length === 2 && $("#lfirst").value === "Write the IL tests", JSON.stringify({ ltask: $("#ltask").hidden, n: $$("#lrepos li[data-i]").length, f: $("#lfirst").value, err: window.__errors }));
      $("#lcancel").click();
    }

  // Tasks, phase 3 (SPEC 27): points, the burndown, planned sprints, Markdown, tags and
  // the filter, the archive, PR merged, Review from a task, and Shift+J/K. A fresh list.
  {
    const TT = window.SBTasks._test;
    const at = (d, hm) => new Date(`${d}T${hm}`).getTime() / 1000;
    const pr = "https://github.com/sigp/lighthouse/pull/10071";
    const d3 = {
      sprints: [{ id: "s1", name: "Sprint 1", goal: "Plan", start: dayOff(-2), end: dayOff(11), state: "open" }],
      tasks: [
        tk({ id: "a1", title: "Alpha infra task", rank: 1024, points: 5, tags: ["infra"], status: "done", done: at(dayOff(-1), "12:00") }),
        tk({ id: "a2", title: "Beta ci task", rank: 2048, points: 3, tags: ["ci"], status: "doing", links: ["https://example.com/doc"] }),
        tk({ id: "a3", title: "Gamma", rank: 3072, points: 2, priority: "high", repo: "/c/lighthouse", notes: "needle in the notes", links: [pr] }),
        tk({ id: "a4", title: "Delta", rank: 4096, links: ["https://example.com/a"] }),
      ],
      links: [
        { pane: "p1", task: "a2", kind: "claude", title: null, tree: "/c/lighthouse", session: "s1", connection: "a", closed: false },
        { pane: "p2", task: "a2", kind: "claude", title: null, tree: "/c/lighthouse-il", session: "s2", connection: "a", closed: false },
      ],
      missing: [],
      archive: [tk({ id: "z1", title: "Old deleted task", sprint: "s0", rank: 1024, deleted: at(dayOff(-1), "09:00") })],
      burn: ["a1", "a2", "a3", "a4"].map((task) => ({ task, at: at(dayOff(-2), "10:00"), status: "todo" })).concat([{ task: "a1", at: at(dayOff(-1), "12:00"), status: "done" }]),
      merged: [pr],
    };
    window.__data.tasks = structuredClone(d3);
    window.__data.history.a2 = [{ at: 1760000000, by: "you", kind: "points", value: "3" }, { at: 1760000001, by: "p1", kind: "link", value: "https://example.com/doc" },
      { at: 1760000002, by: "you", kind: "deleted", value: null }, { at: 1760000003, by: "you", kind: "restored", value: null }];
    window.SB.setTab("tasks");
    $('#tview [data-v="board"]').click();
    window.__emit("tasks", structuredClone(d3));
    await wait(80);
    document.querySelector('[data-act="panel-close"]')?.click();
    await wait(30);
    check("a card shows its points and its tags", $('[data-task="a2"] .tpts')?.textContent === "3" && $('[data-task="a2"] .ttag')?.textContent === "#ci");
    check("the sprint header shows done points over total points", $(".tscount")?.textContent === "5/10 pts", $(".tscount")?.textContent);
    check("the burndown line shows the points left", $("svg.tburn")?.getAttribute("aria-label") === "Burndown: 5 of 10 points left" && !!$("svg.tburn polyline.real"), $("svg.tburn")?.outerHTML);
    {
      const b = TT.burnLeft(d3.sprints[0], TT.T.tasks.filter((t) => t.sprint === "s1"));
      check("the burndown has one value for each day up to today", JSON.stringify(b.left) === "[10,5,5]" && b.days.length === 14, JSON.stringify(b));
      // Local midnight, over the DST change of March 2026 (the checks run in America/New_York).
      const keep = TT.T.burn;
      TT.T.burn = [{ task: "x", at: at("2026-03-06", "09:00"), status: "todo" }, { task: "y", at: at("2026-03-06", "09:00"), status: "todo" },
        { task: "x", at: at("2026-03-08", "23:30"), status: "done" }, { task: "y", at: at("2026-03-09", "00:30"), status: "done" }];
      const d = TT.burnLeft({ start: "2026-03-06", end: "2026-03-10" }, [{ id: "x", points: 1 }, { id: "y", points: 2 }]);
      TT.T.burn = keep;
      check("the burndown splits days at local midnight, also over a DST change", JSON.stringify(d.days) === '["2026-03-06","2026-03-07","2026-03-08","2026-03-09","2026-03-10"]' && JSON.stringify(d.left) === "[3,3,2,0,0]", JSON.stringify(d));
    }

    // The filter.
    const filter = async (text) => { const f = $("#tfilter"); f.focus(); f.value = text; f.dispatchEvent(new Event("input", { bubbles: true })); await wait(30); };
    const cards = () => $$("#view-tasks .tboard [data-task]").map((x) => x.dataset.task).sort().join();
    await filter("#infra");
    check("a #tag filter shows only the tasks with the tag", cards() === "a1", cards());
    check("the filter keeps its focus while you type", document.activeElement?.id === "tfilter" && $("#tfilter").value === "#infra");
    await filter("prio:high");
    check("prio: filters by priority", cards() === "a3", cards());
    await filter("repo:lighth");
    check("repo: filters by repo", cards() === "a3", cards());
    await filter("needle");
    check("a word matches the notes too", cards() === "a3", cards());
    await filter("beta task");
    check("every word must match", cards() === "a2", cards());
    $("#tfilter").dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await wait(30);
    check("Esc in the filter clears it", $("#tfilter").value === "" && cards() === "a1,a2,a3,a4" && document.activeElement?.id !== "tfilter", cards());
    docKey("/");
    await wait(20);
    check("/ focuses the filter", document.activeElement?.id === "tfilter");
    document.activeElement.blur();
    $('[data-task="a1"] .ttag').click();
    await wait(30);
    check("a click on a tag filters by it and opens no panel", $("#tfilter").value === "#infra" && cards() === "a1" && !$(".tpanel"), cards());
    await filter("");
    document.activeElement.blur();
    $('#tview [data-v="list"]').click();
    await wait(30);
    await filter("zzz");
    check("the filter applies to the list view too", $$("#view-tasks .trow").length === 0 && $(".tsec .tnone")?.textContent === "No task matches the filter.");
    await filter("");
    document.activeElement.blur();
    $('#tview [data-v="board"]').click();
    await wait(30);

    // PR merged.
    check("a merged PR link shows the PR merged hint", !!$('[data-task="a3"] [data-hint="done"]') && !$('[data-task="a4"] [data-hint="done"]'));
    const st = called("task_status").length;
    $('[data-task="a3"] [data-hint="done"]').click();
    await wait(30);
    check("PR merged → Done sets the task to Done", called("task_status").slice(-1)[0]?.[1].status === "done" && called("task_status").length === st + 1);
    window.__data.tasks = structuredClone(d3);
    window.__emit("tasks", structuredClone(d3));
    await wait(30);
    $('[data-task="a3"] [data-hint="hush-merged"]').click();
    await wait(30);
    window.__emit("pane", Object.assign(structuredClone(window.__data.panes.find((x) => x.id === "p2")), { lamp: "working" }));
    window.__emit("tasks", structuredClone(d3));
    await wait(40);
    check("Dismiss hides the PR merged hint, also after pane and task events", !$('[data-task="a3"] [data-hint="done"]'));

    // The panel: points, tags and links.
    $('[data-task="a2"]').click();
    await wait(60);
    check("the panel shows the points, the tags and the links", $("#tp-points")?.value === "3" && $("#tp-tags")?.value === "ci" && $$("#tp-linklist a").length === 1);
    check("the history names the new rows", /Points: 3.*Link added: https:\/\/example\.com\/doc.*Moved to the archive.*Restored/s.test([...$$("#tp-hist li")].reverse().map((x) => x.textContent).join("|")), $("#tp-hist")?.textContent);
    const edits = () => called("task_edit");
    const ne = edits().length;
    $("#tp-points").value = "8";
    $("#tp-points").dispatchEvent(new Event("change", { bubbles: true }));
    await wait(30);
    check("a points edit sends the points", edits().length === ne + 1 && edits().slice(-1)[0][1].fields.points === 8, JSON.stringify(edits().slice(-1)[0]));
    $("#tp-points").value = "120";
    $("#tp-points").dispatchEvent(new Event("change", { bubbles: true }));
    await wait(30);
    check("points out of range stay in the window", edits().length === ne + 1 && $("#tp-points").value === "8");
    $("#tp-tags").value = "#Infra, CI  docs";
    $("#tp-tags").dispatchEvent(new Event("change", { bubbles: true }));
    await wait(30);
    check("a tags edit sends lowercase words with no #", JSON.stringify(edits().slice(-1)[0][1].fields.tags) === '["infra","ci","docs"]', JSON.stringify(edits().slice(-1)[0][1].fields));
    {
      // An agent note arrives while you type a link: the next edit keeps the note.
      const d = structuredClone(window.__data.tasks);
      d.tasks.find((t) => t.id === "a2").notes = "**Note from p1:**\ntests pass";
      window.__data.tasks = d;
      $("#tp-link").focus();
      window.__emit("tasks", structuredClone(d));
      await wait(40);
      $("#tp-link").value = "https://example.com/n";
      $("#tp-link").dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      await wait(60);
      const f = edits().slice(-1)[0][1].fields;
      check("an edit while the panel stays keeps a note that arrived meanwhile", f.notes === "**Note from p1:**\ntests pass" && f.points === 8 && JSON.stringify(f.tags) === '["infra","ci","docs"]', JSON.stringify(f));
      $("#tp-linklist [data-unlinkurl='https://example.com/n']").click();
      await wait(60);
    }
    const ln = $("#tp-link");
    ln.focus();
    ln.value = "javascript:alert(1)";
    ln.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await wait(30);
    check("a link that is not http(s) is refused", edits().length === ne + 4 && $("#tp-link").value === "javascript:alert(1)");
    $("#tp-link").value = "https://example.com/new";
    $("#tp-link").dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await wait(60);
    check("Enter in the link field adds the link", JSON.stringify(edits().slice(-1)[0][1].fields.links) === '["https://example.com/doc","https://example.com/new"]' && $("#tp-link").value === "" && $$("#tp-linklist a").length === 2,
      JSON.stringify(edits().slice(-1)[0][1].fields.links));
    document.activeElement.blur();
    $('#tp-linklist [data-unlinkurl="https://example.com/doc"]').click();
    await wait(60);
    check("× removes a link", JSON.stringify(edits().slice(-1)[0][1].fields.links) === '["https://example.com/new"]');
    const opens = called("open_url").length;
    $("#tp-linklist a").click();
    await wait(30);
    check("a link opens in the browser, not in the window", called("open_url").length === opens + 1 && called("open_url").slice(-1)[0][1].url === "https://example.com/new" && location.protocol === "file:");

    // Review from a task: two linked panes, so you pick one.
    $('[data-act="review"]').click();
    await wait(30);
    check("Review with two linked panes asks which one", $$("[data-reviewpane]").length === 2);
    const rv = called("review_open_local").length;
    $('[data-reviewpane="p2"]').click();
    await wait(80);
    check("the pick opens a review of that pane's commits", called("review_open_local").length === rv + 1 && called("review_open_local").slice(-1)[0][1].pane === "p2");
    window.SB.setTab("tasks");
    await wait(30);
    // A PR link of a review open in Switchboard opens its tab.
    $('[data-task="a3"]').click();
    await wait(60);
    const urls = called("open_url").length;
    $("#tp-linklist a").click();
    await wait(30);
    check("a PR link of an open review opens the review tab", window.SB.S.tab === "r1" && called("open_url").length === urls, window.SB.S.tab);
    window.SB.setTab("tasks");
    await wait(30);
    $('[data-act="panel-close"]').click();
    await wait(30);

    // Shift+J and Shift+K move the focused task in its column.
    $('[data-task="a3"]').click();
    await wait(30);
    $('[data-act="panel-close"]').click();
    await wait(30);
    const moves = called("task_move").length;
    docKey("J");
    await wait(40);
    check("J with no Shift (Caps Lock) moves no task", called("task_move").length === moves);
    const shiftKey = (k) => document.body.dispatchEvent(new KeyboardEvent("keydown", { key: k, shiftKey: true, bubbles: true }));
    shiftKey("J");
    await wait(40);
    const mv = called("task_move").slice(-1)[0]?.[1];
    check("Shift+J moves the task down one place", mv?.id === "a3" && mv.to.before === null && mv.to.status === null && mv.to.sprint === "s1", JSON.stringify(mv));
    check("the board shows the new order", $$('.tcol[data-status="todo"] [data-task]').map((x) => x.dataset.task).join() === "a4,a3");
    shiftKey("K");
    await wait(40);
    const mv2 = called("task_move").slice(-1)[0]?.[1];
    check("Shift+K moves it up again", mv2?.id === "a3" && mv2.to.before === "a4" && $$('.tcol[data-status="todo"] [data-task]').map((x) => x.dataset.task).join() === "a3,a4", JSON.stringify(mv2));

    // Copy as Markdown.
    const mdText = TT.sprintMarkdown(TT.T.sprints.find((s) => s.id === "s1"));
    check("Copy as Markdown groups the tasks by status, with points and links",
      /^## Sprint 1 — Plan\n/.test(mdText) && mdText.includes("1 of 4 tasks done · 5 of 15 points done") && /### Done \(1\)\n- Alpha infra task \(5 pts\)/.test(mdText)
      && /### In progress \(1\)\n- Beta ci task \(8 pts\)\n  - https:\/\/example\.com\/new/.test(mdText) && mdText.indexOf("### Done") < mdText.indexOf("### To do"), mdText);
    $('.thead [data-act="sprint-md"]').click();
    await wait(40);
    check("the Copy as Markdown button copies, or shows the text to copy", !$("#toast").hidden || $("#tb-text")?.value === mdText);
    if (!$("#tbox").hidden) $("#tbox [data-b='0']").click();
    await wait(20);

    // A planned sprint.
    check("the head offers Plan next sprint", [...$$('.thead [data-act="sprint-new"]')].some((b) => b.textContent === "Plan next sprint"));
    $('.thead [data-act="sprint-new"]').click();
    await wait(40);
    check("the plan dialog says when the sprint opens", $("#tbox h4")?.textContent === "Plan the next sprint" && $("#tbox").textContent.includes("It opens when you close Sprint 1") && $("#ts-start").value === dayOff(12), $("#ts-start")?.value);
    $("#ts-name").value = "Sprint 2";
    $("#tbox .btn.primary").click();
    await wait(60);
    check("Plan sprint makes a planned sprint", called("sprint_new").slice(-1)[0]?.[1].sprint.name === "Sprint 2" && window.__data.tasks.sprints.find((s) => s.name === "Sprint 2")?.state === "planned");
    check("with a planned sprint the head offers no Plan next sprint", ![...$$('.thead [data-act="sprint-new"]')].length);
    $('#tview [data-v="list"]').click();
    await wait(30);
    check("the list view shows the planned sprint after the open one", $$("#view-tasks .tsec .tsname").slice(0, 3).map((x) => x.textContent).join() === "Sprint 1,Sprint 2,Backlog" && !!$(".tsec .tplan"),
      $$("#view-tasks .tsec .tsname").map((x) => x.textContent).join());
    $('.thead [data-act="sprint-close"]').click();
    await wait(40);
    check("Close names the planned sprint and asks for no new one", $("#tbox").textContent.includes("Sprint 2 opens next.") && !$("#tc-next") && $("#tbox [data-place] option")?.textContent === "Sprint 2");
    $("#tbox .btn.primary").click();
    await wait(60);
    const cl = called("sprint_close").slice(-1)[0]?.[1];
    check("Close sends no new sprint when one is planned", cl?.next === null && cl.places.length === 3, JSON.stringify(cl));
    check("the planned sprint opens", $$("#view-tasks .tsec .tsname")[0]?.textContent === "Sprint 2" && !$(".tsec .tplan"));

    // The archive.
    const live = window.__data.tasks.tasks.find((t) => t.sprint === window.__data.tasks.sprints.find((s) => s.name === "Sprint 2").id);
    $(`.trow[data-task="${live.id}"]`).click();
    await wait(50);
    $('[data-act="delete"]').click();
    await wait(30);
    check("Delete says the task goes to the archive", $("#tbox").textContent.includes("to the archive") && $("#tbox").textContent.includes("30 days"));
    $("#tbox .btn.primary").click();
    await wait(60);
    check("Delete sends task_delete", called("task_delete").slice(-1)[0]?.[1].id === live.id && !$(`.trow[data-task="${live.id}"]`));
    $('#tview [data-v="archive"]').click();
    await wait(30);
    check("the Archive view lists the deleted tasks, newest first", $$("#view-tasks [data-arch]").map((x) => x.dataset.arch).join() === `${live.id},z1` && $$("#view-tasks [data-restore]").length === 2,
      $$("#view-tasks [data-arch]").map((x) => x.dataset.arch).join());
    check("the Archive view has no new-task button", !$('.thead [data-act="new"]'));
    $('[data-restore="z1"]').click();
    await wait(60);
    {
      // The keys do nothing on the Archive view, also with a task focused before.
      TT.T.focus = "a4";
      const n = called("task_status").length + called("task_start_plan").length;
      docKey("5");
      docKey("s");
      docKey("o");
      await wait(40);
      check("the status, start and new-task keys do nothing on the Archive view", called("task_status").length + called("task_start_plan").length === n && !TT.T.adding && $("#launcher").hidden);
    }
    check("Restore takes the task out of the archive", called("task_restore").slice(-1)[0]?.[1].id === "z1" && !$('[data-arch="z1"]') && window.__data.tasks.tasks.find((t) => t.id === "z1")?.sprint === null);
    $('#tview [data-v="board"]').click();
    await wait(30);
  }
    window.SB.setTab("board");
  }

  // Parts of the live pane (SPEC 28): nvim and the shell next to an agent.
  {
    const PT = window.SBParts;
    const leader = (k) => { key(" ", { ctrl: true }); key(k); };
    const text = (t) => { const b = t.buffer.active; let s = ""; for (let i = 0; i < b.length; i++) s += b.getLine(i).translateToString(true) + "\n"; return s; };
    if (window.SB.S.live) window.SB.collapse();
    await window.SB.expand("p1");
    await wait(60);
    check("a lone agent shows no part headers", !$("#lparts").classList.contains("split") && $("#pnvim").hidden && $("#pterm").hidden);
    leader("e");
    await wait(150);
    check("Leader e opens nvim in the agent's worktree, next to it", called("nvim_open").at(-1)?.[1].tree === "/c/lighthouse" && called("pane_attach").at(-1)?.[1].id === "pn1" && !$("#pnvim").hidden && !$("#pagent").hidden && $("#lparts").classList.contains("split"));
    check("the hub gets the size of the nvim terminal", [...called("pane_attach"), ...called("part_resize")].filter((c) => c[1].id === "pn1").length && [...window.__calls].reverse().find((c) => (c[0] === "pane_attach" || c[0] === "part_resize") && c[1].id === "pn1")?.[1].cols === PT.T.nvim.term.cols && PT.T.nvim.term.cols > 10, JSON.stringify(called("pane_attach").at(-1)?.[1]) + " " + PT.T.nvim.term.cols);
    check("nvim draws its screen in its own terminal", text(PT.T.nvim.term).includes("part pn1") && !screen().includes("part pn1"));
    check("the nvim of an agent has no tile", !$('#board .pane[data-id="pn1"]') && !!$('#board .pane[data-id="p3"]'));
    check("the agent tile shows a running nvim chip", /● nvim/.test($('.pane[data-id="p1"] [data-part="nvim"]')?.textContent || ""), $('.pane[data-id="p1"] .ptools')?.innerHTML);
    check("the live title shows nvim pressed", $('#livetitle [data-ptoggle="nvim"]')?.getAttribute("aria-pressed") === "true" && $('#livetitle [data-ptoggle="term"]')?.getAttribute("aria-pressed") === "false");
    check("the nvim header has the keys", $("#pnvim .parthead").classList.contains("on") && !$("#pagent .parthead").classList.contains("on"));
    leader("s");
    await wait(150);
    check("Leader s opens the shell under the agent", called("shell_open").at(-1)?.[1].tree === "/c/lighthouse" && !$("#pterm").hidden && !$("#pnvim").hidden && $("#pterm").parentElement === $("#lleft"));
    window.__emit("pane_output", { pane: "ps1", data: btoa("shell-bytes-1") });
    window.__emit("pane_output", { pane: "pn1", data: btoa("nvim-bytes-1") });
    await wait(90);
    check("output goes to the terminal of its part", text(PT.T.term.term).includes("shell-bytes-1") && text(PT.T.nvim.term).includes("nvim-bytes-1") && !screen().includes("shell-bytes-1"));
    {
      // Keys in a part go to its pane.
      const n = called("pane_input").length;
      PT.T.term.term.input("ls");
      await wait(90);
      check("keys in the shell part go to the shell pane", called("pane_input").slice(n).some((c) => c[1].id === "ps1" && c[1].data === "ls"));
    }
    {
      // A narrow window has no zoom to change.
      const w = Object.getOwnPropertyDescriptor(window, "innerWidth");
      Object.defineProperty(window, "innerWidth", { value: 900, configurable: true });
      const before = PT.P.zoom;
      const r = PT.zoom("nvim");
      Object.defineProperty(window, "innerWidth", w || { value: 1600, configurable: true });
      check("Leader z does nothing in a narrow window", r === false && PT.P.zoom === before);
    }
    leader("z");
    await wait(60);
    check("Leader z zooms the part with the keys", PT.P.zoom === "term" && $("#pagent").hidden && $("#pnvim").hidden && !$("#pterm").hidden);
    leader("x");
    await wait(60);
    check("Leader x takes the zoom back first", PT.P.zoom === null && window.SB.S.live === "p1" && !$("#pagent").hidden && !$("#pnvim").hidden);
    leader("l");
    await wait(30);
    check("Leader l moves the keys to nvim", PT.P.focus === "nvim");
    leader("h");
    await wait(30);
    check("Leader h moves the keys back left", PT.P.focus === "term");
    leader("o");
    await wait(30);
    check("Leader o goes to the next part", PT.P.focus === "nvim");
    $('#pnvim [data-hide="nvim"]').click();
    await wait(60);
    check("× hides nvim and it keeps running", $("#pnvim").hidden && called("pane_detach").at(-1)?.[1].id === "pn1" && !called("pane_close").some((c) => c[1].id === "pn1"));
    check("the top bar counts the hidden parts", $("#tally .phidden")?.textContent === "1 hidden", $("#tally").innerHTML);
    window.SB.collapse();
    await wait(90);
    check("a collapse hides the parts", $("#pterm").hidden && !$("#lparts").classList.contains("split") && PT.T.term.id === null);
    {
      const n = called("pane_attach").length;
      await window.SB.expand("p1");
      await wait(120);
      const again = called("pane_attach").slice(n).map((c) => c[1].id);
      check("the next open shows the parts of the last time", again.join() === "ps1" && !$("#pterm").hidden && $("#pnvim").hidden, again.join());
    }
    $('#livetitle [data-ptoggle="nvim"]').click();
    await wait(150);
    check("the nvim switch in the title shows nvim", !$("#pnvim").hidden && PT.T.nvim.id === "pn1");
    window.__emit("pane", Object.assign(structuredClone(window.__data.panes.find((x) => x.id === "pn1")), { modified: true }));
    await wait(90);
    check("a buffer that is not saved shows [+] on the chip and in the header", /\[\+\]/.test($('.pane[data-id="p1"] [data-part="nvim"]')?.textContent || "") && $("#pnvim .parthead .pmod")?.textContent === "[+]");
    $('#pnvim [data-stop="nvim"]').click();
    await wait(30);
    check("Stop asks first when a buffer is not saved", !!$("#pnvim [data-stopyes]") && !called("pane_close").some((c) => c[1].id === "pn1"));
    $("#pnvim [data-stopyes]").click();
    await wait(90);
    check("Stop anyway closes nvim", called("pane_close").some((c) => c[1].id === "pn1") && $("#pnvim").hidden);
    window.__emit("nvim_reloaded", "/c/lighthouse");
    window.__emit("pane", Object.assign(structuredClone(window.__data.panes.find((x) => x.id === "ps1")), { busy: true }));
    await wait(90);
    check("a busy shell shows ▸ on the chip", /▸ term/.test($('.pane[data-id="p1"] [data-part="term"]')?.textContent || ""), $('.pane[data-id="p1"] .ptools')?.innerHTML);
    window.__emit("pane", Object.assign(structuredClone(window.__data.panes.find((x) => x.id === "ps1")), { busy: false }));
    await wait(90);
    check("an ended command shows ✓ done for a short time", /✓ term done/.test($('.pane[data-id="p1"] [data-part="term"]')?.textContent || ""));
    // Without an agent, the shell gets its tile back.
    window.SB.collapse();
    window.__data.panes = window.__data.panes.filter((x) => x.id !== "pn1");
    const p1 = window.__data.panes.find((x) => x.id === "p1");
    window.__emit("panes", window.__data.panes.filter((x) => x.id !== "p1"));
    await wait(90);
    check("a shell with no agent in its worktree has its own tile", !!$('#board .pane[data-id="ps1"]'));
    {
      // An ended agent gives its shell the tile back.
      window.__emit("panes", window.__data.panes.map((x) => x.id === "p1" ? Object.assign(structuredClone(x), { lamp: "ended" }) : x));
      await wait(90);
      check("an ended agent gives its shell its tile back", !!$('#board .pane[data-id="ps1"]') && !!$('#board .pane[data-id="p1"]'));
    }
    window.__data.panes = window.__data.panes.filter((x) => x.id !== "ps1");
    window.__emit("panes", window.__data.panes);
    await wait(90);
    check("the agent is back", !!$('#board .pane[data-id="p1"]') && !!p1);
  }

  // Two expands of one pane: only the newest answer draws, also when the older
  // one comes last (models/remote.qnt, R7).
  {
    let n = 0;
    window.__expandHook = () => {
      n += 1;
      return n === 1 ? wait(250).then(() => btoa("\x1b[H\x1b[2Jold screen")) : btoa("\x1b[H\x1b[2Jnew screen");
    };
    window.SB.collapse();
    await wait(50);
    window.SB.expand("p1");
    await wait(20);
    window.SB.expand("p1");
    await wait(400);
    window.__expandHook = null;
    check("an older expand answer never draws over the newer one", screen().includes("new screen") && !screen().includes("old screen"), screen().slice(0, 80));
    window.SB.collapse();
    await wait(50);
  }

  // The link to a remote hub (SPEC 29).
  {
    window.SB.setTab("board");
    await window.SB.expand("p1");
    await wait(60);
    window.__emit("link", { up: false, host: "box", why: "ssh: connect to host box port 22: Connection timed out", taken: false });
    await wait(30);
    check("a lost link shows a banner with the host and the reason", !$("#link").hidden && $("#linktext").textContent.includes("box") && $("#linktext").textContent.includes("timed out") && $("#linkgo").textContent === "Reconnect");
    check("the top bar names the host, marked down", !$("#host").hidden && $("#host").textContent === "box" && $("#host").classList.contains("down"));
    check("the banner keeps the board in its place", $("#view-board").getBoundingClientRect().top >= $("#link").getBoundingClientRect().bottom - 1);
    {
      // A command while the link is down: the banner says it, no toast.
      $("#toast").hidden = true;
      window.__failNext = "the link to the server is down";
      await window.SB.call("pane_seen", { id: "p1" }).catch(() => {});
      check("no toast while the link is down", $("#toast").hidden);
    }
    window.__linkFail = "ssh: Could not resolve hostname box";
    $("#linkgo").click();
    await wait(60);
    check("a failed reconnect says why", !$("#link").hidden && $("#linktext").textContent.includes("Could not resolve") && called("link_reconnect").length === 1);
    window.__linkFail = null;
    const n = { settings: called("settings").length, panes: called("panes").length, reviews: called("reviews").length, tasks: called("tasks").length, expand: called("pane_expand").length };
    $("#linkgo").click();
    await wait(30);
    window.__emit("link", { up: true, host: "box" });
    await wait(200);
    check("a reconnect hides the banner", $("#link").hidden && !$("#host").classList.contains("down"));
    check("a reconnect loads everything again", called("settings").length > n.settings && called("panes").length > n.panes && called("reviews").length > n.reviews && called("tasks").length > n.tasks,
      JSON.stringify(n) + " " + called("panes").length);
    check("a reconnect opens the live pane again", called("pane_expand").length > n.expand && window.SB.S.live === "p1");
    window.__emit("link", { up: false, host: "box", why: "another window took over", taken: true });
    await wait(30);
    check("a takeover says so and offers to take it back", $("#linktext").textContent.includes("took over") && $("#linkgo").textContent === "Take it back");
    window.__emit("link", { up: true, host: "box" });
    await wait(200);
    window.SB.collapse();
    Object.assign(window.SB.LINK, { remote: false, up: true });
    $("#host").hidden = true;
  }

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
