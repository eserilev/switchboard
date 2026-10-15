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
- Windows. Switchboard targets Arch Linux on Wayland first, and runs on macOS too (23).
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

### 6.7 Sizes you drag

Handles size the parts of the window (`web/split.js`):
- In a review, one handle between the steps and the middle column, and one between the middle column and the Claude column. Each side column keeps at least 160 px; the middle keeps at least 320 px.
- On the board, one handle between the live terminal and the tiles. The terminal keeps at least 240 px; the tiles keep one row (160 px). A pointer drag ends on whole rows of tiles, unless that takes the terminal's room; the keys move by pixels.
- A handle is a 10 px area with a 1 px line and a three-dot grip, gold on hover and focus. It is a WAI-ARIA separator, with its size, its limits and the element it controls.
- Keys on a handle: the arrows move it 16 px, Shift 64 px; Home and End go to the smallest and the largest size; Enter goes to the smallest size and back. A double-click resets the column of that handle only.
- Sizes are kept in the window's local storage, one set for every review tab. A smaller window shrinks both side columns in proportion, never below their minimum (`fit`).
- The rules are two pure functions, `side` and `fit`. A property test runs 5000 random cases against their rules: a side keeps its minimum, the middle keeps its minimum when there is room, a size that fits stays as asked, and `fit` is stable.
- The live terminal refits on every size change and on every window resize (full screen and back). Its box shrinks with its column: before, the box kept the terminal's old width, the fit read that width again, and after full screen the terminal shrank one column at a time while the program in the pane drew for the old width.

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
| Leader, `t` | Tasks tab (27). |
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
3. The PR head is fetched into a private ref (`refs/switchboard/fetch/...`), read, and the ref is deleted. Then the branch `sb-pr-<n>` points at the head, for you. Nothing in the app reads that branch: a close of another review of the PR can delete it. The base is fetched the same way, so two fetches at the same time do not share `FETCH_HEAD`.
4. Under the tree lock, and only while the review is open: `git worktree add --detach <repo>-pr<n> <head>`, or `<repo>-pr<n>-r<k>` for round k (11.9). An existing folder gets `git checkout --detach <head>` only when it is a detached worktree of this clone with no work in it (11.10). Else the review shows an error: a folder of yours with that name, or a kept folder with your changes, stays as it is. The review's own folder is the exception: an Update moves its HEAD as before, git carries your edits along, and a conflict stops it with git's error.
6. Before an Update fetches, the review's current head is pinned, so a force-push never makes it look like a local commit.
5. The review's head and base get refs of their own: `refs/switchboard/<review>/head` and `.../base` (11.10).

If a closed round of the PR exists and no review of it is open, the launcher offers **Round N+1 of #n** first, and **New full review** below it.

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
| Guide + diff | The current step: what the code does, what to check, pins, context links. Under it, the diff at the PR head: only the parts of each file that the step's ranges name, with 6 rows of context. A bar counts the other changed lines of the file, links the steps that cover them, and opens the whole file in the Files view. Each gap in a diff has buttons that show 20 hidden lines above or below, or the whole gap. The hidden lines come from `all_rows`: the proved numbering with no cut. |
| Claude | A Claude Code pane for questions (11.5). Below it, the drafts. |

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

### 11.5 Claude pane

The question column is a real Claude Code pane, as on the board. It replaces the threads below in the window.

1. The pane starts when the guide is ready: interactive `claude` in the review worktree, with `--resume <guide> --fork-session`, so it has the context of the review. The guide session file does not change (S3).
2. Its first message names the current step and asks Claude to wait for a question.
3. It gets the spec clones with `--add-dir` and the pane hooks. It runs on the connection of the guide session, because the session file is in that connection's folder. It has no `sb` MCP tools, so it cannot change the guide.
4. It runs with the rust-analyzer plugin off: it answers questions, and rust-analyzer takes 1 to 4 GB for each copy.
5. One pane for each review worktree and PR number, titled `Ask #<number>`. It shows as a tile on the board too. The app finds it again by its title and worktree. After a rename, the app starts a new one. An ended pane with no session id is not used again.
6. With `--fork-session`, the hooks report the new session id (checked on 2026-10-11). A later resume of the pane uses that id, so it never writes into the guide session. A resume does not pass `--add-dir` again.
7. The live terminal moves into the column while the review tab shows. Another tab takes it back.
8. The chat of each step. Claude Code runs on the alternate screen, so its history is not in tmux and not in the terminal's scrollback. The app reads the pane's session file (`<claude dir>/projects/*/<session>.jsonl`, the session id from the SessionStart hook) from the review's first message on, and splits it into questions and answers (`chat::turns`). A question belongs to the step in its own step line (Send step), else to the step you viewed when you asked: the window records each step you view (`step_seen`, store version 9). The newer of the two wins. Above the terminal, "On this step" lists the questions of the step, the last one open; "All steps" lists every question with its step. The list loads again when the state of the review's Claude pane changes (it asked, it answered), not on the output of other panes; a chat that did not change draws nothing, so an answer you opened stays open. Claude Code's own lines (commands, reminders, a compact summary, a sub-agent's lines) are not questions. After `/clear`, Claude starts a new session file with no review start, so the list shows nothing new. Tests: `chat::tests` (500 random chats keep every question once, in order, with its step and its answer), and `crates/sb/tests/review_chat.rs` (a real hub, a hook session, a session file, step views).
9. "Send step" types one line with the current step into the prompt, without Enter. A click on a line number types the path, the line and its text. The text goes in as one bracketed paste, with control characters changed to spaces. While the pane asks for a permission or trust, nothing is typed: a "1" in the text would pick an answer.
10. The guide agent writes the step fields, and the first message goes into a tmux command. tmux reads one command per line, and its quotes do not hide a line break. So the step line has no control characters, and the tmux client refuses any command with a line break.

The threads below still exist in the store and the backend. The window no longer starts them.

### 11.5.1 Threads (old)

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
- **Draft** makes a review comment from an answer: full path, line, side, and text. You can edit it. The window marks it as an agent draft.
- You can also write a comment yourself. Hover a diff line and click **+**. Shift-click a second **+** on the same side to comment on a range. **Add to review** keeps the comment as a pending draft.
- The core accepts a comment only on a line that exists in the diff model at the reviewed head.
- Every diff reply carries the head of the model that drew it. The window refetches a diff whose head is not the review's head, and shows "Loading the new commit" meanwhile: during an update the server loads the new model for a while. A **+** comment and a question on a line send the head of the diff on screen and the text of the clicked line. The core refuses both when the head or the text differs.
- Drafts show under their lines in the diff and in the list under the threads. You can edit or delete each one.
- A draft longer than two lines gets a warning.
- **Copy all** copies the drafts as `path:line: text`, one for each line.

### 11.7 New commits

