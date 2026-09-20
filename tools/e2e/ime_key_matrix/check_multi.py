#!/usr/bin/env python3
"""`--repeat=N` で1プロセス内に連続実行したスパイクのログを、実行(RUN)ごとに切り出して判定する。

使い方: check_multi.py [--real-only] <スパイクのlog> <awaseのlog(全体でよい)>
無効(INVALID)判定: (1) 実行中にフォーカス復帰が起きた、(2) スパイクのフックが人の物理キーを見た、
(3) awase のログに、その実行の時間帯の物理キー(engine-input の extra=0x0)がある(スパイク由来は extra=0x5350494B)。
終了コード: 有効回がすべてPASSなら0、失敗があれば1、有効回が無ければ3。
"""
import re
import sys
from datetime import datetime, timezone

sys.path.insert(0, __import__("os").path.dirname(__file__))
import check  # noqa: E402  EXPECT / to_ms を共有

TS = re.compile(r"^\[([\d:.]+)Z\]")


def split_runs(lines):
    runs, cur = [], None
    for ln in lines:
        m = re.match(r"\[RUN (\d+)/(\d+) (?!完了|timeout)", ln)  # 開始/START/文字化け(PowerShell 5の.ps1は非BOMをANSI解釈)のいずれでも
        if m:
            cur = {"n": int(m.group(1)), "lines": []}
            runs.append(cur)
            continue
        if cur is None and "KEY [SCRIPT 1/10" in ln:  # 最初の実行(マーカーは最初には無い)
            cur = {"n": 1, "lines": []}
            runs.append(cur)
        if cur is not None:
            cur["lines"].append(ln)
    return runs


def steps_of(lines):
    steps, cur = {}, None
    for ln in lines:
        m = re.match(r"\[[\d:.]+Z\] KEY \[SCRIPT (\d+)/10 [^\]]*\].*?press=([\d:.]+Z)", ln)
        if m:
            cur = int(m.group(1))
            steps[cur] = {"press": check.to_ms(m.group(2))}
            continue
        m = re.match(r"\s+\+400ms: A\(open=(\d) conv=0x([0-9A-Fa-f]+)\)", ln)
        if m and cur is not None and "open" not in steps[cur]:
            steps[cur]["open"] = int(m.group(1))
            steps[cur]["conv"] = int(m.group(2), 16)
    return steps


def phys_in_awase(path, t0, t1):
    """awase ログの、[t0,t1](ms、UTC時刻)にある物理キー(extra=0x0)の件数。"""
    n = 0
    for ln in open(path, encoding="utf-8", errors="replace"):
        m = re.match(r"\d{4}-\d\d-\d\dT([\d:.]+)Z .*engine-input\] vk=0x\w+ Key(?:Down|Up) .*? extra=(0x\w+)", ln)
        if m and m.group(2) == "0x0" and t0 <= check.to_ms(m.group(1)) <= t1:
            n += 1
    return n


def main():
    real_only = "--real-only" in sys.argv
    args = [a for a in sys.argv[1:] if a != "--real-only"]
    if len(args) != 2:
        print(__doc__)
        return 2
    spike_lines = open(args[0], encoding="utf-8").read().splitlines()
    events, _ = check.parse_awase(args[1])
    runs = split_runs(spike_lines)
    valid = passed = 0
    fail_steps = []
    for r in runs:
        st = steps_of(r["lines"])
        if not st:
            print(f"RUN {r['n']:>3}: 記録なし")
            continue
        t0 = min(v["press"] for v in st.values()) - 3000
        t1 = max(v["press"] for v in st.values()) + 2000
        reasons = []
        # 起動直後のフォーカス取得は正常。手順1の記録以降にフォーカスが外れた回だけ無効(check.py と同じ条件)。
        started = False
        lost = 0
        for ln in r["lines"]:
            if "KEY [SCRIPT 1/10" in ln:
                started = True
            if started and "[AUTO] フォーカス復帰" in ln:
                lost += 1
        if lost:
            reasons.append(f"実行中にフォーカス復帰{lost}回")
        if any(re.search(r"\] KEY \[", ln) and not re.search(r"\((auto|injected)\)", ln) for ln in r["lines"]):
            reasons.append("スパイクが物理キーを観測")
        ph = phys_in_awase(args[1], t0, t1)
        if ph:
            reasons.append(f"awaseログに物理キー{ph}件")
        problems = []
        for n in range(1, 11):
            exp, s = check.EXPECT[n], st.get(n)
            if not s or "open" not in s:
                problems.append((n, "記録なし"))
                continue
            p = s["press"]
            if s["open"] != exp["real_open"] or s["conv"] != exp["real_conv"]:
                problems.append((n, f"実IME open={s['open']} conv=0x{s['conv']:02X} (期待 open={exp['real_open']} conv=0x{exp['real_conv']:02X})"))
                continue
            if real_only:
                continue
            after = [(ms - p, k, d) for ms, k, d in events if -20 <= ms - p <= 1500]
            if "engine" in exp:
                kind, win = exp["engine"]
                if kind == "none":
                    if [x for x in after if x[1] == "activated"]:
                        problems.append((n, "Engineがactivated(OFFのままの期待)"))
                elif not [x for x in after if x[1] == kind and x[0] <= win]:
                    problems.append((n, f"{win}ms以内に{kind}しない"))
            if exp.get("delegate_false") and not [x for x in after if x[1] == "delegate" and x[0] <= 300]:
                problems.append((n, "delegate → false が出ない"))
        if reasons:
            print(f"RUN {r['n']:>3}: INVALID({', '.join(reasons)})  ※判定: {'PASS' if not problems else 'FAIL ' + str([p[0] for p in problems])}")
            continue
        valid += 1
        if not problems:
            passed += 1
            print(f"RUN {r['n']:>3}: PASS")
        else:
            fail_steps += [p[0] for p in problems]
            print(f"RUN {r['n']:>3}: FAIL " + "; ".join(f"step{n}: {d}" for n, d in problems))
    print(f"集計: 全{len(runs)}回 有効={valid} PASS={passed} FAIL={valid - passed} INVALID={len(runs) - valid}"
          + (f" (失敗した手順: {sorted(fail_steps)})" if fail_steps else ""))
    if valid == 0:
        return 3
    return 0 if passed == valid else 1


if __name__ == "__main__":
    sys.exit(main())
