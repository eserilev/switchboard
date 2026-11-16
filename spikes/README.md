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

## Repo knowledge, phase 0 round 2 (`knowledge/`)

Round 1 was too small and its test too strict. Round 2 fixes the test, not the idea. Run on 2026-11-16, from commit cf0e61b, on sigp/lighthouse. Data is in `~/.cache/sb-know-spike/`, not in git.

New scripts: `mine.py backfill 3` (3 years, resumable), `actedon2.py` (a second `acted_on` rule), `replay2.py` (pick, classify, run, judge, score, check, leaks).

### Counts

- 3 years of merges (2023-11-17 to 2026-11-16): 79 slices of 14 days, 245 search pages, 2,090 merged PRs.
- 941 PRs with at least 1 thread (45%), 5,229 threads (5,196 line, 33 file), 9,428 comments.
- Mining took 275 s, 3 slices at a time. Fetching PR heads and 100 missing commits: about 4 min, none lost. Old `acted_on` and weights: 3 min. New rule: 22 min.
- Weight 0: 451 by the PR's author, 7 bot, 3 trivial, 1 hidden. 4,767 weighted (91%).
- 17 threads (14 in 2026) start with "🤖": review text made by an AI tool, posted from a person's account. None is a target.

### acted_on

Against the 50 labels of round 1. These labels were made by sonnet, not by a person, so this is agreement, not accuracy. 31 of the 50 labels are "yes", so always saying yes agrees 62%.

| rule | yes/yes | yes/no | no/yes | no/no | agree |
|---|---|---|---|---|---|
| old: ±3 lines, net diff to the PR head | 25 | 10 | 6 | 9 | 68% |
| ±3 lines, only plain PR commits after the comment | 25 | 10 | 6 | 9 | 68% |
| ±15 lines, same | 28 | 11 | 3 | 8 | 72% |
| the thread's function, same | 28 | 12 | 3 | 7 | 70% |
| new: function, or the author said "done"/"fixed"/..., or a suggestion landed | 28 | 13 | 3 | 6 | 68% |
| only "done" words or a suggestion | 6 | 4 | 25 | 15 | 42% |

- Leaving out merges from the base and commits from before the comment changed nothing: base merges were not the error.
- A wider window finds more true yes, and as many false yes. The new rule says yes on 83% of all threads, so it barely sorts anything.
- `knowledge/labels_round2.csv`: 20 new threads for a hand label (18 of 20 are "yes" by the new rule). The "human" column is empty.

### Replay setup

