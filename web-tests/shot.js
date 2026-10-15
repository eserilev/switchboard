// For screenshots: shows a view by the URL hash: #board, #live, #review, #tasks,
// #tasks-panel, #tasks-list, #tasks-p3, #tasks-p3-list, #tasks-p3-panel or #launcher-task.
(async () => {
  await new Promise((r) => setTimeout(r, 400));
  const v = location.hash.slice(1);
  if (v === "live") window.SB.expand("p1");
  if (v === "review") {
    window.SB.setTab("r1");
    await new Promise((r) => setTimeout(r, 200));
    document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "j", bubbles: true }));
  }
  if (v.startsWith("tasks-p3")) {
    // Phase 3: points, tags, links, a merged PR, a planned sprint and a burndown.
    const d = window.__data.tasks;
    const at = (n, h) => { const x = new Date(); x.setDate(x.getDate() + n); x.setHours(h, 0, 0, 0); return x.getTime() / 1000; };
    Object.assign(d.tasks.find((t) => t.id === "t1"), { points: 5, tags: ["infra", "gloas"], links: ["https://github.com/sigp/lighthouse/pull/10071", "https://example.com/design"] });
    Object.assign(d.tasks.find((t) => t.id === "t2"), { points: 3, tags: ["tests"], status: "done", done: at(-1, 15), links: ["https://github.com/sigp/lighthouse/pull/10090"] });
    Object.assign(d.tasks.find((t) => t.id === "t3"), { points: 8 });
    d.sprints.push({ id: "s2", name: "Sprint 2", goal: "Fork tests", start: dayOff(11), end: dayOff(24), state: "planned" });
    d.tasks.push(tk({ id: "t6", title: "Plan the fork tests", rank: 1024, sprint: "s2", points: 2, tags: ["tests"] }));
    d.archive = [tk({ id: "t9", title: "Drop the old IL cache", rank: 1024, deleted: at(-1, 10) })];
    d.burn = ["t1", "t2", "t3"].map((task) => ({ task, at: at(-3, 10), status: "todo" })).concat([{ task: "t2", at: at(-1, 15), status: "done" }]);
    d.merged = ["https://github.com/sigp/lighthouse/pull/10090", "https://github.com/sigp/lighthouse/pull/10071"];
    d.tasks.find((t) => t.id === "t1").status = "review";
    window.__emit("tasks", structuredClone(d));
    window.SB.setTab("tasks");
    await new Promise((r) => setTimeout(r, 200));
    if (v === "tasks-p3-list") document.querySelector('#tview [data-v="list"]').click();
    if (v === "tasks-p3-panel") document.querySelector('[data-task="t1"]').click();
  } else if (v.startsWith("tasks")) {
    window.SB.setTab("tasks");
    await new Promise((r) => setTimeout(r, 200));
    if (v === "tasks-list") document.querySelector('#tview [data-v="list"]').click();
    if (v === "tasks-panel") document.querySelector('[data-task="t1"]').click();
  }
  if (v === "launcher-task") {
    window.SB.setTab("tasks");
    await new Promise((r) => setTimeout(r, 200));
    document.querySelector('[data-task="t2"]').click();
    await new Promise((r) => setTimeout(r, 50));
    document.querySelector('[data-act="start"]').click();
  }
  if (v === "launcher") document.querySelector("#newpane").click();
})();
