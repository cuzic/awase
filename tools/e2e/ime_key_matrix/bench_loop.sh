#!/usr/bin/env bash
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
export REAL_ONLY=1
for cfg in "e2e-args-default 40 L-same" "e2e-args-loop-fast2x 22 L-fast2x"; do
  set -- $cfg
  s=$(date +%s)
  bash "$HERE/run_loop.sh" "$1" "$HERE/results/loop-$3" "$2" > "$HERE/results/loop-$3.out" 2>&1
  e=$(date +%s)
  echo "$3: 所要 $((e-s))秒 / $(tail -1 "$HERE/results/loop-$3.out")" >> "$HERE/results/BENCH.md"
done
echo done > "$HERE/results/bench-loop.done"
