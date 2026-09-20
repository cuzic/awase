#!/usr/bin/env bash
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
export CLIPD_HOST=dragonflyg4 E4_DELAY="${E4_DELAY:-150}"
CW="$HOME/powershell-clipd/target/release/clipwire"
L="E4-${E4_DELAY}ms"
# 1) デプロイ+通常8ケース×3周
bash "$HERE/ablate_chrome.sh" "$L" "$HERE/ablations/a11-e4-generalize-refresh.sh" 0
# 2) Shift 700ms 長押し(2周)
OUT="$HERE/results/chrome-abl-$L-shifttail700"; mkdir -p "$OUT"
"$CW" exec chrome-probe-args-shifttail >/dev/null 2>&1; "$CW" exec chrome-probe-start >/dev/null 2>&1
sleep 190
for _ in $(seq 1 12); do "$CW" exec chrome-probe-fetch >"$OUT/probe.log" 2>/dev/null; grep -q "全ケース完了" "$OUT/probe.log" && break; sleep 15; done
"$CW" exec e2e-fetch-awase-full >"$OUT/awase-full.log" 2>/dev/null
# 3) 親指キーの通常タイピング(storm)
OUT2="$HERE/results/chrome-storm-$L"; mkdir -p "$OUT2"
"$CW" exec chrome-probe-args-storm >/dev/null 2>&1; "$CW" exec chrome-probe-start >/dev/null 2>&1
sleep 45
for _ in $(seq 1 8); do "$CW" exec chrome-probe-fetch >"$OUT2/probe.log" 2>/dev/null; grep -q "全ケース完了" "$OUT2/probe.log" && break; sleep 10; done
"$CW" exec e2e-fetch-awase-full >"$OUT2/awase-full.log" 2>/dev/null
python3 - "$OUT" "$OUT2" "$L" >> "$HERE/results/CHROME_ABL.md" <<'PY'
import re,sys,collections
def tab(d):
    L=open(d+'/probe.log',encoding='utf8').read().splitlines(); cur=None; res=collections.defaultdict(list)
    for l in L:
        m=re.search(r'\[CASE (\d+)/8 run',l)
        if m: cur=int(m.group(1)); continue
        if cur and 'RESULT' in l: res[cur].append(l.split('RESULT ')[1].split(':')[0].split(' ')[0][:7])
    return next((l.split('SUMMARY ')[1] for l in L if 'SUMMARY' in l),'(未完了)'),res
sm,res=tab(sys.argv[1])
def sec(t): h,m,s=t.split(':'); return int(h)*3600+int(m)*60+float(s)
pl=open(sys.argv[2]+'/probe.log',encoding='utf8').read()
st=re.search(r'\[(\d\d:\d\d:\d\d\.\d+)Z\] STORM start',pl); en=re.search(r'\[(\d\d:\d\d:\d\d\.\d+)Z\] STORM end',pl)
a=b=0; fired=sched=0
if st and en:
    a,b=sec(st.group(1)),sec(en.group(1))
    for l in open(sys.argv[2]+'/awase-full.log',encoding='utf8',errors='replace'):
        m=re.match(r'\S+T(\d\d:\d\d:\d\d\.\d+)Z',l)
        if m and a<=sec(m.group(1))<=b+1:
            fired+='強制conv読み取りを実行' in l and '[E4]' in l
            sched+='follow refresh scheduled' in l
print(f'| {sys.argv[3]}(Shift700ms) | {sm} | 3={res[3]} | 7={res[7]} | storm(親指+J×15): 強制読み取り={fired}回 follow予約={sched}回 |')
PY
echo done > "$HERE/results/run-e4.done"
