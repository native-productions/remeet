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
