# Switchboard

A desktop app for many AI agents on one screen, plus a guided PR review. `SPEC.md` is the design. `spikes/README.md` has the spike results.

## Rules

- Write all prose in simple English: comments, docs, commit messages.
- Commits are one line. No AI credit lines.
- No npm in the build. Front-end libraries are vendored in `web/vendor/` with a `VERSION` file.
- Never use tmux `%pane:off` for a tile. tmux stops reading the pane. Use `pause` and `continue`.
- The app never kills the tmux server. Only tests do.
- Each PR follows the build plan in `SPEC.md` section 20.

## Build and test

```sh
export CARGO_TARGET_DIR=~/.cache/switchboard-target
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo run -p switchboard
```

The core tests start real tmux servers on sockets named `sb-test-*`. They need `tmux` 3.2 or newer.
