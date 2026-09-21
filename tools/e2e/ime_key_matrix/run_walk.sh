#!/usr/bin/env bash
# IMEキー効果学習スパイク: awase を A/B で起動して --walk を1回流し、ログを回収する。
#   A = awase起動・AWASE_TEST_INJECTIONなし(注入は外部注入として素通し)
#   B = awase起動・AWASE_TEST_INJECTION=1(注入を物理キー扱い)
#   Ap = A' = A と同じ起動だが、config.toml の disable_apps にスパイクを入れて awase を完全バイパス(要 elw-bypass-on)
#   Apr = Ap + 開始前にIMEをONにして変換モードを0x19(ローマ字)へ揃える(--roman)
# 使い方: run_walk.sh <A|B|Ap|Apr> <seed(1|2)> <出力ディレクトリ>
# 前提: clipwire ターゲット elw-* / e2e-run / e2e-fetch-spike / e2e-fetch-awase / e2e-awase-start が登録・承認済み。
# 実行中(約4分)は Windows 機のキーボード・マウスに触らない/ロックしない。
set -u
MODE="$1"; SEED="$2"; OUT="$3"
: "${CLIPD_HOST:=dragonflyg4}"; export CLIPD_HOST
CW="${CLIPWIRE:-$HOME/powershell-clipd/target/release/clipwire}"
mkdir -p "$OUT"
"$CW" exec e2e-diag-desktop >"$OUT/desktop.txt" 2>&1
if grep -qE "LockApp|foreground: hwnd=0 " "$OUT/desktop.txt"; then echo "LOCKED"; exit 2; fi
LATE=""   # Ah/Bh: awase をスパイクの初期化の後に起動する(初期化中のIMMプローブ失敗で「IMM不可」と誤学習するのを避ける)
case "$MODE" in Ah|Bh) LATE=1;; esac
case "$MODE" in
  A|Ap|Apr|Apg|Ag) "$CW" exec elw-awase-start-a >"$OUT/awase-start.txt" 2>&1 ;;
  B|Bg) "$CW" exec e2e-awase-start   >"$OUT/awase-start.txt" 2>&1 ;;
  Ah|Bh) echo "late start" >"$OUT/awase-start.txt" ;;
  *) echo "mode must be A|B|Ap|Apr|Apg|Ag|Bg"; exit 2 ;;
esac
[ -n "$LATE" ] || grep -q "procs: 1" "$OUT/awase-start.txt" || { echo "awase起動失敗"; cat "$OUT/awase-start.txt"; exit 2; }
SUF=""; [ "$MODE" = "Apr" ] && SUF="r"; case "$MODE" in Apg|Ag|Bg) SUF="g";; Ah|Bh) SUF="h";; esac
"$CW" exec "elw-args-s${SEED}${SUF}" >"$OUT/args.txt" 2>&1
"$CW" exec e2e-run >"$OUT/run.txt" 2>&1
grep -q "spike procs: 1" "$OUT/run.txt" || { echo "スパイク起動失敗"; cat "$OUT/run.txt"; exit 2; }
if [ -n "$LATE" ]; then
  sleep 8   # スパイクがGJIを有効化し終えた後に awase を起動する
  if [ "$MODE" = "Bh" ]; then "$CW" exec e2e-awase-start >"$OUT/awase-start.txt" 2>&1; else "$CW" exec elw-awase-start-a >"$OUT/awase-start.txt" 2>&1; fi
  grep -q "procs: 1" "$OUT/awase-start.txt" || { echo "awase起動失敗"; exit 2; }
fi
sleep 215   # 100手 x 1.9s + 起動/フォーカス。この間 Windows 側で何も起動しない
for _ in $(seq 1 24); do
  "$CW" exec e2e-fetch-spike >"$OUT/spike.log" 2>/dev/null
  grep -q "手完了" "$OUT/spike.log" && break
  sleep 15
done
grep -q "手完了" "$OUT/spike.log" || { echo "完了せず(timeout)"; exit 2; }
"$CW" exec e2e-fetch-awase >"$OUT/awase.log" 2>/dev/null
"$CW" exec e2e-fetch-awase-full >"$OUT/awase-full.log" 2>/dev/null
echo "OK $MODE seed=$SEED"
