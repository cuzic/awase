#!/usr/bin/env bash
# 高速版E2E(毎回新しいプロセス版): Windows側のループが「スパイク起動→完了待ち→ログ追記」を12回行い、Linux側は最後に1回だけ取得して実行ごとに判定する。
# 1プロセス連続版(run_multi.sh)はBUG-147を再現しなかったため、起動し直す条件は保つ。
# 使い方: run_loop.sh <argsターゲット> [出力ディレクトリ] [1回あたりの目安秒]
#   argsターゲット: e2e-args-default(従来と同じ間隔) | e2e-args-loop-fast2x(--fast --speed=2)
# 環境変数: REAL_ONLY=1 で実IMEだけを判定。 実行中はWindows機に触らない・ロックさせない。
set -u
: "${CLIPD_HOST:=dragonflyg4}"; export CLIPD_HOST
CW="${CLIPWIRE:-$HOME/powershell-clipd/target/release/clipwire}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ARGS="${1:?argsターゲット}"; OUT="${2:-$HERE/results/loop-$(date +%Y%m%d-%H%M%S)}"; PER="${3:-40}"; N="${N:-12}"   # 回数。Windows側の target `e2e-loop-n<N>`(clipwire-targets.example.toml)と一致させる
mkdir -p "$OUT"
"$CW" exec e2e-diag-desktop >"$OUT/desktop.txt" 2>&1
if grep -qE "LockApp|foreground: hwnd=0 " "$OUT/desktop.txt"; then echo "Windows機がロック画面です"; exit 2; fi
"$CW" exec "$ARGS" >"$OUT/args.txt" 2>&1
"$CW" exec "e2e-loop-n$N" 2>&1 | grep -q "n=$N" || { echo "clipwire target e2e-loop-n$N が無い/未承認です(clipwire-targets.example.toml を参照)"; exit 2; }
"$CW" exec e2e-loop-start >"$OUT/start.txt" 2>&1
sleep $((N * PER))
for _ in $(seq 1 $((N * 2 + 8))); do
  "$CW" exec e2e-fetch-multi >"$OUT/spike.log" 2>/dev/null
  grep -q "LOOP DONE" "$OUT/spike.log" && break
  sleep 15
done
grep -q "LOOP DONE" "$OUT/spike.log" || { echo "完了しませんでした(timeout)"; exit 2; }
"$CW" exec e2e-fetch-awase-full >"$OUT/awase-full.log" 2>/dev/null
python3 "$HERE/check_multi.py" ${REAL_ONLY:+--real-only} "$OUT/spike.log" "$OUT/awase-full.log" | tee "$OUT/result.txt"
exit "${PIPESTATUS[0]}"
