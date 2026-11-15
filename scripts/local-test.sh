#!/usr/bin/env bash
# Local pre-flight check and build for Discord Linux Overlay.
#
#   ./scripts/local-test.sh              check the machine, then build (debug)
#   ./scripts/local-test.sh --release    check the machine, then build release
#   ./scripts/local-test.sh --skip-build only run the environment checks
#
# Exits non-zero when something required is missing or a build step fails.
# Nothing here installs packages or changes system configuration.
set -uo pipefail

cd "$(dirname "$0")/.."

RELEASE=0
SKIP_BUILD=0
for arg in "$@"; do
  case "$arg" in
    --release) RELEASE=1 ;;
    --skip-build) SKIP_BUILD=1 ;;
    -h | --help)
      sed -n '2,10p' "$0"
      exit 0
      ;;
    *)
      echo "unknown option: $arg" >&2
      exit 2
      ;;
  esac
done

failures=0
warnings=0

ok() { printf '  ok    %s\n' "$1"; }
bad() {
  printf '  FAIL  %s\n' "$1"
  failures=$((failures + 1))
}
warn() {
  printf '  warn  %s\n' "$1"
  warnings=$((warnings + 1))
}

section() { printf '\n== %s ==\n' "$1"; }

# ---------------------------------------------------------------- tools
section "Required tools"
for tool in cargo rustc node npm pkg-config git; do
  if command -v "$tool" >/dev/null 2>&1; then
    ok "$tool: $(command -v "$tool")"
  else
    bad "$tool is missing (Arch: sudo pacman -S --needed rust nodejs npm pkgconf git)"
  fi
done

if command -v cargo >/dev/null 2>&1; then
  ok "cargo $(cargo --version | cut -d' ' -f2)"