- The app polls `headRefOid` every 2 minutes while the review tab is open.
- When the head changes, the top bar shows the old and the new head.
- Steps whose files changed get a stale mark.
- "Diff since last review" shows `git diff <old>..<new>` for each stale step.
- Threads on a line move with the rule of the drafts below: a thread keeps its head, its line text and up to 2 lines of code around it. If the code is gone, the thread shows "line removed" and keeps the old line.
- Drafts move too. A draft keeps the head, the exact text of its lines, and up to 2 lines of code above and below them.
  - Every model load moves the drafts: update, retry and restart. It runs in the same step as the model change, before the threads move.
  - At a new head, a draft moves to a place with the same lines (in order, same spaces) where the code above or below is also the same. A place where both are the same wins over a place where one is. Two places with the same best match are a tie, and a tie is stale. So a draft on a `}`, a blank line or repeated code never jumps to other code. An edit right above and right below the line also makes the draft stale; that is safe, and you place it again.
  - A draft with no stored code around it moves only to the one place in the file with its lines. Store version 7 marks the older drafts on a line stale once, because an older build moved them by number.
  - An agent draft takes the line of its thread only when the thread is at the model's head, its line is not removed, and the line has the text that the question was about. Else it is stale.
  - The send moves the drafts once more before its plan.
  - Not yet: a file renamed at the new head makes its drafts stale.
  - A stale draft shows a note in the list, with **Place again** and **Delete**, and the diff does not show it. **Place again**: the next **+** opens the form with the draft's text, and the add removes the stale draft. The send refuses a stale draft.
- "Update guide" resumes the guide session with the new head and asks for `guide_update_step` calls.

### 11.8 Send the review

**Finish review** sends all pending drafts to GitHub as one review, like "Start a review" on GitHub. Plain code does every step. No agent takes part: the guide session and the forks have no `gh` tool (11.2), and no MCP tool sends anything.

1. You write a summary and pick the type: Comment, Approve, or Request changes. The summary is saved as you type.
2. The app checks the plan (`post.rs`):
   - The PR head on GitHub must equal the reviewed head. Otherwise the app asks you to update the guide first.
   - The app gets GitHub's own diff of the PR (`gh api`, diff media type).
   - For each draft, every line of its range must be in the same hunk of GitHub's diff, on the same side.
   - The text of each line must be the same in GitHub's diff and in the verified diff model. A difference stops the whole send.
   - A draft with no line, or with a line that GitHub's diff does not show, goes into the summary as `` `path:line`: text``. GitHub refuses a line comment there.
   - A draft on a file or line that the model does not have stops the send.
   - A draft that is stale, that belongs to another head, that has no stored text, or whose stored text is not the text at its lines in the model, stops the send. Without this, a draft from before an update goes on the old line number, which is another line of the new code.
3. The window shows the plan: the GitHub login, the head commit, the comments on lines, the comments that go into the summary and why, and the agent drafts. Nothing is sent before your click on **Submit review**.
4. The send uses the plan that you saw. The app builds the plan again and compares a fingerprint of it. If the plan changed, the app sends nothing and asks you to check again.
5. One `POST /repos/{repo}/pulls/{n}/reviews` call sends the review: `commit_id` is the reviewed head, each comment has `path`, `side` (`RIGHT` or `LEFT`), `line`, and for a range `start_line` and `start_side`. GitHub accepts all comments or none.
6. After GitHub accepts, one store transaction records the review link and removes the sent drafts. The list shows "Sent" with the link.

Limits:
- GitHub refuses an approval of your own PR. The window shows GitHub's reason.
- One send of a review runs at a time.
- If the app stops between GitHub's answer and the store write, the drafts stay. Before the next send, the app reads your reviews on the PR. A review at the same commit, with the same type, summary and line comments, counts as this send: the app records it and does not send again. After new commits the commit differs, so this check does not find it.

### 11.9 Review rounds

After you send a review, the author pushes new commits. A new round reviews only what changed since your round.

- When the head changes (11.7), the header offers **Start round N+1** next to **Update guide**.
- A round is a new review of the same PR, in its own tab (`#10182 R2`) and its own worktree (`pr10182-r2`). The round before stays open until you close it.
- A round can also start after the round before is closed: open the PR again, and the launcher offers the next round (9.2). `since` is then the head of the last closed round with a guide. Its pinned ref keeps that commit, also after a force-push and a `git gc` (11.10).
- `since` is the head that the round before covered. The base of the round's diff is:
  - `since`, when the author only added commits on top of it.
  - `since` with the new base branch merged in (`git merge-tree --write-tree`), when the author rebased or merged the base branch. The round then leaves out the changes that came from the base branch. The header says so.
  - When a file of that merge has a conflict, that file takes its version at `since`, and every other file stays merged. Only the conflicted files can show changes from the base branch, and the header names them. The merge uses a private index file, so the clone's own index does not change.
- The diff model, the verified checker and the drawn rows work on this base and the new head as on any other pair. So G1 to G10 hold for the round: the round guide covers every line changed since `since`, and no other line.
- The guide prompt for a round names the scope, the files of the round, and the comments that you sent in the round before. The agent says in each step whether the change addresses a comment, and lists the comments that no change touches.
- A sent review records its commit, its summary and its line comments, for the next round.
- Tests: git repos for each case (added commits, base merge, rebase, conflict, a conflict with other merged files), and open Lighthouse PRs (`crates/core/tests/real_rounds.rs`), with a commit in the middle of each PR as the round-1 head. On 22 PRs: 12 plain, 6 with a base merge or rebase, 4 with one conflicted file each. No file from the base branch shows, except the conflicted files. Before the per-file rule, those 4 PRs showed up to 266 files of the base branch.
- The GitHub count check (24) is for a first round only, because GitHub counts the whole PR.
- **Finish review** in a round works as in 11.8. A comment on the old side goes into the summary, because the old side of a round is the code of the round before, not GitHub's base.

### 11.10 Close and cleanup

**Close review** ends the review and cleans up after it. The store keeps the review, its drafts and what you sent, for a later round.

1. If the worktree has changes, nothing happens yet. Changes are changed and untracked files (not ignored ones, so `target/` does not count), and commits at HEAD that no branch, tag, remote or pin reaches (`review::local_changes`). The window lists them and offers **Cancel**, **Keep the folder** and **Discard the changes**. Discard sends back the list you saw. A change that is not on it makes the close ask again, and the removal later drops only changes on that list.
2. Under the tree lock, the review leaves `Hub.reviews`. The store gets its last row and its close time.
3. A running guide run gets SIGTERM. Each `claude -p` runs in a process group of its own, and the signal goes to the group, so its tools and its `sb mcp` server stop too. A run that starts after the close stops at its start. The run clears its pid before it reaps the process, so a stale pid is never signalled. At app exit, `stop_runs` stops every running guide run: a process group of its own gets no signal from the terminal.
4. The review's Claude pane closes. Its nvim panes in the worktree close when nvim reports no unsaved file; an nvim that does not answer counts as unsaved.
5. In the background, under the tree lock, `clean_after`:
   - pins the head and the base again;
   - removes the worktree when `cleanup::verdict` allows it: a detached, unlocked worktree that a review row names, that no open review uses, with no other pane in it, and with no change that you did not see and discard;
   - compares folders by their real path, so a path through a symlink matches a pane at the real path;
   - removes with one `--force` at most, so `git worktree lock` always holds;
   - deletes `sb-pr-<n>` when no review of the PR is open and the branch still points at the review's head, and runs `git worktree prune`;
   - when the worktree stays, sends `review_cleanup` with the reason, and the window shows it.
