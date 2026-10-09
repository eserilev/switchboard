# Switchboard

Many AI coding agents on one screen. Each agent gets a tile with a big title and a lamp that stays lit until you look. A guided PR review keeps its step-by-step guide in place while you ask questions.

## Run

```sh
export CARGO_TARGET_DIR=~/.cache/switchboard-target
cargo run -p switchboard
```

Or install it: `cd packaging/arch && makepkg -si`.

Needs: Rust, tmux 3.2+, webkit2gtk-4.1. Optional: nvim, gh, notify-send, secret-tool.

## Use

| Keys | Action |
|---|---|
| `Ctrl+Space`, `n` | Open a repo, a worktree, nvim, a shell, or a PR URL for a review |
| `Ctrl+Space`, `j` / `k` | Next or previous tile that wants you |
| `Ctrl+Space`, `x` | Expand or collapse a tile |
| `Ctrl+Space`, `e` | nvim in the tile's worktree |
| `Ctrl+Space`, `a` | Next connection |
| `Ctrl+Space`, `r` / `b` | Review tabs / board |
| Review: `j` `k` `n` `d` | Steps, mark and next, draft from the active thread |

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
