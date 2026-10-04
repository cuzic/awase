"""ADR-223 段階 0(窓の種類を増やした測定)の集計。

プローブ(`--stage0-ext=<console|uwp|chrome>`)が OS から読んだ真値(`GetGUIThreadInfo` のフォーカススレッドと、前面スレッドの言語)と、
awase.log の `[lang-check:key] vk=0x41 read=..`(awase が保存した hwnd から引いたスレッドの言語)を、時刻で突き合わせる。

- 真値 = フォーカススレッドの言語(OS が実際に入力を渡す先)。awase の read がこれと食い違えば誤り。
- 対照(切替前、真値は日本語): read が Some(false) なら偽陽性。
- 切替後(真値は ru): read が Some(false) なら陽性。None は不明、Some(true) は検知漏れ。
- 前面スレッドとフォーカススレッドの言語が食い違う件数も数える(前面スレッドを読む案を採らない根拠の確認)。
使い方: python summarize_ext.py <awase.log> <ext ログ...>
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
KEY = re.compile(r"\[lang-check:key\] vk=0x41 read=(Some\((?:true|false)\)|None) belief=(true|false) tid=(\d+)")


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
        m, k = TS.match(line), KEY.search(line)
        if m and k:
            t = dt.datetime.fromisoformat(m.group(1)[:26]).replace(tzinfo=dt.timezone.utc).timestamp() * 1000
            out.append((t, k.group(1), int(k.group(3))))
    return out


def nearest(keys, t, window=400):
    cand = [k for k in keys if t - window <= k[0] <= t + 50]
    return min(cand, key=lambda k: abs(k[0] - t)) if cand else None


def main():
    keys = awase_keys(sys.argv[1])
    outs = awase_outputs(sys.argv[1])
    recs = []
    for p in sys.argv[2:]:
        if not os.path.exists(p):
            continue
        for line in open(p, encoding="utf-8", errors="replace"):
            m = re.search(r"LS0X (\{.*\})", line)
            if m:
                recs.append(json.loads(m.group(1)))
    st = defaultdict(lambda: defaultdict(int))
    classes = {}
    for r in recs:
        key = (r["target"], r["method"])
        s = st[key]
        s["n"] += 1
        classes[r["target"]] = (r["after"]["fg_class"], r["after"]["focus_class"], r["after"]["fg_tid"], r["after"]["focus_tid"])
        s["ctrl_out"] += int(has_output(outs, r["t_ctrl_epoch_ms"]))
        if r["before"]["focus_lang"] != "0x0411":
            s["before_not_ja"] += 1
        else:
            c = nearest(keys, r["t_ctrl_epoch_ms"])
            s["ctrl_total"] += 1
            if c is None:
                s["ctrl_missing"] += 1
            elif c[1] == "Some(false)":
                s["ctrl_fp"] += 1
            elif c[1] == "None":
                s["ctrl_unknown"] += 1
        if r["after"]["focus_lang"] != "0x0419":
            s["not_switched"] += 1
            continue
        s["switched"] += 1
        s["k1_out"] += int(has_output(outs, r["t_key1_epoch_ms"]))
        s["k2_out"] += int(has_output(outs, r["t_key2_epoch_ms"]))
        if r["after"]["fg_lang"] != r["after"]["focus_lang"]:
            s["fg_focus_differ"] += 1
        for tag in ("key1", "key2"):
            k = nearest(keys, r["t_%s_epoch_ms" % tag])
            if k is None:
                s[tag + "_missing"] += 1
            elif k[1] == "Some(false)":
                s[tag + "_tp"] += 1
            elif k[1] == "None":
                s[tag + "_unknown"] += 1
            else:
                s[tag + "_miss"] += 1
    out = ["## ADR-223 段階 0: 窓の種類別(真値 = OS のフォーカススレッドの言語)", "",
           "| 対象 | 方法 | 試行 | 切替できた | 偽陽性(対照で false) | 対照 不明/欠落 | 陽性 1 打鍵目 | 陽性 2 打鍵目 | 1 打鍵目 不明/漏れ/欠落 | 前面≠フォーカスの言語 | ローマ字出力あり: 対照 / 切替後1打鍵目 / 2打鍵目 |",
           "|---|---|---|---|---|---|---|---|---|---|---|"]
    for (target, method), s in sorted(st.items()):
        out.append(f"| {target} | {method} | {s['n']} | {s['switched']} | {s['ctrl_fp']}/{s['ctrl_total']} | {s['ctrl_unknown']}/{s['ctrl_missing']} | "
                   f"{s['key1_tp']}/{s['switched']} | {s['key2_tp']}/{s['switched']} | {s['key1_unknown']}/{s['key1_miss']}/{s['key1_missing']} | {s['fg_focus_differ']}/{s['switched']} | {s['ctrl_out']}/{s['k1_out']}/{s['k2_out']} |")
    out += ["", "### 前面窓・フォーカス窓のクラスとスレッド(最後の試行)", "", "| 対象 | 前面のクラス | フォーカスのクラス | 前面 tid | フォーカス tid |", "|---|---|---|---|---|"]
    for t, (fc, oc, ft, ot) in classes.items():
        out.append(f"| {t} | {fc} | {oc} | {ft} | {ot} |")
    out += ["", f"(awase.log の `[lang-check:key] vk=0x41` は {len(keys)} 件、プローブの試行は {len(recs)} 件)"]
    text = "\n".join(out)
    print(text.encode("utf-8", "replace").decode("utf-8"))
    p = os.environ.get("GITHUB_STEP_SUMMARY")
    if p:
        open(p, "a", encoding="utf-8").write(text + "\n")


main()
