#!/usr/bin/env bash
# Installs Switchboard on Linux or macOS.
#
#   curl -fsSL https://raw.githubusercontent.com/eserilev/switchboard/master/scripts/install.sh | bash
#   scripts/install.sh            # from a checkout
#
# Flags:
#   --deps         install missing packages with your package manager (asks for sudo)
#   --source       build from source, also when a prebuilt release exists
#   --prefix DIR   Linux: install the binaries into DIR/bin (default ~/.local)
#   --uninstall    remove the app; your config, store and agents stay
#
# The script checks every dependency first and prints the command that installs
# what is missing. It never changes your shell files or your Claude settings.
set -euo pipefail

REPO_SLUG=eserilev/switchboard
REPO_URL="https://github.com/$REPO_SLUG"
deps=0 source=0 uninstall=0
prefix="$HOME/.local"
while [ $# -gt 0 ]; do
  case "$1" in
    --deps) deps=1 ;;
    --source) source=1 ;;
    --prefix) prefix=$2; shift ;;
    --uninstall) uninstall=1 ;;
    -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
    *) echo "unknown flag: $1"; exit 2 ;;
  esac
  shift
done

os=$(uname -s)
arch=$(uname -m)
data_home=${XDG_DATA_HOME:-$HOME/.local/share}
config_home=${XDG_CONFIG_HOME:-$HOME/.config}
bin_dir="$prefix/bin"
mac_app="$HOME/Applications/Switchboard.app"

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[33mwarn:\033[0m %s\n' "$*"; }
die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

case "$os" in
  Linux|Darwin) ;;
  *) die "Switchboard runs on Linux and macOS, not on $os." ;;
esac

# ---------- uninstall ----------
if [ "$uninstall" = 1 ]; then
  if [ "$os" = Darwin ]; then
    rm -rf "$mac_app"
    rm -f "$bin_dir/sb" "$bin_dir/switchboard"
  else
    rm -f "$bin_dir/switchboard" "$bin_dir/sb"
    rm -f "$data_home/applications/switchboard.desktop" "$data_home/icons/hicolor/256x256/apps/switchboard.png"
  fi
  say "Removed Switchboard."
  echo "Kept: $config_home/switchboard (config), $data_home/switchboard (store), ~/.switchboard (state)."
  echo "Your agents keep running in tmux (socket 'switchboard'). Stop them with: tmux -L switchboard kill-server"
  exit 0
fi

# ---------- package manager ----------
pm=""
for p in brew pacman apt-get dnf zypper; do
  if have "$p"; then pm=$p; break; fi
done
[ "$os" = Darwin ] && ! have brew && warn "Homebrew is missing. Install it from https://brew.sh, then run this again."

# pkg <name>: the package name for this package manager.
pkg() {
  case "$pm:$1" in
    *:tmux) echo tmux ;;
    *:git) echo git ;;
    pacman:gh) echo github-cli ;;
    *:gh) echo gh ;;
    brew:webkit|brew:build) echo "" ;;
    pacman:webkit) echo "webkit2gtk-4.1" ;;
    apt-get:webkit) echo "libwebkit2gtk-4.1-0" ;;
    dnf:webkit|zypper:webkit) echo "webkit2gtk4.1" ;;
    pacman:build) echo "base-devel webkit2gtk-4.1 libsoup3 gtk3 pkgconf" ;;
    apt-get:build) echo "build-essential pkg-config libwebkit2gtk-4.1-dev libsoup-3.0-dev libgtk-3-dev libssl-dev librsvg2-dev" ;;
    dnf:build) echo "gcc gcc-c++ make pkgconf-pkg-config webkit2gtk4.1-devel libsoup3-devel gtk3-devel openssl-devel librsvg2-devel" ;;
    zypper:build) echo "gcc gcc-c++ make pkg-config webkit2gtk3-soup2-devel libsoup-devel gtk3-devel libopenssl-devel librsvg-devel" ;;
    *) echo "$1" ;;
  esac
}

