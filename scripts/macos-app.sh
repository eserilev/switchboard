#!/usr/bin/env bash
# Builds Switchboard.app on macOS with cargo and Apple's own tools. No npm, no Tauri CLI.
# The app and `sb` go into Contents/MacOS, so the app finds `sb` next to itself.
# Usage: scripts/macos-app.sh [out-dir]   (default: target/macos)
set -euo pipefail
if [ "$(uname)" != "Darwin" ]; then echo "run this on macOS"; exit 1; fi
root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-$root/target/macos}
target=${CARGO_TARGET_DIR:-$root/target}
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)
version=${version:-0.1.0}

cargo build --release -p switchboard -p sb --manifest-path "$root/Cargo.toml"

app="$out/Switchboard.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$target/release/switchboard" "$target/release/sb" "$app/Contents/MacOS/"

# The icon: an .icns from the PNG, with sips and iconutil.
icons=$(mktemp -d)
mkdir "$icons/icon.iconset"
for s in 16 32 128 256 512; do
  sips -z $s $s "$root/crates/app/icons/icon.png" --out "$icons/icon.iconset/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2))
  sips -z $d $d "$root/crates/app/icons/icon.png" --out "$icons/icon.iconset/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$icons/icon.iconset" -o "$app/Contents/Resources/icon.icns"
rm -rf "$icons"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Switchboard</string>
  <key>CFBundleDisplayName</key><string>Switchboard</string>
  <key>CFBundleIdentifier</key><string>dev.switchboard.app</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleExecutable</key><string>switchboard</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# An ad-hoc signature: enough to run the app on this Mac. To share the app, sign it
# with a Developer ID and notarize it.
codesign --force --deep --sign - "$app"
echo "built $app"
