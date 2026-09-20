#!/usr/bin/env bash
# ROMAN書き込み(w/n) × ひらがなキーのscan(70/0) の実験を、条件ごとに別ランで実行する。
# 使い方: run_exp.sh <70w|70n|0w|0n> <出力ディレクトリ>   (要 elw-bypass-on 済み: awase完全バイパス)
set -u
COND="$1"; OUT="$2"; AW="${3:-A}"   # AW: A(=素通し/バイパス時のA')|B(AWASE_TEST_INJECTION=1)
: "${CLIPD_HOST:=dragonflyg4}"; export CLIPD_HOST
CW="${CLIPWIRE:-$HOME/powershell-clipd/target/release/clipwire}"
mkdir -p "$OUT"
"$CW" exec e2e-diag-desktop >"$OUT/desktop.txt" 2>&1
grep -qE "LockApp|foreground: hwnd=0 " "$OUT/desktop.txt" && { echo LOCKED; exit 2; }
if [ "$AW" = "B" ]; then "$CW" exec e2e-awase-start >"$OUT/awase-start.txt" 2>&1; else "$CW" exec elw-awase-start-a >"$OUT/awase-start.txt" 2>&1; fi
"$CW" exec "elw-exp-$COND" >"$OUT/args.txt" 2>&1
"$CW" exec e2e-run >"$OUT/run.txt" 2>&1
grep -q "spike procs: 1" "$OUT/run.txt" || { echo "スパイク起動失敗"; exit 2; }
sleep 100   # 12手 x 約5秒 + 起動。この間 Windows 側で何も起動しない
for _ in $(seq 1 24); do
  "$CW" exec e2e-fetch-spike >"$OUT/spike.log" 2>/dev/null
  grep -q "手完了" "$OUT/spike.log" && break
  sleep 15
done
grep -q "手完了" "$OUT/spike.log" || { echo "完了せず"; exit 2; }
"$CW" exec e2e-fetch-awase-full >"$OUT/awase-full.log" 2>/dev/null
echo "OK $COND"
