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
      { id: "s2", title: "Proposal path passes slot - 1", ranges: [{ file: "beacon_node/gloas.rs", side: "new", from: 11, to: 12 }, { file: "beacon_node/gloas.rs", side: "old", from: 11, to: 11 }], what: "Block production asks for the ILs of the slot **before** the proposal slot.", check: ["The spec uses `slot - 1`. Does this match?", "What happens at slot 0?"] },
    ] },
    steps: [{ id: "s1", checked: true, stale: false }],
    threads: [{ id: "t1", step: "s2", path: "beacon_node/gloas.rs", side: "new", line: 11, removed: false, busy: false, messages: [{ id: 1, me: true, text: "safe sub?" }, { id: 2, me: false, text: "Use **safe_sub**. The `-` on `Slot` saturates, so slot 0 gives 0:\n\n```rust\nlet il_slot = builder_params.slot.safe_sub(1)?;\n```\n\n- `slot_epoch_macros.rs:119` marks `-` as deprecated.\n- Small nit, not a bug.\n\n<script>window.__xss = 1</script>" }] }],
    pins: [{ id: 1, step: "s2", text: "Use safe_sub, not -." }],
    drafts: [{ id: 1, path: "beacon_node/gloas.rs", side: "new", line: 11, start_line: null, text: "nit: use safe_sub", agent: true }],
    summary: "", posted: [],
    coverage: { files: 1, changed_lines: 3, accepted: true, missed_lines: 0, binary: [], github: [] },
  }],
  diff: { sections: [{ path: "beacon_node/gloas.rs", old_path: "beacon_node/gloas.rs", context: false, ranges: [{ file: "beacon_node/gloas.rs", side: "new", from: 11, to: 12 }, { file: "beacon_node/gloas.rs", side: "old", from: 11, to: 11 }], rows: [
    { kind: "@", old: null, new: null, text: "@@ -10,3 +10,4 @@" },
    { kind: " ", old: 10, new: 10, text: "let a = 1;" },
    { kind: "-", old: 11, new: null, text: "let il = vec![];" },
    { kind: "+", old: null, new: 11, text: "let il = self.get(slot - 1)?;" },
    { kind: "+", old: null, new: 12, text: "let b = 2;" },
  ] }] },
};
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
        case "review_diff": return { sections: d.diff.sections.map((x) => ({ ...x, other: { lines: 4, steps: ["s1"] } })) };
        case "review_ask": return "t2";
        case "pane_expand": return btoa("\x1b[H\x1b[2Jhello from the live pane\r\n$ ");
        case "pane_open": return pane({ id: "p9", tree: args.req.repo });
        case "nvim_open": case "review_nvim": return "p3";
        case "review_drafts_text": return "beacon_node/gloas.rs:11: nit: use safe_sub";
        case "review_open": return "r1";
        case "review_file_rows": return Array.from({ length: 60 }, (_, i) => i + 1).flatMap((n) =>
          n === 11 ? [{ kind: "-", old: 11, new: null, text: "let il = vec![];" }, { kind: "+", old: null, new: 11, text: "let il = self.get(slot - 1)?;" }, { kind: "+", old: null, new: 12, text: "let b = 2;" }]
          : [{ kind: " ", old: n, new: n < 11 ? n : n + 1, text: n === 10 ? "let a = 1;" : `line ${n}` }]);
        case "review_files": return [
          { path: "beacon_node/gloas.rs", old_path: "beacon_node/gloas.rs", added: 2, removed: 1, note: null, steps: ["s2"] },
          { path: "docs/img.png", old_path: null, added: 0, removed: 0, note: "binary file", steps: [] },
        ];
        case "review_file_diff": return args.path === "beacon_node/gloas.rs" ? d.diff : { sections: [{ path: args.path, old_path: null, context: false, ranges: [], rows: [], note: "binary file" }] };
        case "review_post_preview": return { login: "eserilev", token: "tok1", plan: { head: "ac92ae9aaaa", errors: [],
          inline: [{ draft: 1, path: "beacon_node/gloas.rs", side: "RIGHT", line: 11, start_line: null, text: "nit: use safe_sub" }],
          outside: [{ draft: 2, at: "beacon_node/gloas.rs:40", reason: "the line is outside GitHub's diff", text: "far away" }] } };
        case "review_post": return "https://github.com/sigp/lighthouse/pull/10071#pullrequestreview-1";
        case "log": return null;
        default: return null;
      }
    },
  },
  event: { listen: async (event, h) => { (handlers[event] ||= []).push(h); return () => {}; } },
};
