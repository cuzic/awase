#!/usr/bin/env bash
# BUG-149 のアブレーション実験: 実装ブランチにmutatorを当てて実機へデプロイし、Chromeプローブ(3周)で判定する。
# 使い方: ablate_chrome.sh <label> <mutator.sh|none> [E2_DELAY]
set -u
LABEL=$1; MUT=$2; export E2_DELAY="${3:-250}"
HERE="$(cd "$(dirname "$0")" && pwd)"
WT="$HOME/rust-nicola-worktrees/adr186-ablation"
BASE="${ABL_BASE:-feat/adr186-nonconvert-toggle-belief-follow}"
export CLIPD_HOST=dragonflyg4
CW="$HOME/powershell-clipd/target/release/clipwire"
OUT="$HERE/results/chrome-abl-$LABEL"; mkdir -p "$OUT"
git -C "$WT" checkout -q -f -B e2e/ablation "$BASE" || exit 2
if [ "$MUT" != none ]; then
  ( cd "$WT" && bash "$MUT" ) || { echo "$LABEL: mutator失敗"; exit 2; }
  git -C "$WT" commit -q -am "ablation: $LABEL" || { echo "$LABEL: 差分なし"; exit 2; }
fi
( cd "$WT" && cargo check -q --target x86_64-pc-windows-msvc -p awase-windows 2>&1 | grep -E "^error" -A6 | head -10 )
git -C "$WT" push -q -f origin e2e/ablation 2>&1 | tail -1
"$CW" exec e2e-config-toggle-true >/dev/null 2>&1
"$CW" exec e2e-deploy >/dev/null 2>&1
for _ in $(seq 1 40); do
  out=$("$CW" exec adr186-deploy-check 2>&1)
  echo "$out" | grep -q "BUILD_OK_STARTED_TESTINJ" && break
  echo "$out" | grep -q "BUILD_FAILED" && { echo "$LABEL: awaseのビルド失敗"; echo "$out" | tail -5; exit 2; }
  sleep 15
done
echo "$out" | grep -q "BUILD_OK_STARTED_TESTINJ" || { echo "$LABEL: ビルド待ちtimeout"; exit 2; }
sleep 6
"$CW" exec e2e-diag-desktop >"$OUT/desktop.txt" 2>&1
"$CW" exec "${ARGS_TARGET:-chrome-probe-args-awase}" >/dev/null 2>&1
"$CW" exec chrome-probe-start >"$OUT/start.txt" 2>&1
sleep 250
for _ in $(seq 1 12); do
  "$CW" exec chrome-probe-fetch >"$OUT/probe.log" 2>/dev/null
  grep -q "全ケース完了" "$OUT/probe.log" && break
  sleep 15
done
"$CW" exec e2e-fetch-awase-full >"$OUT/awase-full.log" 2>/dev/null
python3 - "$OUT/probe.log" "$LABEL" >> "$HERE/results/CHROME_ABL.md" <<'PY'
import re,sys,collections
L=open(sys.argv[1],encoding='utf8').read().splitlines(); label=sys.argv[2]
cur=None; res=collections.defaultdict(list)
for l in L:
    m=re.search(r'\[CASE (\d+)/8 run (\d+)/\d+\] (.*)',l)
    if m: cur=int(m.group(1)); continue
    if cur and 'RESULT' in l: res[cur].append(l.split('RESULT ')[1].split(':')[0].split(' ')[0][:7])
sm=next((l.split('SUMMARY ')[1] for l in L if 'SUMMARY' in l),'(未完了)')
print(f'| {label} | {sm} | 3(かな→ひらがな)={res[3]} | 7(かな→Shift+無変換)={res[7]} |')
PY
echo "$LABEL done" >> "$HERE/results/chrome-abl.progress"
