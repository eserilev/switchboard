// The phone screens (SPEC 42.9) at phone width, with a fake hub.
(async () => {
  const results = [];
  const check = (name, ok, info) => results.push({ name, ok: !!ok, info: ok ? undefined : info });
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const $ = (s) => document.querySelector(s);
  const $$ = (s) => [...document.querySelectorAll(s)];
  const calls = [];
  const called = (c) => calls.filter((x) => x[0] === c);
  const handlers = {};
  const emit = (ev, payload) => (handlers[ev] || []).forEach((f) => f({ payload }));
  const rows = [
    { kind: "@", old: null, new: null, text: "@@ -1,12 +1,12 @@" },
    ...Array.from({ length: 6 }, (_, k) => ({ kind: " ", old: k + 1, new: k + 1, text: `let a${k + 1} = ${k + 1};` })),
    { kind: "-", old: 7, new: null, text: "    .unwrap_or_default()" },
    { kind: "+", old: null, new: 7, text: "    .map_err(|e| Error::DBError { message: format!(\"data column keys of {block_root} and a very long line that must wrap: {e}\") })?" },
    { kind: "+", old: null, new: 8, text: "    .collect()" },
    ...Array.from({ length: 4 }, (_, k) => ({ kind: " ", old: k + 8, new: k + 9, text: `let b${k} = ${k};` })),
  ];
  const note = { severity: "must", text: "The LevelDB path still maps a read error to an empty list.", file: "src/a.rs", side: "new", line: 7 };
  let R = {
    id: "r1", kind: "pr", repo: "sigp/lighthouse", number: 10566, title: "Fix redb backend bugs", head: "abc", round: 1, status: "ready",
    guide: { take: { verdict: "change", summary: "compact() can fail.", notes: [] }, steps: [
      { id: "s1", title: "The story", what: "**Before:** it swallowed errors.\n\n**Now:** it returns them." },
      { id: "s2", title: "Data column index: keep errors", what: "It returns the error now.", check: ["Do we want to log it too?"], take: { verdict: "change", notes: [note] }, ranges: [] },
    ] },
    steps: [{ id: "s1", checked: true, stale: false }], dismissed: [], drafts: [], threads: [], summary: "",
  };
  const today = { job: "Everything", jobs: [{ name: "Everything", count: 2 }, { name: "Bug bounty", count: 1 }], sections: [
    { key: "mine", title: "Your PRs", items: [{ repo: "sigp/lighthouse", number: 10510, title: "Don't penalize peers", line: "Reply to dknopik's open thread.", section: "mine", tags: [], url: "https://github.com/sigp/lighthouse/pull/10510", detail: [], why: "" }] },
    { key: "reviews", title: "Reviews", items: [{ repo: "sigp/lighthouse", number: 10566, title: "Fix redb backend bugs", line: "Review asked of you 2 days ago.", section: "reviews", tags: ["tiny"], url: "https://github.com/sigp/lighthouse/pull/10566", detail: ["by eserilev"], why: "Ranked 1st" }] },
  ] };
  const panes = [
    { id: "p61", kind: "claude", repo: "lighthouse", branch: "redb-fixes", title: "#10566 redb fixes", lamp: "needs", permit: { id: "p61-1", summary: "Bash: cargo test -p store" }, summary: null },
    { id: "p7", kind: "claude", repo: "lighthouse", branch: "x", title: "backfill", lamp: "working", permit: null, summary: "Running tests" },
    { id: "p3", kind: "nvim", repo: "x", lamp: "none" },
  ];
  const answers = {
    today: () => today, reviews: () => [R, { ...R, id: "r2", kind: "local" }], panes: () => panes,
    review_diff: () => ({ head: "abc", sections: [{ path: "src/a.rs", rows, ranges: [] }] }),
    review_ask: () => "t1", review_post_preview: () => ({ login: "me", token: "tok", newer: null, plan: {} }), review_post: () => "https://github.com/x",
    review_open: () => "r1",
  };
  const api = {
    call: async (cmd, args) => { calls.push([cmd, args]); const f = answers[cmd]; return f ? JSON.parse(JSON.stringify(f(args))) : null; },
    listen: (ev, f) => (handlers[ev] ||= []).push(f),
    hub: { name: "archlinux", linked: true },
  };
  const ui = window.SBM.mount($("#app"), api);
  const extra = document.createElement("div");
  document.body.appendChild(extra);
  const again = window.SBM.mount(extra, { ...api, hub: { name: "archlinux", linked: true } });
  await wait(60);
  const tap = async (sel) => { const el = typeof sel === "string" ? $(sel) : sel; el?.click(); await wait(40); return el; };
  const swipe = async (el, dx) => {
    const r = el.getBoundingClientRect();
    const at = (x) => ({ bubbles: true, clientX: x, clientY: r.top + 10, pointerId: 1 });
    el.dispatchEvent(new PointerEvent("pointerdown", at(r.left + 150)));
    for (let k = 1; k <= 5; k++) el.dispatchEvent(new PointerEvent("pointermove", at(r.left + 150 + (dx * k) / 5)));
    el.dispatchEvent(new PointerEvent("pointerup", at(r.left + 150 + dx)));
    await wait(60);
  };
  const noSideScroll = () => document.documentElement.scrollWidth <= window.innerWidth && $$(".m .body").every((b) => b.scrollWidth <= b.clientWidth + 1);

  // Today (42.3).
  check("a second mount gives the first copy, adds none, and moves it into the new root", again === ui && $$(".m").length === 1 && extra.contains(ui.root));
  document.body.insertBefore(ui.root, extra);
  extra.remove();
  check("the phone is at phone width", window.innerWidth === 390 && window.innerHeight === 844, [window.innerWidth, window.innerHeight]);
  check("four tabs, Today first, with the hub on top", $$(".tabs button").map((b) => b.textContent.replace(/\d+/g, "").trim()).join() === "Today,Reviews,Agents,You" && $(".tabs [aria-current]").textContent.includes("Today") && $(".hub").textContent.includes("archlinux") && !!$(".hub .dot.on"));
  check("Today shows the sections and rows, and the job tabs", $$(".m h2").map((h) => h.textContent).join("|") === "Your PRs · 1|Reviews · 1" && $$("[data-job]").length === 2 && $(".card .t1").textContent.includes("Reply to dknopik"));
  check("touch targets are at least 44 px", $$(".tabs button, .m .card").every((b) => b.getBoundingClientRect().height >= 44));
  await swipe($('[data-swipe="today"]'), 140);
  check("swipe right is Done, with an undo bar", called("today_done").at(-1)?.[1].number === 10510 && called("today_done").at(-1)[1].on === true && !!$(".undo [data-undo]"));
  await tap(".undo [data-undo]");
  check("Undo takes Done back", called("today_done").at(-1)?.[1].on === false);
  await swipe($$('[data-swipe="today"]')[1], -140);
  check("swipe left is Later, with a sheet", !!$(".sheet") && $(".sheet h3").textContent.startsWith("Later"));
  await tap('[data-later="0"]');
  const lt = called("today_later").at(-1)?.[1];
  check("Later sets a time tomorrow at 9", lt?.number === 10566 && new Date(lt.until * 1000).getHours() === 9 && lt.until * 1000 > Date.now());
  await tap($$(".card[data-item]")[0]);
  check("a tap on an item with no review offers Start review", !!$(".sheet [data-startrev]"), $(".sheet")?.innerHTML);
  await tap(".sheet [data-close]");
  await tap($$(".card[data-item]")[1]);
  check("a tap on an item with a review offers Open review", !!$(".sheet [data-openrev]"), $(".sheet")?.innerHTML);
  await tap(".sheet [data-close]");

  // Reviews (42.4).
  await tap('[data-tab="reviews"]');
  check("Reviews lists the open PR reviews, not local ones", $$("[data-openrev]").length === 1 && $(".card .t2").textContent.includes("2 steps · 1 reviewed"));
  await tap("[data-openrev]");
  check("the overview shows the title, the counts, Claude's take and Go on at step 2", $(".m h1").textContent === "Fix redb backend bugs" && $(".take").textContent.includes("needs a change") && $(".take").textContent.includes("1 must") && $(".bar .gold").textContent === "Go on at step 2");
  await tap(".bar .gold");
  check("the step shows its place, title, text and question", $(".top").textContent.includes("Step 2 of 2") && $(".m h1").textContent.includes("keep errors") && $(".q").textContent.includes("Do we want to log it") && $$(".m h2").some((h) => h.textContent === "For you to decide"));
  check("your place goes to the hub, so the desktop opens there", called("review_step_seen").at(-1)?.[1].step === "s2");
  await wait(60);
  check("the narrow diff has added and removed lines with line numbers", $$(".row.add").length === 2 && $$(".row.del").length === 1 && $(".row.add .ln").textContent === "7");
  check("context far from a change folds", !!$(".fold") && $(".fold").textContent.includes("Show 3 more"));
  check("long lines wrap, with no sideways scroll", noSideScroll(), [document.documentElement.scrollWidth, window.innerWidth]);
  await tap('[data-line="0|new|7"]');
  check("a + selects a line and shows its bar", $$(".row.sel").length === 1 && $(".selbar").textContent.includes("Line 7"));
  await tap('[data-line="0|new|8"]');
  check("a second + grows the selection, and the other side dims", $$(".row.sel").length === 2 && $(".selbar").textContent.includes("Lines 7–8") && $$(".row.del.dim").length === 1);
  await tap(".selbar [data-comment]");
  check("Comment opens the sheet for the range", $(".sheet h3").textContent === "Comment on lines 7–8");
  $(".sheet textarea").value = "Name the slot.";
  await tap("[data-savecomment]");
  const c = called("review_comment").at(-1)?.[1];
  check("Save draft sends one multi-line comment", c?.comment.line === 8 && c.comment.start_line === 7 && c.comment.side === "new" && c.comment.text === "Name the slot." && c.comment.line_text === "    .collect()" && !$(".sheet"));
  // Claude's notes as cards.
  check("a note shows as a card with its kind and line", $(".note .sev-must") && $(".note").textContent.includes("a.rs:7"));
  await swipe($('[data-swipe="note"]'), 140);
  const nc = called("review_comment").at(-1)?.[1];
  check("swipe right on a note adds it as an agent draft", nc?.comment.agent === true && nc.comment.line === 7 && nc.comment.text === note.text);
  await swipe($('[data-swipe="note"]'), -140);
  check("swipe left on a note dismisses it, with undo", called("review_dismiss").at(-1)?.[1].on === true && called("review_dismiss").at(-1)[1].note === "s2|must|src/a.rs:7|" + note.text && !!$(".undo [data-undo]"));
  // Ask.
  await tap(".bar [data-ask]");
  $(".sheet textarea").value = "Why map_err?";
  await tap("[data-sendask]");
  check("Ask sends the question for the step", called("review_ask").at(-1)?.[1].text === "Why map_err?" && called("review_ask").at(-1)[1].step === "s2");
  R = { ...R, threads: [{ id: "t1", step: "s2", busy: false, messages: [{ id: 1, me: true, text: "Why map_err?" }, { id: 2, me: false, text: "It keeps the **error**." }] }] };
  emit("review", R);
  await wait(40);
  check("the answer shows in the sheet when it comes", $(".answer")?.textContent === "It keeps the error." && !!$(".answer b"));
  await tap(".sheet [data-close]");
  // Reviewed.
  await tap("[data-reviewed]");
  check("Reviewed marks the step and goes on", called("review_mark").at(-1)?.[1].step === "s2" && called("review_mark").at(-1)[1].checked === true);
  R = { ...R, steps: [{ id: "s1", checked: true }, { id: "s2", checked: true }] };
  emit("review", R);
  await wait(40);
  check("after the last step, the overview offers Finish review", !!$(".bar [data-finish]") && $(".bar .gold").textContent === "Finish review");
  await tap(".bar [data-finish]");
  await wait(40);
  await tap('[data-event="APPROVE"]');
  await tap("[data-send]");
  const p = called("review_post").at(-1)?.[1];
  check("Finish sends the verdict with the plan token", p?.event === "APPROVE" && p.token === "tok");

  // Agents (42.5).
  await tap('[data-go="reviews"]');
  await tap('[data-tab="agents"]');
  check("Agents puts the permit first, with Allow and Deny", $$(".m h2")[0].textContent.startsWith("Needs you") && $(".permit pre").textContent === "Bash: cargo test -p store" && $$(".card").length === 1);
  check("the tab bar counts what needs you", $('[data-tab="agents"] .badge')?.textContent === "1");
  await tap('[data-permit][data-allow="1"]');
  check("Allow answers the permit", called("permit_answer").at(-1)?.[1].permit === "p61-1" && called("permit_answer").at(-1)[1].allow === true);
  // You (42.6).
  await tap('[data-tab="you"]');
  check("You shows the hub", $(".body").textContent.includes("archlinux") && $(".body").textContent.includes("Linked"));
  ui.setHub({ linked: false, seen: "2 min ago" });
  check("a lost link shows grey with last seen", !$(".hub .dot.on") && $(".hub").textContent.includes("last seen 2 min ago"));
  check("no text from the hub becomes markup", (() => { today.sections[0].items[0].line = "<img src=x onerror=window.__x=1>"; return true; })());
  await tap('[data-tab="today"]');
  await wait(40);
  check("no script from the hub runs", window.__x === undefined && !$(".m img"));
  check("no page error", !window.__errors?.length, window.__errors);

  const pre = document.createElement("pre");
  pre.id = "__results";
  pre.textContent = JSON.stringify(results);
  (window.parent !== window ? window.parent.document.body : document.body).appendChild(pre);
})();
window.__errors = [];
window.addEventListener("error", (e) => window.__errors.push(String(e.message)));