6. At start, the app runs the same cleanup for every closed review with a folder on disk, with no discard. Reviews closed before this version and cleanups cut short by a quit go there.

After **Submit review**, the dialog offers **Close review** and **Keep open**. It does not close by itself: a round 2 often follows, and both choices allow it.

The tree lock (`Hub.trees`) makes the checkout of a review and the removal of a folder exclusive. A fetch makes its worktree only after the network part, under the lock, and only while the review is open. The row takes the path in the same section. So a review closed during its fetch gets no folder, and a close always sees the folder that it has to remove. Lock order: `trees`, then `reviews`.

The pins `refs/switchboard/<review>/head` and `.../base` stay. They are small, and a later round can need any closed round's head.

Tests:
- `cleanup::tests`: 20,000 random fact sets, with locks and partly seen change lists. A removal happens exactly when the folder is safe to remove, and it drops only changes that you saw and discarded.
- `crates/core/tests/cleanup_git.rs`: real git. 40 random worktrees (edits, untracked files, ignored files, local commits, a force-push, a change after you looked) through the rule, the removal and `git gc --prune=now`. No work goes without a choice, and the pinned commits stay. A round after a close, a rebase, a force-push and a gc still gets its base. The test fails when the pin is off. Locks, a kept folder with work, a commit that only a branch or a pin reaches, a folder of yours with the review's name, and symlinked paths each have a case.
- `review::tests::stop_run_stops_the_group`: a tool that a run started stops with the run.
- The end-to-end test also checks that an Update of an open review keeps working with your edit in its folder, and that another review of that folder then refuses it.
- `crates/sb/tests/review_close.rs`: a real hub and tmux, and a local git remote that serves `pull/7/head`. The clone is reached through a symlink and your pane opens at the real path. Close with changes asks and changes nothing; Discard with a short list asks again; Discard closes the Claude pane, keeps your pane and the folder, and sends a note (with a text compare of paths, the folder went: the test fails); the sweep removes the folder after you close your pane; a review closed during its fetch gets no folder; a round starts from a closed round; 25 races of close against the checkout leave no folder, with both orders seen. Before the private fetch ref, this test found a fetch that read the PR branch while a cleanup deleted it.
- `models/review_close.qnt` (26).

### 11.11 Review local commits

An agent commits its work in its own checkout. Before you push, you review those commits in the diff viewer of 11.1, and your comments go back to the agent.

- **review** on a Claude tile, or on the live title, opens a tab named `<repo> commits`. A second click on the same folder opens the same tab.
- The range is `base..HEAD` of the pane's folder (`review::local_range`). HEAD is frozen as a commit id when the review loads. The base is, in order:
  1. the merge base with `@{push}`, else `@{upstream}`, when HEAD has one;
  2. else, with a remote: the one commit where the history leaves every remote (a detached agent worktree), or, when the history leaves the remotes at more than one commit (a merge of remote work), the merge base with the default branch of a remote: `<remote>/HEAD`, `<remote>/main` or `<remote>/master`, with `origin` first;
  3. else the merge base with a local `main` or `master` that HEAD is not on.
  With no unpushed commit, the range is the last commit (the empty tree for a root commit), and the header says so. A shallow clone with no parent of HEAD is an error, so the whole repo never shows as new.
- The tab has Files only: no guide, no agent of its own, no gh. The diff model, the drawn rows, expand, `+` comments and anchors are those of 11.1 and 11.6.
- The right column shows the agent pane itself. A click on a line number types the line into its prompt, as in 11.5.
- **Send to agent** puts every comment that is not stale into the agent's prompt as one bracketed paste, with no Enter: `Review comments on commit <head> (line numbers are at that commit):`, then one `- path:line: text` line for each comment. You read it and press Enter. Nothing is typed while the agent asks for a permission or has ended. The comments then move to a `posted` row with the event `AGENT`, so the tab lists what you sent.
- Every 30 s the app reads HEAD in the folder and compares it with the review's head as it is at that moment. A new HEAD shows **New commits** with **Refresh**. Refresh reads the range again; drafts move with their code, or become stale, as in 11.7. One lock checks that no load runs and starts the load, so two quick clicks start one load. A new base alone (after a push) also loads a new file list.
- **Close review** closes only the tab. A local review never asks about changes, never closes a pane, never pins a ref, and never removes its folder: the folder is the agent's. Three guards hold this: `clean_after` returns at once for a local row, before its pin; `closed_reviews` leaves out local rows, so the start sweep never sees them; and `review_close` skips the change check and the pane close. A fourth, `Facts.in_store` false for a local row, sits behind the first and only matters if it goes. The model of 26 checks this property.
- A second **review** on the same folder opens the same tab, also with a new agent pane there; the check and the insert are one lock.
- PR-only commands (rounds, threads, pins, the GitHub send, the Claude pane of 11.5) return an error for a local review.
- Store version 8 adds `review.kind` (`pr` or `local`) and `review.pane`.
- A local review has no guide, so it has no why, no take and no chat of each step.

Tests:
- `crates/core/tests/local_range.rs`: real git for each rule: an upstream, a detached agent worktree, a merge of remote work (with `origin`, and with a remote named `sigp`), no remote, nothing unpushed, a root commit, a shallow clone, no commit.
- `models/review_close.qnt` (26): `AgentFolderStays`.
- `crates/sb/tests/local_review.rs`: a real hub and tmux, and an agent pane that writes what it gets to a file. Open, Files, a comment, Send to agent (the exact paste, no Enter, the posted row), Refresh after a new commit (the draft moves down one line), and close. After the close, with the pane closed and the folder clean and detached, the start sweep keeps the folder, and its commits still count as work. With the guards off, the test fails.
- Window checks: the tile button, the tab, Files only, the header, the agent pane in the column, Send to agent, Refresh, a new agent pane, no file list while the commits load, and a new base alone.

### 11.12 Why, and Claude's take

The guide agent writes, for each step:
- `why`: why the step matters. `why_split`: why it is a step of its own. The step shows both in one block: "Why it matters", and a muted "Why a separate step".
- `check_answers`: a short answer to each `check` item, shown after the item as "Claude: …".
- `take`: `{verdict, checked, notes}`. `verdict` is `good`, `change` or `question`; `good` needs `checked`, what Claude checked. Each note is `{severity, text, file, side, line}`, with `severity` `must`, `should` or `nit`, and an optional line.

And for the whole PR, `take`: `{verdict, summary, notes}`, for problems that span steps and for missing things.

`take::check` refuses a wrong shape and a note on a line that is not on that side of a changed file. The answer lists every problem of the takes, and the coverage problems of the same guide. A refusal counts as a try, and the guide with the bad parts taken out (`take::strip_bad`) becomes the proposal: if the agent stops, the app completes that. On the last try the app accepts it. A note line of `null` counts as no line. When an update ends with no new guide, the notes on lines that are gone go too. The verified coverage check (24) runs as before; it ignores these fields.

