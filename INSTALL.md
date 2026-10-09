# Install Switchboard

Switchboard runs on Linux and macOS (Apple Silicon). One script does the install on both.

## Quick install

```sh
curl -fsSL https://raw.githubusercontent.com/eserilev/switchboard/master/scripts/install.sh | bash
```

The script:

1. Checks the tools that Switchboard needs. If a tool is missing, it prints the install command for your package manager and stops. To let the script install them, add `--deps` (it asks for `sudo`).
2. Downloads the latest release when one exists for your system. Otherwise it gets the source and builds it.
3. Installs the app:
   - Linux: `switchboard` and `sb` in `~/.local/bin`, an app menu entry, and an icon.
   - macOS: `~/Applications/Switchboard.app`, and links to `sb` and `switchboard` in `~/.local/bin`.
4. Writes a starter config with the code folders that it finds, if you have no config yet.
5. Tells you the steps that are left, for example `gh auth login`.

It does not change your shell files or your Claude Code settings.

To install the missing tools too:

```sh
curl -fsSL https://raw.githubusercontent.com/eserilev/switchboard/master/scripts/install.sh | bash -s -- --deps
```

## What Switchboard needs

| Tool | Why | Linux | macOS |
|---|---|---|---|
| Claude Code (`claude`) | The agents | `curl -fsSL https://claude.ai/install.sh \| bash` | the same |
| tmux 3.2 or newer | The agents keep running when the window closes | your package manager | `brew install tmux` |
| git | Repos, worktrees, PR diffs | your package manager | Xcode tools or `brew install git` |
| GitHub CLI (`gh`) | PR reviews | `github-cli` (Arch), `gh` (others) | `brew install gh` |
| WebKitGTK 4.1 | The window | `webkit2gtk-4.1` | built in |
| Rust | Only to build from source | [rustup.rs](https://rustup.rs) | the same |

Optional: `nvim` for nvim panes, `notify-send` for desktop notifications, `secret-tool` for endpoint tokens (Linux).

## After the install

1. Log in to Claude Code once: run `claude` in a terminal.
2. Log in to GitHub for reviews: `gh auth login`.
3. Check the config: `~/.config/switchboard/config.toml`. `scan` lists the folders where Switchboard looks for your repos. A PR review needs a local clone of the repo in one of these folders. Section 14 of `SPEC.md` has every key.
4. Start Switchboard:
   - Linux: `switchboard`, or from your app menu.
   - macOS: open `~/Applications/Switchboard.app`.

## Other ways to install

**From a checkout.** This always builds from source:

```sh
git clone https://github.com/eserilev/switchboard
cd switchboard
scripts/install.sh            # add --deps to install missing tools
```

**Arch Linux package:**

```sh
cd packaging/arch && makepkg -si
```

**macOS app only.** This builds `Switchboard.app` into `target/macos`:

```sh
scripts/macos-app.sh
```

**Run without an install** (for development):

```sh
export CARGO_TARGET_DIR=~/.cache/switchboard-target
cargo run -p switchboard
```

## Update and uninstall

- Update: run the install command again. Your config, reviews and agents stay.
- Uninstall: `scripts/install.sh --uninstall`, or the curl command with `bash -s -- --uninstall`. This removes the app. It keeps your config (`~/.config/switchboard`), your store (`~/.local/share/switchboard`) and your state (`~/.switchboard`). Your agents keep running in tmux. To stop them: `tmux -L switchboard kill-server`.

## Script flags

| Flag | Effect |
|---|---|
| `--deps` | Install missing tools with your package manager |
| `--source` | Build from source, also when a release exists |
| `--prefix DIR` | Linux: put the binaries in `DIR/bin` (default `~/.local`) |
| `--uninstall` | Remove the app |

## Problems

- **"is not on your PATH".** Add `~/.local/bin` to your `PATH`, so that `sb open .` works in a terminal. The app itself works without it.
- **macOS: the leader key does nothing.** macOS uses `ctrl+space` to switch the input language. The install script sets `leader = "ctrl+;"` on macOS. Change `leader` in the config to any key you like, for example `"cmd+;"`.
- **macOS: "Switchboard.app cannot be opened".** A release downloaded with a browser has a quarantine mark. Remove it with `xattr -dr com.apple.quarantine ~/Applications/Switchboard.app`. The install script uses curl, so its download has no such mark.
- **macOS: tmux, gh or claude "not found".** An app started from the Dock gets a short `PATH`. Switchboard reads the `PATH` of your login shell at start. Check that `tmux`, `gh` and `claude` work in a new terminal.
- **"tmux 3.2 or newer is needed".** Update tmux with your package manager. On older Ubuntu, use a newer release or a backport.
- **A review says "no local clone".** Clone the repo into a folder that `scan` lists, then open the review again.

The log is at `~/.switchboard/logs/switchboard.log`.
