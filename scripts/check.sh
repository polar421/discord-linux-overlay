#!/usr/bin/env bash
# Runs every check that does not need a desktop session.
set -euo pipefail

cd "$(dirname "$0")/.."

echo "== core crate tests =="
cargo test -p discord-overlay-core --features http

echo
echo "== front end typecheck =="
if [ -d node_modules ]; then
  npm run typecheck
else
  echo "node_modules missing; run: npm ci"
  exit 1
fi

echo
echo "== front end build =="
npm run build

echo
echo "== tauri shell build =="
if pkg-config --exists webkit2gtk-4.1 2>/dev/null && pkg-config --exists gtk+-3.0 2>/dev/null; then
  cargo build --manifest-path src-tauri/Cargo.toml --features custom-protocol
  echo "binary: src-tauri/target/debug/discord-linux-overlay"
else
  echo "skipped: libwebkit2gtk-4.1-dev / libgtk-3-dev are not installed."
  echo "On Arch: sudo pacman -S webkit2gtk-4.1 gtk3 libayatana-appindicator"
  echo "The Tauri shell is still checked by CI (.github/workflows/ci.yml)."
fi
