# Spikes

Each spike answers one question from SPEC.md section 18. Run them from this folder.

| ID | Status | Result |
|---|---|---|
| S1 | Waiting | Docs answer half. A real limit event is needed. |
| S2 | Pass | A blocking `PermissionRequest` hook works in `-p` and in an interactive pane. |
| S3 | Pass | `--fork-session` leaves the parent byte-equal. A follow-up keeps the fork id. |
| S4 | Waiting | Needs the second account logged in. |
| S5 | Pass | A Tauri 2 window runs on GNOME Wayland with no workaround. WebGL2 works. |
| S6 | Pass, with a change | Pause tile panes. Do not use `off`, and do not rely on `pause-after`. |
| S7 | Blocked | No local model server is installed. |

## S1: usage limit (`s1_capture.sh`)

From the hooks docs (code.claude.com/docs/en/hooks, 2026-10-08):

- `StopFailure` input has `error`, `error_details` and `last_assistant_message`.
- `error` is one of `rate_limit`, `overloaded`, `authentication_failed`, `oauth_org_not_allowed`, `account_on_hold`, `billing_error`, `invalid_request`, `model_not_found`, `server_error`, `max_output_tokens`, `cloud_credential_error`, `unknown`.
- No field gives a reset time.
- Claude Code can wait for a claude.ai usage limit and continue at the reset (`autoContinueAtUsageLimit`). Three `Notification` types report it: `quota_auto_resume_fired`, `quota_auto_resume_stale`, `quota_auto_resume_disabled`.

Open: does a subscription usage limit fire `StopFailure` with `rate_limit`, or does the session wait with no `StopFailure`?
Next: add `s1_capture.sh` to the `StopFailure` and `Notification` hooks. Read `~/.switchboard/s1.log` after the next real limit.

Spec impact: the Limit lamp has two sources, `StopFailure` `rate_limit` and the quota notifications. A pane that waits for the reset is not an Error.

## S2: permission from the tile (`s2_permit.sh`, `s2_permit_tui.sh`)

- `-p` mode: the hook blocked for 8 s, then "allow" ran the command. "deny" blocked it, and Claude saw the denial.
- Interactive pane in tmux: the Claude dialog showed in the pane while the hook waited. An outside "allow" closed the dialog and ran the command.
- So the tile and the pane both work. The first answer wins.

Findings:

- `sb permit` needs no 10-minute fallback. The pane always shows the dialog. `sb permit` can wait up to the hook timeout (600 s default).
- A new folder shows the trust dialog first. The app must report it as Needs you, or trust new worktrees through config. A plain Enter picks "No, exit".
- Your default permission mode is auto. In auto mode, most tools never reach `PermissionRequest`.
- Claude asks for tmux `focus-events on`. The Switchboard tmux server sets it.

## S3: fork (`s3_fork.sh`)

- Two `--resume <id> --fork-session` calls each got a new session id and the full context ("lantern", "nretnal").
- The parent session file was byte-equal before and after.
- `--resume <fork-id>` with no fork flag kept the fork id and its context ("LANTERN").

Finding: `claude -p` waits 3 s for stdin when stdin is not a terminal. The runner passes `</dev/null`.

## S4: resume across accounts

Needs `~/.claude-b` with `/login` done for the second account. Then: start a session on A, resume it on B, check the context. Run two live panes on A and B for 10 minutes and check `projects/`.

## S5: Tauri on Wayland (`s5_tauri/`)

Build: `CARGO_TARGET_DIR=~/.cache/switchboard-spike-target cargo build` in `s5_tauri/`. The first build took a few minutes. A rebuild took 45 s.

- GNOME on Wayland, webkit2gtk-4.1 2.52.6, Tauri 2.
- The page ran its script, drew to a 2D canvas, got two animation frames and called a Tauri command. Exit code 0.
- WebGL2 is available. The same result came with and without `WEBKIT_DISABLE_DMABUF_RENDERER=1`.
- `document.title` does not change the window title. The app sets it through the Tauri API.

Spec change: the DMABUF workaround is not needed on this machine. xterm.js can use its WebGL renderer, with canvas as the fallback.
This spike checks by script. It does not prove that the compositor shows the pixels. PR 1 gets a visual check.

## S6: tmux flood (`s6_tmux_flood.py`)

One control client, one live pane printing every 50 ms, 10 panes of `yes`, 10 s:

| Mode | Bytes to the client | Live latency p50 / p99 | Max gap |
|---|---|---|---|
| `pause-after=1` | 203 MB | 11 / 60 ms | 88 ms |
| No flow control | 244 MB | 9 / 16 ms | 62 ms |
| Tile panes `%N:off` | 0.1 MB | 1 / 5 ms | 63 ms |

- `pause-after` never paused a pane. The reader kept up, so tmux streamed everything.
- `refresh-client -A '%N:off'` stops the stream for a pane. `capture-pane -p -e -S -4` still reads a tile in 11 ms.
- The tmux server used 95% of one core for 10 `yes` panes in every mode. That is the cost of the programs, not of the client.

Correction (PR 1 tests): `off` is wrong for tiles. When every client turns a pane `off`, tmux stops reading it.
`capture-pane` then shows nothing new, and a busy program blocks on a full terminal buffer.
The 11 ms capture above read text from before the pane went `off`.
`refresh-client -A '%N:pause'` is right: tmux keeps reading the pane, `capture-pane` shows new text, and nothing streams.
`%N:continue` streams again from new output only. A trace with `cat` showed this.

Spec change: tiles are paused and read with `capture-pane`. Only the live pane streams. `pause-after` stays as a guard.

## S7: local endpoint

Needs a local server that speaks the Anthropic Messages API, for example llama.cpp server or Ollama. None is installed.

## Repo knowledge, phase 0 (`knowledge/`)

