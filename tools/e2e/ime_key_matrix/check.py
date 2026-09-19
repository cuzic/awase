#!/usr/bin/env python3
"""ADR-186 実機E2Eの合否判定。スパイク(`--auto`)のログと awase のデバッグログを突き合わせ、
期待表(EXPECT)と照合して PASS/FAIL を出す。終了コード: 全PASSなら0、1件でもFAILなら1。

使い方: check.py <スパイクのlog> <awaseのlog(スパイク起動時刻以降を抜粋したもの)>
"""
import re
import sys
from datetime import datetime

# STEP番号(1-10) → 期待。real_open/real_conv は押下 +400ms 時点の実IME(ImmGet*)。
# engine: ('activated'|'deactivated', 許容ms) = 押下後その時間内にEngineが切り替わる
#         ('none', 1500) = 押下後1500ms、Engineがactivatedにならない(OFFのまま)
# delegate_false: 押下後300ms以内に「IME open axis delegated → false」が出る
EXPECT = {
    1: dict(real_open=1, real_conv=0x10, engine=("deactivated", 300)),  # ひらがな: かな→半角英数
    2: dict(real_open=0, real_conv=0x10, engine=("none", 1500)),  # 無変換: 半角英数ON→OFF
    3: dict(real_open=1, real_conv=0x10, engine=("none", 1500)),  # 無変換: OFF→ON(半角英数のまま、決定2の核心)
    4: dict(real_open=1, real_conv=0x19, engine=("activated", 300)),  # ひらがな: 半角英数→かな
    5: dict(real_open=0, real_conv=0x19, delegate_false=True),  # 無変換: かなON→OFF
    6: dict(real_open=1, real_conv=0x19, engine=("activated", 300)),  # 無変換: OFF→ON(かな)
    7: dict(real_open=1, real_conv=0x10, engine=("deactivated", 300)),  # ひらがな: かな→半角英数
    8: dict(real_open=0, real_conv=0x10, engine=("none", 1500)),  # 無変換: 半角英数ON→OFF
    9: dict(real_open=1, real_conv=0x10, engine=("none", 1500)),  # 無変換: OFF→ON(退行窓の確認)
    10: dict(real_open=1, real_conv=0x19, engine=("activated", 300)),  # ひらがな: 半角英数→かな
}


def to_ms(t: str) -> float:
    h, m, s = t.rstrip("Z").split(":")
    return (int(h) * 3600 + int(m) * 60) * 1000 + float(s) * 1000


def parse_spike(path):
    steps = {}
    cur = None
    for line in open(path, encoding="utf-8").read().splitlines():
        m = re.match(r"\[[\d:.]+Z\] KEY \[SCRIPT (\d+)/10 [^\]]*\].*?press=([\d:.]+Z)", line)
        if m:
            cur = int(m.group(1))
            steps[cur] = {"press": to_ms(m.group(2))}
            continue
        m = re.match(r"\s+\+400ms: A\(open=(\d) conv=0x([0-9A-Fa-f]+)\)", line)
        if m and cur is not None and "open" not in steps[cur]:
            steps[cur]["open"] = int(m.group(1))
            steps[cur]["conv"] = int(m.group(2), 16)
    return steps


def parse_awase(path):
    events = []  # (ms, kind, detail)
    unwarranted = 0
    for line in open(path, encoding="utf-8", errors="replace").read().splitlines():
        m = re.match(r"\d{4}-\d\d-\d\dT([\d:.]+)Z\s+\w+\s+(.*)", line)
        if not m:
            continue
        ms = to_ms(m.group(1))
        msg = m.group(2)
        if "outcome=Unwarranted" in msg:
            unwarranted += 1
        r = re.search(r"Engine (activated|deactivated) .*reason=(\S+?)\)?$", msg)
        if r:
            events.append((ms, r.group(1), r.group(2)))
        elif "IME open axis delegated" in msg:
            events.append((ms, "delegate", msg[msg.find("→"):].strip()))
    return events, unwarranted


def main():
    if len([a for a in sys.argv if a != "--real-only"]) != 3:
        print(__doc__)
        return 2
    real_only = "--real-only" in sys.argv
    argv = [a for a in sys.argv if a != "--real-only"]
    sys.argv = argv
    steps = parse_spike(sys.argv[1])
    events, unwarranted = parse_awase(sys.argv[2])
    fails = 0
    print(f"{'STEP':>4} {'実IME(+400ms)':<16} {'Engine':<34} 判定")
    for n in range(1, 11):
        exp = EXPECT[n]
        st = steps.get(n)
        if not st or "open" not in st:
            print(f"{n:>4} 記録なし → FAIL")
            fails += 1
            continue
        p = st["press"]
        problems = []
        if st["open"] != exp["real_open"] or st["conv"] != exp["real_conv"]:
            problems.append(
                f"実IME期待 open={exp['real_open']} conv=0x{exp['real_conv']:02X}"
            )
        after = [(ms - p, k, d) for ms, k, d in events if -20 <= ms - p <= 1500]
        desc = "; ".join(f"{k}@{dt:+.0f}ms" for dt, k, d in after) or "(なし)"
        if "engine" in exp and not real_only:
            kind, win = exp["engine"]
            if kind == "none":
                bad = [x for x in after if x[1] == "activated"]
                if bad:
                    problems.append("Engineがactivatedになった(OFFのままの期待)")
            else:
                ok = [x for x in after if x[1] == kind and x[0] <= win]
                if not ok:
                    problems.append(f"{win}ms以内に{kind}しない")
        if exp.get("delegate_false") and not real_only:
            if not [x for x in after if x[1] == "delegate" and x[0] <= 300]:
                problems.append("delegate → false が出ない")
        verdict = "PASS" if not problems else "FAIL: " + " / ".join(problems)
        fails += bool(problems)
        real = f"open={st['open']} conv=0x{st['conv']:02X}"
        print(f"{n:>4} {real:<16} {desc[:34]:<34} {verdict}")
    if unwarranted and not real_only:
        print(f"FAIL: outcome=Unwarranted が {unwarranted} 件")
        fails += 1
    print("結果:", "ALL PASS" if fails == 0 else f"{fails} 件 FAIL")
    return 0 if fails == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