In the window:
- A take bar above the diff, in purple (`--limit`), the color of agent drafts: "Claude's opinion · not verified", the verdict, the counts by severity, the head it is for, Show list and Re-run. Green stays for checked steps and the proved coverage line.
- Each note with a line shows under its line, as an agent card with its severity, and a dot in the line number. Notes with no line, or with a line that is not on screen, are in the take bar's list.
- Note actions: Add as comment opens the comment form on the line with the note's text, and the comment saves as an agent draft. Ask types the note into the Claude pane's prompt. Dismiss folds the note to "A dismissed note", with Show and Undo; the store keeps the dismissed keys (store version 9).
- Re-run asks the guide session for the take again, with the notes you dismissed, so it does not write them again (`review_retake`). For a step it uses `guide_update_step`, and only that step must pass the take check; for the PR it uses `guide_set_steps`. The check of "running" and the switch to "updating" are one lock, so a double click starts one run. A dismissal names the note by its text: a note that a Re-run rewords shows again.
- The steps list shows the worst severity and a count, or "Claude: no notes" for a good step. The header shows the PR verdict; a click opens its notes.
- While the head has new commits that the guide does not cover, the take shows "stale".

Tests: `take::tests` (a full take, every problem listed, 5000 random note lines against the model, `strip_bad`), and window checks for the why block, the answers, the take bar, the notes under their lines, Show list, the steps badge, the PR verdict, Ask, Re-run, Dismiss and Undo, and Add as comment as an agent draft.

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

[tasks]                         # SPEC 27
sprint_days = 14
branch_prefix = "sb/"
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

- macOS runs Switchboard too:
  - With no `XDG_RUNTIME_DIR` (macOS), the socket folder is `/tmp/sb-<uid>`. The macOS temp folder is too long: a socket path has at most 104 bytes there. The app makes the socket folder private (mode 700), and stops when the folder belongs to another user.
  - An app that starts from Finder or the Dock has a short `PATH`. At start, the macOS app takes the `PATH` of your login shell (`$SHELL -l`, at most 5 seconds) and adds `/opt/homebrew/bin`, `/usr/local/bin`, `~/.local/bin` and `~/.cargo/bin` when they are missing.
  - Links open with `open`, not `xdg-open`.
  - The rust-analyzer memory on a tile comes from one `ps` table, because macOS has no `/proc`.
  - The leader key comes from `leader` in the config, for example `ctrl+space`, `ctrl+a` or `cmd+;`. On macOS, `ctrl+space` can switch the input language; then set another key.
  - `scripts/macos-app.sh` builds `Switchboard.app` with cargo, `sips`, `iconutil` and an ad-hoc `codesign`. No npm and no Tauri CLI. To share the app, sign it with a Developer ID and notarize it.
  - `scripts/install.sh` installs on Linux and macOS (`INSTALL.md`). It checks the tools, installs the missing ones with `--deps`, takes the latest release when one exists for the system or else builds from source, installs the binaries (Linux) or `~/Applications/Switchboard.app` (macOS), and writes a starter config with the code folders it finds. On macOS the starter config sets `leader = "ctrl+;"`.
  - `.github/workflows/release.yml`: a `v*` tag builds `switchboard-linux-x86_64.tar.gz` (Ubuntu 22.04) and `Switchboard-macos-arm64.zip`, and puts them on a GitHub release.
  - `.github/workflows/macos.yml` runs clippy, the tests with tmux from Homebrew, the window checks in Chrome, and the app build on a macOS runner. It runs when the repo is on GitHub.
  - From Linux, `cargo clippy --target aarch64-apple-darwin` checks the macOS code. A C compiler that does nothing lets the SQLite build step pass, because a check does not link.

- `time` in a state message is seconds since the Unix epoch.
- `idle_end_minutes` is 0 (off) by default. Ending a session is safe, because the tile resumes it, but it is a surprise the first time.
- The guide session has no board tile. Its state shows in the review header.
- A connection with no `dir` uses your own `~/.claude` and its login.
- A thread fork and the pin and draft summaries use the active connection.
- A review run stops at once when Claude reports that the `sb` MCP server failed, and before the start when the `sb` binary is missing. A missing `sb` once made the agent run on without its guide tools.
- Review runs get each spec clone with `--add-dir`.
- Start agent waits for the trust dialog, a permission prompt and Needs you before it pastes the first message. It does not see other startup dialogs of Claude Code, for example "New MCP server found in .mcp.json". A paste into such a dialog has no Enter and no control characters, but its newlines can still move the choice. Answer such a dialog before the 1.5 s after the session starts, or paste the message again.
- Tasks (27): a new sprint opens when no sprint is open. With one open, the new sprint is the planned one. So a planned sprint exists only while a sprint is open.
- Tasks (27): the filter syntax is words, `#tag`, `repo:name` and `prio:high`. A `#tag` word matches the start of a tag, so it works while you type. The filter stays in memory only.
- Tasks (27): the sprint bar shows tasks when no task of the sprint has points. The burndown shows only when it has points. It uses the points of today for every day, and the status history for what was done. It shows the tasks that are in the sprint today: a task done in the backlog before it came into the sprint counts as done from day 1, and a task moved out of the sprint leaves the past days too.
- Tasks (27): Review from a task needs a linked pane that is on the board. A closed pane has no worktree that the hub tracks.
- Tasks (27): the PR poll runs 10 s after the start, then every 5 minutes. GitHub reports merged as final, so the hub does not read a merged PR again until the next start. A `gh` failure only goes to the log. A `gh` that does not end in 30 s is killed.
- Tasks (27): a task edit adds a `points` row, a `link` or `link_removed` row for each link, and one `edited` row for the other fields. Delete and Restore add `deleted` and `restored`.
- Tasks (27): Copy as Markdown lists Done, In review, In progress, Blocked and To do, in that order. When the copy fails, a dialog shows the text.
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
- A diff header `@@ old N · new M @@` gives the number of the next old line and of the next new line, also on a side where the next row has no line. The "No newline at end of file" note follows its line and does not count for the context cut.

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

The diff rows that the window draws come from the kernel too. `number_rows` gives each row a kind and its line numbers, with no text. `cut_rows` keeps the rows near a change. These statements need a file that rebuilds (`Rebuilds`: the first three parts of `FileGood`, so `Accept` gives it):

- **G5.** Every row names the right lines. An old number `n` is a line `1 ≤ n ≤` the old line count: removed for a `-` row, not removed for a context row. The same holds for new numbers. A context row pairs two equal lines.
- **G6.** The text of the `-` and context rows, in order, is exactly the old file. The text of the `+` and context rows is exactly the new file.
- **G7.** Each line comes once. On each side, the numbers of the rows go 1, 2, …, line count, with no gap and no repeat.
- **G8.** The cut keeps every `+` and `-` row and only rows at most `ctx` rows from one. It changes no row and keeps the order. It starts with a header, and a header marks every gap. A header carries the numbers of the next kept row. On a side where that row has no line, it carries the number of the next line of that side.
- **G9.** `number_rows` and `cut_rows` never panic, on any input within the size bound. The cut is correct on any rows with no header.
- **G10.** The kind of every row, and so its color, follows from the masks. A row is `-` exactly when it shows an old line that the diff removes. It is `+` exactly when it shows a new line that the diff adds. It is context exactly when it shows an old line that the diff does not remove and a new line that the diff does not add. Core maps the kind to `-`, `+` or ` ` in one `match`, and the window maps that to a CSS class in one table (`KIND_CLASS`).

