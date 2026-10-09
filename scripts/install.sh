#!/usr/bin/env bash
# Installs Switchboard on Linux or macOS. INSTALL.md has the details.
#
#   curl -fsSL https://raw.githubusercontent.com/eserilev/switchboard/master/scripts/install.sh | bash
#   scripts/install.sh            # from a checkout
#
# The whole script is one function, called on the last line, so a download that
# stops part way runs nothing.
set -euo pipefail

REPO_SLUG=eserilev/switchboard
REPO_URL="https://github.com/$REPO_SLUG"

usage() {
  cat <<'EOF'
Usage: install.sh [flags]
  --deps         install missing packages with your package manager (asks for sudo)
  --source       build from source, also when a prebuilt release exists
  --prefix DIR   put the binaries (Linux) or the links to them (macOS) in DIR/bin
                 (default ~/.local)
  --uninstall    remove the app; your config, store and agents stay
The script checks every dependency first and prints the command that installs
what is missing. It never changes your shell files or your Claude settings.
EOF
}

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[33mwarn:\033[0m %s\n' "$*"; }
die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

main() {
  local deps=0 source=0 uninstall=0 prefix="$HOME/.local"
  while [ $# -gt 0 ]; do
    case "$1" in
      --deps) deps=1 ;;
      --source) source=1 ;;
      --prefix) [ $# -ge 2 ] || die "--prefix needs a folder"; prefix=$2; shift ;;
      --uninstall) uninstall=1 ;;
      -h|--help) usage; exit 0 ;;
      *) usage; die "unknown flag: $1" ;;
    esac
    shift
  done

  local os arch
  os=$(uname -s)
  arch=$(uname -m)
  case "$os" in
    Linux|Darwin) ;;
    *) die "Switchboard runs on Linux and macOS, not on $os." ;;
  esac
  data_home=${XDG_DATA_HOME:-$HOME/.local/share}
  config_home=${XDG_CONFIG_HOME:-$HOME/.config}
  bin_dir="$prefix/bin"
  mac_app="$HOME/Applications/Switchboard.app"
  # The files that this script installed, one path on each line. Install and
  # uninstall only replace or remove a file in this list, or a new file.
  manifest="$data_home/switchboard/installed-files"

  if [ "$uninstall" = 1 ]; then
    do_uninstall
    exit 0
  fi

  pick_package_manager "$os"

  # The app runs the `sb` next to it for every hook and for its MCP server. So both
  # binaries must be ours: a file of another tool there stops the install, before
  # any download or build.
  local b
  if [ "$os" = Linux ]; then
    for b in switchboard sb; do
      ours "$bin_dir/$b" || die "$bin_dir/$b is not from Switchboard. The app runs the sb next to it for every hook, so the install stops. Move that file away, or use --prefix, then run this again."
    done
  fi

  # The checkout this script is in, only when it runs from a real file.
  local src="" here
  if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
    here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
    if [ -f "$here/../Cargo.toml" ] && [ -f "$here/../crates/app/tauri.conf.json" ]; then
      src=$(cd "$here/.." && pwd)
    fi
  fi

  # A prebuilt release for this system, when there is one and no checkout.
  local asset="" release_url=""
  if [ "$os" = Linux ] && [ "$arch" = x86_64 ]; then asset=switchboard-linux-x86_64.tar.gz; fi
  if [ "$os" = Darwin ] && [ "$arch" = arm64 ]; then asset=Switchboard-macos-arm64.zip; fi
  if [ "$source" = 0 ] && [ -z "$src" ] && [ -n "$asset" ] && have curl; then
    release_url=$(curl -fsSL "https://api.github.com/repos/$REPO_SLUG/releases/latest" 2>/dev/null \
      | grep -o "\"browser_download_url\": *\"[^\"]*/$asset\"" | sed 's/.*"\(http[^"]*\)"/\1/' || true)
  fi
  local build=1
  [ -n "$release_url" ] && build=0
  [ "$build" = 1 ] && [ "$os" = Darwin ] && [ "$arch" != arm64 ] && say "No prebuilt app for Intel Macs: the script builds from source."

  check_dependencies "$os" "$build" "$deps"

  local work
  work=$(mktemp -d)
  # shellcheck disable=SC2064
  trap "rm -rf '$work'" EXIT

  if [ "$build" = 0 ]; then
    say "Downloading the latest release"
    curl -fsSL "$release_url" -o "$work/$asset"
    mkdir -p "$work/out"
    if [ "$os" = Darwin ]; then
      ditto -x -k "$work/$asset" "$work/out"
    else
      tar -xzf "$work/$asset" -C "$work/out"
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

  do_install "$os" "$work/out"
  write_config "$os"
  next_steps "$os"
}

