// Handover (SPEC 38): an agent on the server continues on this computer, with its
// commits, the work it did not commit, and its conversation.
//
// The dialog runs in two parts. First it prepares: the server takes a snapshot
// (handover_out), and this computer fetches it and picks the folder and branch
// (handover_check, a dry run). You see what comes along and where it lands. Then
// Continue here makes the worktree (handover_in), stops the agent on the server if
// you chose that, and switches the app to this computer. There the agent opens.
(function () {
  const { $, esc, invoke, toast, LINK, paneById, paneName, home } = window.SB;
  // `gen` drops the answers of a dialog that was closed or opened again.
  const H = { id: null, plan: null, target: null, phase: "idle", step: null, err: null, stop: true, gen: 0, showFiles: false };

  const can = (p) => LINK.remote && p?.kind === "claude" && p.lamp !== "ended";
  const short = (c) => (c || "").slice(0, 7);
  const kb = (s) => (s.length < 1 << 20 ? `${Math.max(1, Math.round(s.length / 1024))} KB` : `${(s.length / (1 << 20)).toFixed(1)} MB`);
  const plural = (n, w) => `${n} ${w}${n === 1 ? "" : "s"}`;
  const isOpen = () => !$("#handover").hidden;

  // The steps of each part, in order. A step is done, now, wait, or fail.
  const PREP = [["snap", "Snapshot of the work on the server"], ["fetch", "Fetch over ssh, and pick the folder here"]];
  const GO = () => [["build", "Make the worktree and put the changes back"], ...(H.stop ? [["stop", "Stop the agent on the server"]] : []), ["switch", "Switch to this computer"]];
  function steps(list) {
    const at = list.findIndex(([k]) => k === H.step);
    return `<ol class="hsteps">${list.map(([k, label], i) => {
      const st = H.phase === "error" && i === at ? "fail" : i < at || (H.phase === "ready" && list === PREP) ? "done" : i === at ? "now" : "wait";
      const mark = st === "done" ? "✓" : st === "fail" ? "✕" : st === "now" ? '<i class="busy"></i>' : "";
      return `<li class="hs-${st}"><span class="hmark">${mark}</span>${esc(label)}</li>`;
    }).join("")}</ol>`;
  }

  function changesHtml(plan) {
    const n = plan.changes.length;
    if (!n) return `<dd>None</dd>`;
    const list = H.showFiles ? `<ul class="hfiles">${plan.changes.slice(0, 40).map((c) => {
      const [st, ...rest] = c.split("\t");
      return `<li><span class="hst hst-${esc(st[0])}">${esc(st[0])}</span><code>${esc(rest.join(" → "))}</code></li>`;
    }).join("")}${n > 40 ? `<li class="hmore">and ${n - 40} more</li>` : ""}</ul>` : "";
    return `<dd><button type="button" class="hlink" data-h="files" aria-expanded="${H.showFiles}">${plural(n, "file")}</button>${list}</dd>`;
  }

  function draw() {
    const p = paneById(H.id);
    const box = $("#handover .hbody");
    const plan = H.plan, t = H.target;
    const name = p ? paneName(p) : plan?.title || "the agent";
    $("#htitle").textContent = `Continue ${name} on this computer`;
    $("#hsub").textContent = [p?.repo, plan?.branch || p?.branch, LINK.host ? `on ${LINK.host}` : ""].filter(Boolean).join(" · ");
    let html = "";
    if (H.phase === "prep" || (H.phase === "error" && !t)) html += steps(PREP);
    if (H.phase === "going" || (H.phase === "error" && t)) html += steps(GO());
    if (H.phase === "error") html += `<div class="herr" role="alert">${esc(H.err)}</div>`;
    if (H.phase === "ready" && plan && t) {
      const convo = plan.session_data
        ? `<dd>The whole conversation, ${kb(plan.session_data)}. The agent picks up where it stopped.</dd>`
        : `<dd class="hwarn">No conversation file on the server. A new agent starts in the folder.</dd>`;
      html += `<section class="hsec"><h4>What comes along</h4><dl class="hfacts">
          <dt>Commits</dt><dd><code>${esc(plan.branch || "detached")}</code> at <code>${esc(short(plan.head))}</code></dd>
          <dt>Not committed</dt>${changesHtml(plan)}
          <dt>Conversation</dt>${convo}
        </dl></section>
        <section class="hsec"><h4>Where it lands</h4><dl class="hfacts">
          <dt>Clone</dt><dd><code>${esc(home(t.repo))}</code></dd>
          <dt>Folder</dt><dd><code>${esc(home(t.tree))}</code></dd>
          <dt>Branch</dt><dd><code>${esc(t.branch)}</code>${t.note ? `<span class="hwarn"> · ${esc(t.note)}</span>` : ""}</dd>
        </dl></section>
        <label class="hcheck"><input type="checkbox" id="hstop"${H.stop ? " checked" : ""}><span><b>Stop the agent on the server</b><small>Two agents on one piece of work drift apart. The conversation stays on the server too.</small></span></label>
        <p class="hnote">${LINK.both ? "Then the window shows this computer, where the agent opens. The other agents on the server keep running." : "The app cannot switch by itself while SB_REMOTE is set. After this, start the app without SB_REMOTE: the agent opens then."}</p>`;
    }
    box.innerHTML = html;
    const go = $("#hgo");
    go.hidden = H.phase === "going";
    go.disabled = H.phase !== "ready" && H.phase !== "error";
    go.textContent = H.phase === "error" ? "Try again" : "Continue here";
    $("#hcancel").hidden = H.phase === "going";
    $("#hcancel").textContent = H.phase === "error" ? "Close" : "Cancel";
  }

  async function open(id) {
    const p = paneById(id || window.SB.S.live || window.SB.S.focus);
    if (!p) return;
    if (!LINK.remote) return toast("A handover brings an agent from the server. The window is on this computer now.");
    if (!can(p)) return toast("Only a running agent can continue on this computer.");
    const gen = ++H.gen;
    Object.assign(H, { id: p.id, plan: null, target: null, phase: "prep", step: "snap", err: null, showFiles: false });
    $("#handover").hidden = false;
    draw();
    $("#hcancel").focus();
    try {
      const plan = await invoke("handover_out", { id: p.id });
      if (gen !== H.gen) return;
      H.plan = plan;
      H.step = "fetch";
      draw();
      const target = await invoke("handover_check", { plan });
      if (gen !== H.gen) return;
      H.target = target;
      H.phase = "ready";
      draw();
      $("#hgo").focus();
    } catch (e) {
      if (gen !== H.gen) return;
      H.phase = "error";
      H.err = String(e);
      draw();
    }
  }

  async function go() {
    if (H.phase === "error") return H.target && H.step !== "build" ? finish() : open(H.id);
    if (H.phase !== "ready") return;
    H.phase = "going";
    H.step = "build";
    draw();
    try {
      await invoke("handover_in", { plan: H.plan });
    } catch (e) {
      H.phase = "error";
      H.err = String(e);
      return draw();
    }
    finish();
  }

  // After the worktree: stop the agent there if you chose that, then switch.
  async function finish() {
    H.phase = "going";
    try {
      if (H.stop && paneById(H.id)) {
        H.step = "stop";
        draw();
        await invoke("pane_close", { id: H.id });
      }
      H.step = "switch";
      draw();
      if (!LINK.both) {
        H.phase = "idle";
        close();
        return toast(`The work is in ${home(H.target.tree)}. Start the app without SB_REMOTE: the agent opens then.`);
      }
      // No restart (SPEC 29.13): the window shows this computer, and the agent opens.
      H.phase = "idle";
      close();
      await window.SB.showMachine(false);
      await arrive();
    } catch (e) {
      H.phase = "error";
      H.err = String(e);
      draw();
    }
  }

  function close() {
    if (H.phase === "going") return;
    H.gen++;
    H.phase = "idle";
    $("#handover").hidden = true;
    window.SB.term?.focus();
  }

  $("#handover").addEventListener("click", (e) => {
    if (e.target === $("#handover")) return close();
    if (e.target.closest("#hcancel")) return close();
    if (e.target.closest("#hgo")) return go();
    if (e.target.closest('[data-h="files"]')) { H.showFiles = !H.showFiles; draw(); }
  });
  $("#handover").addEventListener("change", (e) => { if (e.target.id === "hstop") H.stop = e.target.checked; });
  $("#handover").addEventListener("keydown", (e) => {
    e.stopPropagation();
    if (e.key === "Escape") { e.preventDefault(); close(); }
    else if (e.key === "Enter" && !e.target.closest("button, input") && H.phase === "ready") { e.preventDefault(); go(); }
  });

  // On this computer, at the start: the agent of a handover opens, and a note says
  // what came along.
  async function arrive() {
    if (LINK.remote) return;
    let a;
    try {
      a = await invoke("handover_take");
    } catch (e) {
      return toast(`The handover did not open: ${e}`);
    }
    if (!a) return;
    await window.SB.refreshPanes();
    window.SB.expand(a.pane.id);
    const what = [`branch ${a.branch}`, a.changes ? plural(a.changes, "changed file") : "", a.resumed ? "the conversation" : ""].filter(Boolean);
    toast(`Continued from ${a.from}: ${what.join(", ")}.`);
  }

  window.SBHandover = { open, can, arrive, isOpen };
})();
