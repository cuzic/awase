#!/usr/bin/env python3
"""ADR-199 T17 Phase 4 の B4 計測(使い捨て、測定用ブランチ ci/b4-msime-toggle-probe 専用)。

ime_key_matrix_spike の --seq 実行ログを手順ごとの表にする: 押下前の実IME開閉・+1500ms 後の実IME開閉・
awase.log の同じ手順の窓([press, 次の press))内の観測件数(observed=`observer_poll=Some`/`ObserverReported`)。
awase.log を省くと awase なしの対照(観測列なし)。

使い方: check_b4_seq.py <label> <ime_key_matrix_spike.log> [awase.log]
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from check_consistency import parse_spike, to_ms  # noqa: E402

OBSERVED = re.compile(r"observer_poll=Some|ObserverReported")
STRATEGY = re.compile(r"\[stage-observe\] strategy=(\w+)")
ENGINE = re.compile(r"Engine (de)?activated")
DELEGATED = re.compile(r"IME open axis delegated|apply_ime_open|forced_open|shadow-toggle")


def load_awase(path):
    out = []
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                m = re.match(r"\d{4}-\d\d-\d\dT([\d:.]+)Z\s+(.*)", line)
                if m:
                    out.append((to_ms(m.group(1)), line))
    except OSError:
        pass
    return out


def main():
    label, spike = sys.argv[1], sys.argv[2]
    awase = load_awase(sys.argv[3]) if len(sys.argv) > 3 else None
    steps, invalid = parse_spike(spike)
    print("### %s (invalid_focus_loss=%d)" % (label, invalid))
    hdr = "| n | key | 前 open | +1500ms open | 変化 |"
    if awase is not None:
        hdr += " observed | strategy | engine変化 | 送信/意図 |"
    print(hdr)
    print("|" + "---|" * (hdr.count("|") - 1))
    for i, st in enumerate(steps):
        before, after = st.get("before_open"), st.get("open")
        change = "-" if before is None or after is None else ("TOGGLED" if before != after else "same")
        row = "| %s | %s | %s | %s | %s |" % (st["n"], st["name"], before, after, change)
        if awase is not None:
            lo = st["press"]
            hi = steps[i + 1]["press"] if i + 1 < len(steps) else lo + 3000
            win = [ln for ts, ln in awase if lo <= ts < hi]
            obs = sum(1 for ln in win if OBSERVED.search(ln))
            strat = {}
            for ln in win:
                m = STRATEGY.search(ln)
                if m:
                    strat[m.group(1)] = strat.get(m.group(1), 0) + 1
            eng = "".join("A" if "deactivated" not in ln else "D" for ln in win if ENGINE.search(ln))
            sent = sum(1 for ln in win if DELEGATED.search(ln))
            row += " %d | %s | %s | %d |" % (obs, strat or "-", eng or "-", sent)
        print(row)


if __name__ == "__main__":
    main()
