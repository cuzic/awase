#!/usr/bin/env bash
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
export CLIPD_HOST=dragonflyg4
CW="$HOME/powershell-clipd/target/release/clipwire"
OUT="$HERE/results/chrome-abl-E2-300ms-shifttail700"; mkdir -p "$OUT"
"$CW" exec e2e-diag-desktop >/dev/null 2>&1
"$CW" exec chrome-probe-args-shifttail >/dev/null 2>&1
"$CW" exec chrome-probe-start >/dev/null 2>&1
sleep 190
for _ in $(seq 1 12); do "$CW" exec chrome-probe-fetch >"$OUT/probe.log" 2>/dev/null; grep -q "全ケース完了" "$OUT/probe.log" && break; sleep 15; done
"$CW" exec e2e-fetch-awase-full >"$OUT/awase-full.log" 2>/dev/null
# 回帰: Win32 EDITの通常10手順(倍速12回、Engine判定も含む)
REAL_ONLY= bash "$HERE/run_loop.sh" e2e-args-loop-fast2x "$HERE/results/loop-e2-300-regress" 22 > "$HERE/results/loop-e2-300-regress.out" 2>&1
echo done > "$HERE/results/verify-e2-300.done"
