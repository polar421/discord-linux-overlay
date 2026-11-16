#!/usr/bin/env bash
# Reproducible memory/CPU benchmark for Discord Linux Overlay.
#
# Measures the *entire* application process tree (main process, WebKit web and
# network processes, any helpers) using smaps_rollup PSS, which is the honest
# measure of physical memory attributable to the app. RSS is reported too, but
# it double-counts shared pages when summed across processes.
#
#   ./scripts/benchmark.sh                     # 3 runs, 20 s sampling, settings closed
#   ./scripts/benchmark.sh --settings          # same, then open Settings and sample again
#   ./scripts/benchmark.sh --repeat 1 --duration 60
#   ./scripts/benchmark.sh --long              # 3 runs of 5 minutes (stability)
#
# Requirements: bash, awk, ps, pgrep; optional nvidia-smi for GPU context.
# Nothing is installed and no system configuration is changed.
set -uo pipefail

cd "$(dirname "$0")/.."

BIN=${BIN:-src-tauri/target/release/discord-linux-overlay}
REPEAT=3
DURATION=20
SETTINGS=0
LONG=0
KEEP=0

while [ $# -gt 0 ]; do
  case "$1" in
    --repeat) REPEAT=$2; shift 2 ;;
    --duration) DURATION=$2; shift 2 ;;
    --settings) SETTINGS=1; shift ;;
    --long) LONG=1; REPEAT=1; DURATION=300; shift ;;
    --keep-running) KEEP=1; shift ;;
    --binary) BIN=$2; shift 2 ;;
    -h | --help) sed -n '2,18p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

if [ ! -x "$BIN" ]; then
  echo "release binary not found: $BIN" >&2
  echo "build it first: cargo build --manifest-path src-tauri/Cargo.toml --features custom-protocol --release" >&2
  exit 1
fi

CLK=$(getconf CLK_TCK 2>/dev/null || echo 100)
APP_PID=""

# --- helpers ---------------------------------------------------------------

tree_pids() {
  # Breadth-first walk of the process tree from $1 using ps (comm-safe).
  local root="$1"
  local all
  all=$(ps -eo pid=,ppid= 2>/dev/null)
  local frontier="$root" next="" pid child
  echo "$root"
  while [ -n "$frontier" ]; do
    next=""
    for pid in $frontier; do
      for child in $(printf '%s\n' "$all" | awk -v p="$pid" '$2 == p {print $1}'); do
        echo "$child"
        next="$next $child"
      done
    done
    frontier="$next"
  done
}

sum_field() {
  # sum_field <field-name> <pid...>
  local field="$1" total=0 pid value
  shift
  for pid in "$@"; do
    value=$(awk -v key="$field" '$1 == key {print $2}' "/proc/$pid/smaps_rollup" 2>/dev/null)
    [ -n "$value" ] && total=$((total + value))
  done
  echo "$total"
}

status_field() {
  awk -v key="$1" '$1 == key {print $2}' "/proc/$2/status" 2>/dev/null
}

pid_cpu_ticks() {
  awk '{print $14 + $15}' "/proc/$1/stat" 2>/dev/null || echo 0
}

gpu_line() {
  if command -v nvidia-smi >/dev/null 2>&1; then
    nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader 2>/dev/null | head -1
  else
    echo "n/a"
  fi
}

wait_ready() {
  local deadline=$((SECONDS + 30))
  while [ $SECONDS -lt $deadline ]; do
    if "$BIN" --diagnose 2>/dev/null | grep -q "^visible: yes"; then
      return 0
    fi
    sleep 0.2
  done
  return 1
}

sample() {
  local pid root="$APP_PID" pids
  pids=$(tree_pids "$root" | tr '\n' ' ')
  local pss rss priv_dirty priv_clean shared pids_arr
  pss=$(sum_field "Pss:" $pids)
  rss=$(sum_field "Rss:" $pids)
  priv_dirty=$(sum_field "Private_Dirty:" $pids)
  priv_clean=$(sum_field "Private_Clean:" $pids)
  shared=$(sum_field "Shared_Clean:" $pids)
  pids_arr=$pids
  echo "$pss $rss $priv_dirty $priv_clean $shared $pids_arr"
}

per_process() {
  local pid comm pss rss
  printf '    %-22s %8s %8s\n' "process" "PSS kB" "RSS kB"
  for pid in $(tree_pids "$APP_PID"); do
    comm=$(awk '{gsub(/[()]/,"",$2); print $2}' "/proc/$pid/stat" 2>/dev/null)
    pss=$(awk '/^Pss:/{print $2}' "/proc/$pid/smaps_rollup" 2>/dev/null)
    rss=$(awk '/^Rss:/{print $2}' "/proc/$pid/smaps_rollup" 2>/dev/null)
    [ -n "$pss" ] && printf '    %-22s %8s %8s\n' "${comm:-?}" "$pss" "$rss"
  done
}

start_app() {
  # Match the normal desktop launch: native Wayland unless the caller says
  # otherwise. GDK_BACKEND is intentionally not inherited from this shell.
  setsid env -u GDK_BACKEND "$BIN" > /tmp/dlo-benchmark.log 2>&1 < /dev/null &
  local deadline=$((SECONDS + 30))
  while [ $SECONDS -lt $deadline ]; do
    APP_PID=$(pgrep -x discord-linux-o | head -1)
    [ -n "$APP_PID" ] && break
    sleep 0.1
  done
  [ -n "$APP_PID" ]
}

quit_app() {
  [ -n "$APP_PID" ] && "$BIN" --quit >/dev/null 2>&1
  local deadline=$((SECONDS + 10))
  while [ $SECONDS -lt $deadline ] && pgrep -x discord-linux-o >/dev/null; do
    sleep 0.2
  done
  if pgrep -x discord-linux-o >/dev/null; then
    kill "$APP_PID" 2>/dev/null
  fi
  APP_PID=""
}

