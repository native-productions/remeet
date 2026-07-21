#!/usr/bin/env bash
#
# Build the release bundle and (re)install it to /Applications — the one command to
# refresh the real, installed Remeet after code changes. The dev build (`bun run app`)
# is separate and untouched.
#
# Usage: scripts/update-app.sh

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_SRC="$ROOT/target/release/bundle/macos/Remeet.app"
APP_DST="/Applications/Remeet.app"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"

# The AEC dependency (webrtc-audio-processing, bundled) compiles its C++ with meson +
# ninja. They must be on PATH, and the deployment target must be macOS 10.15+ or the
# bundled ggml/webrtc C++ fails on std::filesystem (see tauri.conf.json). The release
# build otherwise inherits Tauri's default (10.13); pin it here to match the app's real
# floor and keep the native builds green.
for tool in meson ninja; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "error: '$tool' not found on PATH — needed to build the bundled webrtc AEC." >&2
    echo "       install it (e.g. 'brew install meson ninja') and re-run." >&2
    exit 1
  fi
done
export MACOSX_DEPLOYMENT_TARGET=15.0

echo "==> Building release bundle…"
( cd "$ROOT/app/ui" && bun run app:build )

if [[ ! -d "$APP_SRC" ]]; then
  echo "build did not produce $APP_SRC" >&2
  exit 1
fi

# Re-sign ad-hoc: the seal Tauri writes can be inconsistent after the bundle is copied,
# which trips `codesign --verify`; a fresh ad-hoc signature keeps it clean for local use.
echo "==> Re-signing ad-hoc…"
codesign --force --deep --sign - "$APP_SRC"

echo "==> Installing to /Applications…"
rm -rf "$APP_DST"
cp -R "$APP_SRC" "$APP_DST"

# Register so Spotlight and Launchpad pick it up immediately.
"$LSREGISTER" -f "$APP_DST" || true

VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP_DST/Contents/Info.plist" 2>/dev/null || echo '?')"
echo "==> Installed Remeet v${VERSION} → /Applications"
echo "    Launch from Spotlight/Launchpad, or: open \"$APP_DST\""