A step shows only its part of a file. `step_cut` makes one part per range of the step: the rows that show the range, with up to `ctx` rows of context that stop at another change. Parts that overlap or touch merge, and each part is cut as `cut_rows` does it. It also gives the change rows that the step does not show. For a file that rebuilds:

- **S1.** A step shows all of its own changes: every `+` or `-` row that shows a line of one of the step's ranges is in the step's rows.
- **S2.** Without the headers, the step's rows are rows of the full list, unchanged and in order. So G5, G7 and G10 hold for them. Every row is at most `ctx` rows from a change.
- **S3.** The step's rows start with a header, and a header marks every gap. A header carries the numbers of the next row, counted from the start of the file. When that row shows an old (new) line, the header carries exactly its number.
- **S4.** The hidden list is exactly the change rows that the step does not show, in file order. The shown and the hidden change rows together are every change of the file.
- **S5.** `step_cut` never panics, on any rows with no header within the size bound and any ranges.

G5 to G7 and G10 need `removed + added ≤ usize::MAX` lines. G8, G9 and S1 to S4 need twice that, and S5 needs twice the row count. This is always true: a `Vec<u8>` takes 24 bytes, so a line list has fewer than `usize::MAX / 24` lines.

### The flow

1. The app fetches the PR. The head must equal GitHub's `headRefOid`. The base is the merge base of GitHub's `baseRefOid` and the head, as on GitHub.
2. The app reads every changed file at both commits from git, and the changed line numbers from `git diff -U0`. Lines keep their newline byte.
3. **The file list is checked against the git objects.** `git ls-tree -r` of both commits gives every path with its mode and object id, with no diff code. Every path whose object or mode differs, or that exists on one side only, must be in the model, and the model may have no other path. Else the review stops.
4. The verified rebuild check runs on every file. If one fails, the review stops. A wrong diff parse can never reach you.
5. A file that GitHub lists and git does not change stops the review. A count difference for the same file is a warning, because rename detection can split counts.
6. `guide_set_steps` runs the verified `check`. A refused guide goes back to the agent with every missed line and every bad range. After 3 refusals, the app keeps the good ranges and adds a step "Not in the guide" with every missed change.
7. When the agent ends, the app checks the stored guide again. You only ever see a guide that `check` accepted.
8. **The diff column draws from the model, through the kernel.** `number_rows` walks the model's masks in the order of the rebuild check, and `cut_rows` keeps the rows near a change. The core only gets the text of each row, with one lookup by its number, so a number and its text cannot disagree. There is no second `git diff`.

### What stays trusted

- git returns the right file content for a commit. git checks object hashes itself.
- The window draws the accepted guide and the kernel's diff rows. The line numbers, the row kinds and the cut are proved (G5 to G10), and so are the parts that a step shows (S1 to S5). Not proved: the map from a guide range to a `StepRange` (`side == "old"`), the text lookup by number, the UTF-8 decoding, the "No newline at end of file" note, the header text, and the drawing code.
- In a review round, the base of the diff can be a tree from `git merge-tree` (11.9). The proofs hold for any base and head; the meaning of the round's base (the reviewed code plus the new base branch) comes from git, not from a proof.
- Files with no changed line have nothing for the line checker: binary files, submodules, pure renames and mode changes. A step must name each one in `files`. That rule is plain code, not proved.
- What the agent writes about the code. No checker can prove that an explanation is true.

### Tests

- 40,000 random inputs compare `check` with a plain model of the spec.
- 20,000 random files compare `cut_rows` with a plain model, and check the G5 to G7 facts on the rows.
- 20,000 random files and step ranges compare `step_cut` with the old core code of `step_rows` as a plain model.
- The last 300 Lighthouse commits (`crates/core/tests/real_diffs.rs`): 2,449 files, 119,439 changed lines, 10 renames, 2 binary files. For each commit, the file list matches the git trees, every file rebuilds, the completed guide passes the checker, the drawn rows rebuild both versions of every file, and in the cut with context 12 every line number names the line that its row shows. Next to the old row code, the kernel rows were the same in 2,257 of 2,446 files. The other 189 differed only in header numbers where a side had no line (it was 0).
- On the same 300 commits, each range of the completed guide is drawn as its own step. Every step row is a row of the full diff, every header names the numbers of its next row, and the steps together show every changed line.
- A live review of a real PR: the agent's guide passed the checker with no added step.

## 25. Model check of the permission flow

`models/permit.qnt` is a Quint model of the permission flow (6.5). It follows the code, and each action names the function that it models. `scripts/check-models.sh` runs the checks. It needs `quint` 0.32 or newer, and no Apalache: the checks are random runs (`quint run`), not a proof.

### The protocol in the code

- Claude runs the `PreToolUse` hook (`sb state working`), then the `PermissionRequest` hook (`sb permit`). The dialog shows in the pane at the same time. The first answer wins (S2).
- `sb state` writes its line and exits. It does not wait for the hub. So the hub can read a `working` line after the `Permit` request that came after it.
- `sb permit` sends `Permit` and waits for one reply line. The hub thread of that connection runs `on_permit`. The id is `{pane}-{n}`, with `n` from a counter. Under the `permits` lock, `on_permit` sets `Pane.permit` and puts `(pane, Sender)` in `permits`. A pane whose process exited (`Pane.dead`) gets no buttons, and `sb permit` gets "none".
- A tile click calls `permit_answer(id, allow)`. Under the `permits` lock, it removes the entry of that id and sends the answer on its own channel. Then it clears `Pane.permit` and sets the lamp, but only if `Pane.permit` holds this id. An id with no entry gives the error "the request is gone".
- `answer_permits(pane, None)` removes every entry of the pane, replies "none" to each, and clears `Pane.permit`, all under the `permits` lock. `sb permit` then prints nothing, and the dialog in the pane stays. It runs on `working`, `turn`, `limit` and `error` lines, on a key from `input`, on `close`, when `tick` sees the process exit, and in `respawn_claude`.
- A key in the pane goes to the dialog that shows when the key arrives. It has no request id.
- The hook timeout is 600 s. Claude then kills `sb permit`, and the hub thread waits on until another event clears the entry.
- Lock order: `permits`, then `panes`, then `store` and `live`. No code takes `permits` while it holds `panes`.

### Properties that hold

`check-models.sh` runs four sets of 100,000 random runs of 50 steps, over two panes and four requests: with and without crashes, resumes and closes, and with and without the timing rule below.

- **P1.** A tool runs only after an Allow for that request.
- **P2.** A tile answer applies only to the request that the clicked tile showed. On the hub side, "allow" and "deny" go only to the socket of that request.
- **P3.** At most one answer applies to a request, and the hub writes at most one line on each `sb permit` socket.
- **P4.** A Deny never leads to the tool running.
- **P5.** An answer for pane A never answers a request of pane B.
- **P6.** A tile answer never reaches a request of a later process start of the pane.
- **T3.** A click never takes the buttons of another request.
- **T4.** Each entry in `permits` has its own id.

