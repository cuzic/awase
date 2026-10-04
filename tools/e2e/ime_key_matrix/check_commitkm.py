#!/usr/bin/env python3
"""typing_stress --mode=keymatrix --km-commit=<key> の `km_trial` を集計する(BUG-185 方針C、観測のみ、rc 常に 0。試行0件は 3)。

見るもの(composition あり=--km-comp、なし=無指定):
- text_commit: 確定系キーの直後の本文(composition あり: 文字が残れば確定、空なら取り消し/無効。なし: 改行等の副作用)。
- closed: 続く OFF キーで実 IME が閉じたか(presses の最後の api2000 / api500 が target=閉)。
- text_post: OFF の後の本文(確定された文字が残っているか)。
使い方: check_commitkm.py typing_stress.log
"""
import json
import sys


def main():
    trials = []
    for line in open(sys.argv[1], encoding="utf-8", errors="replace"):
        i = line.find("[TS-JSON] ")
        if i < 0:
            continue
        try:
            r = json.loads(line[i + len("[TS-JSON] "):])
        except ValueError:
            continue
        if r.get("type") == "km_trial":
            trials.append(r)
    if not trials:
        print("COMMITKM: km_trial が0件(INVALID)")
        return 3
    cells = {}
    for t in trials:
        cells.setdefault(t["cell"], []).append(t)
    for cell, ts in cells.items():
        made = [t for t in ts if t.get("pre_ok") and t.get("pre_api") == t.get("r0") and not t.get("focus_lost")]
        closed = 0
        tc, tp = {}, {}
        for t in made:
            ps = t.get("presses") or []
            if ps:
                v = ps[-1].get("api2000") if ps[-1].get("api2000") is not None else ps[-1].get("api500")
                if v == t.get("target"):
                    closed += 1
            k = repr(t.get("text_commit"))
            tc[k] = tc.get(k, 0) + 1
            k = repr(t.get("text_post"))
            tp[k] = tp.get(k, 0) + 1
        print(f"COMMITKM: cell={cell} commit={ts[0].get('commit')} n={len(ts)} made={len(made)} reached_target={closed} text_commit={tc} text_post={tp}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
