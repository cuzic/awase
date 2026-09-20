#!/usr/bin/env bash
# 同じ構成で、有効な回(フォーカス移動・物理入力の混入なし)がN回たまるまで run.sh を繰り返し、PASS/FAIL/INVALID を集計する。
# 使い方: stat_runs.sh <label> <N>   (最大 2N 回まで試行)
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
LABEL=$1; N=$2
pass=0; fail=0; inv=0; att=0; sigs=""
while [ $((pass+fail)) -lt "$N" ] && [ "$att" -lt $((N*2)) ]; do
  att=$((att+1)); d="$HERE/results/$LABEL-s$att"
  bash "$HERE/run.sh" "$d" >/dev/null 2>&1; rc=$?
  phys=$(grep -E "^\[.*\] KEY" "$d/spike.log" 2>/dev/null | grep -vE "\((auto|injected)\)" | wc -l)
  if [ "$phys" != "0" ] || [ "$rc" -ge 2 ]; then inv=$((inv+1)); continue; fi
  if [ "$rc" = "0" ]; then pass=$((pass+1)); else fail=$((fail+1)); sigs="$sigs $(grep FAIL "$d/result.txt" | grep -v '^結果' | sed -E 's/^ *([0-9]+) .*/\1/' | tr '\n' ',')|"; fi
done
echo "$LABEL: 有効=$((pass+fail)) PASS=$pass FAIL=$fail INVALID=$inv (失敗した手順:$sigs)" | tee -a "$HERE/results/STATS.md"
