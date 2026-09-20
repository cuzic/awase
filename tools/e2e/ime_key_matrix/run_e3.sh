#!/usr/bin/env bash
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
# E3: Shift長押し(700ms)で効くか、通常(40ms)の8ケースで壊れていないか
ARGS_TARGET=chrome-probe-args-shifttail bash "$HERE/ablate_chrome.sh" E3-300ms-shifttail700 "$HERE/ablations/a10-e3-retry-while-shift-guard.sh" 300
# 同じビルドのまま通常のShift(40ms)でも(デプロイ済みなのでプローブだけ)
export CLIPD_HOST=dragonflyg4; CW="$HOME/powershell-clipd/target/release/clipwire"
OUT="$HERE/results/chrome-abl-E3-300ms"; mkdir -p "$OUT"
"$CW" exec chrome-probe-args-awase >/dev/null 2>&1; "$CW" exec chrome-probe-start >/dev/null 2>&1
sleep 250
for _ in $(seq 1 12); do "$CW" exec chrome-probe-fetch >"$OUT/probe.log" 2>/dev/null; grep -q "全ケース完了" "$OUT/probe.log" && break; sleep 15; done
"$CW" exec e2e-fetch-awase-full >"$OUT/awase-full.log" 2>/dev/null
python3 - "$OUT/probe.log" E3-300ms >> "$HERE/results/CHROME_ABL.md" <<'PY'
import re,sys,collections
L=open(sys.argv[1],encoding='utf8').read().splitlines()
cur=None; res=collections.defaultdict(list)
for l in L:
    m=re.search(r'\[CASE (\d+)/8 run (\d+)/\d+\] (.*)',l)
    if m: cur=int(m.group(1)); continue
    if cur and 'RESULT' in l: res[cur].append(l.split('RESULT ')[1].split(':')[0].split(' ')[0][:7])
sm=next((l.split('SUMMARY ')[1] for l in L if 'SUMMARY' in l),'(未完了)')
print(f'| {sys.argv[2]} | {sm} | 3={res[3]} | 7={res[7]} |')
PY
echo done > "$HERE/results/run-e3.done"
