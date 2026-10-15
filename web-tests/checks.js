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
  check("the board takes the terminal back", $("#live").parentElement === $("#view-board") && $("#live").hidden && window.SB.S.live === null && window.SB.S.termHost === null);
  window.SB.setTab("r1");
  await wait(200);
  check("the review shows its Claude pane again", !!$("#rterm #live") && !$("#live").hidden && window.SB.S.live === "p8" && called("review_term").every((c) => c[1].id === "r1"));
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
