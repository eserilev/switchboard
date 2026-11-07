// The Work environment tab (SPEC 30): one repo, with nvim on the left, Claude on
// the top right and a terminal under it. It is the live pane with its parts, moved
// into this tab and turned around. app.js calls in here through `SBWork`.
(() => {
  const { S, invoke, call, esc, setHTML, paneById, toast, WORK_TITLE, workOf } = window.SB;
  const $ = (s) => document.querySelector(s);

  // `repo`: the path you picked, kept for the next start. `pane`: its Claude pane.
  const W = { repo: null, pane: null, opening: false };
  try { W.repo = localStorage.getItem("sb.work.repo"); } catch (_) { /* no storage: start empty */ }

  // Only the tab's own agent: an agent of the board stays on the board.
  const agentIn = (path) => S.panes.find((p) => workOf(p) && p.tree === path && p.lamp !== "ended");
  // An ended agent of the tab: it shows here with Resume and Close, as a tile does on
  // the board, so it never stays out of reach.
  const endedIn = (path) => S.panes.find((p) => workOf(p) && p.tree === path && p.lamp === "ended");
  const shows = () => S.tab === "work" && S.termHost === "work" && !!S.live && S.live === W.pane;

  // The bar is built once and then changed in place: a pane event comes about once
  // a second, and a new select or button under the mouse drops the click.
  function drawBar() {
    const bar = $("#wbar");
    if (!$("#wrepo")) {
      bar.innerHTML = '<label class="wpick">Repo <select id="wrepo" aria-label="Repo"></select></label>' +
        '<button class="btn small primary" id="wgo" type="button"></button><span class="wnote"></span>';
    }
    const sel = $("#wrepo");
    const opts = S.repos.map((r) => `<option value="${esc(r.path)}"${r.path === W.repo ? " selected" : ""}>${esc(r.name)}${r.branch ? ` · ${esc(r.branch)}` : ""}</option>`).join("");
    // A repo that is no longer in the list (a scan folder went) still shows.
    const gone = W.repo && !S.repos.some((r) => r.path === W.repo) ? `<option value="${esc(W.repo)}" selected>${esc(W.repo)}</option>` : "";
    // A select that has the focus may be open: its list stays until it lets go.
    if (document.activeElement !== sel) {
      setHTML(sel, `<option value="">Pick a repo…</option>${gone}${opts}`);
      if (sel.value !== (W.repo || "")) sel.value = W.repo || "";
    }
    const open = shows();
    const go = $("#wgo");
    const label = W.repo && agentIn(W.repo) ? "Open" : "Start Claude";
    if (go.hidden !== open) go.hidden = open;
    if (go.textContent !== label) go.textContent = label;
    const off = !W.repo || W.opening;
    if (go.disabled !== off) go.disabled = off;
    const note = open ? "nvim, Claude and a terminal in " + W.repo : W.repo ? "Claude, nvim and a terminal open in the repo's main checkout." : "Pick a repo to work in.";
    const n = bar.querySelector(".wnote");
    if (n.textContent !== note) n.textContent = note;
  }

  // Empties the tab's box, but never the live pane: after a collapse it stays here,
  // hidden, until another tab takes it.
  function clearHost() {
    for (const c of [...$("#whost").children]) if (c.id !== "live") c.remove();
  }

  // The box stays and changes in place, as the bar does.
  function drawEmpty() {
    if (shows()) return;
    let box = $("#whost > .wempty");
    if (!box) {
      clearHost();
      box = document.createElement("div");
      box.className = "wempty";
      $("#whost").appendChild(box);
    }
    const end = !W.opening && W.repo && !agentIn(W.repo) ? endedIn(W.repo) : null;
    if (end) seen(end);
    setHTML(box, W.opening ? "Opening…"
      : end ? `<div class="wended">Claude ended in this repo.${end.session ? ` <button class="btn small primary" type="button" data-wresume="${esc(end.id)}">Resume</button>` : ""} <button class="btn small" type="button" data-wclose="${esc(end.id)}">Close</button></div>`
      : W.repo ? "Press Start Claude, or Open for the agent that runs there." : "Pick a repo above. Its Claude agent, nvim and a terminal open here.");
  }

  // An ended agent that shows here is seen: Leader j does not come back to it.
  function seen(p) {
    if (!p.unseen) return;
    p.unseen = false;
    invoke("pane_seen", { id: p.id }).catch(() => {});
  }

  // Shows the repo's Claude pane in this tab, with nvim and the terminal. Starts a
  // Claude pane in the main checkout when none runs there. Each await can end with
  // you in another tab or on another repo: then the open stops. A part shown after
  // you left would expand the agent again and pull you back here.
  async function open() {
    if (!W.repo) return;
    // An open asked for while one runs (a resync, a pick) runs again after it.
    if (W.opening) { W.again = true; return; }
    const repo = W.repo;
    const still = () => S.tab === "work" && W.repo === repo;
    W.opening = true;
    drawBar();
    drawEmpty();
    try {
      let a = agentIn(repo);
      if (!a) {
        const v = await invoke("pane_open", { req: { repo, place: "main", branch: null, tree: null, run: "claude", title: WORK_TITLE, task: null, first: null } });
        await window.SB.refreshPanes();
        a = paneById(v.id);
      }
      if (!a) throw "The Claude pane did not start.";
      if (!still()) return;
      W.pane = a.id;
      clearHost();
      await window.SB.expand(a.id, $("#whost"));
      if (!still() || !shows()) return;
      // nvim first: it is the left half. A missing nvim leaves Claude and the terminal.
      if (S.settings?.nvim) await window.SBParts.show("nvim", a.id);
      if (!still() || !shows()) return;
      await window.SBParts.show("term", a.id);
      if (!still() || !shows()) return;
      // Claude has the keys first.
      window.SB.term.focus();
    } catch (e) {
      toast(e);
    } finally {
      W.opening = false;
      const again = W.again;
      W.again = false;
      if (S.tab === "work") {
        // You picked another repo while this one opened, or an open was asked for
        // meanwhile: it runs now.
        if (W.repo !== repo || again) draw();
        else { drawBar(); drawEmpty(); }
      }
    }
  }

  // Resume and Close of an ended agent, as on its tile.
  async function resume(id) {
    try {
      await call("pane_resume", { id });
      await window.SB.refreshPanes();
    } catch (_) {
      return;
    }
    if (S.tab === "work" && paneById(id)?.tree === W.repo) draw();
  }
  async function close(id) {
    try {
      await call("pane_close", { id });
      await window.SB.refreshPanes();
    } catch (_) {
      return;
    }
    if (S.tab === "work") draw();
  }
  $("#whost").addEventListener("click", (e) => {
    const b = e.target.closest("button");
    if (b?.dataset.wresume) resume(b.dataset.wresume);
    else if (b?.dataset.wclose) close(b.dataset.wclose);
  });

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
    if (p.lamp === "ended") seen(p);
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
    panes: () => { if (S.tab === "work" && !W.opening) { drawBar(); drawEmpty(); } },
  };
})();
