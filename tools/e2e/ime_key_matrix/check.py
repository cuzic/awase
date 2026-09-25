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
# 手順5・6（BUG-162 B、2026-09-25）: ADR-191 で無変換/変換の単独タップ代行（delegate）を撤去した。Engine が有効な間、
# 無変換は既定設定（muhenkan_solo_tap_always_suppress = true）で Suppress され、実 IME に届かない。よって
# 手順5は「IME は ON のまま（かな）、Engine は変わらない」。手順6は前提状態（IME OFF）を無変換では作れないので、
# スパイクが「前提状態にできずスキップ」と記録する（SKIPPABLE に載せた手順だけ、その記録があれば SKIP とする）。
EXPECT = {
    1: dict(real_open=1, real_conv=0x10, engine=("deactivated", 300)),  # ひらがな: かな→半角英数
    2: dict(real_open=0, real_conv=0x10, engine=("none", 1500)),  # 無変換: 半角英数ON→OFF
    3: dict(real_open=1, real_conv=0x10, engine=("none", 1500)),  # 無変換: OFF→ON(半角英数のまま、決定2の核心)
    4: dict(real_open=1, real_conv=0x19, engine=("activated", 300)),  # ひらがな: 半角英数→かな
    5: dict(real_open=1, real_conv=0x19, engine=("none", 1500)),  # 無変換: Suppress される(IME ON のまま・かな)
    6: dict(real_open=1, real_conv=0x19, engine=("activated", 300)),  # 無変換: OFF→ON(かな)。前提を作れなければ SKIP
    7: dict(real_open=1, real_conv=0x10, engine=("deactivated", 300)),  # ひらがな: かな→半角英数
    8: dict(real_open=0, real_conv=0x10, engine=("none", 1500)),  # 無変換: 半角英数ON→OFF
    9: dict(real_open=1, real_conv=0x10, engine=("none", 1500)),  # 無変換: OFF→ON(退行窓の確認)
    10: dict(real_open=1, real_conv=0x19, engine=("activated", 300)),  # ひらがな: 半角英数→かな
}

# スパイクが前提状態にできず「スキップ」と記録したとき、SKIP（FAILでも PASS でもない）としてよい手順。
SKIPPABLE = {6}


def to_ms(t: str) -> float:
    h, m, s = t.rstrip("Z").split(":")
    return (int(h) * 3600 + int(m) * 60) * 1000 + float(s) * 1000


def parse_spike(path):
    steps = {}
    cur = None
    for line in open(path, encoding="utf-8").read().splitlines():
        sk = re.match(r"\[AUTO\] STEP (\d+) .*前提状態にできずスキップ", line)
        if sk:
            steps.setdefault(int(sk.group(1)), {})["skipped"] = True
            continue
        m = re.match(r"\[[\d:.]+Z\] KEY \[SCRIPT (\d+)/10 [^\]]*\].*?press=([\d:.]+Z)", line)
        if m:
            cur = int(m.group(1))
            steps.setdefault(cur, {})["press"] = to_ms(m.group(2))
            continue
        m = re.match(r"\s+\+400ms: A\(open=(\d) conv=0x([0-9A-Fa-f]+)\)", line)
        if m and cur is not None and "open" not in steps[cur]:
            steps[cur]["open"] = int(m.group(1))
            steps[cur]["conv"] = int(m.group(2), 16)
    return steps


def parse_awase(path):
    events = []  # (ms, kind, detail)
    # `ime open applied seq=N … outcome="Unwarranted"`（journal の1行）だけを、seq ごとに1件と数える。
    # 旧実装は "outcome=Unwarranted" を含む行を全て数え、同じ span（`on_ime_apply_complete{… outcome=Unwarranted …}`）
    # の別の行（Timer set 等）まで数えたので、1件が2件になった（BUG-162 C）。
    unwarranted_seqs = set()
    for line in open(path, encoding="utf-8", errors="replace").read().splitlines():
        m = re.match(r"\d{4}-\d\d-\d\dT([\d:.]+)Z\s+\w+\s+(.*)", line)
        if not m:
            continue
        ms = to_ms(m.group(1))
        msg = m.group(2)
        u = re.search(r'\bime open applied seq=(\d+)\b.*\boutcome="Unwarranted"', msg)
        if u:
            unwarranted_seqs.add(u.group(1))
        r = re.search(r"Engine (activated|deactivated) .*reason=(\S+?)\)?$", msg)
        if r:
            events.append((ms, r.group(1), r.group(2)))
    return events, len(unwarranted_seqs)


def main():
    if len([a for a in sys.argv if a != "--real-only"]) != 3:
        print(__doc__)
        return 2
    real_only = "--real-only" in sys.argv
    argv = [a for a in sys.argv if a != "--real-only"]
    sys.argv = argv
    steps = parse_spike(sys.argv[1])
    events, unwarranted = parse_awase(sys.argv[2])
    # 実行の途中(手順1の記録以降)でスパイクがフォーカスを取り戻していたら、その回は無効(INVALID)。
    # フォーカス移動は awase の FocusChange(cold化・belief書き換え)を誘発し、結果を汚す。
    invalid = 0
    started = False
    for line in open(sys.argv[1], encoding="utf-8").read().splitlines():
        if "KEY [SCRIPT 1/10" in line:
            started = True
        if started and "[AUTO] フォーカス復帰" in line:
            invalid += 1
    if invalid:
        print(f"INVALID: 実行中にフォーカスが外れた({invalid}回)。この回は判定に使わない")
        return 3
    fails = 0
    print(f"{'STEP':>4} {'実IME(+400ms)':<16} {'Engine':<34} 判定")
    for n in range(1, 11):
        exp = EXPECT[n]
        st = steps.get(n)
        if n in SKIPPABLE and st and st.get("skipped") and "open" not in st:
            print(f"{n:>4} {'(スキップ)':<16} {'(前提状態にできない=設計上)':<34} SKIP")
            continue
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