With the timing rule (the hub reads each `Permit` line before its request ends: before Claude reads an answer key, times out the hook, or exits), these also hold when nothing is in flight:

- **T1.** A request whose `sb permit` waits in `permits` has its buttons on the tile.
- **T2.** The tile shows buttons only for a request that waits.

Without the timing rule, a `Permit` line that a hub thread reads after its request ended shows buttons for that ended request. They go at the next hook event of the pane. A click on them answers only the dead `sb permit`.

P2 and P6 hold because each request has its own socket and channel, so a reply cannot reach another request. P3 also needs the S2 rule: Claude takes the first answer.

### Findings

`models/permit_test.qnt` holds a fixed trace for each finding. The traces of fixed findings now end in a good state. `crates/sb/tests/permit_races.rs` checks F1, F2 and F6 on the real hub; each test failed before its fix. A unit test in `hub.rs` checks F5.

| ID | Finding | Status |
|---|---|---|
| F1 | A `working` line that the hub read between the two halves of `on_permit` took `Pane.permit`, but the entry stayed. The request waited with no buttons. | Fixed: `on_permit` holds the `permits` lock across both. `answer_permits` and `permit_answer` hold it across their `Pane.permit` change. |
| F2 | Claude exited while a request waited. The Ended tile kept Allow and Deny, also after a resume. | Fixed: `tick` calls `answer_permits` on a process exit, and so does `respawn_claude`. `on_permit` gives no buttons to a pane with `Pane.dead`. |
| F3 | You type the Allow key for request 1 in the pane. A tile click answers request 1 first, and request 2 opens. The key answers request 2. | Open. A terminal key has no request id. Do not answer in the pane and on the tile at the same time. |
| F4 | The `working` line of a request arrives after its `Permit`, or a subagent sends `working`. `answer_permits` replies "none", and the buttons go at once. Answer in the pane. | Open. A fix: `sb state` waits for an ack line from the hub, so the hub reads the lines in order. |
| F5 | The id came from `now_nanos`, which can give the same value twice. Then a click on old buttons could allow the new request. | Fixed: the id comes from a counter (`next_permit`). |
| F6 | `permit_answer` set `Pane.permit = None` with no id check. A click on an old entry took the buttons of a newer request of the pane. | Fixed: it clears `Pane.permit` and sets the lamp only when `Pane.permit` holds this id. |

### What the model leaves out

- Time. A trace can delay one thread for as long as it likes, except for the timing rule above.
- Claude itself. The model trusts S2: one dialog for each pane, the first answer wins, and a hook answer applies only to its own request. Two dialogs at once (subagents) are not in the model. After an answer, Claude sends a next hook event (`PreToolUse` or `Stop`).
- Pane answers from a tmux client outside the app.
- The `needs` notification, the lamps other than the buttons, and the store.
- The window draws all tiles from the `pane` events. The model has one copy of each tile, and it can be old.
- Pane ids are not used again after a close.

## 26. Model check of review close

`models/review_close.qnt` models the review map, the folders on disk, the background jobs (fetch, then checkout), close with its three choices, the cleanup threads, the sweep at start, and your panes, locks and numbered changes. It also has a review of local commits (11.11), "l1", whose folder "a1" is the agent's: it exists from the start, has the agent's pane, and holds unpushed commits. The agent commits and pushes at any time. A pin hides those commits from the work check, as `local_changes` does with `refs/switchboard/*`. Two round-1 reviews of one PR share a folder; a round 2 has its own. Each action is one critical section of the code under the tree lock.

The invariant `Always`:
- `OpenKeepsFolder`: an open review that has its folder keeps it.
- `NoLostWork`: a removal never drops a change that you did not see and discard.
- `NoCarry`: a checkout never runs over your work in a kept folder. An Update of a review in its own folder is not a carry: git moves your edits along, as you expect.
- `LockHolds`: a removal never takes a locked worktree.
- `AgentFolderStays`: no close, cleanup or sweep removes the agent's folder of a local review.
- `NoSilentLeak`: when no job and no cleanup is left, each folder on disk is in use by an open review, has a pane of yours, or a note told you why it stays.

A `Mode` turns one fix off at a time: the open check at the checkout, the Discard list, the checkout guard, the lock rule, and the local guards. With the local guards off, the model finds the real bug path at once: the close starts a cleanup, the pin hides the agent's commits, the folder looks clean, and it goes. `scripts/check-models.sh` runs 12 fixed traces and 100,000 random runs of 50 steps of the code. Then it runs each mode with one fix off, and each must break `Always`; the script fails if one passes, so the model keeps the bugs it is for. Removing the open-review check from the cleanup rule also makes the random runs fail.

Not checked: the full state space, because Apalache needs Java, which is not installed here. The model is small (3 reviews, 2 folders), and the random runs reach the bugs above in under a second.

## 27. Tasks

Status: built, in three phases (27.13).

A list of the tasks you work on, as in sprint planning. You write them in Switchboard, set their status, and start agents from them. Tasks live only in the local store. Switchboard never reads them from or writes them to an issue tracker (27.10).

### 27.1 Terms

| Term | Meaning |
|---|---|
| Task | One piece of work: a title, notes, a status, and optional links. |
| Sprint | A named time box with a start date, an end date and a goal. It holds tasks. |
| Backlog | The tasks in no sprint. |
| Current sprint | The open sprint, whatever its dates. At most one sprint is open. |

### 27.2 A task

| Field | Rule |
|---|---|
| Title | One line, required, at most 200 characters. |
| Notes | Markdown, optional. The window renders them with the sanitizer of the review: no raw HTML, and images show as text. A link opens in the system browser, never in the window. |
| Status | `todo`, `doing`, `blocked`, `review` or `done`. The window names them To do, In progress, Blocked, In review, Done. |
| Blocked by | Text, required when the status is `blocked`: who or what blocks the task. |
| Points | An optional whole number from 0 to 99. |
| Priority | `high`, `normal` (the default) or `low`. |
| Sprint | One sprint, or none (the backlog). |
| Repo | One repo folder, optional. Start (27.6) needs it. |
| Links | URLs, for example a PR or an issue. A PR link of a review that is open in Switchboard opens that review tab. |
| Tags | Short words, for a filter. |
| Panes | The panes started from the task or linked to it (27.6). |
| Rank | The order of the task in its sprint, or in the backlog. |
| Times | Created, changed, started (first `doing`), done. |

Rules:

- In an open or planned sprint and in the backlog, every status change is allowed. The window never blocks a move there.
- A task in a closed sprint cannot change, from the window or the socket. "Copy to backlog" makes a new task from it.
- A move to `done` sets the done time. A move out of `done` clears it.
- A move out of `blocked` clears "Blocked by".
- Each change adds a row to the task history (27.8).
- Delete asks first. A deleted task goes to the archive. The Archive view lists it, with Restore. At hub start, the store removes archived tasks older than 30 days.
- A deleted task loses its pane links. Its panes stay.
- When the repo folder of a task is missing, Start is off and the card shows "repo missing".

### 27.3 Sprints

