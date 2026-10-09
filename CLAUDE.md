# Switchboard

A desktop app for many AI agents on one screen, plus a guided PR review. `SPEC.md` is the design. `spikes/README.md` has the spike results.

## Rules

- Write all prose in simple English: comments, docs, commit messages.
- Commits are one line. No AI credit lines.
- No npm in the build. Front-end libraries are vendored in `web/vendor/` with a `VERSION` file.
- Never use tmux `%pane:off` for a tile. tmux stops reading the pane. Use `pause` and `continue`.
- The app never kills the tmux server. Only tests do.
- `SPEC.md` section 23 records where the build differs from the spec. Keep it true.
- A `claude -p` run puts the prompt first. List flags take it otherwise.
- Lock rule: never hold a lock in an `if let` on the guard when the block locks again. Take the value first.

## Proofs

`crates/guide-check` is proved in Lean with Aeneas (`proofs/`, SPEC 24). It stays in the Aeneas subset:
- `while` loops and small functions. No `?`, no closures, no `&&` or `||` in a loop condition, no iterators.
- A `return` inside a loop only when the loop is the last statement.
- No add that can overflow: loop with an index `k < bound`, not `l <= to`.
After a change to `guide-check`, run `scripts/check-proofs.sh`. It regenerates `proofs/GuideCheck/Code`; commit that too. Never weaken a statement in `proofs/Statements.lean` to make a proof pass.

## Build and test

```sh
export CARGO_TARGET_DIR=~/.cache/switchboard-target
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo run -p switchboard
web-tests/run.sh          # the window checks alone
```

The core tests start real tmux servers on sockets named `sb-test-*`. They need `tmux` 3.2 or newer.
