// A stand-in for window.__TAURI__: canned data, a call log, and an event bus.
window.__errors = [];
window.addEventListener("error", (e) => window.__errors.push(String(e.message)));
window.addEventListener("unhandledrejection", (e) => window.__errors.push("rejection: " + String(e.reason)));

const pane = (o) => Object.assign({ kind: "claude", repo: "lighthouse", tree: "/c/lighthouse", branch: "unstable", title: null, connection: "a", session: "s1", lamp: "working", unseen: false, summary: null, tail: [], rss: null, permit: null, trust: false, live: false }, o);
window.__data = {
  panes: [
    pane({ id: "p1", lamp: "turn", unseen: true, summary: "PR 1 rewritten. 48 tests pass.", tail: ["\x1b[32mtest result: ok\x1b[0m", "Done."], rss: 2.4 * 1073741824 }),
    pane({ id: "p2", tree: "/c/lighthouse-il", branch: "il", lamp: "needs", unseen: true, summary: "Bash: cargo nextest", permit: { id: "p2-1", summary: "Bash: cargo nextest" } }),
    pane({ id: "p3", kind: "nvim", lamp: "none", repo: "timeways", tree: "/c/timeways", branch: "master", session: null }),
  ],
  connections: { active: "a", list: [{ name: "a", kind: "claude", limit_until: null }, { name: "b", kind: "claude", limit_until: null }] },
  settings: { leader: "ctrl+space", nvim: true },
  repos: [
    { name: "lighthouse", path: "/c/lighthouse", branch: "unstable", dirty: true, worktrees: [{ path: "/c/lighthouse-il", branch: "il" }], last_used: 9, rust: true },
    { name: "timeways", path: "/c/timeways", branch: "master", dirty: false, worktrees: [], last_used: 5, rust: true },
  ],
  reviews: [{
    id: "r1", url: "https://github.com/sigp/lighthouse/pull/10071", repo: "sigp/lighthouse", number: 10071, title: "Add IL transactions", head: "ac92ae9aaaa", base: "b", tree: "/c/lighthouse-pr10071",
    status: "ready", error: null, new_head: null,
    guide: { steps: [
      { id: "s1", title: "Read the PR description", file: null, what: "The PR adds ILs.", check: ["slot - 1"] },
      { id: "s2", title: "Proposal path passes slot - 1", ranges: [{ file: "beacon_node/gloas.rs", side: "new", from: 11, to: 12 }, { file: "beacon_node/gloas.rs", side: "old", from: 11, to: 11 }], what: "Block production asks for the ILs of the slot **before** the proposal slot.", check: ["The spec uses `slot - 1`. Does this match?", "What happens at slot 0?"],
        why: "A wrong slot builds a block with the wrong inclusion list.", why_split: "The only change on the proposal path.", check_answers: ["matches the spec", "see the note on line 11"],
        take: { verdict: "change", notes: [
          { severity: "must", text: "`slot - 1` saturates at slot 0. Use `safe_sub`.", file: "beacon_node/gloas.rs", side: "new", line: 11 },
          { severity: "should", text: "No test covers slot 0." },
          { severity: "nit", text: "A name far below.", file: "beacon_node/gloas.rs", side: "new", line: 40 },
        ] } },
    ], take: { verdict: "change", summary: "no test for slot 0", notes: [{ severity: "should", text: "No test covers slot 0." }] } },
    dismissed: [],
    steps: [{ id: "s1", checked: true, stale: false }],
    threads: [{ id: "t1", step: "s2", path: "beacon_node/gloas.rs", side: "new", line: 11, removed: false, busy: false, messages: [{ id: 1, me: true, text: "safe sub?" }, { id: 2, me: false, text: "Use **safe_sub**. The `-` on `Slot` saturates, so slot 0 gives 0:\n\n```rust\nlet il_slot = builder_params.slot.safe_sub(1)?;\n```\n\n- `slot_epoch_macros.rs:119` marks `-` as deprecated.\n- Small nit, not a bug.\n\n<script>window.__xss = 1</script>" }] }],
    pins: [{ id: 1, step: "s2", text: "Use safe_sub, not -." }],
    drafts: [{ id: 1, path: "beacon_node/gloas.rs", side: "new", line: 11, start_line: null, text: "nit: use safe_sub", agent: true }],
    summary: "", posted: [],
    coverage: { files: 1, changed_lines: 3, accepted: true, missed_lines: 0, binary: [], github: [] },
  }],
  diff: { head: "ac92ae9aaaa", sections: [{ path: "beacon_node/gloas.rs", old_path: "beacon_node/gloas.rs", context: false, ranges: [{ file: "beacon_node/gloas.rs", side: "new", from: 11, to: 12 }, { file: "beacon_node/gloas.rs", side: "old", from: 11, to: 11 }], rows: [
    { kind: "@", old: null, new: null, text: "@@ -10,3 +10,4 @@" },
    { kind: " ", old: 10, new: 10, text: "let a = 1;" },
    { kind: "-", old: 11, new: null, text: "let il = vec![];" },
    { kind: "+", old: null, new: 11, text: "let il = self.get(slot - 1)?;" },
    { kind: "+", old: null, new: 12, text: "let b = 2;" },
  ] }] },
};
// Tasks (SPEC 27): a small copy of the hub rules, and a `tasks` event after each change.
const pad2 = (n) => String(n).padStart(2, "0");
const dayOff = (n) => { const x = new Date(); x.setDate(x.getDate() + n); return `${x.getFullYear()}-${pad2(x.getMonth() + 1)}-${pad2(x.getDate())}`; };
const tk = (o) => Object.assign({ notes: "", status: "todo", blocked_by: null, priority: "normal", sprint: "s1", repo: null, created: 1, changed: 1, started: null, done: null, points: null, tags: [], links: [] }, o);
window.__data.tasks = {
  sprints: [{ id: "s0", name: "Sprint 0", goal: "", start: dayOff(-30), end: dayOff(-17), state: "closed" }, { id: "s1", name: "Sprint 1", goal: "Ship inclusion lists", start: dayOff(-3), end: dayOff(10), state: "open" }],
  tasks: [
    tk({ id: "t1", title: "Review PR 10071", rank: 1024, priority: "high", repo: "/c/lighthouse", status: "doing" }),
    tk({ id: "t2", title: "Fix the slot 0 test", rank: 2048 }),
    tk({ id: "t3", title: "Wait for the spec", rank: 3072, status: "blocked", blocked_by: "spec PR 4512", repo: "/c/lighthouse" }),
    tk({ id: "t4", title: "Old done task", rank: 1024, sprint: "s0", status: "done", done: 5 }),
    tk({ id: "t5", title: "Write the design doc", rank: 1024, sprint: null, repo: "/gone/repo", notes: "See **this**. <script>window.__xss = 2</script> ![img](https://evil.example/x.png) [bad](javascript:window.__xss=3) [good](https://example.com)" }),
  ],
};
window.__data.tasks.links = [
  { pane: "p1", task: "t1", kind: "claude", title: null, tree: "/c/lighthouse", session: "s1", connection: "a", closed: false },
  { pane: "p7", task: "t3", kind: "claude", title: "spec agent", tree: "/c/lighthouse-spec", session: "s7", connection: "a", closed: true },
];
window.__data.tasks.missing = ["/gone/repo"];
Object.assign(window.__data.tasks, { archive: [], burn: [], merged: [] });
window.__data.history = { t1: [{ at: 1760000000, by: "you", kind: "created", value: "todo" }, { at: 1760000100, by: "you", kind: "status", value: "doing" }] };
function taskShim(cmd, a) {
  const d = window.__data.tasks;
  const find = (id) => d.tasks.find((t) => t.id === id);
  const closed = (t) => d.sprints.find((s) => s.id === t.sprint)?.state === "closed";
  const renumber = (sprint, id, before) => {
    const ids = d.tasks.filter((t) => (t.sprint || null) === (sprint || null) && t.id !== id).sort((x, y) => x.rank - y.rank).map((t) => t.id);
    const at = before && ids.includes(before) ? ids.indexOf(before) : ids.length;
    ids.splice(at, 0, id);
    ids.forEach((x, i) => (find(x).rank = (i + 1) * 1024));
  };
  const status = (t, s, why) => {
    if (s === "blocked" && !why) throw "a blocked task needs who or what blocks it";
    t.blocked_by = s === "blocked" ? why : null;
    t.done = s === "done" ? (t.done || 9) : null;
    t.status = s;
  };
  let out = null;
  switch (cmd) {
    case "task_add": {
      if (!a.task.title.trim()) throw "a task needs a title";
      const t = tk({ id: `t${d.tasks.length + 10}`, title: a.task.title.trim(), status: a.task.status || "todo", sprint: a.task.sprint || null, rank: 0 });
      d.tasks.push(t);
      renumber(t.sprint, t.id, null);
      out = t;
      break;
    }
    case "task_status": { const t = find(a.id); if (closed(t)) throw "closed"; status(t, a.status, a.blockedBy); out = t; break; }
    case "task_move": {
      const t = find(a.id);
      if (closed(t)) throw "closed";
      if (a.to.status) status(t, a.to.status, a.to.blocked_by || t.blocked_by);
      t.sprint = a.to.sprint;
      renumber(t.sprint, t.id, a.to.before);
      out = t;
      break;
    }
    case "task_edit": { const t = find(a.id); Object.assign(t, a.fields); out = t; break; }
    case "task_delete": {
      const t = find(a.id);
      d.tasks = d.tasks.filter((x) => x.id !== a.id);
      d.links = d.links.filter((l) => l.task !== a.id);
      d.archive.unshift(Object.assign(t, { deleted: 1760000000 }));
      break;
    }
    case "task_restore": {
      const t = d.archive.find((x) => x.id === a.id);
      if (!t) throw "not in the archive";
      d.archive = d.archive.filter((x) => x.id !== a.id);
      delete t.deleted;
      if (t.sprint && d.sprints.find((x) => x.id === t.sprint)?.state !== "open" && d.sprints.find((x) => x.id === t.sprint)?.state !== "planned") t.sprint = null;
      d.tasks.push(t);
      renumber(t.sprint, t.id, null);
      out = t;
      break;
    }
    case "task_start_plan": { const t = find(a.id); out = { task: t.id, repo: t.repo, branch: "sb/" + t.title.toLowerCase().replace(/[^a-z0-9]+/g, "-"), title: t.title, first: t.title + (t.notes ? "\n\n" + t.notes : "") }; break; }
    case "task_link": d.links = d.links.filter((l) => l.pane !== a.pane).concat([{ pane: a.pane, task: a.task, kind: "claude", title: null, tree: "/c/x", session: null, connection: "a", closed: false }]); break;
    case "task_unlink": d.links = d.links.filter((l) => l.pane !== a.pane); break;
    case "task_resume": out = "p9"; break;
    case "task_copy": { const t = find(a.id); out = tk({ id: `t${d.tasks.length + 10}`, title: t.title, sprint: null, rank: 99999 }); d.tasks.push(out); break; }
    case "sprint_new": {
      const open = d.sprints.some((x) => x.state === "open");
      if (open && d.sprints.some((x) => x.state === "planned")) throw "the next sprint is planned already";
      out = Object.assign({ id: `s${d.sprints.length}${d.sprints.some((x) => x.id === `s${d.sprints.length}`) ? "b" : ""}`, state: open ? "planned" : "open" }, a.sprint);
      d.sprints.push(out);
      break;
    }
    case "sprint_edit": out = Object.assign(d.sprints.find((s) => s.id === a.id), a.sprint); break;
    case "sprint_close": {
      d.sprints.find((s) => s.id === a.id).state = "closed";
      const planned = d.sprints.find((s) => s.state === "planned");
      if (planned) planned.state = "open";
      const next = planned || (a.next && Object.assign({ id: `s${d.sprints.length}`, state: "open" }, a.next));
      if (next && !planned) d.sprints.push(next);
      for (const [id, p] of a.places) {
        const t = find(id);
        if (p === "done") status(t, "done");
        else { t.sprint = p === "next" ? next.id : null; renumber(t.sprint, t.id, null); }
      }
      out = next;
      break;
    }
  }
  setTimeout(() => window.__emit("tasks", structuredClone(d)), 0);
  return structuredClone(out);
}
window.__calls = [];
const handlers = {};
window.__emit = (event, payload) => (handlers[event] || []).forEach((h) => h({ payload }));
window.__TAURI__ = {
  core: {
    invoke: async (cmd, args) => {
      window.__calls.push([cmd, args || {}]);
      const d = window.__data;
      switch (cmd) {
        case "panes": return structuredClone(d.panes);
        case "connections": return d.connections;
        case "settings": return d.settings;
        case "repos": return d.repos;
        case "reviews": return d.reviews;
        case "layouts": return ["day"];
        case "review_view": return d.reviews.find((x) => x.id === args.id) || d.reviews[0];
        case "review_round": {
          d.reviews.push(Object.assign(structuredClone(d.reviews[0]), { id: "r2", round: 2, since: "ac92ae9aaaa", head: "fff0000bbbb", new_head: null,
            scope_note: "The author rebased or merged the base branch. This round leaves out the changes from the base branch." }));
          return "r2";
        }
        case "review_diff": return { head: d.diff.head, sections: d.diff.sections.map((x) => ({ ...x, other: { lines: 4, steps: ["s1"] } })) };
        case "review_term": {
          if (args.id === "rL") return "p1";
          // The review's Claude pane starts on the first call with start.
          if (args.start && !d.panes.some((x) => x.id === "p8")) d.panes.push(pane({ id: "p8", title: "Ask #10071", tree: "/c/lighthouse-pr10071", lamp: "turn" }));
          return d.panes.some((x) => x.id === "p8") ? "p8" : null;
        }
        case "review_step_note": return 'I am on step s2 "Proposal path passes slot - 1" (beacon_node/gloas.rs:11-12 (new)).';
        case "pane_expand": return btoa("\x1b[H\x1b[2Jhello from the live pane\r\n$ ");
        case "pane_open": return pane({ id: "p9", tree: args.req.repo });
        case "review_nvim": return "p3";
        // One nvim and one shell for each worktree (SPEC 28).
        case "nvim_open": case "shell_open": {
          const kind = cmd === "nvim_open" ? "nvim" : "shell";
          const have = d.panes.find((x) => x.kind === kind && x.tree === args.tree);
          if (have) return have.id;
          const id = cmd === "nvim_open" ? "pn1" : "ps1";
          d.panes.push(pane({ id, kind, tree: args.tree, lamp: "none", session: null, busy: false, modified: false }));
          return id;
        }
        case "pane_attach": return btoa(`\x1b[H\x1b[2Jpart ${args.id}\r\n`);
        case "review_drafts_text": return "beacon_node/gloas.rs:11: nit: use safe_sub";
        case "review_open": return "r1";
        // A review of the unpushed commits of pane p1.
        case "review_open_local": {
          if (!d.reviews.some((x) => x.id === "rL")) d.reviews.push(Object.assign(structuredClone(d.reviews[0]), {
            id: "rL", kind: "local", pane: args.pane, url: "", number: 0, repo: "lighthouse", title: "2 commits on feat", base: "b0b0b0b0b0b0", head: "ac92ae9aaaa",
            guide: null, steps: [], threads: [], pins: [], drafts: d.reviews[0].drafts, posted: [], scope_note: "Commits not on origin/unstable.", coverage: null }));
          return "rL";
        }
        case "review_send_agent": return 1;
        case "review_chat": return [
          { step: "s1", question: "what does the PR add?", answer: "ILs for **gloas**.", at: 1 },
          { step: "s2", question: "is slot 0 real?", answer: "Yes: it saturates.", at: 2 },
        ];
        case "review_step_seen": case "review_dismiss": case "review_retake": return null;
        // A closed round of PR 999 exists; PR 10071 has none.
        case "review_prior": return args.url.includes("/pull/999") ? { id: "r0", round: 1, head: "abc1234567890", sent: 2 } : null;
        // A first close finds changes; a close with a choice closes.
        case "review_close": return args.keep || args.discard ? { closed: true, changes: [] } : { closed: false, changes: [" M beacon_node/gloas.rs", "?? notes.txt"] };
        case "review_file_rows": return { head: d.diff.head, rows: Array.from({ length: 60 }, (_, i) => i + 1).flatMap((n) =>
          n === 11 ? [{ kind: "-", old: 11, new: null, text: "let il = vec![];" }, { kind: "+", old: null, new: 11, text: "let il = self.get(slot - 1)?;" }, { kind: "+", old: null, new: 12, text: "let b = 2;" }]
          : [{ kind: " ", old: n, new: n < 11 ? n : n + 1, text: n === 10 ? "let a = 1;" : `line ${n}` }]) };
        case "review_files": return [
          { path: "beacon_node/gloas.rs", old_path: "beacon_node/gloas.rs", added: 2, removed: 1, note: null, steps: ["s2"] },
          { path: "docs/img.png", old_path: null, added: 0, removed: 0, note: "binary file", steps: [] },
        ];
        case "review_file_diff": return args.path === "beacon_node/gloas.rs" ? d.diff : { head: d.diff.head, sections: [{ path: args.path, old_path: null, context: false, ranges: [], rows: [], note: "binary file" }] };
        case "review_post_preview": return { login: "eserilev", token: "tok1", plan: { head: "ac92ae9aaaa", errors: [],
          inline: [{ draft: 1, path: "beacon_node/gloas.rs", side: "RIGHT", line: 11, start_line: null, text: "nit: use safe_sub" }],
          outside: [{ draft: 2, at: "beacon_node/gloas.rs:40", reason: "the line is outside GitHub's diff", text: "far away" }] } };
        case "review_post": return "https://github.com/sigp/lighthouse/pull/10071#pullrequestreview-1";
        case "log": return null;
        case "tasks": return structuredClone(d.tasks);
        case "task_history": return d.history[args.id] || [];
        case "task_add": case "task_status": case "task_move": case "task_edit": case "task_delete": case "task_copy":
        case "task_start_plan": case "task_link": case "task_unlink": case "task_resume": case "task_restore":
        case "sprint_new": case "sprint_edit": case "sprint_close": return taskShim(cmd, args);
        default: return null;
      }
    },
  },
  event: { listen: async (event, h) => { (handlers[event] ||= []).push(h); return () => {}; } },
};
