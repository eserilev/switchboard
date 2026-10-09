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
  check("pin shows in the guide", $("#rguide").textContent.includes("Use safe_sub, not -."));
  check("diff renders with focus lines", $$("#rdiff tr.focus").length === 2, $$("#rdiff tr.focus").length);
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
  check("drafts list with copy", $("#rdrafts").textContent.includes("nit: use safe_sub") && !!$("#rdrafts [data-copy]"));
  $(".thread[data-t='t1']").click();
  await wait(50);
  check("click a thread makes it active", $(".thread[data-t='t1']").classList.contains("active") && $("#ranchor").textContent === "");
  docKey("n");
  await wait(100);
  check("n marks the step", called("review_mark").some((c) => c[1].step === "s2" && c[1].checked));

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
