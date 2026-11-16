#!/usr/bin/env bash
# Long-running stability sampler for the native overlay.
#
# Samples the whole process tree once per minute and exercises the window at
# fixed points:
#   minute 5  10 x --toggle (show/hide)
#   minute 10 / 20  open Settings for 3 s, close it, wait 5 s
#
# Writes CSV (PSS/RSS in kB, child count, cumulative CPU ticks) and always asks
# the test instance to quit when it exits.
#
#   ./scripts/stability.sh              # 30 minutes, /tmp/dlo-stability.csv
#   ./scripts/stability.sh 10           # shorter smoke run
#   ./scripts/stability.sh 30 out.csv   # custom output path
#
# This starts a fresh instance: it first asks any running instance to quit.
set -uo pipefail

cd "$(dirname "$0")/.."

BIN=${BIN:-src-tauri/target/release/discord-linux-overlay}
DURATION=${1:-30}
OUT=${2:-/tmp/dlo-stability.csv}
LOG=/tmp/dlo-stability-app.log

if [ ! -x "$BIN" ]; then
  echo "release binary not found: $BIN" >&2
  echo "build it first: cargo build --manifest-path src-tauri/Cargo.toml --features custom-protocol --release" >&2
  exit 1
fi

cleanup() {
  "$BIN" --quit >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

"$BIN" --quit >/dev/null 2>&1
sleep 1

setsid env -u GDK_BACKEND "$BIN" >"$LOG" 2>&1 < /dev/null &
ROOT=""
for _ in $(seq 1 150); do
  ROOT=$(pgrep -x discord-linux-o | head -1)
  [ -n "$ROOT" ] && "$BIN" --diagnose 2>/dev/null | grep -q '^visible: yes' && break
  sleep 0.2
done
if [ -z "$ROOT" ]; then
  echo "failed to start the application; see $LOG" >&2
  exit 1
fi

tree_pids() {
  local all f n p c
  all=$(ps -eo pid=,ppid=)
  f="$1"
  n=""
  echo "$1"
  while [ -n "$f" ]; do
    n=""
    for p in $f; do
      for c in $(printf '%s\n' "$all" | awk -v p="$p" '$2 == p {print $1}'); do
        echo "$c"
        n="$n $c"
      done
    done
    f="$n"
  done
}

# Sets S_PSS, S_RSS, S_CHILDREN, S_TICKS for the current tree.
sample() {
  local p v
  S_PSS=0
  S_RSS=0
  S_TICKS=0
  S_CHILDREN=$(ps -eo ppid= | awk -v r="$ROOT" '$1 == r' | wc -l)
  for p in $(tree_pids "$ROOT"); do
    v=$(awk '/^Pss:/{print $2}' "/proc/$p/smaps_rollup" 2>/dev/null)
    S_PSS=$((S_PSS + ${v:-0}))
    v=$(awk '/^Rss:/{print $2}' "/proc/$p/smaps_rollup" 2>/dev/null)
    S_RSS=$((S_RSS + ${v:-0}))
    v=$(awk '{print $14 + $15}' "/proc/$p/stat" 2>/dev/null)
    S_TICKS=$((S_TICKS + ${v:-0}))
  done
}

echo "minute,event,pss_kb,rss_kb,children,cpu_ticks" >"$OUT"
echo "stability: $DURATION min, binary $BIN, output $OUT"

for i in $(seq 0 "$DURATION"); do
  event=idle
  [ "$i" -eq 5 ] && event=toggles
  [ "$i" -eq 10 ] && event=settings1
  [ "$i" -eq 20 ] && event=settings2

  sample
  echo "$i,$event,$S_PSS,$S_RSS,$S_CHILDREN,$S_TICKS" >>"$OUT"
  printf 'minute %2d  %-14s PSS %6.1f MiB  RSS %6.1f MiB  children %s\n' \
    "$i" "$event" \
    "$(awk -v v="$S_PSS" 'BEGIN{print v/1024}')" \
    "$(awk -v v="$S_RSS" 'BEGIN{print v/1024}')" "$S_CHILDREN"

  if [ "$i" -eq 5 ]; then
    for _ in $(seq 1 10); do "$BIN" --toggle >/dev/null 2>&1; done
  fi

  if [ "$i" -eq 10 ] || [ "$i" -eq 20 ]; then
    "$BIN" --settings >/dev/null 2>&1
    sleep 3
    sample
    echo "$i,settings-open,$S_PSS,$S_RSS,$S_CHILDREN,$S_TICKS" >>"$OUT"
    printf '          %-14s PSS %6.1f MiB  RSS %6.1f MiB  children %s\n' \
      "settings-open" \
      "$(awk -v v="$S_PSS" 'BEGIN{print v/1024}')" \
      "$(awk -v v="$S_RSS" 'BEGIN{print v/1024}')" "$S_CHILDREN"

    "$BIN" --close-settings >/dev/null 2>&1
    sleep 5
    sample
    echo "$i,settings-closed,$S_PSS,$S_RSS,$S_CHILDREN,$S_TICKS" >>"$OUT"
    printf '          %-14s PSS %6.1f MiB  RSS %6.1f MiB  children %s\n' \
      "settings-closed" \
      "$(awk -v v="$S_PSS" 'BEGIN{print v/1024}')" \
      "$(awk -v v="$S_RSS" 'BEGIN{print v/1024}')" "$S_CHILDREN"
  fi

  [ "$i" -lt "$DURATION" ] && sleep 60
done

echo "DONE" >>"$OUT"
echo "stability: done; CSV at $OUT"