pick_package_manager() {
  pm=""
  local p order
  # On Linux the system package manager comes first: Homebrew on Linux has no
  # WebKitGTK for the window.
  if [ "$1" = Darwin ]; then order="brew"; else order="pacman apt-get dnf zypper brew"; fi
  for p in $order; do
    if have "$p"; then pm=$p; break; fi
  done
  if [ "$1" = Darwin ] && [ -z "$pm" ]; then
    warn "Homebrew is missing. Install it from https://brew.sh, then run this again."
  fi
}

# pkg <key>: the package names for this package manager.
pkg() {
  case "$pm:$1" in
    pacman:gh) echo github-cli ;;
    *:tmux|*:git|*:gh) echo "$1" ;;
    pacman:webkit) echo webkit2gtk-4.1 ;;
    apt-get:webkit) echo libwebkit2gtk-4.1-0 ;;
    dnf:webkit) echo webkit2gtk4.1 ;;
    zypper:webkit) echo libwebkit2gtk-4_1-0 ;;
    pacman:build) echo "base-devel webkit2gtk-4.1 libsoup3 gtk3 pkgconf" ;;
    apt-get:build) echo "build-essential pkg-config libwebkit2gtk-4.1-dev libsoup-3.0-dev libgtk-3-dev libssl-dev librsvg2-dev" ;;
    dnf:build) echo "gcc gcc-c++ make pkgconf-pkg-config webkit2gtk4.1-devel libsoup3-devel gtk3-devel openssl-devel librsvg2-devel" ;;
    zypper:build) echo "gcc gcc-c++ make pkg-config webkit2gtk3-devel libopenssl-devel librsvg-devel" ;;
    *) echo "" ;;
  esac
}

install_cmd() {
  case "$pm" in
    brew) echo "brew install $*" ;;
    pacman) echo "sudo pacman -S --needed $*" ;;
    apt-get) echo "sudo apt-get install -y $*" ;;
    dnf) echo "sudo dnf install -y $*" ;;
    zypper) echo "sudo zypper install -y $*" ;;
    *) echo "" ;;
  esac
}

# The WebKitGTK 4.1 library, for the window on Linux. `ldconfig` is in /sbin on
# Debian, which is not on a user's PATH.
has_webkit() {
  if have pkg-config && pkg-config --exists webkit2gtk-4.1 2>/dev/null; then return 0; fi
  local l
  for l in ldconfig /sbin/ldconfig /usr/sbin/ldconfig; do
    if "$l" -p 2>/dev/null | grep -q 'libwebkit2gtk-4\.1'; then return 0; fi
  done
  compgen -G "/usr/lib*/libwebkit2gtk-4.1.so*" >/dev/null || compgen -G "/usr/lib/*/libwebkit2gtk-4.1.so*" >/dev/null
}

