# Switchboard

Many AI coding agents on one screen. Each agent gets a tile with a big title and a lamp that stays lit until you look. A guided PR review keeps its step-by-step guide in place while you ask questions.

## Install

Linux and macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/eserilev/switchboard/master/scripts/install.sh | bash
```

`INSTALL.md` has the details: what the script does, the tools that Switchboard needs, other ways to install, and fixes for common problems.

For development:

```sh
export CARGO_TARGET_DIR=~/.cache/switchboard-target
cargo run -p switchboard
```

## Use

The leader key is `leader` in the config: `ctrl+space` on Linux, and `ctrl+;` from the macOS install. The table uses `Ctrl+Space`.

| Keys | Action |
|---|---|
| `Ctrl+Space`, `n` | Open a repo, a worktree, nvim, a shell, or a PR URL for a review |
| `Ctrl+Space`, `j` / `k` | Next or previous tile that wants you |
| `Ctrl+Space`, `x` | Expand or collapse a tile |
| `Ctrl+Space`, `e` | nvim in the tile's worktree |
| `Ctrl+Space`, `a` | Next connection |
| `Ctrl+Space`, `<` / `>` | Move the focused tile left or right |
| `Ctrl+Space`, `r` / `b` | Review tabs / board |
| Review: `j` `k` `n` `d` | Steps, mark and next, draft from the active thread |

Drag a tile onto another tile to move it. The order is saved.

From a terminal: `sb open .`, `sb open <path> -w <branch>`, `sb open <path> --nvim`.

A second Claude account: `sb connect add b`, then log in once with the command it prints.

## Config

`~/.config/switchboard/config.toml`. See `SPEC.md` section 14.

## Tests

```sh
cargo test --workspace                                       # all offline tests, with the window checks
cargo test -p sb --test live_review -- --ignored --nocapture  # a real review; costs tokens
cargo test -p sb --test live_switch -- --ignored --nocapture  # a real connection switch; costs tokens
```
