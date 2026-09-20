#!/usr/bin/env bash
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
export CLIPD_HOST=dragonflyg4
REAL_ONLY= bash "$HERE/run_loop.sh" e2e-args-loop-fast2x "$HERE/results/loop-e3-regress" 22 > "$HERE/results/loop-e3-regress.out" 2>&1
echo done > "$HERE/results/verify-e3-regress.done"