- 30 newest PRs with at least 2 weighted target threads. 3 skipped for a diff over 120 KB (#10418, #9729, #10036).
- The snapshot is the PR at its first review: the last commit with a commit time before the first review comment by someone other than the author. GitHub's `pushedDate` is null, so the commit time stands in. When a force push came after the review (3 PRs), the newest commit before the review in the history of that push's `beforeCommit`. Fallback to the first commit: 0 of 30. The snapshot is the first review comment's own commit in 28 of 30.
- Targets: weighted human line threads (RIGHT side) whose range maps to the snapshot with the line tracker: 143. By original commit: 85 the snapshot itself, 3 before it, 55 later but on lines that were already there.
- Sonnet marked 121 of 143 as "raises a problem or asks for a change"; the rest are praise, thanks or pure questions.
- The review writes every kind of note, tagged bug, question, nit, docs or test. Default model, a temporary clone with only the base and the snapshot, tools Read, Grep and Glob, no web, no Bash, `--strict-mcp-config`, no settings.
- B adds the top 30 past threads, fenced and marked as data: same file first, then same folder, then same crate; then weight; then the newest. Only PRs merged before this PR was created, and each comment before that too. The age in the weight is the age at that time.
- B2 is B plus one line per thread: did the author change the code after it (the new rule). Run once on 13 PRs only, for budget.
- Concern: one sonnet judge run per PR. It sees each target thread (up to 3 comments), its hunk, and every note on the same file from all runs of the PR, shuffled, with no run or condition name. It says which notes raise the same concern. Place: a note within 3 lines.

### Replay result

| measure | targets | A | B | B − A | 95% interval |
|---|---|---|---|---|---|
| concern, all | 143 | 32.9% | 36.4% | +3.5 | −2.8 to +11.0 |
| place, all | 143 | 40.9% | 46.9% | +5.9 | +0.7 to +11.6 |
| concern, problem only | 121 | 34.7% | 37.6% | +2.9 | −4.1 to +11.1 |
| place, problem only | 121 | 40.1% | 45.9% | +5.8 | −1.1 to +13.2 |

Recall is the mean of the 2 runs. The interval is a paired bootstrap over PRs (10,000 draws) of pooled recall.

- B2 (one run, 13 PRs, 51 targets), by concern: A 31.4%, B 36.3%, B2 39.2%. Too small to read anything into.
- Noise floor, the same condition run twice: A 35.7% and 30.1%; B 37.1% and 35.7%. Two runs of A differ by 5.6 points, more than B − A.
- By PR: B better on 9, worse on 4, the same on 17.
- The two biggest PRs (#9934 with 22 targets, #10015 with 14) swing it. Without them: A 33.6%, B 40.2%, interval −0.5 to +13.5.
- The knowledge block was full in all 30 PRs: 30 past threads, all from the same file in 26 of them. 3 years is enough history for lighthouse.

### Per PR and noise

| PR | targets (problem) | past threads (file/dir/crate) | A place | A concern | B place | B concern | B2 concern | notes A | notes B |
|---|---|---|---|---|---|---|---|---|---|
| #10071 | 2 (2) | 30/0/0 | 0/0 | 1/2 | 1/0 | 2/1 | 2 | 9/14 | 12/9 |
| #10396 | 2 (2) | 30/0/0 | 2/1 | 0/1 | 2/2 | 1/0 | 1 | 10/9 | 8/8 |
| #10082 | 5 (5) | 30/0/0 | 3/1 | 2/0 | 3/2 | 2/2 | 2 | 9/11 | 11/11 |
| #10038 | 11 (9) | 30/0/0 | 6/6 | 2/2 | 4/5 | 2/2 | 2 | 17/15 | 9/8 |
| #9171 | 2 (1) | 30/0/0 | 2/2 | 2/2 | 2/2 | 2/2 | 2 | 14/14 | 15/14 |
| #10407 | 2 (2) | 8/22/0 | 2/2 | 2/2 | 2/2 | 2/2 | 2 | 4/4 | 3/2 |
| #10385 | 5 (5) | 30/0/0 | 1/0 | 0/0 | 1/0 | 0/0 | 0 | 11/8 | 7/11 |
| #10352 | 2 (2) | 9/21/0 | 1/1 | 1/0 | 1/1 | 0/0 | 0 | 10/8 | 6/8 |
| #10268 | 3 (1) | 30/0/0 | 2/2 | 1/1 | 2/2 | 1/1 | 1 | 12/10 | 12/10 |
| #10162 | 7 (6) | 30/0/0 | 0/3 | 0/1 | 1/4 | 3/3 | 3 | 20/20 | 15/20 |
| #10256 | 2 (2) | 30/0/0 | 0/0 | 1/1 | 0/1 | 1/1 | 1 | 14/13 | 14/15 |
| #10122 | 3 (3) | 30/0/0 | 3/3 | 3/3 | 3/3 | 3/3 | 3 | 14/15 | 11/17 |
| #10182 | 5 (5) | 30/0/0 | 1/1 | 1/1 | 1/1 | 1/0 | 1 | 12/10 | 16/15 |
| #10198 | 4 (2) | 30/0/0 | 2/2 | 2/2 | 2/2 | 2/2 | | 14/20 | 17/17 |
| #10236 | 2 (2) | 30/0/0 | 1/1 | 0/1 | 1/1 | 0/1 | | 15/17 | 11/13 |
| #10207 | 3 (3) | 28/2/0 | 3/2 | 3/1 | 3/3 | 2/2 | | 16/12 | 12/13 |
| #10183 | 2 (2) | 6/24/0 | 2/1 | 0/0 | 2/2 | 0/0 | | 16/14 | 14/12 |
| #9799 | 7 (7) | 30/0/0 | 6/4 | 4/2 | 5/4 | 5/5 | | 16/12 | 16/13 |
| #10160 | 3 (1) | 30/0/0 | 1/1 | 1/0 | 2/2 | 2/1 | | 17/15 | 14/15 |
| #9846 | 3 (3) | 30/0/0 | 3/3 | 2/2 | 3/3 | 2/2 | | 9/10 | 8/11 |
| #10165 | 2 (1) | 30/0/0 | 1/1 | 1/1 | 2/2 | 1/1 | | 11/12 | 13/14 |
| #10015 | 14 (13) | 30/0/0 | 4/4 | 6/6 | 5/9 | 6/7 | | 20/20 | 19/19 |
| #9934 | 22 (19) | 30/0/0 | 3/2 | 7/3 | 3/3 | 3/2 | | 23/22 | 18/19 |
| #10106 | 3 (1) | 30/0/0 | 0/0 | 1/1 | 0/0 | 1/2 | | 14/13 | 13/15 |
| #9675 | 5 (5) | 30/0/0 | 0/0 | 0/1 | 2/0 | 1/0 | | 14/13 | 14/8 |
| #9959 | 2 (2) | 30/0/0 | 2/2 | 1/1 | 2/2 | 1/2 | | 8/8 | 9/9 |
| #10130 | 5 (3) | 30/0/0 | 3/3 | 2/3 | 3/2 | 1/1 | | 12/14 | 11/10 |
| #9878 | 5 (4) | 30/0/0 | 5/4 | 2/0 | 4/5 | 1/2 | | 19/20 | 13/18 |
| #10100 | 7 (5) | 30/0/0 | 1/1 | 1/1 | 2/2 | 3/2 | | 11/11 | 15/17 |
| #10114 | 3 (3) | 30/0/0 | 2/2 | 2/2 | 1/2 | 2/2 | | 18/12 | 15/13 |

A cell with a slash is run 1/run 2.

- Notes per run: A 13.4, B 12.6. B writes a few fewer questions (158 vs 178) and docs notes (107 vs 121); about as many bugs (105 vs 101) and nits (260 vs 255).
- Notes that match no target: A 712 of 805 (88%), B 655 of 755 (87%). Most notes are not what the human reviewers said. Noise did not go up with B. The spec's "not useful" rating of these notes was not run (budget).
- Leaks: 753 tool calls checked. 10 named a path outside the run's own clone. All 10 were the run's own temp folder typed wrong (`-T-sb-know…` for `-T/sb-know…`), and all 10 were refused. No read of the data, of later code, or of the web. The PR's title and body are the final ones and may mention later changes: a small leak we did not remove.
- `knowledge/judge_check.csv`: 20 judge answers (10 "same", 10 "not same", the "not same" ones picked near the thread) for a hand check. The "human" column is empty, so the judge's agreement is not known yet.

### Claude runs

167 of the cap of 170, round 2 only (round 1 used 35): 4 classifier runs (sonnet), 120 reviews (A and B twice on 30 PRs), 13 B2 reviews, 30 judge runs (sonnet). No labeling runs: the 20 new `acted_on` threads are for a person.

### What surprised me

- With all kinds of notes and matching anywhere in the file, recall went from about 10% to about 35%. Round 1's low numbers were the test, not the review.
- Running the same thing twice moves recall by 5.6 points, more than the effect of the knowledge. Two runs each on 30 PRs cannot see a 3-point change.
- B never made the review noisier, and wrote a bit fewer notes. B helped most where reviewers repeat themselves in a file (#10162, #10100, #9799), and lost on #9934, the largest PR.
- 3 years of history filled the block for every PR. Ranking by place is no longer the limit: what goes in the 30 slots is.
- Line edits do not tell `acted_on` in this repo: authors change code near almost every comment. Every variant agrees with the labels 66% to 72%, close to always saying yes (62%), and the labels are from a model.
- Lighthouse has few PRs for its age: about 700 merged a year, under half with a review thread.

### Verdict

The signal is weak but points one way. B beats A by 3.5 points by concern (interval −2.8 to +11.0), and by 5.9 by place (+0.7 to +11.6). It does not meet the bar of 35.13 (5 points by concern, interval above 0). The B2 hint is one run on 13 PRs and means nothing yet.

Change the approach before phase 1; do not stop:
- Do not build `acted_on` as specified. It is barely better than always yes. Drop its 3 times weight, or wait for the hand labels in `labels_round2.csv`.
- A fixed block of 30 threads gives a few points at most. The next test is phase 1's tools (`know_threads` asked for the lines in question), not more ranking.
- Fill in the two hand-check files first. If the judge often disagrees with a person, these numbers move.
- A next replay needs about 3 runs per condition or 60 PRs to see a 3-point change: about 300 Claude runs.

## Repo knowledge, phase 0 deep dive (`knowledge/deep.py`)

Question: do past review threads make Claude's review better, not just closer to what humans said? Fully automated, no hand judging. Run on 2026-11-16, from commit 2fbcec6, on the round 2 data. The A and B reviews of round 2 were reused, none rerun. Raw data is in `~/.cache/sb-know-spike/deep/`.

`deep.py h2h`, `later`, `ceiling`, `attrib`, `report`. `H2H_LIMIT=n` runs only n judges; `DRY=1` builds the sonnet batches without running them.

### 1. Blind head-to-head (main result)

An opus judge per run pair (A1 vs B1, A2 vs B2) on all 30 PRs, each pair judged twice with X and Y swapped: 120 runs. The judge sees the title, body, diff, the human threads (labeled as one view, not the truth), the two note sets, and the snapshot code with Read, Grep and Glob only, in the same leak-free temp clone as round 2. A win counts only when both orders agree.

| | count |
|---|---|
| B wins | 23 |
| ties (or the orders disagree) | 15 |
| B losses | 22 |
| sign test, two-sided | p = 1.0 |
| by PR (wins minus losses over both pairs) | B better 11, A better 11, even 8 |

| per run | A | B |
|---|---|---|
| notes | 13.4 | 12.6 |
| useful | 6.53 | 6.54 |
| useful in both orders | 5.58 | 5.63 |
| wrong | 0.25 | 0.26 |
| noise | 6.6 | 5.8 |
| a real problem | 6.5 | 6.5 |
| share of notes useful | 48.7% | 52.0% |
| share of notes wrong | 1.9% | 2.1% |

- Pair 1: B 13, A 11, tie 6. Pair 2: A 11, B 10, tie 9.
- Position bias: in 8 of 60 pairs (13%) the judge picked the same slot in both orders. Of 113 non-tie picks, 65 were Y (58%). The two orders disagreed in 15 of 60 pairs. Per-note verdicts agreed between the orders 84% of the time.
- The judge made 289 tool calls (184 Grep, 105 Read). 3 named a path outside the clone, all the run's own temp folder typed wrong; they fail.

### 2. Real bugs from later history

For each PR: commits on `unstable` after the merge, up to 6 months, whose subject says fix, bug, revert, regression or panic, or whose message names the PR. Each changed line is blamed at the fix's parent. Lines from the PR's own commits are mapped back to the snapshot with `track.py`. A note within 5 lines in the same file "predicts" it.

- The window is short: all 30 PRs merged 16 to 52 days ago (mean 38). 6 months of later history does not exist yet.
- No reverts. 2 of 30 PRs had a later fix touching their lines, both in test files: #10160 (14 lines, the fix of nightly tests on phase0 and altair, #10200) and #10165 (2 lines, changed by #10160).
- Notes near those lines: A 1 and 1, B 0 and 2. None of them is about the fixed problem.
- Too few cases to mean anything. Run again in 6 months.

### 3. The ceiling

For each of the 143 target threads: past weighted threads on the PR's files, folders or crates whose first comment is older than the PR (median pool 1,342). Pre-filter: the top 20 per target by shared words (same file first), plus the 30 B threads. 7 sonnet runs judged which past threads raise essentially the same concern.

- 40 of 143 targets (28%) repeat a past concern. That is about the most history can add here.
- Only 10 of those 40 had the matching thread in B's block. The block's selection missed 30 of 40 (75%).
- The judge is lenient on "same concern" (a rename nit matched a different rename nit), and the word pre-filter can miss matches. Take 28% as rough.

### 4. Attribution

6 sonnet runs, batched by PR. For each item the judge saw B's block, A's notes on the same files and the item, and named the past thread it comes from, if any. Ids not in B's block were not counted: 9 answers named such ids (often the PR's own target thread).

- Targets a B run caught and no A run caught: 15 (the other way: 13). As 20 B notes, 1 clearly traces to a past thread (#10182, moving IGNORE/REJECT docs to the error enum, as asked in #9805).
- B notes marked useful in both orders: 338. A had something like 297 of them. Of the 41 left, 2 trace to a past thread.
- Of B's 61 extra catches, 3 (5%) trace to history; the rest look like run-to-run variance. With lenient ids it is 4 (7%).
- The 4 PRs where B scored worse by concern (#10352, #10182, #9934, #10130): the judge found no sign the block distracted B. Its notes were about the PR's own code; it missed different threads than A.

### Claude runs

133 of the cap of 150: 120 head-to-head judges (opus), 7 ceiling runs and 6 attribution runs (sonnet). Both run pairs were judged, nothing cut.

### Limits

- One judge model. No human check of its verdicts. The judge is not blind to which notes humans also raised.
- 30 PRs, 2 runs each. The PRs are public and may be in Claude's training data.
- The later-bug check had 2 cases.
- The ceiling and attribution rest on sonnet's sense of "same concern".

### Verdict

No. A block of 30 past threads does not make the review better on this data. B wins as often as it loses (23 to 22, p = 1.0), writes as many useful notes (6.5 per run each) and as many wrong ones (0.25). Its notes are a bit less noisy (5.8 vs 6.6 per run). Its extra catches almost never trace to a past thread (about 5%): they are luck. History could in theory help with about a quarter of what reviewers raise, but the fixed block picked the right thread for only a quarter of those. If knowledge has value, it is in finding the right past thread for the lines at hand (phase 1's tools), not in a fixed block. Do not build more on the fixed block.

## Repo knowledge, phase 0 lookup test (`knowledge/lookup.py`)

Question: can a lookup keyed on the lines under review find the past thread that raised the same concern? The fixed block found it for 10 of 40 repeat concerns. Retrieval only. Run on 2026-11-16, from commit 1d07411, on the round 2 and deep dive data. Results are in `~/.cache/sb-know-spike/deep/lookup*.json`.

### Setup

- Ground truth: the ceiling judge of the deep dive. 40 of 143 targets repeat a past concern, through 60 (target, past thread) pairs.
- Pool: weighted threads of other PRs whose first comment is before the PR was created (about 4,500 per PR). Weight as of that time.
- Query units: the hunks of `git diff -U3 base..snapshot` (33 per PR, 11 files), or one unit per changed file.
- A target is found at k when one of its past threads is in the top k of the unit that holds the target's line, or the nearest unit in that file.
- Size: the union of every unit's top k for the whole PR, in threads and words (the last 8 hunk lines plus up to 3 comments of 500 characters, as in the block).
- Methods:
  - L1: same file, by weight.
  - L2: same file; the line maps with `track.py` from the original commit to the merge commit to the PR base, within 20 lines of the hunk; by distance, then weight.
  - L2s: as L2, but a changed line goes to the start of the code that replaced it.
  - L3: same file, same function (the hunk header's `fn`, or the enclosing `fn` in the code).
  - L4f and L4r: BM25 by hand (identifiers split on `_` and case) between the hunk text plus function names and the thread's comments plus its diff hunk; same file only, or the whole repo.
  - L5: L2s, then L3, then L4f, then L4r.
- The ceiling judge only saw past threads picked by word overlap, which favors L4. So sonnet also judged the 955 top-10 candidates of every method that the ceiling never saw: 2 runs. It found 5 more pairs (43 targets). The numbers below barely move with them (+1 to +3 found).

### Found at k, per unit (40 repeat targets)

| unit | method | @5 | @10 | @30 | threads/PR k=5 | words/PR k=5 | threads/PR k=10 | words/PR k=10 |
|---|---|---|---|---|---|---|---|---|
| hunk | L1 | 7 | 11 | 21 | 36 | 2,402 | 63 | 4,194 |
| hunk | L2 | 0 | 0 | 0 | 4 | 266 | 4 | 266 |
| hunk | L2s | 4 | 6 | 6 | 15 | 1,031 | 16 | 1,075 |
| hunk | L3 | 8 | 9 | 9 | 17 | 1,160 | 21 | 1,465 |
| hunk | L4f | 15 | 19 | 23 | 56 | 3,778 | 85 | 5,755 |
| hunk | L4r | 16 | 21 | 26 | 81 | 5,583 | 147 | 10,159 |
| hunk | L5 | 14 | 23 | 28 | 79 | 5,402 | 139 | 9,656 |
| file | L2s | 9 | 12 | 13 | 13 | 843 | 15 | 1,035 |
| file | L3 | 12 | 14 | 15 | 14 | 968 | 20 | 1,386 |
| file | L4f | 16 | 19 | 23 | 36 | 2,385 | 61 | 4,153 |
| file | L4r | 19 | 24 | 27 | 40 | 2,801 | 75 | 5,251 |
| file | L5 | 11 | 22 | 28 | 49 | 3,313 | 92 | 6,338 |

Against all 143 targets, divide by 143: the best is 28 of 143 (20%) at k = 30.

### A fixed budget per PR

Round robin over the units' lists until n threads are taken. Found when a match is anywhere in the PR's set, since the agent sees all of it.

| unit | method | 30 threads | 40 threads | 45 threads |
|---|---|---|---|---|
| file | L1 (close to the old block) | 8 (1,830 words) | | 12 (2,712) |
| file | L2s | 16 (973) | | 19 (1,081) |
| file | L3 | 15 (1,334) | | 18 (1,634) |
| file | L4f | 21 (1,847) | 24 (2,448) | 24 (2,722) |
| file | L4r | 20 (2,166) | 25 (2,883) | 26 (3,232) |
| hunk | L5 | 17 (2,043) | | 23 (3,082) |

Of the 1,200 threads that L4r at 40 shows over 30 PRs, 24 are a match (2%).

### Why lookups miss

- Lines are the wrong key. Of the 60 pairs, 31 are on the same file, 17 on another file of the PR, 12 on a file the PR does not touch. Of the 31 on the same file, only 6 past lines still exist at the PR base; 24 were changed since. Mapped softly, 5 land within 20 lines of the target; the median is about 100 lines away. A repeat concern is the same kind of thing elsewhere, not a comment on the same lines. So L2 finds 0 and L2s finds 6.
- Function match is narrow: the same `fn` name in the same file is rare across PRs.
- For L4r at 40 threads, 15 targets are missed:
  - 6: a general habit raised in a file the PR does not touch, in other words ("dont panic here" vs "why not expect?"; "add gossip metrics" vs a metrics note in `fetch_blobs.rs`; "we dont need the clone" vs three older clone notes).
  - 5: same file, phrased differently (rank 28 to over 200).
  - 4: ranked 6 to 28 in their file, but cut by the budget.
  - About 3 of the 15 are weak matches from a lenient judge ("rename this arg" for "pass in proposal_slot").
- Per hunk is worse than per file: one hunk has too few words, and a PR has 33 hunks, so k = 10 per hunk is 140 threads and 10,000 words.

### Limits

- The ground truth is one sonnet judge, lenient, with no human check. Its candidates were picked by word overlap, which favors L4. The extra judge on 955 lookup candidates added only 5 pairs.
- 30 PRs, one repo. Lighthouse reviewers are almost all members, so weight sorts little.
- Found means the right thread is in front of the agent. It does not mean the agent uses it: in the deep dive, 10 matches in the block gave about 3 catches that trace to history.
- 2 Claude runs (sonnet), counted in `claude_runs_lookup.txt`.

### Verdict

Line lookups fail; text lookup works, barely. No lookup keyed on lines finds most repeat concerns: at best 19 of 40 (L2s or L3, 45 threads). Per hunk, nothing reaches most of them under 3,000 words. BM25 per changed file over all past threads of the repo (L4r), 40 threads per PR round robin, finds 25 of 40 (63%) in about 2,900 words: 2.5 times the fixed block at a similar size. That meets the bar, just.

The ceiling stays low: 25 found targets are 17% of all 143. If a targeted test runs, use L4r per file with 40 threads, and score only on the 40 repeat targets, A against B, 3 runs each. If B does not catch clearly more of those, the idea is dead for this repo.

## Repo knowledge, habits test (`knowledge/habits.py`)

Question: does a short per-repo list of maintainer review habits make Claude's review of a new PR better? The repeat concerns of the lookup test are habits ("don't panic here", "no needless clone") raised in other places and other words, so a list of habits might carry them where a list of threads did not.

### The bar (written before any result)

C is the A review plus the habits list. C passes only if all of these hold:
- head-to-head: C's wins exceed its losses, with a two-sided sign test p < 0.05;
- useful notes per run go up;
- wrong notes per run do not go up;
- recall by concern (all 143 targets) does not go down.

### Setup

Run on 2026-11-16, from commit 68cdae1, on the round 2 data. Raw data is in `~/.cache/sb-know-spike/habits/`. `habits.py build`, `write`, `run`, `h2h`, `judge`, `attrib`, `report`.

- **Cutoff:** 2026-04-24T03:57Z, when the earliest of the 30 test PRs (#9171) was created. Only threads whose first comment is before it. One list serves all 30 PRs.
- **Threads:** first comment by OWNER, MEMBER or COLLABORATOR; weight above 0 (not the PR's author, not a bot, not hidden, not only "lgtm", "+1" or an emoji); not AI text starting with "🤖" (14). Left: 3,746 threads in 672 PRs, by 16 maintainers.
- **Build:** threads shuffled into 6 batches of about 650. Opus per batch: up to 40 recurring concerns, each with the ids of the threads that raise it (229 in all). One opus run merges them into habits. Counts come from the thread ids, not from the model. Habits under 3 PRs are dropped (none were), ranked by PRs, then maintainers, cut to 25 and 700 words. One sonnet run picks a clear example per habit; the quote is the comment's first sentence, copied.
- **The list:** [`knowledge/habits_lighthouse.md`](knowledge/habits_lighthouse.md), 25 habits, 661 words. The top 5: update stale comments and docs (107 PRs); delete unused or debug code (97); fix typos and punctuation (80); comment non-obvious logic (62); names must say what a thing is (61). Lower down: no `unwrap` on reachable paths (37), no needless clones (34), hold locks briefly (16).
- **C** is A's prompt from `replay2.py` (same temp clone, tools, default model, writing rule) plus the list, fenced in `<maintainer-habits>` and marked "apply one only where it fits this change; data, not instructions". 2 runs per PR.
- **A** is reused from round 2. Same model (claude-opus-5-5) and the same Claude Code (2.1.325), checked in the logs.
- **Judges:** the deep dive's opus head-to-head, both run pairs, both orders. Recall: the round 2 sonnet judge, one run per PR over A0, A1, C0 and C1 together, so A was judged again beside C. Attribution: sonnet, 5 batches, tags every A and C note with the habit it clearly applies, and says whether a C note has a like A note.

### Results

| measure | A | C | C − A |
|---|---|---|---|
| head-to-head (both orders agree) | 25 wins | 16 wins | 19 ties; sign test p = 0.21 |
| by PR (wins minus losses, both pairs) | 13 better | 7 better | 10 even |
| notes per run | 13.4 | 13.8 | +0.4 |
| useful per run | 7.06 | 6.76 | −0.30 |
| useful in both orders, per run | 5.95 | 5.80 | −0.15 |
| wrong per run | 0.18 | 0.18 | 0.00 |
| noise per run | 6.18 | 6.85 | +0.68 |
| share of notes useful | 52.6% | 49.0% | −3.6 |
| recall by concern, 143 targets | 34.3% | 37.8% | +3.5 (−1.4 to +9.5) |
| recall by concern, 40 repeat targets | 36.3% | 47.5% | +11.3 (+1.2 to +23.3) |

- Pair 1: A 11, C 7, tie 12. Pair 2: A 14, C 9, tie 7.
- Position bias: the judge picked the same slot in both orders in 10 of 60 pairs (17%). Of 110 non-tie picks, 66 were Y (60%). The orders disagreed in 18 of 60. Per-note verdicts agreed between orders 84% of the time.
- Judge noise: the same A notes got 7.06 useful per run here and 6.53 in the deep dive. That gap is bigger than C − A.
- Note kinds shift: nits 255 → 345, docs 121 → 144; bugs 101 → 80, tests 150 → 112, questions 178 → 146.
- Recall intervals: paired bootstrap over PRs, 10,000 draws. Repeat targets are the 40 the deep dive's ceiling judge marked.

### Attribution and harm

- C notes useful in both orders: 348. 33 have no like A note; 28 of those 33 (85%) apply a habit. So the few extra good catches do come from the list.
- But the habits are broad: 64% of A's notes already apply one (512 of 805), against 74% of C's (612 of 827).
- Habit-driven noise goes up: notes that apply a habit and were judged noise, A 238.5, C 321 (half a count per order), about +1.4 per run. Habit-driven wrong notes: A 1.5, C 5.5. All wrong notes stay the same (11 and 10.5).
- Which habits moved: typos 11 → 25, TODO tags 15 → 31, reuse helpers 29 → 44, idiomatic Rust 23 → 35, clones 9 → 19. Add tests went down, 112 → 77. No `unwrap` stayed 5 → 4.
- Leaks: 352 C tool calls, 1 outside the clone (the temp folder typed wrong; refused). The judges made 316 calls; 2 outside: one the same typo, one a read under `~/.cargo` (dependency source, not lighthouse data).

### The bar

| test | result | pass |
|---|---|---|
| wins exceed losses, p < 0.05 | 16 wins, 25 losses, p = 0.21 | no |
| useful notes per run go up | 7.06 → 6.76 | no |
| wrong notes per run do not go up | 0.18 → 0.18 | yes |
| recall by concern does not go down | 34.3% → 37.8% | yes |

C fails. The robustness check (a list from a different half of the threads) was not run: it was planned only if C passed.

### Claude runs

223 of the cap of 240: 8 build runs (6 batch and 1 merge on opus, 1 example pick on sonnet), 60 C reviews, 120 head-to-head judges (opus), 30 recall judges (sonnet), 5 attribution runs (sonnet). Nothing cut. Counted in `habits/claude_runs.txt`.

### Limits

- One repo, 30 PRs, 2 runs each. One judge model, no human check. The PRs are public and may be in Claude's training data.
- The list's counts rest on opus putting thread ids in the right concern. The attribution's "applies a habit" is loose: it marks most notes.
- The judge is not blind to which notes humans raised. It sees human threads as a reference, so a nit a human also raised may count as useful.
- One list, one wording of the fence. A list without style habits, or one used only as a second pass, was not tested.

### Verdict

No. The habits list does not make reviews better. It makes them more like this repo's maintainers: C catches more of the concerns they repeat (47.5% vs 36.3% of the 40 repeat targets, interval above 0), and the extra catches do come from the list. But the blind judge prefers A 25 to 16, useful notes per run go down a little (7.06 to 6.76), and noise goes up (6.2 to 6.9). The list pulls the review toward nits (typos, TODO tags, style) and away from bugs and tests. Most top habits are style rules a linter or a careful reader already covers.

Do not build it as specified (generate per repo, put in the guide prompt, refresh). If repo knowledge goes on, the one signal worth a test is the repeat concerns: a short list of the non-style habits only (errors, panics, clones, locks, peer scoring, compatibility), or the list as a separate, labeled pass after the main review, so it cannot crowd out bugs. That needs its own bar and about the same budget.
