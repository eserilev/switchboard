# Switchboard: Specification

Status: draft 5, 2026-10-08. Built: PRs 1 to 13. Section 23 lists where the build differs from this text, and what is not tested.
Draft 1 was the design page and mockup.
Draft 2 applies two reviews: one of the design, and one of the rust-analyzer ideas. It also adds account switching.
Draft 3 makes the LLM connection app wide (D4) and fixes the name (D5).
Draft 4 applies the results of spikes S2, S3, S5 and S6 (`spikes/README.md`).
Draft 5 records the build.

Mockup: https://claude.ai/artifact/DeFPkn6Zk5q4VRUK1gzbpr

## 1. Summary

Switchboard is a desktop app for many AI coding agents on one screen.
Each agent runs in a tile with a big title and a lamp.
The lamp shows if the agent works, waits for you, or has finished its turn. It stays lit until you look.

Switchboard also has a guided PR review.
The agent writes a step-by-step guide, and the guide stays in place.
Your questions go into threads that are attached to a step or a diff line.

Switchboard has three parts:

- **The window**: a Tauri 2 app. The front end is plain HTML and web components.
- **The core**: Rust. It drives tmux, reads agent state, runs reviews and keeps the store.
- **`sb`**: a small CLI. Hooks call it to report state. It also runs the MCP server for reviews.

## 2. Goals

1. See the state of every agent in one look, with no terminal output to read.
2. Never miss an agent that waits for you.
3. Cut context switches: switch on your own schedule, in batches.
4. Review a PR step by step. Side questions never push the guide out of view.
5. Open a repo, a worktree or nvim in two keys.
6. One active LLM connection for the whole app. Switch to a second Claude account when one hits its limit, with no lost context.
7. Use only the tools you have installed. The build has no npm step.

## 3. Non-goals

- A general terminal emulator. Switchboard shows agents, nvim and shells. It does not replace your terminal.
- Posting to GitHub. Switchboard never posts a review, a comment or an issue.
- Edits by the review agent. The review agent only reads.
- Windows and macOS. Switchboard targets Arch Linux on Wayland first.
- A hosted service. Everything runs on the local machine.

## 4. The problem