install_cmd() {
  case "$pm" in
    brew) echo "brew install $*" ;;
    pacman) echo "sudo pacman -S --needed $*" ;;
    apt-get) echo "sudo apt-get install -y $*" ;;
    dnf) echo "sudo dnf install -y $*" ;;
    zypper) echo "sudo zypper install -y $*" ;;
    *) echo "install: $*" ;;
  esac
}

missing=()
need() { # need <command> <package key>
  have "$1" || missing+=("$(pkg "$2")")
}

# ---------- source or prebuilt ----------
here=$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" 2>/dev/null && pwd || echo "")
src=""
if [ -n "$here" ] && [ -f "$here/../Cargo.toml" ] && grep -q 'crates/app' "$here/../Cargo.toml" 2>/dev/null; then
  src=$(cd "$here/.." && pwd)
fi

asset=""
if [ "$os" = Linux ] && [ "$arch" = x86_64 ]; then asset=switchboard-linux-x86_64.tar.gz; fi
if [ "$os" = Darwin ] && [ "$arch" = arm64 ]; then asset=Switchboard-macos-arm64.zip; fi
release_url=""
if [ "$source" = 0 ] && [ -z "$src" ] && [ -n "$asset" ] && have curl; then
  release_url=$(curl -fsSL "https://api.github.com/repos/$REPO_SLUG/releases/latest" 2>/dev/null \
    | grep -o "\"browser_download_url\": *\"[^\"]*/$asset\"" | sed 's/.*"\(http[^"]*\)"/\1/' || true)
fi
build=1
[ -n "$release_url" ] && build=0

# ---------- dependencies ----------
say "Checking dependencies"
need tmux tmux
need git git
need gh gh
if [ "$os" = Linux ] && [ "$build" = 0 ]; then
  # The prebuilt app needs the WebKitGTK library at run time.
  if ! ldconfig -p 2>/dev/null | grep -q 'libwebkit2gtk-4.1'; then missing+=("$(pkg webkit)"); fi
fi
if [ "$build" = 1 ]; then
  [ -n "$(pkg build)" ] && [ "$os" = Linux ] && ! pkg-config --exists webkit2gtk-4.1 2>/dev/null && missing+=("$(pkg build)")
  [ "$os" = Darwin ] && ! xcode-select -p >/dev/null 2>&1 && warn "The Xcode command line tools are missing. Run: xcode-select --install"
fi