check_dependencies() {
  local os=$1 build=$2 deps=$3
  say "Checking dependencies"
  local missing=() names=()
  have tmux || missing+=(tmux)
  have git || missing+=(git)
  if [ "$os" = Linux ]; then
    if [ "$build" = 1 ]; then
      { have pkg-config && pkg-config --exists webkit2gtk-4.1 2>/dev/null; } || missing+=(build)
    else
      has_webkit || missing+=(webkit)
    fi
  fi
  if [ "$os" = Darwin ] && [ "$build" = 1 ] && ! xcode-select -p >/dev/null 2>&1; then
    die "The Xcode command line tools are missing. Run: xcode-select --install"
  fi

  if [ ${#missing[@]} -gt 0 ]; then
    local k n
    for k in "${missing[@]}"; do
      n=$(pkg "$k")
      if [ -z "$n" ]; then
        case "$k" in
          build) n="a C compiler, pkg-config and the WebKitGTK 4.1 development package" ;;
          webkit) n="the WebKitGTK 4.1 library" ;;
          *) n=$k ;;
        esac
        warn "Your package manager is not known here. Install: $n"
        exit 1
      fi
      read -r -a words <<< "$n"
      names+=("${words[@]}")
    done
    local cmd
    cmd=$(install_cmd "${names[@]}")
    if [ "$deps" = 1 ]; then
      say "Installing: ${names[*]}"
      eval "$cmd"
    else
      echo "Missing: ${names[*]}"
      echo "Install them with:"
      echo "  $cmd"
      echo "Or run this script with --deps."
      exit 1
    fi
  fi

  # tmux 3.2 or newer.
  local tmaj tmin
  read -r tmaj tmin <<< "$(tmux -V | sed 's/[^0-9]*\([0-9]*\)\.\([0-9]*\).*/\1 \2/')"
  if [ "${tmaj:-0}" -lt 3 ] || { [ "$tmaj" -eq 3 ] && [ "${tmin:-0}" -lt 2 ]; }; then
    die "tmux 3.2 or newer is needed. You have $(tmux -V)."
  fi

  if [ "$build" = 1 ] && ! have cargo; then
    echo "Rust is missing. Install it with:"
    echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    echo "Then open a new terminal and run this script again."
    exit 1
  fi
}

# True when we can write `path`: it does not exist, or this script put it there.
ours() {
  [ ! -e "$1" ] && [ ! -L "$1" ] && return 0
  [ -f "$manifest" ] && grep -qxF "$1" "$manifest"
}

# put <from> <to> <mode>: installs a file, or a link with mode "link".
put() {
  local from=$1 to=$2 mode=$3
  if ! ours "$to"; then
    warn "$to is not from Switchboard, so it stays. Remove it, then run this again."
    return 0
  fi
  mkdir -p "$(dirname "$to")"
  if [ "$mode" = link ]; then
    ln -sfn "$from" "$to"
  else
    install -m "$mode" "$from" "$to"
  fi
  installed+=("$to")
}

