#!/usr/bin/env bash
# Fast performance regression gate.
#
# Runs one short benchmark of the release build and fails when the whole-tree
# idle footprint, CPU usage or startup time regresses beyond the configured
# thresholds. This is a local check: it needs a desktop session (GTK/Wayland or
# X11), so it is not part of GitHub Actions.
#
#   ./scripts/perf-check.sh
#   PERF_MAX_PSS_MIB=200 PERF_MAX_CPU=0.8 PERF_MAX_STARTUP_MS=1500 ./scripts/perf-check.sh
#
# Exit status: 0 = within thresholds, 1 = regression or benchmark failure.
set -uo pipefail

cd "$(dirname "$0")/.."

MAX_PSS=${PERF_MAX_PSS_MIB:-200}
MAX_CPU=${PERF_MAX_CPU:-0.8}
MAX_STARTUP=${PERF_MAX_STARTUP_MS:-1500}

OUTPUT=$(./scripts/benchmark.sh --repeat 1 --duration 8 2>&1)
STATUS=$?
printf '%s\n' "$OUTPUT"
if [ "$STATUS" -ne 0 ]; then
  echo "perf-check: benchmark failed" >&2
  exit 1
fi

LINE=$(printf '%s\n' "$OUTPUT" | awk '/^  idle/{print; exit}')
if [ -z "$LINE" ]; then
  echo "perf-check: could not find an idle measurement" >&2
  exit 1
fi

read -r PSS CPU STARTUP <<<"$(printf '%s\n' "$LINE" | awk '{
  for (i = 1; i <= NF; i++) {
    if ($i == "PSS") pss = $(i+1);
    if ($i == "CPU") cpu = $(i+1);
    if ($i == "startup") startup = $(i+1);
  }
  gsub("%", "", cpu);
  print pss, cpu, startup;
}')"

fail=0
awk -v v="$PSS" -v m="$MAX_PSS" 'BEGIN{exit !(v > m)}' && {
  echo "perf-check: FAIL idle PSS ${PSS} MiB > ${MAX_PSS} MiB" >&2
  fail=1
}
awk -v v="$CPU" -v m="$MAX_CPU" 'BEGIN{exit !(v > m)}' && {
  echo "perf-check: FAIL idle CPU ${CPU}% > ${MAX_CPU}%" >&2
  fail=1
}
awk -v v="$STARTUP" -v m="$MAX_STARTUP" 'BEGIN{exit !(v > m)}' && {
  echo "perf-check: FAIL startup ${STARTUP} ms > ${MAX_STARTUP} ms" >&2
  fail=1
}

if [ "$fail" -ne 0 ]; then
  exit 1
fi

echo "perf-check: OK (PSS ${PSS} MiB <= ${MAX_PSS}, CPU ${CPU}% <= ${MAX_CPU}, startup ${STARTUP} ms <= ${MAX_STARTUP})"