# Words, one package each, with no empty ones.
read -r -a missing <<< "${missing[*]:-}"
if [ ${#missing[@]} -gt 0 ]; then
  cmd=$(install_cmd "${missing[@]}")
  if [ "$deps" = 1 ] && [ -n "$pm" ]; then
    say "Installing: ${missing[*]}"
    eval "$cmd"
  else
    echo "Missing: ${missing[*]}"
    echo "Install them with:"
    echo "  $cmd"
    echo "Or run this script with --deps."
    exit 1
  fi
fi

# tmux 3.2 or newer.
tv=$(tmux -V | sed 's/[^0-9.]*\([0-9]*\)\.\([0-9]*\).*/\1 \2/')
read -r tmaj tmin <<< "$tv"
if [ "${tmaj:-0}" -lt 3 ] || { [ "$tmaj" -eq 3 ] && [ "${tmin:-0}" -lt 2 ]; }; then
  die "tmux 3.2 or newer is needed. You have $(tmux -V)."
fi

if [ "$build" = 1 ] && ! have cargo; then
  echo "Rust is missing. Install it with:"
  echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  echo "Then open a new terminal and run this script again."
  exit 1
fi

# ---------- get the app ----------
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if [ "$build" = 0 ]; then
  say "Downloading the latest release"
  curl -fsSL "$release_url" -o "$work/$asset"
  if [ "$os" = Darwin ]; then
    ditto -x -k "$work/$asset" "$work/out"
  else
    mkdir -p "$work/out" && tar -xzf "$work/$asset" -C "$work/out"
  fi
else
  if [ -z "$src" ]; then
    src="$data_home/switchboard/src"
    if [ -d "$src/.git" ]; then
      say "Updating the source in $src"
      git -C "$src" pull --ff-only -q
    else
      say "Getting the source into $src"
      mkdir -p "$(dirname "$src")"
      git clone -q "$REPO_URL" "$src"
    fi
  fi
  export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$src/target}
  say "Building (the first build takes a few minutes)"
  if [ "$os" = Darwin ]; then
    "$src/scripts/macos-app.sh" "$work/out"
  else
    cargo build --release --locked -p switchboard -p sb --manifest-path "$src/Cargo.toml"
    mkdir -p "$work/out"
    cp "$CARGO_TARGET_DIR/release/switchboard" "$CARGO_TARGET_DIR/release/sb" "$work/out/"
    cp "$src/packaging/switchboard.desktop" "$work/out/"
    cp "$src/crates/app/icons/icon.png" "$work/out/switchboard.png"
  fi
fi

# ---------- install ----------
mkdir -p "$bin_dir"
if [ "$os" = Darwin ]; then
  mkdir -p "$HOME/Applications"
  rm -rf "$mac_app"
  cp -R "$work/out/Switchboard.app" "$mac_app"
  # `sb open .` from a terminal: links to the binaries in the app.
  ln -sf "$mac_app/Contents/MacOS/sb" "$bin_dir/sb"
  ln -sf "$mac_app/Contents/MacOS/switchboard" "$bin_dir/switchboard"
  say "Installed $mac_app"
else
  install -m755 "$work/out/switchboard" "$bin_dir/switchboard"
  install -m755 "$work/out/sb" "$bin_dir/sb"
  mkdir -p "$data_home/applications" "$data_home/icons/hicolor/256x256/apps"
  sed "s|^Exec=.*|Exec=$bin_dir/switchboard|" "$work/out/switchboard.desktop" > "$data_home/applications/switchboard.desktop"
  install -m644 "$work/out/switchboard.png" "$data_home/icons/hicolor/256x256/apps/switchboard.png"
  say "Installed $bin_dir/switchboard and $bin_dir/sb, and a menu entry"
fi

case ":$PATH:" in
  *":$bin_dir:"*) ;;
  *) warn "$bin_dir is not on your PATH. Add it to use 'sb' in a terminal." ;;
esac

# ---------- a starter config ----------
config="$config_home/switchboard/config.toml"
if [ ! -f "$config" ]; then
  roots=()
  for d in "$HOME/Documents/Code" "$HOME/code" "$HOME/Code" "$HOME/src" "$HOME/dev" "$HOME/projects" "$HOME/Projects" "$HOME/workspace"; do
    [ -d "$d" ] && roots+=("\"${d/#$HOME/~}\"")
  done
  [ ${#roots[@]} -eq 0 ] && roots=('"~/Documents/Code"')
  leader='ctrl+space'
  # On macOS ctrl+space often switches the input language.
  [ "$os" = Darwin ] && leader='ctrl+;'
  mkdir -p "$(dirname "$config")"
  {
    echo "# Switchboard config. SPEC.md section 14 has every key."
    echo "# The folders where the app looks for your git repos and PR clones."
    echo "scan = [$(IFS=,; echo "${roots[*]}" | sed 's/,/, /g')]"
    echo "# The leader key: the first key of every Switchboard shortcut."
    echo "leader = \"$leader\""
  } > "$config"
  say "Wrote a starter config: $config"
fi

# ---------- next steps ----------
echo
have claude || echo "- Install Claude Code: curl -fsSL https://claude.ai/install.sh | bash   (then run 'claude' once to log in)"
gh auth status >/dev/null 2>&1 || echo "- Log in to GitHub for PR reviews: gh auth login"
if [ "$os" = Darwin ]; then
  echo "- Start it: open ~/Applications/Switchboard.app"
else
  echo "- Start it: switchboard   (or from your app menu)"
fi
echo "- Update later: run this script again."