fi
if command -v node >/dev/null 2>&1; then
  node_version=$(node --version)
  ok "node $node_version"
  major=${node_version#v}
  major=${major%%.*}
  if [ "${major:-0}" -lt 20 ] 2>/dev/null; then
    bad "Node.js 20 or newer is required (found $node_version)"
  fi
fi

# ---------------------------------------------------------------- libraries
section "Linux libraries (pkg-config)"
check_pkg() {
  local package="$1" hint="$2" minimum="${3:-}"
  if ! command -v pkg-config >/dev/null 2>&1; then
    bad "pkg-config unavailable, cannot check $package"
    return
  fi
  local version
  version=$(pkg-config --modversion "$package" 2>/dev/null)
  if [ -z "$version" ]; then
    bad "$package not found — $hint"
    return
  fi
  if [ -n "$minimum" ] && ! printf '%s\n%s\n' "$minimum" "$version" | sort -V -C; then
    bad "$package $version is older than $minimum — $hint"
    return
  fi
  ok "$package $version"
}

check_pkg webkit2gtk-4.1 "sudo pacman -S --needed webkit2gtk-4.1" "2.38"
check_pkg gtk+-3.0 "sudo pacman -S --needed gtk3" "3.24"
check_pkg ayatana-appindicator3-0.1 "sudo pacman -S --needed libayatana-appindicator" ""
check_pkg librsvg-2.0 "sudo pacman -S --needed librsvg" ""

# ---------------------------------------------------------------- session
section "Session"
session_type=${XDG_SESSION_TYPE:-unknown}
desktop=${XDG_CURRENT_DESKTOP:-${DESKTOP_SESSION:-unknown}}
printf '  XDG_SESSION_TYPE=%s\n  XDG_CURRENT_DESKTOP=%s\n' "$session_type" "$desktop"
printf '  DISPLAY=%s\n  WAYLAND_DISPLAY=%s\n  XDG_RUNTIME_DIR=%s\n' \
  "${DISPLAY:-<unset>}" "${WAYLAND_DISPLAY:-<unset>}" "${XDG_RUNTIME_DIR:-<unset>}"

if [ "${WAYLAND_DISPLAY:-}" = "" ] && [ "${DISPLAY:-}" = "" ]; then
  warn "no display server detected — the app can be built but not launched from this shell"
fi

is_wayland=0
if [ "${WAYLAND_DISPLAY:-}" != "" ] && [ "${GDK_BACKEND:-}" != "x11" ]; then
  is_wayland=1
fi

if [ -z "${XDG_RUNTIME_DIR:-}" ]; then
  bad "XDG_RUNTIME_DIR is unset; Discord IPC and the control socket need it"
fi

# ---------------------------------------------------------------- GPU
section "Graphics"
nvidia=0
if [ -r /proc/driver/nvidia/version ]; then
  nvidia=1
  header=$(head -1 /proc/driver/nvidia/version)
  driver=$(printf '%s' "$header" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)
  ok "NVIDIA kernel driver: ${driver:-unknown}"
  if command -v nvidia-smi >/dev/null 2>&1; then
    nvidia-smi --query-gpu=name,driver_version --format=csv,noheader | sed 's/^/  gpu   /'
  fi
else
  ok "no NVIDIA driver interface (/proc/driver/nvidia/version absent)"
fi

printf '  __NV_DISABLE_EXPLICIT_SYNC=%s\n' "${__NV_DISABLE_EXPLICIT_SYNC:-<unset>}"
printf '  WEBKIT_DISABLE_DMABUF_RENDERER=%s\n' "${WEBKIT_DISABLE_DMABUF_RENDERER:-<unset>}"
printf '  DLO_NO_GRAPHICS_WORKAROUNDS=%s\n' "${DLO_NO_GRAPHICS_WORKAROUNDS:-<unset>}"
printf '  GDK_BACKEND=%s\n' "${GDK_BACKEND:-<unset>}"

if [ "$nvidia" -eq 1 ] && [ "$is_wayland" -eq 1 ]; then
  ok "the application will set __NV_DISABLE_EXPLICIT_SYNC=1 itself at startup"
fi

# ---------------------------------------------------------------- Discord
section "Discord IPC"
runtime=${XDG_RUNTIME_DIR:-}
sockets=()
[ -n "$runtime" ] && sockets+=("$runtime/discord-ipc-0" "$runtime/app/com.discordapp.Discord/discord-ipc-0" "$runtime/snap.discord/discord-ipc-0")
sockets+=("/tmp/discord-ipc-0")
if [ -n "${DISCORD_IPC_PATH:-}" ]; then
  sockets=("$DISCORD_IPC_PATH" "${sockets[@]}")
fi

found_socket=0
for path in "${sockets[@]}"; do
  if [ -S "$path" ]; then
    ok "socket present: $path"
    found_socket=1
  fi
done
if [ "$found_socket" -eq 0 ]; then
  warn "no Discord IPC socket found — start the official Discord client before using the overlay"
fi
if command -v pgrep >/dev/null 2>&1 && pgrep -x discord >/dev/null 2>&1; then
  ok "Discord process is running"
fi

# ---------------------------------------------------------------- caveats
section "KDE / Wayland caveats"
if [ "$is_wayland" -eq 1 ]; then
  echo "  - Wayland does not let applications raise themselves: add a KWin rule"
  echo "    (Window Rules → Keep above other windows) for the overlay window."
  echo "  - Absolute window positioning is ignored by KWin; drag the overlay once."
  echo "  - Global hotkeys are unavailable; bind the CLI in System Settings → Shortcuts."
  echo "  - Exclusive fullscreen games cannot show an overlay; use borderless windowed."
else
  echo "  - X11 session: placement and always-on-top use the normal window-manager path."
fi
if [ "${XDG_SESSION_TYPE:-}" = "wayland" ] && [ "$is_wayland" -eq 0 ] && [ "${GDK_BACKEND:-}" = "x11" ]; then
  echo "  - this shell forces XWayland (GDK_BACKEND=x11). Unset GDK_BACKEND to test"
  echo "    native Wayland: env -u GDK_BACKEND ./src-tauri/target/debug/discord-linux-overlay"
fi
if [ "$is_wayland" -eq 1 ] && [ "$nvidia" -eq 1 ]; then
  echo "  - NVIDIA + Wayland: if the window is blank, run with"
  echo "    WEBKIT_DISABLE_DMABUF_RENDERER=1 ./target/debug/discord-linux-overlay"
fi

# ---------------------------------------------------------------- builds
if [ "$SKIP_BUILD" -eq 0 ]; then
  section "Core tests"
  if cargo test -p discord-overlay-core --features http; then
    ok "core tests passed"
  else
    bad "core tests failed"
  fi

  section "Front end"
  if [ ! -d node_modules ]; then
    if npm ci --no-audit --no-fund; then
      ok "npm ci"
    else
      bad "npm ci failed"
    fi
  fi
  if npm run build; then
    ok "front end built (dist/)"
  else
    bad "front end build failed"
  fi

  section "Tauri application"
  build_args=(--manifest-path src-tauri/Cargo.toml --features custom-protocol)
  profile=debug
  if [ "$RELEASE" -eq 1 ]; then
    build_args+=(--release)
    profile=release
  fi
  if cargo build "${build_args[@]}"; then
    ok "binary: src-tauri/target/$profile/discord-linux-overlay"
  else
    bad "Tauri build failed — read the first cargo error above"
  fi
fi

# ---------------------------------------------------------------- summary
section "Summary"
echo "  failures: $failures"
echo "  warnings: $warnings"
if [ "$failures" -eq 0 ]; then
  echo
  echo "Environment and build look usable. Launch with:"
  echo "  ./src-tauri/target/debug/discord-linux-overlay        # or target/release/…"
  echo "Then check the live state with:"
  echo "  ./src-tauri/target/debug/discord-linux-overlay --diagnose"
  exit 0
fi

echo
echo "Fix the items marked FAIL, then run this script again."
exit 1