Three past reviews (PRs #10071, #121 and #9729) follow the same pattern:

1. You paste a PR link. You ask for a review "step by step, file by file", with file names, line numbers and what to look for.
2. The agent writes a plan. It reads the simple files first.
3. You read one step. Then you ask a side question: "safe_sub or saturating_sub?", "explain #5710", "how do we prevent these off-by-one footguns?"
4. The answer is long. The guide scrolls far up. You scroll back, or you ask "what is the next step?"

The guide and the conversation are in one stream. Switchboard splits them.
The guide is a document. Questions are threads attached to it.

Rules for review sessions, from your memory files:

- The agent only reads and diffs. It does not build, test, edit or push.
- The agent never posts to GitHub.
- Review comments are short, one or two lines each.

## 5. Terms

| Term | Meaning |
|---|---|
| Pane | One process that Switchboard runs: an agent, nvim or a shell. One tmux window. |
| Tile | The status card of a pane on the board. |
| Live pane | The one pane that shows a real terminal. |
| Lamp | The state color of a tile. |
| Unseen | A lamp that turned on after you last looked at the pane. |
| Leader | The key that sends the next key to the app, not to the terminal. |
| Connection | One LLM account: a Claude subscription or an endpoint. One is active for the whole app (8). |
| Guide | The step list of a review. |
| Guide session | The Claude session that writes the guide. |
| Thread | A question and its answers, anchored to a step or a line. |
| Fork | A Claude session forked from the guide session for one thread. |
| Pin | A one-line note from a thread, shown in the guide under its step. |
| Draft | A review comment with a path and a line. You post it yourself. |

## 6. Board

### 6.1 Tiles

The board is a grid of tiles. A tile shows:

- The title in large condensed capitals. The default is the branch for a new worktree, or the repo name.
- The lamp: a soft tint and a 4 px stripe in the title bar, and a border in the state color.
- The meta line: repo, branch, connection, and rust-analyzer memory when it runs.
- The summary line (6.4).
- The last 4 lines of the pane, from `tmux capture-pane -p -e`.
- The actions for the state: Allow and Deny for Needs you, Switch for Limit.

Expand a tile, and it becomes the live pane. It takes the full width and shows the full scrollback.
Only one pane is live at a time. The Claude TUI never draws at tile width, so many panes fit on a small screen.

### 6.2 Lamps

| Lamp | Meaning | Source |
|---|---|---|
| Working | The agent is busy. | `UserPromptSubmit`, `PreToolUse` |
| Needs you | A permission prompt, a question dialog, or the folder trust dialog. The border pulses slowly. | `PermissionRequest`; `Notification` with `permission_prompt` or `elicitation_dialog`; the trust dialog (6.6) |
| Your turn | The turn ended. The agent finished, or it asked a question in plain text. | `Stop` |
| Limit | The connection hit its usage limit. The session ended, or it waits for the reset. | `StopFailure` with `error: rate_limit`; `Notification` with `quota_auto_resume_*` (S1) |
| Error | An API error ended the turn, or the agent crashed. | `StopFailure`; tmux pane exit with a non-zero code |
| Idle | A new pane before the first prompt. | Pane start |

nvim and shell panes have no lamp. They have a dashed border.

Rules:

- A plain `Notification` does not set a lamp. Its `idle_prompt` type fires when an agent only sits idle.
- A failed command inside Claude, for example `cargo clippy`, is not an Error. Claude sees the failure and goes on.
- `quota_auto_resume_fired` sets the lamp back to Working. `quota_auto_resume_disabled` keeps it at Limit.
- In auto mode, most tools never reach `PermissionRequest`. Needs you then shows only for the questions and checks that auto mode sends to you.
- The colors are muted. A full board of lamps does not glare.

### 6.3 Unseen state

- A lamp that turns on sets the tile to unseen.
- Expand the tile, or type into the pane, to clear it. Focus alone does not clear it.
- If the window is not focused when a lamp turns on, the app sends a desktop notification and sets the window urgency hint.
- The tally in the top bar counts unseen tiles for each state: Needs you, Limit, Error, Your turn.
- Leader, `j` opens the next unseen tile. The order is Needs you, Limit, Error, Your turn.

### 6.4 Summary line

- On `Stop`, the hook input has `last_assistant_message`. `sb state` sends its first line, cut to 120 characters. It never reads `transcript_path`: the docs say the file can lag at `Stop` time.
- On `PermissionRequest`, the summary is the tool and its input, for example `Bash: cargo nextest run -p beacon_chain`.
- On `StopFailure`, the summary is `last_assistant_message`, for example `API Error: Rate limit reached`.
- You can type a title for a tile. The app keeps it in the store.

### 6.5 Permission from the tile

The `PermissionRequest` hook runs `sb permit`. That command blocks. It sends the request to the app and waits for the answer.
Spike S2 showed that the Claude dialog shows in the pane at the same time. The first answer wins.

- Allow and Deny on the tile send the answer. `sb permit` prints the hook decision JSON and exits.
- If you answer in the pane, Claude goes on. `sb permit` gets killed or times out. The app removes the tile buttons on the next hook event.
- If the app is not running, `sb permit` exits at once with no decision.
- `sb permit` has no timeout of its own. The hook timeout is 600 s by default.

The decision JSON:

```json
{ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": { "behavior": "allow" } } }
```

### 6.6 Folder trust

Claude shows a trust dialog the first time it starts in a new folder. Every new worktree is a new folder.
No hook fires for this dialog, and a plain Enter picks "No, exit".

- At pane start, the core reads `capture-pane` until the Claude prompt or the trust dialog shows, for at most 10 s.
- If the trust dialog shows, the tile goes to Needs you with the summary "Trust this folder?" and Trust and Exit buttons.
- Trust sends `Down Enter`. Exit closes the pane.
- This is the one place where the core reads the screen. A test pins the dialog text, so a CLI change fails the test, not the user.

## 7. Key map

A live pane gets every key you type. Esc in Claude Code stops the agent. So app keys need a leader.

| Keys | Action |
|---|---|
| `Ctrl+Space` | Leader. The next key goes to the app. The leader times out after 1.5 s. |
| Leader, `x` | Expand the focused tile, or collapse the live pane. |
| Leader, `j` / `k` | Next or previous unseen tile. |
| Leader, `H` `J` `K` `L` | Move focus between tiles. |
| Leader, `e` | nvim in the worktree of the focused pane. |
| Leader, `n` | Launcher. |
| Leader, `r` | Review tab. |
| Leader, `b` | Board tab. |
| Leader, `a` | Switch the active connection. |
| Leader, `<` / `>` | Move the focused tile one place left or right. |
| Review: `j` / `k` | Next or previous step. |
| Review: `n` | Mark the step reviewed and go to the next one. |
| Review: `d` | Draft from the active thread. |

The review keys work only when no text field and no terminal has focus.
The leader key is set in the config (14). Decision D2.

## 8. Connections

A connection is one LLM account: a Claude subscription, or another endpoint.
One connection is active, and it applies to the whole app: new panes, guide sessions and review threads.
You have two Claude subscriptions. When one hits its limit, you switch to the other and keep the session.

### 8.1 Kinds

Every connection runs through Claude Code, so hooks, lamps, tools and reviews work the same for all of them.

| Kind | How Claude Code starts |
|---|---|
| Claude account | `CLAUDE_CONFIG_DIR=<dir> claude` |
| Endpoint | `ANTHROPIC_BASE_URL=<url> ANTHROPIC_AUTH_TOKEN=<token> ANTHROPIC_MODEL=<model> claude`, with `CLAUDE_CONFIG_DIR` set to the endpoint's own folder |

An endpoint must speak the Anthropic Messages API. Spike S7 checks this for a local server, for example llama.cpp server or Ollama.
An endpoint with only the OpenAI API is not supported.
The token comes from the system keyring through `secret-tool`. It is never in the config file.

### 8.2 Connection folders

- Each connection has one folder, for example `~/.claude-b`. A connection with no `dir` uses your own `~/.claude` and its login. So your current account needs no new folder.
- A Claude account folder has its own `.credentials.json` and `.claude.json`. You run `/login` once in each.
- Each folder has symlinks to `~/.claude` for `settings.json`, `CLAUDE.md`, `projects/`, `skills/` and `plugins/`.
- All connections share your memory, settings and session history.

`sb connect add <name>` makes the folder and the symlinks. For a Claude account, it then starts `claude` in a pane for `/login`.

### 8.3 Active connection

- The top bar shows the active connection. Leader, `a` or a click on it opens the list.
- A switch changes the connection for new panes, new guide sessions and new review threads.
- Running panes keep their connection. The tile meta line shows it.
- A thread fork runs on the active connection, also when its guide session ran on another one (S4).

### 8.4 Limit: switch and resume

- When a connection hits its limit, the app marks it "at limit" for one hour. The hook input has no reset time (S1). The next Working event on that connection clears the mark.
- If the active connection is at limit, the top bar shows it in the Limit color.
- A Limit tile has one action: Switch to the next connection that is not at limit.
- That action makes the next connection active, then resumes the pane on it:
  1. The app stops the agent with `C-c` twice. If the process does not exit in 5 s, it kills the tmux pane.
  2. The app starts `claude --resume <session-id>` with the new connection in the same tmux window.
- The session file is in the shared `projects/`, so the agent keeps its full context (S4).
- The session id comes from the hook input. The store keeps the last one for each pane.

Decision D3: a switch button, or an automatic switch.

## 9. Open a repo

### 9.1 Launcher

Leader, `n` or `+` opens the launcher.

| Choice | Behavior |
|---|---|
| Repo | Fuzzy search. The list is the union of the `cwd` values in the session files under `~/.claude/projects/*/*.jsonl` and a scan of the folders in the config. The most recent repo is first. |
| Where | Main checkout, new worktree, or an existing worktree from `git worktree list`. |
| Run | Claude Code, nvim, or a shell. Claude Code uses the active connection. |

Rules:

- The folder names under `~/.claude/projects` replace `/` with `-`. The app never decodes them. It reads `cwd` from the files.
- New worktree is the default when the main checkout is dirty, or when the tree already has an agent pane.
- A new worktree goes next to the repo: `<repo>-<branch>`. The app runs `git worktree add -b <branch> <path>`.
- Enter opens. Esc closes. Arrow keys move in the list.

### 9.2 PR URL

Paste a GitHub PR URL into the launcher, and it opens a review (11).

1. `gh pr view <url> --json number,headRefOid,headRefName,headRepository,baseRefName,title`.
2. The app finds the local clone of the base repo from the repo list.
3. `git fetch <remote> pull/<n>/head:pr-<n>`.
4. `git worktree add <repo>-pr<n> pr-<n>`.

`gh pr checkout` is not used. It changes the current folder and makes no worktree.

### 9.3 `sb open`

- `sb open .` opens the current folder as a Claude pane.
- `sb open <path> -w <branch>` opens a new worktree.
- `sb open <path> --nvim` and `--shell` open the other pane kinds.
- If the app is not running, `sb open` starts it.

### 9.4 Layouts

A layout is a saved set of panes: repo, tree, kind and title. Its agents start on the active connection.
Opening a layout starts all its panes. Panes that already run are not started again.

## 10. nvim

- Every agent tile has an nvim action. Leader, `e` does the same.
- There is one nvim for each worktree. It runs in its own pane next to the agent.
- The first open runs `nvim --listen $XDG_RUNTIME_DIR/switchboard/nvim/<tree-hash>.sock`.
- Later opens use msgpack-RPC `nvim_cmd` with `edit +<line> <file>`. RPC works in every mode.
- At start, the app removes sockets with no live nvim.
- From a review, every file:line opens nvim at that line in the review worktree.
- After a `PostToolUse` hook for Edit or Write in a worktree, the app runs `checktime` in that nvim.
- If the changed buffer has unsaved changes, the app does not reload it. The nvim tile shows the file name as changed on disk.
- The app runs the `nvim` on `PATH` with your config. If `nvim` is not on `PATH`, the action is hidden.

## 11. Review

### 11.1 Layout

| Column | Content |
|---|---|
| Steps | The guide in reading order. Each step: title, path and lines, a check mark, the thread count, a stale mark (11.7). |
| Guide + diff | The current step: what the code does, what to check, pins, context links. Under it, the diff at the PR head, with the step lines marked. |
| Threads | The threads of the current step. Below them, the drafts. |

A review has its own tab. More than one review can be open, one tab for each PR.
The review header shows the state of the guide session: fetching, writing, updating, ready or error.

### 11.2 Guide session

- The app runs `claude -p` in the review worktree with the review prompt (11.3).
- Flags: `--output-format stream-json --verbose --include-partial-messages --mcp-config <sb mcp> --allowedTools <list>`.
- stdin is `/dev/null`. With any other stdin, `claude -p` waits 3 s for input (S3).
- `--allowedTools`: `Read,Grep,Glob,Bash(git diff:*),Bash(git log:*),Bash(git show:*),mcp__sb__guide_set_steps,mcp__sb__guide_update_step`.
- `--disallowedTools`: `Edit,Write,NotebookEdit,Bash(gh:*)`.
- In `-p` mode a tool that is not allowed is denied with no prompt.
- Spec context comes from local clones that the config names, for example `consensus-specs`. The prompt gives their paths.
- The guide session ends after one turn. The store keeps its session id.

### 11.3 Review prompt

The prompt has these parts:

1. The PR: repo, number, title, head commit, base branch, the file list with line counts.
2. The rules: read only, never post, file:line at the head commit for every claim.
3. The order: simple files first, so the reader has the context for the hard part.
4. The output: call `guide_set_steps` once with every step. Do not write the guide as text.
5. The style: one fact per sentence, short, no recap.
6. The context: the paths of the spec clones, and any spec or issue links in the PR body.

The prompt lives in `prompts/review.md` in the repo. You can override it in the config.

### 11.4 MCP tools

`sb mcp` is an MCP server over stdio. It sends each call to the core over the socket.

`guide_set_steps(guide)`: replaces the guide.

```json
{
  "pr": { "repo": "sigp/lighthouse", "number": 10071, "head": "ac92ae9" },
  "context": [{ "label": "consensus-specs heze/validator.md", "ref": "validator.md:212" }],
  "steps": [{
    "id": "s3",
    "title": "Proposal path passes slot - 1",
    "file": "beacon_node/beacon_chain/src/block_production/gloas.rs",
    "side": "new",
    "lines": [1374, 1384],
    "what": "Block production asks for the ILs of the slot before the proposal.",
    "check": ["The spec uses slot - 1. Does this match?",
              "Slot `-` saturates. What happens at slot 0?"],
    "context": ["slot_epoch_macros.rs:119"]
  }]
}
```

`guide_update_step(id, fields)`: changes the named fields of one step. Pins and threads are not fields of a step, so an update never removes them.

Validation:

- `file` must exist at `head` for `side: "new"`, or at the base for `side: "old"`. `file` is null only for a step with no code, for example the PR description.
- `lines` must be inside the file.
- On a failed check, the tool returns an error with the reason. The agent can call again.
- If the turn ends with no `guide_set_steps` call, the review tab shows the error and offers a retry.

### 11.5 Threads

1. Click a line number in the diff to anchor a question to it. An anchor is the full path, the side (old or new), the line number, and the text of the line. Deleted lines take an anchor too.
2. With no line, the question anchors to the step.
3. The first question of a thread forks the guide session: `claude -p --resume <guide> --fork-session`. Spike S3 showed that the guide session file stays byte-equal. The prompt has the step, the anchor, the pins of the step, and the question.
4. The fork uses the same tool list as the guide session.
5. Click a thread to make it active. A follow-up resumes the fork with `--resume <fork>`. It does not fork again. The fork keeps its session id (S3).
6. Answers stream into the thread.

Cost: each fork sends the guide context again. The prompt cache lasts a few minutes, so a question after a long pause costs more.

### 11.6 Pins and drafts

- **Pin** asks the fork for a one-line summary of its last answer. You can edit the pin. It goes under the step in the guide.
- Pins live in the store, not in the guide. Each new fork of a step gets that step's pins in its prompt.
- **Draft** makes a review comment from an answer: full path, line, side, and text. You can edit it.
- A draft longer than two lines gets a warning.
- **Copy all** copies the drafts as `path:line: text`, one for each line.

### 11.7 New commits

- The app polls `headRefOid` every 2 minutes while the review tab is open.
- When the head changes, the top bar shows the old and the new head.
- Steps whose files changed get a stale mark.
- "Diff since last review" shows `git diff <old>..<new>` for each stale step.
- Threads re-anchor by the line text. If the text is gone, the thread shows "line removed" and keeps the old anchor.
- "Update guide" resumes the guide session with the new head and asks for `guide_update_step` calls.

## 12. Agents

| Where | How |
|---|---|
| Panes | `claude` in a tmux window, with hooks from `--settings` (13.2), on the active connection. |
| Guide sessions | `claude -p`, as in 11.2, on the active connection. |
| Review threads | Forks of the guide session, as in 11.5, on the active connection. |

There is no separate local LLM setting. A local model is one more connection (8.1).

## 13. Architecture

```
┌──────────── Tauri window (web components, no framework) ───────────┐
│  Board: status tiles + one live xterm.js     Review: three columns │
└───────────────▲──────────────────────────────▲─────────────────────┘
                │ IPC events                   │ IPC commands
┌───────────────┴──────────── Rust core ───────┴─────────────────────┐
│ tmux     tmux -L switchboard, one control client, a window per pane │
│ state    socket + ~/.switchboard/state/<pane>  ← sb state           │
│ mcp      sb mcp: guide_set_steps, guide_update_step                 │
│ review   PR fetch + worktree, head polling, claude -p runner        │
│ nvim     msgpack-RPC to one nvim for each worktree                  │
│ store    SQLite: panes, connections, reviews, threads, pins, drafts │
└────────────────────────────────────────────────────────────────────┘
```

### 13.1 tmux

- One tmux server: `tmux -L switchboard`. Your own tmux server is not touched.
- The server sets `focus-events on`. Claude Code asks for it.
- One session, `sb`, with one window for each pane.
- One control-mode client (`tmux -L switchboard -C attach -t sb`). The core reads `%output` and `%window-*` events.
- Only the live pane streams. Every other pane is paused: `refresh-client -A '%<pane>:pause'`. Spike S6: 10 panes of `yes` sent 244 MB in 10 s with all panes streaming, and 0.1 MB with tiles stopped.
- Never use `%<pane>:off`. When every client turns a pane off, tmux stops reading it, and a busy agent blocks.
- `refresh-client -f pause-after=2` stays on as a guard for the live pane.
- When you expand a tile, the core draws the scrollback with `capture-pane -p -e -S -` and sends `%<pane>:continue`. When you collapse it, the pane is paused again.
- Tiles refresh from `capture-pane -p -e -S -4` once a second. It takes about 11 ms for each pane.
- A paused pane still sends `%window-*` events, so the core sees exits.
- Each window gets `SB_PANE=<id>` with `set-environment` before the command starts.
- Close the app, and tmux keeps running. Open it, and the core attaches again (16).

### 13.2 Hooks

The app writes `~/.switchboard/hooks.json` and passes it with `claude --settings ~/.switchboard/hooks.json`.
Your global settings stay the same. Outside Switchboard, `SB_PANE` is not set and `sb state` exits at once.

| Hook | Command |
|---|---|
| `UserPromptSubmit` | `sb state working` |
| `PreToolUse` | `sb state working` |
| `PermissionRequest` | `sb permit` (6.5) |
| `Notification` (`permission_prompt`, `elicitation_dialog`) | `sb state needs` |
| `PostToolUse` (`Edit`, `Write`) | `sb state edited` |
| `Stop` | `sb state turn` |
| `StopFailure` | `sb state failure` |
| `SessionStart` | `sb state session` |

Every hook gets its JSON input on stdin. `sb state` reads it for `session_id`, `transcript_path`, `cwd` and the event fields.

### 13.3 `sb state` message

`sb state` sends one JSON line to `$XDG_RUNTIME_DIR/switchboard/sock`:

```json
{ "v": 1, "pane": "p7", "event": "turn", "session": "4f2a9c1e-…", "cwd": "/home/eitan/…/timeways",
  "time": 1791503457, "summary": "Moved links to SQLite. 31 tests pass.", "detail": {} }
```

- `event` is one of `session`, `working`, `needs`, `turn`, `failure`, `edited`.
- `detail` holds the event fields: the tool and its input for `needs`, the error for `failure`, the file for `edited`.
- `sb state` also writes the same line to `~/.switchboard/state/<pane>.json` with an atomic rename.
- `sb state` never blocks Claude. If the socket is gone, it writes the file and exits 0.
- `sb state` exits in under 50 ms. It does no network calls.

### 13.4 IPC

Commands from the window to the core:

| Command | Arguments |
|---|---|
| `pane_open` | repo, tree, kind, title |
| `pane_input` | pane, bytes |
| `pane_resize` | pane, cols, rows |
| `pane_expand` / `pane_collapse` | pane |
| `pane_close` | pane |
| `pane_seen` | pane |
| `permit_answer` | request, allow |
| `connection_set` | name |
| `connection_resume` | pane |
| `nvim_open` | tree, file, line |
| `review_open` | url |
| `review_ask` | review, step, anchor, thread, text |
| `review_pin` / `review_draft` | review, thread, message |
| `review_mark` | review, step, checked |
| `review_update` | review |

Events from the core to the window:

| Event | Payload |
|---|---|
| `pane_state` | pane, lamp, unseen, summary |
| `pane_tail` | pane, lines |
| `pane_output` | pane, bytes (live pane only) |
| `pane_closed` | pane, exit code |
| `permit_request` | request, pane, tool, input |
| `review_guide` | review, guide |
| `review_stream` | review, thread, delta |
| `review_head` | review, old, new, stale steps |
| `error` | scope, message |

## 14. Config

`~/.config/switchboard/config.toml`:

```toml
leader = "ctrl+space"
scan = ["~/Documents/Code"]
idle_end_minutes = 0           # 15: end idle sessions; 0 is off
poll_head_minutes = 2

[connections.a]                 # no dir: your own ~/.claude and its login
kind = "claude"

[connections.b]
kind = "claude"
dir = "~/.claude-b"

[connections.local]             # example
kind = "endpoint"
dir = "~/.claude-local"
url = "http://127.0.0.1:8080"
model = "qwen3-coder"
token = "keyring:switchboard/local"

[review]
prompt = "prompts/review.md"   # optional override
spec_clones = ["~/Documents/Code/Ethereum/Consensus/consensus-specs"]   # example path
```

A missing file gives the defaults. An unknown key is an error with the key name.

## 15. Memory

The global `rust-analyzer-lsp` plugin gives each Claude session its own rust-analyzer.
It starts after the first `.rs` edit and lives until the session ends.
On 2026-10-08, one copy used 3.7 GB, and another was 1 day 20 hours old with 1.1 GB.

In the app:

1. **End idle sessions.** A pane in Your turn, unseen for `idle_end_minutes`, gets its session ended. The tile keeps the session id. Your next prompt resumes it. The app never stops rust-analyzer alone: the plugin restarts it, and after `maxRestarts` the LSP tool stays dead.
2. **RSS on tiles.** The core walks the process tree of each pane and reads `VmRSS` of `rust-analyzer`.
3. **The plugin only for Rust panes.** The pane `--settings` turn the plugin on for repos with a `Cargo.toml`.

Outside the app (later, lowest priority):

4. A custom rust-analyzer plugin with `cachePriming.enable: false`, `numThreads: 4`, `cachePriming.numThreads: 2` and `MALLOC_ARENA_MAX=2`. Measure before and after.
5. A soft cap on rust-analyzer only: `systemd-run --user --scope -p MemoryHigh=3G -p MemoryMax=5G -p OOMPolicy=continue`. A cap on the whole pane stops Claude too.
6. `cargo.targetDir = true` and `check.workspace = false` in a Lighthouse `rust-analyzer.toml`. These cut lock waits, not RAM.
7. lspmux with a strict `pass_environment`, to share one copy for each checkout.

Not used: a project `rust-analyzer.toml` for `cachePriming`, `procMacro` or `numThreads` (they are global keys), and proc macros off (38 Lighthouse files use `#[superstruct]`).

## 16. Store and restart

SQLite at `~/.local/share/switchboard/store.db`. Schema version in `PRAGMA user_version`.

```sql
CREATE TABLE connection (name TEXT PRIMARY KEY, active INTEGER NOT NULL, limit_until TEXT);
CREATE TABLE pane (
  id TEXT PRIMARY KEY, kind TEXT NOT NULL,          -- claude | nvim | shell | guide
  repo TEXT NOT NULL, tree TEXT NOT NULL, title TEXT,
  connection TEXT REFERENCES connection(name), session TEXT,
  lamp TEXT NOT NULL, unseen INTEGER NOT NULL, summary TEXT,
  tmux_window TEXT, created TEXT NOT NULL, closed TEXT);
CREATE TABLE layout (name TEXT PRIMARY KEY, panes TEXT NOT NULL);   -- JSON list
CREATE TABLE review (
  id TEXT PRIMARY KEY, repo TEXT NOT NULL, number INTEGER NOT NULL,
  head TEXT NOT NULL, tree TEXT NOT NULL, guide_session TEXT, guide TEXT, opened TEXT NOT NULL);
CREATE TABLE step_state (review TEXT, step TEXT, checked INTEGER, stale INTEGER, PRIMARY KEY (review, step));
CREATE TABLE thread (
  id TEXT PRIMARY KEY, review TEXT NOT NULL, step TEXT NOT NULL,
  path TEXT, side TEXT, line INTEGER, line_text TEXT, fork_session TEXT);
CREATE TABLE message (id TEXT PRIMARY KEY, thread TEXT NOT NULL, me INTEGER NOT NULL, text TEXT NOT NULL, time TEXT NOT NULL);
CREATE TABLE pin (id TEXT PRIMARY KEY, review TEXT NOT NULL, step TEXT NOT NULL, text TEXT NOT NULL, message TEXT);
CREATE TABLE draft (id TEXT PRIMARY KEY, review TEXT NOT NULL, path TEXT, side TEXT, line INTEGER, text TEXT NOT NULL);
```

At start:

1. Read the store.
2. If the `switchboard` tmux server runs, attach. Match each window to a pane by `SB_PANE`.
3. Read `~/.switchboard/state/*.json`. A state file newer than the stored lamp wins.
4. A stored pane with no window is closed. Its tile shows "ended" with a resume action when it has a session id.

## 17. Failure cases

| Case | Behavior |
|---|---|
| The tmux server dies | All tiles show "ended". The app offers to resume every pane with a session id. |
| The control client disconnects | The core attaches again, with backoff up to 5 s. |
| A hook finds no socket | `sb state` writes the state file and exits 0. |
| The trust dialog shows | The tile shows Needs you with Trust and Exit (6.6). |
| `sb permit` gets no answer | The pane dialog stays open. The hook timeout ends `sb permit`. |
| The guide agent never calls `guide_set_steps` | The review tab shows the error and the last text, and offers a retry. |
| A fork fails mid-answer | The thread keeps the partial answer and shows the error. Ask again resumes the fork. |
| `gh` is not logged in | The launcher shows the `gh auth login` command. |
| A worktree path exists | The launcher offers the existing worktree. |
| Two agents in one tree | The launcher warns and selects New worktree. |
| A connection is not logged in | The pane shows Claude's own login prompt. The connection gets a warning in the top bar. |
| An endpoint does not answer | The pane shows Claude's own API error, and the tile shows Error. |
| Every connection is at limit | The top bar shows the earliest reset time. |
| nvim socket is stale | The core removes it and starts a new nvim. |

## 18. Spikes

Each spike is a small script in `spikes/`. S1 to S6 must pass before PR 1. S7 must pass before endpoints ship in PR 5, or the design changes.

| ID | Question | Pass |
|---|---|---|
| S1 | Does a subscription usage limit fire `StopFailure` with `rate_limit`, or only the quota notifications? | The capture log of a real limit shows the fields. Status: waiting. |
| S2 | Does a blocking `PermissionRequest` hook wait for an outside answer, and does Claude apply its decision? | Pass, in `-p` and in a pane. |
| S3 | Does `--resume <id> --fork-session` leave the guide session unchanged? | Pass. |
| S4 | Does `--resume` work across two connections with a shared `projects/`? | Connection B resumes a session of connection A with full context. Two live panes on two connections do not corrupt `projects/`. Status: waiting for the second login. |
| S5 | Does a Tauri 2 window draw on your Wayland setup? | Pass, with no workaround. WebGL2 works. |
| S6 | Does tmux control mode keep up with 10 noisy panes? | Pass, with tile panes paused. |
| S7 | Does Claude Code work against a local endpoint through `ANTHROPIC_BASE_URL`? | A llama.cpp or Ollama server runs a pane and a guide session with tool calls. |

## 19. Tests

- **Core**: unit tests for the hook-to-lamp state machine, the unseen rules, the tally order and the leader timeout.
- **`sb state`**: tests for every hook input shape, the atomic state file, and the no-socket case. A timing test for the 50 ms limit.
- **tmux**: integration tests against a real `tmux -L sb-test` server: open, output, pause, resume, attach again.
- **Store**: migration tests from every past `user_version`. A restart test that kills the app and checks every tile.
- **MCP**: tests for `guide_set_steps` validation with good and bad paths and lines.
- **Review**: a recorded `stream-json` run replayed into the review engine. No live API in CI.
- **Anchors**: re-anchor tests across a head change, including a removed line.
- **Connections**: `sb connect add` on a temp `HOME`. The symlinks resolve. The token never reaches the config file or a log.
- **Window**: a few Playwright-free DOM tests in plain JS for the board and the review columns. They run under `node --test` with no npm packages.

## 20. Build plan

Each step is one PR.

1. **Shell and one pane.** Tauri window, the tmux server in control mode, one live xterm.js pane.
2. **State.** `sb state`, `sb permit`, `hooks.json`, `SB_PANE`, state files. With no UI: `notify-send` when a lamp turns on.
3. **Board.** Tiles, lamps, unseen state, tally, expand to the live pane, the key map.
4. **Launcher and worktrees.** Repo list, worktrees, `sb open`.
5. **Connections.** `sb connect add`, the active connection, endpoints, the Limit lamp, switch and resume.
6. **nvim.** One nvim for each worktree, RPC, `checktime` after edits.
7. **Store and restart.** SQLite, attach after a restart, layouts.
8. **Memory.** RSS on tiles, end idle sessions, the plugin only for Rust panes.
9. **Review: fetch.** PR URL, fetch and worktree, head polling.
10. **Review: guide.** `sb mcp`, the read-only guide session, the steps and diff columns.
11. **Review: threads.** Anchors, forks, follow-ups, pins.
12. **Drafts.** Draft, edit, copy all.
13. **Packaging.** PKGBUILD.

PRs 1 and 2 already help: the lamps reach you as desktop notifications.
PRs 1 to 5 give the full board.

## 21. Repository layout

```
switchboard/
├── SPEC.md
├── Cargo.toml              # workspace
├── crates/
│   ├── core/               # tmux, state, review engine, store
│   ├── sb/                 # the CLI: state, permit, open, connect, mcp
│   └── app/                # Tauri 2 shell
├── web/                    # HTML, web components, vendored xterm.js
├── prompts/
│   └── review.md
└── spikes/
```

## 22. Decisions

| ID | Question | Recommendation |
|---|---|---|
| D1 | tmux under the hood, or the app owns the terminals? | tmux. Agents live on when the app closes. |
| D2 | The leader key. | `Ctrl+Space`. It is free in Claude Code and most shells. A tmux prefix clashes with a nested tmux. |
| D3 | On a usage limit: a switch button, or an automatic switch? | A button first. |
| D4 | The LLM for review threads. | Decided: the active connection (8). |
| D5 | The name. | Decided: Switchboard. |

## 23. Build notes

Where the build differs from the text above:

- `time` in a state message is seconds since the Unix epoch.
- `idle_end_minutes` is 0 (off) by default. Ending a session is safe, because the tile resumes it, but it is a surprise the first time.
- The guide session has no board tile. Its state shows in the review header.
- A connection with no `dir` uses your own `~/.claude` and its login.
- A thread fork and the pin and draft summaries use the active connection.
- `--allowedTools`, `--disallowedTools` and `--mcp-config` take lists. The prompt of a `claude -p` run goes first, or one of them takes it.
- Every `nvim --server` call times out after 3 s. An nvim that waits at "Press ENTER" never answers.
- tmux `%pane:off` stops tmux from reading the pane. Tiles use `pause`.
- A hidden keeper pane (`@sb_keep`) keeps the tmux session alive when every tile closes.
- `remain-on-exit` is on for the whole server, so a pane that fails at once still shows its error.
- After an expand, the app changes the size by one row and back. The program in the pane then draws its whole screen, and output lost in the expand gap does not matter.
- A key typed in a pane clears the tile's permit buttons: you answered in the pane.
- Rename works on the live title (double-click). A click on a tile expands it.
- Drag a tile onto another tile to move it. A move of less than 6 px is a click. The order is saved in the store (schema version 2) and kept after a restart.
- Each `claude -p` run is killed after 30 minutes. Its stderr goes to its own thread.

Tests:

- 61 core unit tests, 11 tmux tests on real servers, 5 MCP tests.
- An end-to-end test: hub, tmux, the `sb` binary, hooks, permits, worktrees, nvim, layouts, a crash, and a board with every tile closed.
- A review of the code by a second agent found 15 bugs and no deadlocks. All 15 are fixed.
- 50 window checks in headless Chromium (`web-tests/run.sh`), with a stand-in for the Tauri API.
- Two live tests, ignored by default because they cost tokens: a review of a real PR (`live_review`), and a connection switch with resume (`live_switch`). Both passed on 2026-10-08.
- A smoke test of the real app: `sb open`, the trust dialog, a real prompt to Your turn, a restart.

Not tested yet:

- The real window by eye: drawing, fonts, and keys in a live xterm.js pane.
- A real usage limit (S1) and a second account (S4).
- An endpoint connection (S7).
- New commits during a review (11.7) against a real PR.
- Desktop notifications when the window is in the background.

## 24. Verified guide check

The LLM proposes the review guide. A small checker decides if you see it. The checker is the crate `guide-check`, and `proofs/` proves it in Lean with Aeneas.

### What is proved

`Statements.lean` holds the approved statements. `scripts/check-proofs.sh` regenerates the Lean from the Rust, builds the proofs, and allows only the axioms `propext`, `Classical.choice` and `Quot.sound`.

- **G1.** `check` never panics, and it returns `true` exactly when the guide is accepted (`Accept` in `proofs/GuideCheck/Spec.lean`).
- **G2.** In an accepted guide, every added and every removed line is in a step range of the same file and side.
- **G3.** In an accepted guide, the lines the diff does not mark are the same in the old and the new file, in the same order. So the diff names every change.
- **G4.** In an accepted guide, a file where the diff marks no line did not change.

`Accept` also needs every range to hold a changed line, and every line of a range to be at most 20 lines (`PAD`) from a changed line.

### The flow

1. The app fetches the PR. The head must equal GitHub's `headRefOid`. The base is the merge base of GitHub's `baseRefOid` and the head, as on GitHub.
2. The app reads every changed file at both commits from git, and the changed line numbers from `git diff -U0`. Lines keep their newline byte.
3. The verified rebuild check runs on every file. If one fails, the review stops. A wrong diff parse can never reach you.
4. git's counts are compared with GitHub's counts for each file. A difference shows as a warning in the header.
5. `guide_set_steps` runs the verified `check`. A refused guide goes back to the agent with every missed line and every bad range. After 3 refusals, the app keeps the good ranges and adds a step "Not in the guide" with every missed change.
6. When the agent ends, the app checks the stored guide again. You only ever see a guide that `check` accepted.

### What stays trusted

- git returns the right file content for a commit. git checks object hashes itself.
- The window draws the accepted guide and diff. It draws from the same data, with no second copy.
- Binary files have no lines. A step must name each one in `files`. That rule is plain code, not proved.
- What the agent writes about the code. No checker can prove that an explanation is true.

### Tests

- 40,000 random inputs compare `check` with a plain model of the spec.
- The diff model and the rebuild check on the last 50 Lighthouse commits: 334 files, 15,819 changed lines, 6 renames, 1 binary file, all pass (`crates/core/tests/real_diffs.rs`).
- A live review of a real PR: the agent's guide passed the checker with no added step.
