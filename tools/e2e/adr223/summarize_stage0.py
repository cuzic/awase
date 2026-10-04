"""ADR-223 段階 0 の測定結果を集計する。

lang_switch_probe.log の `LS0 {json}`(プローブ側の真値: 切替の時刻と、各打鍵の時刻)と、awase.log の
`[lang-check:key] vk=0x41 read=Some(..) belief=..`(打鍵ごとの記録)を、時刻で突き合わせる。

- 対照の打鍵(ja のまま): read が Some(false) なら**偽陽性**(日本語なのに非日本語と読んだ)。
- 切替後の 1 打鍵目・2 打鍵目: read が Some(false) なら**陽性**(正しく検知)。None は「不明」、Some(true) は検知漏れ。
合格条件(ADR-223 D2): 偽陽性 0(打鍵 0 件は不合格)、陽性は各方法で N/N。
使い方: python summarize_stage0.py <lang_switch_probe.log> <awase.log>
"""
import datetime as dt
import json
import os
import re
import sys
from collections import defaultdict

TS = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d+)Z\s")
# Engine の文字出力: TSF/VK 経路は `[key-output]`、Unicode 経路は `awase_windows::output: send_keys:`。
OUT = re.compile(r"\[key-output\]|awase_windows::output: send_keys:")
KEY = re.compile(r"\[lang-check:key\] vk=0x41 read=(Some\((?:true|false)\)|None) belief=(true|false)")


def awase_outputs(path):
    out = []
    for line in open(path, encoding="utf-8", errors="replace"):
        m = TS.match(line)
        if m and OUT.search(line):
            out.append(dt.datetime.fromisoformat(m.group(1)[:26]).replace(tzinfo=dt.timezone.utc).timestamp() * 1000)
    return out


def has_output(outs, t, window=450):
    return any(t <= o <= t + window for o in outs)


def awase_keys(path):
    out = []
    for line in open(path, encoding="utf-8", errors="replace"):
        m = TS.match(line)
        k = KEY.search(line)
        if not (m and k):
            continue
        t = dt.datetime.fromisoformat(m.group(1)[:26]).replace(tzinfo=dt.timezone.utc).timestamp() * 1000
        out.append((t, k.group(1), k.group(2)))
    return out


def nearest(keys, t, window=400):
    """時刻 t(打鍵直後の epoch ms)に最も近い、直前 window ms 以内の `[lang-check:key]` を返す。"""
    cand = [k for k in keys if t - window <= k[0] <= t + 50]
    return min(cand, key=lambda k: abs(k[0] - t)) if cand else None


def main():
    probe, awase = sys.argv[1], sys.argv[2]
    keys = awase_keys(awase)
    outs = awase_outputs(awase)
    trials = []
    for line in open(probe, encoding="utf-8", errors="replace"):
        m = re.search(r"LS0 (\{.*\})", line)
        if m:
            trials.append(json.loads(m.group(1)))
    stats = defaultdict(lambda: dict(n=0, ctrl_total=0, ctrl_fp=0, ctrl_unknown=0, ctrl_missing=0, ctrl_out=0, k1_out=0, k2_out=0,
                                     k1_tp=0, k1_unknown=0, k1_miss=0, k1_missing=0,
                                     k2_tp=0, k2_unknown=0, k2_miss=0, k2_missing=0))
    rows = []
    for r in trials:
        s = stats[r["method"]]
        s["n"] += 1
        c = nearest(keys, r["t_ctrl_epoch_ms"])
        k1 = nearest(keys, r["t_key1_epoch_ms"])
        k2 = nearest(keys, r["t_key2_epoch_ms"])
        s["ctrl_total"] += 1
        s["ctrl_out"] += int(has_output(outs, r["t_ctrl_epoch_ms"]))
        s["k1_out"] += int(has_output(outs, r["t_key1_epoch_ms"]))
        s["k2_out"] += int(has_output(outs, r["t_key2_epoch_ms"]))
        if c is None:
            s["ctrl_missing"] += 1
        elif c[1] == "Some(false)":
            s["ctrl_fp"] += 1
        elif c[1] == "None":
            s["ctrl_unknown"] += 1
        for tag, k in (("k1", k1), ("k2", k2)):
            if k is None:
                s[tag + "_missing"] += 1
            elif k[1] == "Some(false)":
                s[tag + "_tp"] += 1
            elif k[1] == "None":
                s[tag + "_unknown"] += 1
            else:
                s[tag + "_miss"] += 1
        rows.append((r["method"], r["n"], c and c[1], k1 and k1[1], k2 and k2[1], r.get("t_switch_epoch_ms")))
    out = ["## ADR-223 段階 0: 打鍵時の入力言語の記録 × 真値", "",
           "| 方法 | 試行 | 対照(ja のまま)の記録 | 切替後 1 打鍵目 | 2 打鍵目(マーカーなし) |", "|---|---|---|---|---|"]
    for m, n, c, a, b, sw in rows:
        out.append(f"| {m} | {n} | {c} | {a} | {b} |")
    out += ["", "### 集計", "", "| 方法 | 試行 | 偽陽性(対照で false) | 対照 不明/欠落 | 陽性 1 打鍵目 | 陽性 2 打鍵目 | 1 打鍵目 不明/漏れ/欠落 | ローマ字出力あり: 対照 / 切替後1打鍵目 / 2打鍵目 |", "|---|---|---|---|---|---|---|---|"]
    ok = bool(trials)
    for m, s in stats.items():
        out.append(f"| {m} | {s['n']} | {s['ctrl_fp']}/{s['ctrl_total']} | {s['ctrl_unknown']}/{s['ctrl_missing']} | {s['k1_tp']}/{s['n']} | {s['k2_tp']}/{s['n']} | {s['k1_unknown']}/{s['k1_miss']}/{s['k1_missing']} | {s['ctrl_out']}/{s['k1_out']}/{s['k2_out']} |")
        if s["ctrl_fp"] != 0 or s["k1_tp"] != s["n"] or s["k1_out"] != 0 or s["k2_out"] != 0:
            ok = False
    out += ["", f"**判定(ADR-223 段階 1: 偽陽性 0、陽性 N/N、切替後の 1・2 打鍵目でローマ字出力 0): {'合格' if ok else '不合格または測定不足'}**", "",
            f"(awase.log の `[lang-check:key] vk=0x41` は {len(keys)} 件、プローブの試行は {len(trials)} 件)"]
    text = "\n".join(out)
    print(text.encode("utf-8", "replace").decode("utf-8"))
    p = os.environ.get("GITHUB_STEP_SUMMARY")
    if p:
        open(p, "a", encoding="utf-8").write(text + "\n")


main()
