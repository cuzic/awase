#!/usr/bin/env bash
# ADR-186 撤去・統合の実験を一括で回す(各構成: デプロイ+3回実行)。結果は results/SUMMARY.md に集約する。
# 実験中は Windows 機のキーボード・マウスに触らない/ロックさせないこと。
# E1(KeyUp解決)・E2(ATOK custom表)・E7(gji_thumb_key_ime_toggle)は ADR-191 で対象の機構・設定ごと撤去したため、このスクリプトからも外した(履歴は ADR-186)。
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
A="$HERE/ablations"
SUM="$HERE/results/SUMMARY.md"
mkdir -p "$HERE/results"
echo "# ADR-186 撤去・統合の実験結果 ($(date '+%F %T'))" > "$SUM"
echo >> "$SUM"
echo "| 実験 | 内容 | 構成 | 3回の結果(PASS/FAIL件数) | 失敗の型(例) |" >> "$SUM"
echo "|---|---|---|---|---|" >> "$SUM"

run_exp() {  # label desc mutator args cfg
  local label=$1 desc=$2 mut=$3 args=$4 cfg=$5
  echo "=== $label ($desc)"
  bash "$HERE/ablate.sh" "$label" "$mut" "$args" "$cfg" >/dev/null 2>&1
  for i in 2 3; do bash "$HERE/run.sh" "$HERE/results/$label-$i" >/dev/null 2>&1; done
  local cells="" sig=""
  for d in "$HERE/results/$label" "$HERE/results/$label-2" "$HERE/results/$label-3"; do
    local phys r
    phys=$(grep -E "^\[.*\] KEY" "$d/spike.log" 2>/dev/null | grep -vE "\((auto|injected)\)" | wc -l)
    r=$(grep -E "^結果" "$d/result.txt" 2>/dev/null | tail -1)
    [ -z "$r" ] && r="実行不能"
    [ "$phys" != "0" ] && r="$r(物理混入)"
    cells="$cells ${r#結果: } /"
    sig="$sig $(grep -E 'FAIL' "$d/result.txt" 2>/dev/null | grep -v '^結果' | sed -E 's/^ *([0-9]+) .*FAIL: (.*)/\1:\2/' | head -2 | tr '\n' ';')"
  done
  echo "| $label | $desc | mut=$(basename "$mut") args=${args#e2e-args-} cfg=${cfg#e2e-config-toggle-} | ${cells% /} | $(echo "$sig" | cut -c1-140) |" >> "$SUM"
}

run_exp E0h "基準(変換キーで検証)" none e2e-args-henkan-hold180 e2e-config-toggle-true
run_exp E3 "eisu reset抑止(ADR-186決定2)を撤去" "$A/a3-no-eisu-suppress.sh" e2e-args-hold180 e2e-config-toggle-true
run_exp E4 "eisu resetの全経路を撤去" "$A/a4-no-eisu-reset.sh" e2e-args-hold180 e2e-config-toggle-true
run_exp E5 "物理キー後の20ms再読み取りを撤去" "$A/a5-no-refresh20.sh" e2e-args-hold180 e2e-config-toggle-true
run_exp E6 "idle-conv-checkを無効化" "$A/a6-no-idle-check.sh" e2e-args-hold180 e2e-config-toggle-true
# 後片付け: 設定を true に戻し、基準ビルドで awase を再起動する。
bash "$HERE/ablate.sh" restore none e2e-args-default e2e-config-toggle-true >/dev/null 2>&1
echo "完了: $SUM"