Question (SPEC 35.16, phase 0): do past review threads of a repo make Claude's review of a new PR catch more of what real reviewers raised?

Run on 2026-11-16, from commit 14db719, on sigp/lighthouse. Python 3, `gh`, `git` and `claude -p` only. Data is in `~/.cache/sb-know-spike/`, not in git.

- `mine.py`: GraphQL search in 14-day merge slices, newest first, pages of 10, follow-ups for threads, comments and commits.
- `actedon.py`: fetch PR heads into a private bare clone, `acted_on`, weights, model-made labels.
- `track.py`: the line tracker on `git diff -U0 --diff-algorithm=histogram -M50%`. `test_track.py` checks it against 400 random git diffs: 0 bad.
- `replay.py`: the replay of SPEC 35.13, small.

### Counts

- 200 merged PRs with at least 1 review thread, merged 2026-04-28 to 2026-11-16. About half of all merged PRs had a thread.
- 1,072 threads (1,067 line, 5 file), 1,927 comments.
- First comment's role: MEMBER 1,043, CONTRIBUTOR 19, NONE 10. In lighthouse almost every reviewer is a member, so the role weight barely sorts anything.
- GitHub has no merge method field. Inferred from git: 199 squash, 1 merge commit.
- All 2,940 commits needed were fetched: 36 by id after the PR heads, none lost.

### acted_on and weights

- acted_on: 768 yes, 257 no, 47 unknown (42 LEFT side, 5 file threads). 36 by a landed suggestion, 12 by a deleted file.
- Weight 0: 64 threads by the PR's author, 1 "lgtm". No bot threads. 1,007 of 1,072 (94%) have weight above 0.
- Heuristic against 50 labels made by Claude (sonnet), not by a person. The 50 are in `knowledge/labels.csv` for a hand check.

| | label yes | label no |
|---|---|---|
| heuristic yes | 25 | 10 |
| heuristic no | 6 | 9 |

Accuracy 68%. The heuristic says yes when the code near the line changed for another reason, or the comment was a question. It says no when the fix landed away from the line.

### Replay

12 newest PRs with at least 2 weighted target threads (one more was skipped for a diff over 120 KB). A target is a human thread whose line maps to the PR's first commit: 122 of 243 weighted threads in the newest 60 PRs map. Each review ran once per condition in a temporary clone with only the base and first commit, tools Read, Grep and Glob. The tool calls were checked: no read of the data or of later code (one read of `~/.cargo/registry`, one of a wrong temp path).

B adds up to 25 past threads on the same files, from PRs merged before the PR was opened, each comment older than that. Place: a note within 3 lines. Concern: the judge (sonnet) on notes within 10 lines. Far: notes with no target within 10 lines.

| PR | targets | past threads in B | A place | A concern | A notes | A far | B place | B concern | B notes | B far |
|---|---|---|---|---|---|---|---|---|---|---|
| #10071 | 2 | 25 | 1 | 1 | 1 | 0 | 0 | 1 | 1 | 0 |
| #10396 | 2 | 25 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| #10082 | 3 | 25 | 0 | 0 | 3 | 2 | 0 | 0 | 2 | 2 |
| #10407 | 2 | 2 | 2 | 2 | 2 | 0 | 2 | 2 | 2 | 0 |
| #10385 | 2 | 11 | 0 | 0 | 2 | 2 | 0 | 0 | 3 | 3 |
| #10352 | 2 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 1 | 1 |
| #10268 | 3 | 25 | 0 | 0 | 0 | 0 | 1 | 1 | 1 | 0 |
| #10162 | 6 | 25 | 0 | 0 | 2 | 2 | 0 | 0 | 3 | 3 |
| #10182 | 5 | 25 | 1 | 0 | 3 | 1 | 1 | 0 | 2 | 1 |
| #9729 | 2 | 0 | 0 | 0 | 1 | 1 | 0 | 0 | 1 | 1 |
| #10207 | 3 | 5 | 0 | 0 | 1 | 1 | 0 | 0 | 1 | 1 |
| #10183 | 2 | 1 | 0 | 0 | 2 | 1 | 0 | 0 | 1 | 0 |
| total | 34 | | 4 | 3 | 17 | 10 | 4 | 4 | 18 | 12 |

Recall by place: A 12%, B 12%. Recall by concern: A 9% (3 of 34), B 12% (4 of 34). The gap is one thread. 12 PRs, one run each, is far too small to tell this from chance.

Claude runs: 35 of the cap of 60. 5 for labels (10 threads each), 24 reviews, 6 judge runs.

### What surprised me

- Most target threads are not bugs: questions, nits, naming, docs, "why did you change this", even praise. A review told to report "real problems only" gives 1.4 notes per PR and cannot hit them. Recall near 10% says more about what the target is than about the knowledge.
- The first commit is often a small part of the PR. Several notes say "the diff does not do what the description says": true for the first commit, noise for the score.
- Place scoring misses real hits. In #10162 the B note on the ban filter running after the top bid is picked is the same concern as a target thread, but 49 lines away.
- The notes were good and specific. Most far notes look like real issues the reviewers did not raise.
- 5 of 12 PRs had few or no past threads on their files (0 to 5): with 6 months of history, same-path matching is thin.
- acted_on at 68% is weak. A rebase or merge from base does not seem to be the main error: nearby edits for other reasons are.

### Verdict

Not yet clear. B was never worse and got one more concern hit, but that is within noise on 12 PRs and 34 targets. Before the bigger plan, a second phase 0 round should: score against weighted threads that raise a problem (or let the review give all kinds of notes), use the PR's last commit before the first review instead of its first commit, widen place matching, run each case twice, and use 30 or more PRs. acted_on needs a better signal than a line window before it gets 3 times the weight.
