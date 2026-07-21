#!/usr/bin/env bash
#
# Bump the app version. `tauri.conf.json` is the source of truth — it drives
# package_info().version and the macOS CFBundleShortVersionString — and the crate's
# Cargo.toml is kept in lockstep so `cargo` metadata agrees.
#
# Usage: scripts/bump-version.sh <x.y.z>

set -euo pipefail

VERSION="${1:-}"
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: $0 <x.y.z>   (semver, e.g. 0.2.0)" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONF="$ROOT/app/src-tauri/tauri.conf.json"
CARGO="$ROOT/app/src-tauri/Cargo.toml"

# tauri.conf.json has exactly one "version" key (top level), so a targeted replace is
# safe and preserves the file's formatting.
/usr/bin/sed -i '' -E \
  "s/(\"version\"[[:space:]]*:[[:space:]]*\")[0-9]+\.[0-9]+\.[0-9]+/\1${VERSION}/" "$CONF"

# The crate's only `version = "x.y.z"` literal is the package version; every other field
# here inherits from the workspace, so an anchored match hits just that line.
/usr/bin/sed -i '' -E \
  "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+\"/version = \"${VERSION}\"/" "$CARGO"

echo "Bumped to v${VERSION}"
echo "  $CONF"
echo "  $CARGO"
echo "Now run scripts/update-app.sh to build and install it."