- New sprint: a name, a start date, an end date and a goal. The default length is 14 days and is set in the config (27.11).
- Dates are local calendar dates (`YYYY-MM-DD`). The end date is a sprint day. The dates of two sprints can overlap.
- Days left and the burndown use local midnight.
- At most one sprint is open, and at most one is planned. Plan the next sprint while the current one is open.
- Close the sprint: a dialog lists each task that is not `done`. For each task, or for all of them at once, choose: the next sprint, the backlog, or `done`. Close then sets the sprint to `closed`. The planned sprint becomes open. With no planned sprint, the dialog asks for a new one, or leaves no sprint open.
- A closed sprint keeps its tasks and its history, read-only. It shows in the sprint list with its done points and its total points.
- A sprint has no automatic close. When its end date is past, the sprint header turns amber.

### 27.4 The Tasks tab

The Tasks tab sits next to Board in the tab bar. It has two views. A switch at the top picks one, and the app keeps your choice.

- **Board view**: one column for each status, for the current sprint. A column shows the tasks of one status in rank order. Drag a card to another column or another place. The `done` column shows the last 10 done tasks and a "show all" link. With no open sprint, the board view shows the backlog and a New sprint button.
- **List view**: the backlog and the open and planned sprints, as sections. Drag a task between sections to plan the sprint. Each section header shows the task count and the points: done, and total.

A task card shows the title, the priority mark, the points, the tags, the repo name, and one lamp dot for each linked pane, in the lamp colors of 6.2. A card with a pane that needs you pulses, as its tile does.

The sprint header shows the name, the goal, the days left, and a bar of done points over total points.

Click a card to open its panel on the right: every field, the notes editor, the history, and the actions Start agent, Link pane, Review and Delete.

A filter field at the top filters by text, tag, repo and priority: words, `#tag`, `repo:name` and `prio:high`. Every word must match. The filter applies to both views and to the archive.

### 27.5 Keys

The Tasks keys work only on the Tasks tab, and only when no text field and no terminal has focus. The review keys (7) work only on a review tab. The arrow keys do the same as `h`, `j`, `k` and `l`.

| Keys | Action |
|---|---|
| Leader, `t` | Tasks tab. |
| `j` / `k` | Next or previous task in the column. |
| `h` / `l` | Previous or next column (board view). |
| `o` | New task in the focused column or section. |
| `Enter` | Open the panel of the focused task. |
| `1` to `5` | Set the status: To do, In progress, Blocked, In review, Done. |
| `Shift+J` / `Shift+K` | Move the task down or up. |
| `s` | Start an agent for the task (27.6). |
| `/` | Focus the filter. |
| `Esc` | Close the panel. |

### 27.6 Tasks and agents

**Start agent** opens the launcher (9.1), filled in from the task:

- The repo is the task's repo.
- The place is a new worktree. The branch is `<prefix><slug>`. The prefix is `sb/` by default and is set in the config.
- The slug is the title in lowercase ASCII, with `-` for each run of other characters, at most 40 characters. An empty slug becomes `task-<id>`. When the branch exists, the app adds `-2`, `-3` and so on.
- The pane title is the task title.
- The first message is the task title and its notes. The launcher shows it. You can change it before you start.

The app types the first message into the new Claude pane as a paste with no Enter, as Send to agent does (11.11). You read it and send it. The paste waits until the agent is ready: the pane has its session id from the `SessionStart` hook, no dialog shows, and 1.5 s went by. A new pane is Idle from the start, so the Idle lamp is not a ready signal. The wait is at most 20 s; time in the folder trust dialog does not count. After 20 s, or when the pane ends first, a dialog shows the message with a Copy button. A tab change does not close it.

The paste has no control characters but newline and tab. So a note from an agent cannot end the paste early or press Enter. Send to agent (11.11) uses the same paste.

An agent can write task notes (27.7), so a note from one agent can become the prompt of another. You read the paste before you press Enter.

Start links the new pane to the task. When the task is `todo`, Start sets it to `doing`.

**Link pane** links a pane that already runs. A pane links to at most one task. A link to a second task moves it. A tile has a task menu too: "Link to task…", "Unlink", and "New task from this pane". The **task**, **review** and **nvim** buttons of a tile sit at the end of its info line, under the title, so the title keeps its room. A task from a pane takes the pane name as its title, the pane's repo, the open sprint, and In progress.

A linked pane shows the task title on its tile, under the title. A click on it opens the task.

**Review** opens a review of the local commits of the linked pane (11.11). With more than one linked pane, a menu asks which one.

The app never changes a status on its own, except the `todo` to `doing` move at Start. It shows hints on the card instead:

- "Agent finished": a linked pane went to Your turn, and the task is `doing`. One click sets `review`.
- "PR merged": a link is a PR that GitHub reports as merged. One click sets `done`. Every 5 minutes, the hub reads the PR links of the tasks that are not `done`, with `gh pr view --json state`. It only reads. It never posts (3).

A hint goes away after one click, a status change, or Dismiss. Dismiss lasts until that pane leaves Your turn. The window keeps it in memory only.

When a linked pane closes, the link stays and shows "ended". The link keeps the session id, so Resume on the task resumes the agent (16). A pane that is still on the board resumes in place, only when it has ended, or has an Error and its process is gone: a resume never restarts an agent that runs. A second click while a resume runs fails. A closed pane starts again as a new pane in its folder, with its session and its connection, and the link moves to the new pane.

### 27.7 `sb task`

Agents and scripts read and change tasks with `sb task`. It talks to the hub over the socket (13.4).

```
sb task list [--sprint current|backlog|<name>] [--status <s>] [--json]
sb task show [<id>]
sb task add <title> [--notes <text>] [--repo <dir>] [--sprint current|backlog|<name>]
sb task note <id> <text>
sb task status <id> todo|doing|blocked|review [--by <who or what blocks it>]
```

- In a pane, `sb task show` with no id shows the task linked to that pane (`SB_PANE`).
- `sb task add --sprint <name>` takes the name of the open or the planned sprint. A closed sprint is refused.
- `sb task note` adds the note at the end of the notes, under "**Note from <pane or cli>:**". The history keeps the text.
- `--json` prints the reply as JSON. A failure prints the reason and exits 1.
- The socket never sets `done`, never changes the status of a `done` task, never deletes, and never closes a sprint, whoever calls. Only the window does these.
- The history names the caller's pane only when `SB_PANE` is a pane on the board. Any other value gives `cli`.
- This rule is a guard against mistakes, not a security boundary. An agent pane runs as you and can write the store. `SB_PANE` only labels the history row.
- The task tools are not in the review MCP server. The review agent only reads (4).

### 27.8 History

The store keeps one row for each change: a status, a sprint, the points, a note, a link. Each row has the time and who made it: `you` (the window), `cli`, or a pane id.
The task panel shows the history. A sprint shows its done points for each day as a small burndown line, from the history.

### 27.9 Export

- "Copy as Markdown" on a sprint copies its tasks, grouped by status, with the points and links. Use it for a standup or a sprint review.
- `sb task list --json` gives the same data to scripts.

### 27.10 Not in scope

