#!/usr/bin/env bash
# ADR-186 実機E2E(ime_key_matrix): スパイクの --auto が手順のキーを SendInput で自動注入し、
# 実IMEの状態と awase の Engine 切り替えを記録 → check.py で PASS/FAIL を判定する。
#
# 前提(Windows側): awase が AWASE_TEST_INJECTION=1 かつ RUST_LOG=debug で起動していること
#   (awase は ADR-186 の実装ブランチのビルド、gji_thumb_key_ime_toggle=true)。
#   clipwire ターゲット e2e-run / e2e-fetch-awase / e2e-fetch-spike / e2e-diag-desktop が登録・承認済みであること
#   (clipwire-targets.example.toml 参照)。実行中(約40秒)は、Windows機のキーボード・マウスに触らない。
#
# 使い方: run.sh [出力ディレクトリ]   終了コード: 0=ALL PASS / 1=FAIL / 2=実行できず
set -u
: "${CLIPD_HOST:=dragonflyg4}"
export CLIPD_HOST
CW="${CLIPWIRE:-$HOME/powershell-clipd/target/release/clipwire}"
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="${1:-$HERE/out/$(date +%Y%m%d-%H%M%S)}"
mkdir -p "$OUT"

# 実行前にデスクトップがロックされていないか確認する(ロック中は SendInput も前面化も効かない)。
"$CW" exec e2e-diag-desktop >"$OUT/desktop.txt" 2>&1
if grep -qE "LockApp|foreground: hwnd=0 " "$OUT/desktop.txt"; then
  echo "Windows機がロック画面です。ロックを解除してから、もう一度実行してください。"
  grep -E "foreground:" "$OUT/desktop.txt"
  exit 2
fi

"$CW" exec e2e-run >"$OUT/run.txt" 2>&1
grep -q "spike procs: 1" "$OUT/run.txt" || { echo "スパイクを起動できませんでした:"; cat "$OUT/run.txt"; exit 2; }

# 実行中(約45秒)は Windows 側で何も起動しない: ログ取得の PowerShell 起動がフォーカスを奪い、
# 注入が入力欄に届かなくなる(実機で発生)。固定時間だけ待ってから取得し、未完了なら少し待って再取得する。
sleep 50
for _ in 1 2 3 4; do
  "$CW" exec e2e-fetch-spike >"$OUT/spike.log" 2>/dev/null
  grep -q "全手順完了" "$OUT/spike.log" && break
  sleep 15
done
grep -q "全手順完了" "$OUT/spike.log" || { echo "自動実行が完了しませんでした(timeout)。$OUT/spike.log を確認してください"; exit 2; }

"$CW" exec e2e-fetch-awase >"$OUT/awase.log" 2>/dev/null
python3 "$HERE/check.py" "$OUT/spike.log" "$OUT/awase.log" | tee "$OUT/result.txt"
exit "${PIPESTATUS[0]}"
