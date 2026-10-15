// For screenshots: shows a view by the URL hash: #board, #live, #review, #tasks,
// #tasks-panel or #tasks-list.
(async () => {
  await new Promise((r) => setTimeout(r, 400));
  const v = location.hash.slice(1);
  if (v === "live") window.SB.expand("p1");
  if (v === "review") {
    window.SB.setTab("r1");
    await new Promise((r) => setTimeout(r, 200));
    document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "j", bubbles: true }));
  }
  if (v.startsWith("tasks")) {
    window.SB.setTab("tasks");
    await new Promise((r) => setTimeout(r, 200));
    if (v === "tasks-list") document.querySelector('#tview [data-v="list"]').click();
    if (v === "tasks-panel") document.querySelector('[data-task="t1"]').click();
  }
  if (v === "launcher") document.querySelector("#newpane").click();
})();
