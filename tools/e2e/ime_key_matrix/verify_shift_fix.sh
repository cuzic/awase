#!/usr/bin/env bash
# Shift+無変換の修正を実機で検証する: 実装ブランチのawaseをデプロイ → --shiftmuh(観測) → 通常の10手順(回帰、倍速)。
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
export CLIPD_HOST=dragonflyg4 REAL_ONLY=1
CW="$HOME/powershell-clipd/target/release/clipwire"
WT="$HOME/rust-nicola-worktrees/adr186-ablation"
git -C "$WT" checkout -q -f -B e2e/ablation feat/adr186-nonconvert-toggle-belief-follow && git -C "$WT" push -q -f origin e2e/ablation 2>&1 | tail -1
"$CW" exec e2e-config-toggle-true >/dev/null 2>&1
"$CW" exec e2e-deploy >/dev/null 2>&1
for _ in $(seq 1 40); do
  out=$("$CW" exec adr186-deploy-check 2>&1)
  echo "$out" | grep -q "BUILD_OK_STARTED_TESTINJ" && break
  echo "$out" | grep -q "BUILD_FAILED" && { echo "awaseのビルド失敗"; echo "$out" | tail -5; exit 2; }
  sleep 15
done
echo "$out" | grep -q "BUILD_OK_STARTED_TESTINJ" || { echo "ビルド待ちtimeout"; exit 2; }
sleep 5
# (1) Shift+無変換の観測(4回)
"$CW" exec e2e-diag-desktop >/dev/null 2>&1
"$CW" exec e2e-args-shiftmuh >/dev/null 2>&1; "$CW" exec e2e-loop-n4 >/dev/null 2>&1; "$CW" exec e2e-loop-start >/dev/null 2>&1
sleep 130
mkdir -p "$HERE/results/shiftmuh-fixed"
for _ in $(seq 1 12); do "$CW" exec e2e-fetch-multi >"$HERE/results/shiftmuh-fixed/spike.log" 2>/dev/null; grep -q "LOOP DONE" "$HERE/results/shiftmuh-fixed/spike.log" && break; sleep 15; done
"$CW" exec e2e-fetch-awase-full >"$HERE/results/shiftmuh-fixed/awase-full.log" 2>/dev/null
# (2) 回帰: 通常の10手順(倍速、実IMEだけでなくEngine判定も含む)を12回
REAL_ONLY= bash "$HERE/run_loop.sh" e2e-args-loop-fast2x "$HERE/results/loop-shiftfix-regress" 22 > "$HERE/results/loop-shiftfix-regress.out" 2>&1
echo done > "$HERE/results/verify-shift.done"
