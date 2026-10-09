// Review tabs (SPEC 11): steps, guide + diff, threads and drafts.

(() => {
  const { S, call, invoke, esc, toast, $ } = window.SB;
  const R = new Map(); // review id -> { cur, anchor, active, pending: Map(thread -> text), diff: Map(step -> view), busy: Set }
  const local = (id) => {
    if (!R.has(id)) R.set(id, { cur: 0, anchor: null, active: null, pending: new Map(), diff: new Map(), busy: new Set() });
    return R.get(id);
  };
  const view = () => $("#view-review");
  const steps = (r) => r.guide?.steps || [];
  const stepState = (r, sid) => r.steps.find((s) => s.id === sid) || { checked: false, stale: false };
  const short = (h) => (h || "").slice(0, 9);

  function draw(id) {
    const r = S.reviews.get(id);
    if (!r) return;
    const l = local(id);
    view().innerHTML = `
      <div class="rhead" id="rhead"></div>
      <div class="rcols">
        <div class="col"><ol class="steps" id="rsteps"></ol></div>
        <div class="col center" id="rcenter"><div class="guide" id="rguide"></div><div id="rdiff"></div></div>
        <div class="col">
          <div class="threads" id="rthreads"></div>
          <div class="anchorchip" id="ranchor"></div>
          <form class="ask" id="rask"><label for="rq" hidden>Question</label><textarea id="rq"></textarea><button class="btn primary" type="submit">Ask</button></form>
          <div class="drafts" id="rdrafts"></div>
        </div>
      </div>`;
    $("#rask").addEventListener("submit", (e) => { e.preventDefault(); ask(id); });
    $("#rq").addEventListener("keydown", (e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); ask(id); } });
    update(r, true);
    if (steps(r).length) loadDiff(id, l.cur);
  }

  function update(r, fresh) {
    if (S.tab !== r.id) return;
    if (!fresh && !$("#rsteps")) return draw(r.id);
    const l = local(r.id);
    l.cur = Math.min(l.cur, Math.max(0, steps(r).length - 1));
    drawHead(r);
    drawSteps(r);
    drawGuide(r);
    drawThreads(r);
    drawDrafts(r);
    if (!fresh && steps(r).length && !l.diff.has(steps(r)[l.cur].id)) loadDiff(r.id, l.cur);
    else drawDiff(r);
  }

  function drawHead(r) {
    const status = { fetching: "Fetching the PR", writing: "Writing the guide", updating: "Updating the guide", error: "Error", ready: "" }[r.status] ?? r.status;
    const newHead = r.new_head ? `<span class="newhead">New commits: ${esc(short(r.head))} → ${esc(short(r.new_head))}<button class="btn small" data-act="update">Update guide</button></span>` : "";
    const retry = r.status === "error" ? `<button class="btn small" data-act="retry">Retry</button>` : "";
    $("#rhead").innerHTML = `<div><div class="t">${esc(r.title)}</div><div class="m">${esc(r.repo)} #${esc(r.number)} · ${esc(short(r.head))}${r.tree ? " · " + esc(r.tree) : ""}</div></div>
      ${status ? `<span class="status${r.status === "error" ? " error" : ""}">${esc(status)}</span>` : ""}${newHead}
      <div class="right">${retry}<button class="btn small" data-act="close">Close review</button></div>
      ${r.error ? `<div class="rerror" style="flex-basis:100%;padding:0">${esc(r.error)}</div>` : ""}`;
    $("#rhead").onclick = async (e) => {
      const act = e.target.closest("[data-act]")?.dataset.act;
      if (act === "update") { local(r.id).diff.clear(); call("review_update", { id: r.id }); }
      if (act === "retry") call("review_retry", { id: r.id });
      if (act === "close") call("review_close", { id: r.id });
    };
  }

  function drawSteps(r) {
    const l = local(r.id);
    $("#rsteps").innerHTML = steps(r).map((s, i) => {
      const st = stepState(r, s.id);
      const n = r.threads.filter((t) => t.step === s.id).length;
      const loc = s.file ? `${s.file}${s.lines?.length ? ":" + s.lines.join("-") : ""}` : "";
      const badges = [n ? `<span class="badge th">${n} thread${n > 1 ? "s" : ""}</span>` : "", st.stale ? `<span class="badge stale">stale</span>` : ""].join("");
      return `<li class="step${st.checked ? " checked" : ""}" tabindex="0" data-i="${i}" ${i === l.cur ? 'aria-current="step"' : ""}>
        <span class="n">${st.checked && i !== l.cur ? "✓" : i + 1}</span>
        <div><div class="st">${esc(s.title)}</div>${loc ? `<div class="loc" title="${esc(loc)}">${esc(loc)}</div>` : ""}${badges ? `<div class="badges">${badges}</div>` : ""}</div></li>`;
    }).join("") || `<li class="empty-review">${r.status === "error" ? "" : "The guide shows here when the agent is done."}</li>`;
    $("#rsteps").onclick = (e) => { const s = e.target.closest(".step"); if (s) go(r.id, +s.dataset.i); };
  }

  function drawGuide(r) {
    const l = local(r.id);
    const s = steps(r)[l.cur];
    if (!s) { $("#rguide").innerHTML = ""; return; }
    const st = stepState(r, s.id);
    const pins = r.pins.filter((p) => p.step === s.id);
    const ctx = [...(s.context || [])];
    $("#rguide").innerHTML = `
      <div class="eyebrow">Step ${l.cur + 1} of ${steps(r).length}</div>
      <h4>${esc(s.title)}</h4>
      ${s.what ? `<p style="margin:0">${esc(s.what)}</p>` : ""}
      ${s.check?.length ? `<div class="check"><ul>${s.check.map((c) => `<li>${esc(c)}</li>`).join("")}</ul></div>` : ""}
      ${pins.map((p) => `<div class="pinned" data-pin="${p.id}"><span contenteditable="true">${esc(p.text)}</span><button class="tool x" data-unpin="${p.id}" aria-label="Remove">×</button></div>`).join("")}
      ${ctx.length ? `<div class="ctx">${ctx.map((c) => `<span class="chip">${esc(c)}</span>`).join("")}</div>` : ""}
      <div class="gactions">
        <button class="btn" data-act="mark">${st.checked ? "Unmark" : "Mark reviewed"}</button>
        ${l.cur < steps(r).length - 1 ? `<button class="btn primary" data-act="next">Next step</button>` : ""}
      </div>`;
    $("#rguide").onclick = (e) => {
      const b = e.target.closest("button");
      if (!b) return;
      if (b.dataset.act === "mark") call("review_mark", { id: r.id, step: s.id, checked: !st.checked });
      if (b.dataset.act === "next") next(r.id);
      if (b.dataset.unpin) call("review_pin_edit", { id: r.id, pin: +b.dataset.unpin, text: null });
    };
    $("#rguide").querySelectorAll("[data-pin] span").forEach((el) => {
      el.addEventListener("blur", () => call("review_pin_edit", { id: r.id, pin: +el.parentElement.dataset.pin, text: el.textContent.trim() || null }));
      el.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); el.blur(); } e.stopPropagation(); });
    });
  }

  async function loadDiff(id, i) {
    const r = S.reviews.get(id);
    const s = steps(r)[i];
    if (!s) return;
    const l = local(id);
    if (!l.diff.has(s.id)) {
      try {
        l.diff.set(s.id, await invoke("review_diff", { id, step: s.id }));
      } catch (e) {
        l.diff.set(s.id, { path: s.file, rows: [], error: String(e) });
      }
    }
    if (S.tab === id && local(id).cur === i) drawDiff(S.reviews.get(id));
  }

  function drawDiff(r) {
    const l = local(r.id);
    const s = steps(r)[l.cur];
    const d = s && l.diff.get(s.id);
    if (!s || !d || !d.path) { $("#rdiff").innerHTML = d?.error ? `<div class="rerror">${esc(d.error)}</div>` : ""; return; }
    const [from, to] = [s.lines?.[0] ?? -1, s.lines?.[s.lines.length - 1] ?? -1];
    const side = s.side || "new";
    const anchored = new Set(r.threads.filter((t) => t.step === s.id && t.line && t.path === d.path).map((t) => `${t.side || "new"}:${t.line}`));
    const rows = d.rows.map((row) => {
      if (row.kind === "@") return `<tr class="hunk"><td class="ln"></td><td class="src">${esc(row.text)}</td></tr>`;
      const rowSide = row.kind === "-" ? "old" : "new";
      const num = rowSide === "old" ? row.old : row.new;
      const focus = rowSide === side && num >= from && num <= to;
      const isAnchor = l.anchor && l.anchor.side === rowSide && l.anchor.line === num;
      const cls = [row.kind === "+" ? "add" : row.kind === "-" ? "del" : "", focus ? "focus" : "", isAnchor ? "anchor" : "", anchored.has(`${rowSide}:${num}`) ? "anchored" : ""].join(" ");
      const sign = row.kind === " " ? " " : row.kind;
      return `<tr class="${cls}"><td class="ln" data-side="${rowSide}" data-ln="${num}">${num}</td><td class="src">${esc(sign + row.text)}</td></tr>`;
    }).join("");
    $("#rdiff").innerHTML = `<div class="file"><span>${esc(d.path)}${d.context ? ' <span class="ctxnote">not changed by the PR</span>' : ""}</span><button class="tool" data-nvim>nvim</button></div>
      <div class="codewrap"><table class="code">${rows}</table></div>`;
    $("#rdiff").onclick = async (e) => {
      if (e.target.closest("[data-nvim]")) {
        const line = l.anchor?.line || (from > 0 ? from : 1);
        const pane = await call("review_nvim", { id: r.id, path: d.path, line });
        await window.SB.refreshPanes();
        window.SB.expand(pane);
        return;
      }
      const td = e.target.closest("td.ln[data-ln]");
      if (!td || td.dataset.ln === "undefined") return;
      const line = +td.dataset.ln;
      const same = l.anchor && l.anchor.line === line && l.anchor.side === td.dataset.side;
      l.anchor = same ? null : { path: d.path, side: td.dataset.side, line, text: td.nextElementSibling.textContent.slice(1) };
      l.active = null;
      drawDiff(r);
      drawThreads(r);
      $("#rq").focus();
    };
    const first = $("#rdiff tr.focus");
    if (first && !l.scrolled?.has(s.id)) {
      (l.scrolled ||= new Set()).add(s.id);
      first.scrollIntoView({ block: "center" });
    }
  }

  function drawThreads(r) {
    const l = local(r.id);
    const s = steps(r)[l.cur];
    if (!s) { $("#rthreads").innerHTML = ""; return; }
    const mine = r.threads.filter((t) => t.step === s.id);
    $("#rthreads").innerHTML = mine.map((t) => {
      const where = t.line ? `${(t.path || "").split("/").pop()}:${t.line}${t.side === "old" ? " (old)" : ""}` : `Step ${l.cur + 1}`;
      const busy = t.busy || l.busy.has(t.id);
      const msgs = t.messages.map((m, i) => {
        if (m.me) return `<div class="msg me">${esc(m.text)}</div>`;
        const lastAnswer = i === t.messages.length - 1 && !busy;
        const acts = lastAnswer ? `<div class="macts"><button data-pin="${esc(t.id)}">Pin</button><button data-draft="${esc(t.id)}">Draft</button></div>` : "";
        return `<div class="msg">${esc(m.text)}${acts}</div>`;
      }).join("");
      const pending = busy ? `<div class="msg wait">${esc(l.pending.get(t.id) || "…")}</div>` : "";
      return `<div class="thread${l.active === t.id ? " active" : ""}" data-t="${esc(t.id)}">
        <div class="anchor" title="${esc(t.path || "")}">${esc(where)}${t.removed ? '<span class="gone">line removed</span>' : ""}</div>${msgs}${pending}</div>`;
    }).join("");
    const anchorText = l.active ? "" : l.anchor ? `→ ${l.anchor.path.split("/").pop()}:${l.anchor.line}` : "";
    $("#ranchor").textContent = anchorText;
    $("#rthreads").onclick = async (e) => {
      const b = e.target.closest("button");
      if (b?.dataset.pin || b?.dataset.draft) {
        e.stopPropagation();
        const t = b.dataset.pin || b.dataset.draft;
        b.disabled = true;
        b.textContent = "…";
        await call(b.dataset.pin ? "review_pin" : "review_draft", { id: r.id, thread: t }).catch(() => {});
        return;
      }
      const th = e.target.closest(".thread");
      if (!th) return;
      l.active = l.active === th.dataset.t ? null : th.dataset.t;
      drawThreads(r);
      if (l.active) $("#rq").focus();
    };
  }

  function drawDrafts(r) {
    if (!r.drafts.length) { $("#rdrafts").innerHTML = ""; return; }
    $("#rdrafts").innerHTML = `<div style="display:flex;justify-content:space-between;align-items:center"><span class="badge">${r.drafts.length} draft${r.drafts.length > 1 ? "s" : ""}</span><button class="btn small" data-copy>Copy all</button></div>` +
      r.drafts.map((d) => {
        const loc = d.path ? `${d.path}${d.line ? ":" + d.line : ""}` : "";
        const long = d.text.split("\n").length > 2 || d.text.length > 240;
        return `<div class="draft${long ? " long" : ""}" data-d="${d.id}"><b>${esc(loc)}</b><span contenteditable="true">${esc(d.text)}</span></div>`;
      }).join("");
    $("#rdrafts").querySelectorAll("[data-d] span").forEach((el) => {
      el.addEventListener("blur", () => call("review_draft_edit", { id: r.id, draft: +el.parentElement.dataset.d, text: el.textContent.trim() || null }));
      el.addEventListener("keydown", (e) => e.stopPropagation());
    });
    $("#rdrafts [data-copy]").onclick = async (e) => {
      const text = await call("review_drafts_text", { id: r.id });
      copy(text);
      e.target.textContent = "Copied";
      setTimeout(() => (e.target.textContent = "Copy all"), 1500);
    };
  }

  function copy(text) {
    const ta = document.createElement("textarea");
    ta.value = text;
    document.body.appendChild(ta);
    ta.select();
    try { document.execCommand("copy"); } catch (_) {}
    navigator.clipboard?.writeText(text).catch(() => {});
    ta.remove();
  }

  async function ask(id) {
    const r = S.reviews.get(id);
    const l = local(id);
    const s = steps(r)[l.cur];
    const text = $("#rq").value.trim();
    if (!s || !text) return;
    const busy = l.active && (l.busy.has(l.active) || r.threads.find((t) => t.id === l.active)?.busy);
    if (busy) return toast("This thread is still answering. Wait for it, or click the thread to start a new one.");
    $("#rq").value = "";
    try {
      const t = await invoke("review_ask", { id, step: s.id, anchor: l.active ? null : l.anchor, thread: l.active, text });
      l.active = t;
      l.anchor = null;
      l.busy.add(t);
      l.pending.set(t, "");
      const fresh = await invoke("review_view", { id });
      S.reviews.set(id, fresh);
      window.SBReview.update(fresh);
    } catch (e) {
      $("#rq").value = text;
      toast(e);
    }
  }

  function stream({ review, thread, delta }) {
    const l = local(review);
    l.pending.set(thread, (l.pending.get(thread) || "") + delta);
    if (S.tab !== review) return;
    const el = document.querySelector(`.thread[data-t="${CSS.escape(thread)}"] .msg.wait`);
    if (el) el.textContent = l.pending.get(thread);
  }

  function go(id, i) {
    const r = S.reviews.get(id);
    const l = local(id);
    l.cur = Math.max(0, Math.min(steps(r).length - 1, i));
    l.anchor = null;
    l.active = null;
    update(r);
    loadDiff(id, l.cur);
    $(`#rsteps [data-i="${l.cur}"]`)?.scrollIntoView({ block: "nearest" });
  }

  function next(id) {
    const r = S.reviews.get(id);
    const s = steps(r)[local(id).cur];
    if (s && !stepState(r, s.id).checked) call("review_mark", { id, step: s.id, checked: true });
    go(id, local(id).cur + 1);
  }

  // Review keys work only when no text field has focus (SPEC 7).
  document.addEventListener("keydown", (e) => {
    if (S.tab === "board" || e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.target.closest?.("textarea, input, select, [contenteditable='true']")) return;
    const id = S.tab;
    const l = local(id);
    if (e.key === "j") go(id, l.cur + 1);
    else if (e.key === "k") go(id, l.cur - 1);
    else if (e.key === "n") next(id);
    else if (e.key === "d" && l.active) call("review_draft", { id, thread: l.active });
    else return;
    e.preventDefault();
  });

  window.SBReview = {
    draw,
    update: (r) => {
      const l = local(r.id);
      if (l.head !== r.head || l.guideLen !== JSON.stringify(r.guide || "").length) {
        l.diff.clear();
        l.head = r.head;
        l.guideLen = JSON.stringify(r.guide || "").length;
      }
      for (const t of r.threads) if (!t.busy) { l.busy.delete(t.id); l.pending.delete(t.id); }
      update(r);
    },
    stream,
  };
})();
