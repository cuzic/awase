#!/usr/bin/env bash
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
export REAL_ONLY=1
for cfg in "e2e-args-multi12 32 A-same" "e2e-args-multi12-fast 25 B-fast" "e2e-args-multi12-fast2x 15 C-fast2x"; do
  set -- $cfg
  s=$(date +%s)
  bash "$HERE/run_multi.sh" "$1" "$HERE/results/multi-$3" "$2" > "$HERE/results/multi-$3.out" 2>&1
  e=$(date +%s)
  echo "$3: 所要 $((e-s))秒 / $(tail -1 "$HERE/results/multi-$3.out")" >> "$HERE/results/BENCH.md"
done
echo done > "$HERE/results/bench-multi.done"
