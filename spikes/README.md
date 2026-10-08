# Spikes

Each spike answers one question from SPEC.md section 18. Run them from this folder.

| ID | Status | Result |
|---|---|---|
| S1 | Waiting | Docs answer half. A real limit event is needed. |
| S2 | Pass | A blocking `PermissionRequest` hook works in `-p` and in an interactive pane. |
| S3 | Pass | `--fork-session` leaves the parent byte-equal. A follow-up keeps the fork id. |
| S4 | Waiting | Needs the second account logged in. |
| S5 | Pass | A Tauri 2 window runs on GNOME Wayland with no workaround. WebGL2 works. |
| S6 | Pass, with a change | Turn tile output off. Do not rely on `pause-after`. |
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

Spec change: tiles use `%N:off` and `capture-pane`. Only the live pane is `on`. `pause-after` stays as a guard.

## S7: local endpoint

Needs a local server that speaks the Anthropic Messages API, for example llama.cpp server or Ollama. None is installed.
