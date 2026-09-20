#!/usr/bin/env bash
# 高速版E2E: スパイクを1回だけ起動し、全手順を N 回連続実行 → ログを1回だけ取得 → 実行ごとに判定する。
# 従来の run.sh(1回≒59秒、うち約30秒は起動・終了・ログ取得の往復)に対し、起動/取得を1回にまとめる。
#
# 使い方: run_multi.sh <argsターゲット> [出力ディレクトリ] [1回あたりの目安秒]
#   argsターゲット: e2e-args-multi12(従来と同じ間隔) | e2e-args-multi12-fast(+1500ms観測なし) | e2e-args-multi12-fast2x(さらに2倍速)
# 環境変数: REAL_ONLY=1 で実IMEだけを判定(GJI単体・awase起動の比較用)
# 実行中は Windows 機のキーボード・マウスに触らず、ロックさせない(取得の PowerShell 起動がフォーカスを奪うため、固定時間待ってから1回だけ取得する)。
set -u
: "${CLIPD_HOST:=dragonflyg4}"; export CLIPD_HOST
CW="${CLIPWIRE:-$HOME/powershell-clipd/target/release/clipwire}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ARGS="${1:?argsターゲット}"; OUT="${2:-$HERE/results/multi-$(date +%Y%m%d-%H%M%S)}"; PER="${3:-32}"
N=12
mkdir -p "$OUT"
"$CW" exec e2e-diag-desktop >"$OUT/desktop.txt" 2>&1
if grep -qE "LockApp|foreground: hwnd=0 " "$OUT/desktop.txt"; then echo "Windows機がロック画面です"; exit 2; fi
"$CW" exec "$ARGS" >"$OUT/args.txt" 2>&1
"$CW" exec e2e-run >"$OUT/run.txt" 2>&1
grep -q "spike procs: 1" "$OUT/run.txt" || { echo "スパイクを起動できません"; cat "$OUT/run.txt"; exit 2; }
sleep $((N * PER + 10))
for _ in $(seq 1 20); do
  "$CW" exec e2e-fetch-spike >"$OUT/spike.log" 2>/dev/null
  grep -q "全手順完了" "$OUT/spike.log" && break
  sleep 15
done
grep -q "全手順完了" "$OUT/spike.log" || { echo "完了しませんでした(timeout)"; exit 2; }
"$CW" exec e2e-fetch-awase-full >"$OUT/awase-full.log" 2>/dev/null
python3 "$HERE/check_multi.py" ${REAL_ONLY:+--real-only} "$OUT/spike.log" "$OUT/awase-full.log" | tee "$OUT/result.txt"
exit "${PIPESTATUS[0]}"