run_once() {
  local scenario="$1" start_ms end_ms startup
  start_ms=$(date +%s%3N)
  if ! start_app; then
    echo "  FAILED to start the application (see /tmp/dlo-benchmark.log)" >&2
    return 1
  fi
  if ! wait_ready; then
    echo "  FAILED: overlay did not become visible (see /tmp/dlo-benchmark.log)" >&2
    quit_app
    return 1
  fi
  end_ms=$(date +%s%3N)
  startup=$((end_ms - start_ms))
  sleep 3

  local samples=() i
  for i in $(seq 1 5); do
    samples+=("$(sample)")
    sleep "$(awk -v d="$DURATION" 'BEGIN{print d/5}')"
  done

  # Median of the five samples: stable, and robust to one transient spike.
  local pss rss pd pc sh
  median() {
    printf '%s\n' "${samples[@]}" | awk -v col="$1" '{print $col}' | sort -n | awk '{a[NR]=$1} END{if(NR==0){print 0}else if(NR%2){print a[(NR+1)/2]}else{printf "%d", (a[NR/2]+a[NR/2+1])/2}}'
  }
  pss=$(median 1)
  rss=$(median 2)
  pd=$(median 3)
  pc=$(median 4)
  sh=$(median 5)

  local t1 t2 cpu_ticks=0 pid
  t1=0
  for pid in $(tree_pids "$APP_PID"); do
    t1=$((t1 + $(pid_cpu_ticks "$pid")))
  done
  local gpu_before gpu_after
  gpu_before=$(gpu_line)
  sleep 5
  t2=0
  for pid in $(tree_pids "$APP_PID"); do
    t2=$((t2 + $(pid_cpu_ticks "$pid")))
  done
  gpu_after=$(gpu_line)

  local cpu_pct
  cpu_pct=$(awk -v t="$((t2 - t1))" -v hz="$CLK" 'BEGIN{printf "%.2f", (t * 100.0) / (hz * 5.0)}')

  printf '  %-8s PSS %7.1f MiB | RSS %7.1f MiB | priv_dirty %7.1f MiB | priv_clean %7.1f MiB | shared %7.1f MiB | CPU %5s%% | startup %5d ms\n' \
    "$scenario" \
    "$(awk -v v="$pss" 'BEGIN{print v/1024}')" \
    "$(awk -v v="$rss" 'BEGIN{print v/1024}')" \
    "$(awk -v v="$pd" 'BEGIN{print v/1024}')" \
    "$(awk -v v="$pc" 'BEGIN{print v/1024}')" \
    "$(awk -v v="$sh" 'BEGIN{print v/1024}')" \
    "$cpu_pct" "$startup"

  if [ "$SETTINGS" -eq 1 ] && [ "$scenario" = "idle" ]; then
    "$BIN" --settings >/dev/null 2>&1
    sleep 3
    local s_pss s_rss s_pd s_pc s_sh
    read -r s_pss s_rss s_pd s_pc s_sh _ <<<"$(sample)"
    printf '  %-8s PSS %7.1f MiB | RSS %7.1f MiB | priv_dirty %7.1f MiB | priv_clean %7.1f MiB | shared %7.1f MiB\n' \
      "settings" \
      "$(awk -v v="$s_pss" 'BEGIN{print v/1024}')" \
      "$(awk -v v="$s_rss" 'BEGIN{print v/1024}')" \
      "$(awk -v v="$s_pd" 'BEGIN{print v/1024}')" \
      "$(awk -v v="$s_pc" 'BEGIN{print v/1024}')" \
      "$(awk -v v="$s_sh" 'BEGIN{print v/1024}')"
    "$BIN" --close-settings >/dev/null 2>&1
    sleep 4
    local c_pss c_rss c_pd c_pc c_sh
    read -r c_pss c_rss c_pd c_pc c_sh _ <<<"$(sample)"
    printf '  %-8s PSS %7.1f MiB | RSS %7.1f MiB | priv_dirty %7.1f MiB | priv_clean %7.1f MiB | shared %7.1f MiB\n' \
      "closed" \
      "$(awk -v v="$c_pss" 'BEGIN{print v/1024}')" \
      "$(awk -v v="$c_rss" 'BEGIN{print v/1024}')" \
      "$(awk -v v="$c_pd" 'BEGIN{print v/1024}')" \
      "$(awk -v v="$c_pc" 'BEGIN{print v/1024}')" \
      "$(awk -v v="$c_sh" 'BEGIN{print v/1024}')"
  fi

  if [ "${DLO_BENCH_VERBOSE:-0}" = "1" ]; then
    per_process
  fi

  printf '  gpu: before=%s after=%s\n' "$gpu_before" "$gpu_after"

  if [ "$KEEP" -eq 0 ]; then
    quit_app
  fi
}

# --- main ------------------------------------------------------------------

echo "Discord Linux Overlay benchmark"
echo "binary:   $BIN"
echo "scenario: settings $([ "$SETTINGS" -eq 1 ] && echo "open/close sampled" || echo closed), $REPEAT run(s) x ${DURATION}s sampling"
echo "kernel:   $(uname -r), session: ${XDG_SESSION_TYPE:-unknown}"
echo "gpu:      $(gpu_line)"
echo

trap 'quit_app' EXIT
for run in $(seq 1 "$REPEAT"); do
  echo "run $run/$REPEAT"
  run_once "idle" || exit 1
  if [ "$KEEP" -eq 1 ]; then
    echo "application left running (pid $APP_PID)"
    trap - EXIT
    exit 0
  fi
  echo
done

echo "done. Values are for the whole process tree; PSS is the primary metric."
