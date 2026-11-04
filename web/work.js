// The Work environment tab (SPEC 30): one repo, with nvim on the left, Claude on
// the top right and a terminal under it. It is the live pane with its parts, moved
// into this tab and turned around. app.js calls in here through `SBWork`.
(() => {
  const { S, invoke, esc, paneById, toast, WORK_TITLE, workOf } = window.SB;
  const $ = (s) => document.querySelector(s);

  // `repo`: the path you picked, kept for the next start. `pane`: its Claude pane.
  const W = { repo: null, pane: null, opening: false };
  try { W.repo = localStorage.getItem("sb.work.repo"); } catch (_) { /* no storage: start empty */ }

  // Only the tab's own agent: an agent of the board stays on the board.
  const agentIn = (path) => S.panes.find((p) => workOf(p) && p.tree === path && p.lamp !== "ended");
  const shows = () => S.tab === "work" && S.termHost === "work" && !!S.live && S.live === W.pane;

  function drawBar() {
    const opts = S.repos.map((r) => `<option value="${esc(r.path)}"${r.path === W.repo ? " selected" : ""}>${esc(r.name)}${r.branch ? ` · ${esc(r.branch)}` : ""}</option>`).join("");
    // A repo that is no longer in the list (a scan folder went) still shows.
    const gone = W.repo && !S.repos.some((r) => r.path === W.repo) ? `<option value="${esc(W.repo)}" selected>${esc(W.repo)}</option>` : "";
    const open = shows();
    const label = W.repo && agentIn(W.repo) ? "Open" : "Start Claude";
    $("#wbar").innerHTML = `<label class="wpick">Repo <select id="wrepo" aria-label="Repo"><option value="">Pick a repo…</option>${gone}${opts}</select></label>` +
      (open ? "" : `<button class="btn small primary" id="wgo" type="button"${W.repo && !W.opening ? "" : " disabled"}>${label}</button>`) +
      `<span class="wnote">${open ? "nvim, Claude and a terminal in " + esc(W.repo) : W.repo ? "Claude, nvim and a terminal open in the repo's main checkout." : "Pick a repo to work in."}</span>`;
  }

  // Empties the tab's box, but never the live pane: after a collapse it stays here,
  // hidden, until another tab takes it.
  function clearHost() {
    for (const c of [...$("#whost").children]) if (c.id !== "live") c.remove();
  }

  function drawEmpty() {
    if (shows()) return;
    clearHost();
    $("#whost").insertAdjacentHTML("beforeend", `<div class="wempty">${W.opening ? "Opening…" : W.repo ? "Press Start Claude, or Open for the agent that runs there." : "Pick a repo above. Its Claude agent, nvim and a terminal open here."}</div>`);
  }

  // Shows the repo's Claude pane in this tab, with nvim and the terminal. Starts a
  // Claude pane in the main checkout when none runs there.
  async function open() {
    if (!W.repo || W.opening) return;
    W.opening = true;
    drawBar();
    drawEmpty();
    try {
      let a = agentIn(W.repo);
      if (!a) {
        const v = await invoke("pane_open", { req: { repo: W.repo, place: "main", branch: null, tree: null, run: "claude", title: WORK_TITLE, task: null, first: null } });
        await window.SB.refreshPanes();
        a = paneById(v.id);
      }
      if (!a) throw "The Claude pane did not start.";
      W.pane = a.id;
      if (S.tab !== "work") return;
      clearHost();
      await window.SB.expand(a.id, $("#whost"));
      if (!shows()) return;
      // nvim first: it is the left half. A missing nvim leaves Claude and the terminal.
      if (S.settings?.nvim) await window.SBParts.show("nvim", a.id);
      await window.SBParts.show("term", a.id);
      // Claude has the keys first.
      window.SB.term.focus();
    } catch (e) {
      toast(e);
    } finally {
      W.opening = false;
      if (S.tab === "work") { drawBar(); drawEmpty(); }
    }
  }

  // The tab shows: the bar, and the repo's agent when one runs there.
  function draw() {
    drawBar();
    if (shows()) return;
    if (W.repo && agentIn(W.repo)) return open();
    drawEmpty();
  }

  $("#wbar").addEventListener("change", (e) => {
    if (e.target.id !== "wrepo") return;
    W.repo = e.target.value || null;
    W.pane = null;
    try { W.repo ? localStorage.setItem("sb.work.repo", W.repo) : localStorage.removeItem("sb.work.repo"); } catch (_) { /* a convenience only */ }
    // Another repo: its own agent, not the one that shows now.
    if (S.termHost === "work" && S.live) window.SB.collapse(true);
    clearHost();
    if (W.repo && agentIn(W.repo)) open();
    else draw();
  });
  $("#wbar").addEventListener("click", (e) => { if (e.target.closest("#wgo")) open(); });

  // Shows a work agent from elsewhere (a notice, the next pane that waits): its repo
  // in this tab.
  function go(p) {
    if (W.repo !== p.tree) {
      W.repo = p.tree;
      try { localStorage.setItem("sb.work.repo", W.repo); } catch (_) { /* a convenience only */ }
    }
    W.pane = p.id;
    if (S.tab === "work") draw();
    else window.SB.setTab("work");
  }

  window.SBWork = {
    W,
    draw,
    go,
    // The live pane left this tab (a collapse, or its pane ended).
    collapsed: () => { if (S.tab === "work") { drawBar(); drawEmpty(); } },
    // The repo list or the panes changed: the bar names the right button.
    panes: () => { if (S.tab === "work" && !W.opening) drawBar(); },
  };
})();
