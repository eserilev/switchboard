# Switchboard

Many AI coding agents on one screen. Each agent gets a big title and a lamp that stays lit until you look. A guided PR review keeps its step-by-step guide in place while you ask questions.

Status: PR 1 of the build plan. The window shows one live pane of the Switchboard tmux server.

```sh
cargo run -p switchboard
```

Needs: Rust, tmux 3.2+, webkit2gtk-4.1. See `SPEC.md` for the design.
