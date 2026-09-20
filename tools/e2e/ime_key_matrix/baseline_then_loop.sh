#!/usr/bin/env bash
# awase を基準ビルド(ADR-186実装ブランチ、A7なし)に戻し、ループ版で従来間隔と倍速間隔の失敗率を測る。
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
  echo "$out" | grep -q "BUILD_FAILED" && { echo "awaseのビルド失敗"; exit 2; }
  sleep 15
done
echo "$out" | grep -q "BUILD_OK_STARTED_TESTINJ" || { echo "ビルド待ちtimeout"; exit 2; }
sleep 5
for cfg in "e2e-args-default 40 base-same" "e2e-args-loop-fast2x 22 base-fast2x"; do
  set -- $cfg
  s=$(date +%s)
  bash "$HERE/run_loop.sh" "$1" "$HERE/results/loop-$3" "$2" > "$HERE/results/loop-$3.out" 2>&1
  e=$(date +%s)
  echo "$3: 所要 $((e-s))秒 / $(tail -1 "$HERE/results/loop-$3.out")" >> "$HERE/results/BENCH.md"
done
echo done > "$HERE/results/baseline-loop.done"