# The value of a .desktop Exec key: quoted, with \ " ` $ escaped.
desktop_exec() {
  local v=$1
  v=${v//\\/\\\\}
  v=${v//\"/\\\"}
  v=${v//\`/\\\`}
  v=${v//\$/\\\$}
  # The file format reads a string once more: each \ is \\ there. And % starts
  # a field code: %% is a %.
  v=${v//\\/\\\\}
  v=${v//%/%%}
  printf '"%s"' "$v"
}

do_install() {
  local os=$1 out=$2
  installed=()

  if [ "$os" = Darwin ]; then
    if ! ours "$mac_app"; then
      die "$mac_app is not from this script. Move it away, then run this again."
    fi
    mkdir -p "$HOME/Applications"
    rm -rf "$mac_app"
    cp -R "$out/Switchboard.app" "$mac_app"
    installed+=("$mac_app")
    # `sb open .` from a terminal: links to the binaries in the app.
    put "$mac_app/Contents/MacOS/sb" "$bin_dir/sb" link
    put "$mac_app/Contents/MacOS/switchboard" "$bin_dir/switchboard" link
    say "Installed $mac_app"
  else
    put "$out/switchboard" "$bin_dir/switchboard" 755
    put "$out/sb" "$bin_dir/sb" 755
    local desktop="$data_home/applications/switchboard.desktop" tmp
    tmp=$(mktemp)
    grep -v '^Exec=' "$out/switchboard.desktop" > "$tmp"
    printf 'Exec=%s\n' "$(desktop_exec "$bin_dir/switchboard")" >> "$tmp"
    put "$tmp" "$desktop" 644
    rm -f "$tmp"
    put "$out/switchboard.png" "$data_home/icons/hicolor/256x256/apps/switchboard.png" 644
  fi
  [ "$os" = Darwin ] || say "Installed: ${installed[*]+"${installed[*]}"}"
  mkdir -p "$(dirname "$manifest")"
  { [ -f "$manifest" ] && cat "$manifest"; printf '%s\n' "${installed[@]+"${installed[@]}"}"; } | sort -u > "$manifest.new"
  mv "$manifest.new" "$manifest"
  case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) warn "$bin_dir is not on your PATH. Add it to use 'sb' in a terminal." ;;
  esac
}

do_uninstall() {
  if [ ! -f "$manifest" ]; then
    say "Nothing to remove: no install by this script was found ($manifest)."
    return 0
  fi
  local f
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    rm -rf "$f"
  done < "$manifest"
  rm -f "$manifest"
  say "Removed Switchboard."
  echo "Kept: $config_home/switchboard (config), $data_home/switchboard (store), ~/.switchboard (state)."
  if [ -d "$data_home/switchboard/src" ]; then
    echo "Kept the source and its build: $data_home/switchboard/src. Remove it with: rm -rf '$data_home/switchboard/src'"
  fi
  echo "Your agents keep running in tmux (socket 'switchboard'). Stop them with: tmux -L switchboard kill-server"
}

write_config() {
  local os=$1 config="$config_home/switchboard/config.toml"
  [ -f "$config" ] && return 0
  local roots=() seen=() d real r dup
  for d in "$HOME/Documents/Code" "$HOME/code" "$HOME/Code" "$HOME/src" "$HOME/dev" "$HOME/projects" "$HOME/Projects" "$HOME/workspace"; do
    [ -d "$d" ] || continue
    # On macOS ~/code and ~/Code are one folder: list it once.
    real=$(cd "$d" && pwd -P)
    dup=0
    for r in "${seen[@]+"${seen[@]}"}"; do [ "$r" = "$real" ] && dup=1; done
    [ "$dup" = 1 ] && continue
    seen+=("$real")
    roots+=("\"~${d#"$HOME"}\"")
  done
  [ ${#roots[@]} -eq 0 ] && roots=('"~/Documents/Code"')
  local leader='ctrl+space'
  # On macOS ctrl+space often switches the input language.
  [ "$os" = Darwin ] && leader='ctrl+;'
  local list
  list=$(printf '%s, ' "${roots[@]}")
  mkdir -p "$(dirname "$config")"
  {
    echo "# Switchboard config. SPEC.md section 14 has every key."
    echo "# The folders where the app looks for your git repos and PR clones."
    echo "scan = [${list%, }]"
    echo "# The leader key: the first key of every Switchboard shortcut."
    echo "leader = \"$leader\""
  } > "$config"
  say "Wrote a starter config: $config"
}

next_steps() {
  echo
  have claude || echo "- Install Claude Code: curl -fsSL https://claude.ai/install.sh | bash   (then run 'claude' once to log in)"
  if ! have gh; then
    local c
    c=$(install_cmd "$(pkg gh)")
    echo "- For PR reviews, install the GitHub CLI${c:+: $c}   (then: gh auth login)"
  elif ! gh auth status >/dev/null 2>&1; then
    echo "- Log in to GitHub for PR reviews: gh auth login"
  fi
  if [ "$1" = Darwin ]; then
    echo "- Start it: open ~/Applications/Switchboard.app"
  else
    echo "- Start it: switchboard   (or from your app menu)"
  fi
  echo "- Update later: run this script again."
}

main "$@"
