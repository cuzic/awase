#!/usr/bin/env python3
"""ADR-199 T17 Phase 4 の B4 計測(使い捨て、測定用ブランチ ci/b4-msime-toggle-probe 専用)。

msime_native_composing_probe --matrix の結果 JSON を表にし、awase.log があれば各シナリオの
キー押下窓 [t_start, t_end] 内の観測件数(observed=ImeModel への開閉観測、check_drift_recovery.py と同じ正規表現)を添える。

使い方: check_b4_toggle.py <label> <result.json> [awase.log]
"""
import json
import re
import sys

TIME = re.compile(r"T(\d\d:\d\d:\d\d\.\d{3})")
OBSERVED = re.compile(r"\[stage-observe\] observer_poll=Some|ObserverReported")
ENGINE = re.compile(r"Engine (de)?activated")
SEND = re.compile(r"apply_ime_open|send_ime_control|IME open axis delegated")


def load(path):
    out = []
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                m = TIME.search(line[:40])
                if m:
                    out.append((m.group(1), line))
    except OSError:
        pass
    return out


def count(lines, pat, lo, hi):
    return sum(1 for ts, ln in lines if lo <= ts <= hi and pat.search(ln))


def verdict(r):
    b, a = r["before"], r["after"]
    if b.get("open_status") is None or a.get("open_status") is None:
        return "NO_IMM"
    if b["open_status"] != a["open_status"]:
        return "TOGGLED(%s->%s)" % (int(b["open_status"]), int(a["open_status"]))
    if (b.get("comp_str") or "") != (a.get("comp_str") or ""):
        return "OPEN_SAME_COMP_CHANGED"
    return "NO_EFFECT"


def main():
    label, res = sys.argv[1], sys.argv[2]
    lines = load(sys.argv[3]) if len(sys.argv) > 3 else None
    with open(res, encoding="utf-8") as f:
        rows = json.load(f)
    print("### %s" % label)
    hdr = "| key | scenario | verdict | before open/comp | after open/comp | text |"
    if lines is not None:
        hdr += " observed | engine | send |"
    print(hdr)
    print("|" + "---|" * (hdr.count("|") - 1))
    for r in rows:
        b, a = r["before"], r["after"]
        row = "| %s | %s | %s | %s / %r | %s / %r | %r |" % (
            r["key"], r["scenario"], verdict(r), b.get("open_status"), b.get("comp_str"),
            a.get("open_status"), a.get("comp_str"), r.get("edit_text"))
        if lines is not None:
            lo, hi = r["t_start"], r["t_end"]
            row += " %d | %d | %d |" % (count(lines, OBSERVED, lo, hi), count(lines, ENGINE, lo, hi), count(lines, SEND, lo, hi))
        print(row)


if __name__ == "__main__":
    main()