- An issue tracker sync: GitHub Issues, Jira or Linear. Switchboard keeps its rule: it never posts (3). A later version can import a GitHub issue by URL, read-only, into a new task.
- Subtasks. Use a checklist in the notes.
- More than one person. The store is local.
- Time tracking.

### 27.11 Store, IPC and config

Store version 10 adds these tables. `tags` and `links` are JSON lists.

```sql
CREATE TABLE sprint (id TEXT PRIMARY KEY, name TEXT NOT NULL, goal TEXT NOT NULL DEFAULT '',
  start_day TEXT NOT NULL, end_day TEXT NOT NULL, state TEXT NOT NULL);   -- planned | open | closed
CREATE TABLE task (id TEXT PRIMARY KEY, title TEXT NOT NULL, notes TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL, blocked_by TEXT, points INTEGER, priority TEXT NOT NULL DEFAULT 'normal',
  sprint TEXT, repo TEXT, tags TEXT NOT NULL DEFAULT '[]', links TEXT NOT NULL DEFAULT '[]',
  rank INTEGER NOT NULL, created INTEGER NOT NULL, changed INTEGER NOT NULL,
  started INTEGER, done INTEGER, deleted INTEGER);
CREATE TABLE task_pane (pane TEXT PRIMARY KEY, task TEXT NOT NULL, session TEXT);
CREATE TABLE task_event (id INTEGER PRIMARY KEY, task TEXT NOT NULL, at INTEGER NOT NULL,
  who TEXT NOT NULL, kind TEXT NOT NULL, value TEXT);                 -- who: you | cli | a pane id
```

`start_day` and `end_day` are not `start` and `end`, because `END` is an SQL keyword. `who` is not `by` for the same reason.

Ranks are unique in each sprint and in the backlog. Each add and each move renumbers its list in the same transaction, with a step of 1024. A list holds tens of tasks, so the renumber is cheap.

The window gets the commands `tasks`, `task_add`, `task_edit`, `task_status`, `task_move`, `task_delete`, `task_restore`, `task_copy`, `task_history`, `sprint_new`, `sprint_edit` and `sprint_close`. After each change, the hub sends the whole list as a `tasks` event. A refused change sends nothing. The socket `Request` gets the `Task*` requests of 27.7.

```toml
[tasks]
sprint_days = 14
branch_prefix = "sb/"
```

### 27.12 Tests

- Unit: each rule of 27.2, `sb task` argument checks, the slug and its `-2` suffix, dates at local midnight and at a DST change.
- Property: random actions (add, move, status, close sprint, delete, restore, link) on random tasks. After each step:
  - at most one sprint is open and at most one is planned;
  - each task is in at most one sprint;
  - ranks are unique in each sprint and in the backlog, and the shown order equals the rank order;
  - a task is `done` exactly when it has a done time;
  - a task is `blocked` exactly when it has "Blocked by";
  - a closed sprint holds only `done` tasks and never changes again;
  - a pane links to at most one task;
  - each change has one history row.
- Store: `every_old_version_migrates_with_its_rows` runs from every old version with random rows.
- The socket rule: 3,000 random `sb task` requests, with random ids, statuses and sprints. After each one, no task became `done`, no task is gone, and no sprint changed. A build that lets the socket set `done` fails it. There is no Quint model: each request is one call in one transaction, so the order of clients does not matter, and a model adds nothing that the random requests do not test.
- End-to-end, on a real hub and tmux: Start from a task opens a worktree pane, waits for Idle, pastes the first message with no Enter, links the pane, and sets `doing`. `sb task status <id> done` fails from any caller, with and without `SB_PANE`. A closed pane keeps its link and resumes.
- Window checks: create, edit, drag between columns and sections, the keys, the filter, the close-sprint dialog, the hints. A note with `<script>` and a `javascript:` link renders as text.

### 27.13 Build phases

1. **Tasks and one sprint.** Built.
   - Tasks with title, notes, status, Blocked by, priority, repo and rank. The backlog. One open sprint, with New, Edit and Close. Copy to backlog.
   - The board and list views. Leader `t`, and the keys except `s`, `/` and `Shift+J`/`Shift+K`.
   - The history. Delete with no archive. Store version 10, with the tables of phases 2 and 3.
   - A new task in the Blocked column asks who or what blocks it, as a move to Blocked does.
   - A new sprint lasts `sprint_days` days (27.11).
   - The store does not check that `task.sprint` names a sprint. SQLite foreign keys are off. The rules in `tasks.rs` check it.
   - Tests: 5 unit and property tests in `tasks.rs` (3,000 random steps; a mutant with the closed-sprint guard off fails them), a hub test (`crates/sb/tests/tasks.rs`), and 36 window checks.
2. **Agents.** Built.
   - Start agent (the panel, the `s` key) fills the launcher from the task. Open links the pane, sets `doing`, and pastes the first message when the agent is ready (27.6).
   - Link pane in the panel and in the tile menu, Unlink, New task from this pane, Resume of a linked pane.
   - The task title on the tile, lamp dots on cards and list rows, a pulse when a pane needs you, "repo missing", and the "Agent finished" hint.
   - `sb task` and its socket rule (27.7). History rows `linked`, `unlinked` and `note`.
   - The `[tasks]` config of 27.11 (`sprint_days`, `branch_prefix`), from phase 3.
   - Tests: unit tests for branch names, links and the socket; the random socket test; an end-to-end test (`crates/sb/tests/task_agent.rs`) on a real hub, tmux and git, with a stand-in `claude`: the next free branch, the link, `doing`, no paste before the session, one paste with no Enter, the clipboard path after the wait, `sb task` from the pane (never done), no resume of an agent that runs, and Resume of a closed pane with `--resume`. 32 more window checks.
3. **Planning.** Built.
   - Points (0 to 99), tags and links in the panel. The card shows the points and the tags; a click on a tag filters by it. A PR link of a review open in Switchboard opens its tab; other links open in the browser.
   - The sprint bar of done points over total points, and the burndown line (27.8). Section headers show the points too.
   - Planned sprints: Plan next sprint while one is open. Close names it as the next sprint, and it opens.
   - Copy as Markdown on a sprint (27.9), in the header and on each list section.
   - The filter (`/` focuses it, Esc clears it), for the board, the list and the archive.
   - The archive: Delete moves a task there, the Archive view lists it with Restore, and the hub start removes what is older than 30 days. A task whose sprint closed comes back to the backlog.
   - The "PR merged" hint, from the PR poll, with → Done and Dismiss.
   - Review from a task. With two or more linked panes, you pick one.
   - `Shift+J` and `Shift+K`, also Shift with the arrow keys. `sb task show` prints the points, the tags and the links.
   - The window gets `task_restore`. The `tasks` event has `archive`, `burn` (the status changes of the open sprint) and `merged`.
   - Tests: unit tests for points, tags, links and their history rows, the archive and the prune, planned sprints, the burn, and PR URLs. The random test adds restore and planned sprints; a mutant with two planned sprints and one with a restore into a closed sprint fail it. A hub test for the prune at start and Restore. A hub test (`crates/sb/tests/task_poll.rs`) with a stand-in `gh`: only `gh pr view --json state`, no read of a merged PR again, none for a done task. 48 more window checks; they run in America/New_York, so the burndown check crosses a DST change.
