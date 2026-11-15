#!/usr/bin/env bash
# Starts the overlay in development mode (Vite + Tauri, hot reload).
set -euo pipefail

cd "$(dirname "$0")/.."

if [ ! -d node_modules ]; then
  npm ci --no-audit --no-fund
fi

exec npm run tauri dev
