"""BUG-183 の再現結果を集計する。lang_switch_probe.log の `LS {json}` と awase.log の Engine 活性/非活性を時刻で突き合わせる。

使い方: python summarize.py <lang_switch_probe.log> <awase.log>
"""
import collections
import datetime as dt
import json
import re
import sys

TS = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d+)Z\s")


def awase_events(path):
    ev = []
    for line in open(path, encoding="utf-8", errors="replace"):
        m = TS.match(line)
        if not m:
            continue
        if "Engine deactivated" not in line and "Engine activated" not in line:
            continue
        t = dt.datetime.fromisoformat(m.group(1)[:26]).replace(tzinfo=dt.timezone.utc).timestamp() * 1000
        kind = "deact" if "Engine deactivated" in line else "act"
        r = re.search(r"reason=([^\s)]+(?:\([^)]*\))?)", line)
        ev.append((t, kind, r.group(1) if r else "", line.strip()[:160]))
    return ev


def main():
    probe_log, awase_log = sys.argv[1], sys.argv[2]
    listener = []
    if len(sys.argv) > 3:
        for line in open(sys.argv[3], encoding="utf-8", errors="replace"):
            m = re.match(r"EV (\d+) (.*)", line.strip())
            if m:
                listener.append((int(m.group(1)), m.group(2)))
    ev = awase_events(awase_log)
    rows = collections.defaultdict(list)
    t0_of = {}
    for line in open(probe_log, encoding="utf-8", errors="replace"):
        m = re.search(r"LS (\{.*\})", line)
        if not m:
            continue
        r = json.loads(m.group(1))
        t0 = r["t_inject_epoch_ms"]
        t0_of[(r["method"], r["n"])] = t0
        lay = r["layout_ms_after_inject"]
        deact = [e for e in ev if t0 <= e[0] <= t0 + 14000 and e[1] == "deact"]
        first = deact[0] if deact else None
        rows[r["method"]].append(
            (r["n"], lay, None if first is None else round(first[0] - t0), None if first is None else first[2], r["lang_end"],
             None if not r.get("t_focus_epoch_ms") else round(r["t_focus_epoch_ms"] - t0), r.get("events", []))
        )
    out = ["## BUG-183: 切替方法ごとの「実際の切替」と awase の非活性化の遅れ", "",
           "| 方法 | 試行 | 切替までの ms | awase 非活性までの ms(注入から) | 理由 | 最終言語 | フォーカス移動の ms |", "|---|---|---|---|---|---|---|"]
    ev_rows = []
    for method, rs in rows.items():
        for n, lay, d, why, end, fo, evs in rs:
            out.append(f"| {method} | {n} | {lay} | {d} | {why} | {end} | {fo} |")
            ev_rows.append((method, n, evs, [(e[0] - t0_of[(method, n)], e[1]) for e in listener if t0_of[(method, n)] - 100 <= e[0] <= t0_of[(method, n)] + 600]))
    out.append("")
    out.append("## 通知イベント(注入からの ms。HSHELL_LANGUAGE は `shell wparam=8`)")
    out.append("")
    out.append("| 方法 | 試行 | 前面プロセスの通知(ms: 種別) | 背景リスナー(別プロセス)の通知 |")
    out.append("|---|---|---|---|")
    for method, n, evs, lev in ev_rows:
        compact = " / ".join(f"{t}ms {e}" for t, e in evs if e.startswith("tsf") or "wparam=8 " in e)[:200]
        lcompact = " / ".join(f"{t}ms {e}" for t, e in lev if e.startswith("tsf") or "wparam=8 " in e)[:200]
        out.append(f"| {method} | {n} | {compact or '(なし)'} | {lcompact or '(なし)'} |")
    out.append("")
    out.append("(切替までが None = 3.5 秒以内に言語が切り替わらなかった。非活性が None = 14 秒以内に Engine deactivated が出なかった)")
    text = "\n".join(out)
    print(text)
    import os

    p = os.environ.get("GITHUB_STEP_SUMMARY")
    if p:
        open(p, "a", encoding="utf-8").write(text + "\n")


main()
