#!/usr/bin/env bash
# Builds the release binary and (optionally) installers.
#
#   ./scripts/build.sh            -> binary + deb + AppImage
#   ./scripts/build.sh --no-bundle -> binary only
set -euo pipefail

cd "$(dirname "$0")/.."

if [ ! -d node_modules ]; then
  npm ci --no-audit --no-fund
fi

npm run build
exec npm run tauri build -- "$@"
