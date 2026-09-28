#!/usr/bin/env python3
"""GJI の CUSTOM 表の Hankaku/Zenkaku 行を変えた利用者の Alt+半角/全角(0x19)で、実IMEと Engine(belief)がずれないことの判定
(ADR-202 T16-3、スパイクの `--chord=A4,19 --chord-prep=16`)。

0x19 の KEY 行(実IME: A の open)と awase のフルデバッグログで見る:
  --expect=open    行が閉じないコマンド。前 open=1、+1500ms も open=1(IME は開いたまま)で、押下から1.5秒のあいだに
                   awase が Engine を OFF にしたり(`Engine deactivated`)、閉じる書き込み(`actuation decision ... open=false`)を出したりしない。
                   修正前は静的 Toggle で belief だけ反転してここが崩れる。
  --expect=closed  行がトグル。前 open=1、+1500ms は open=0(IME が閉じる)で、Engine も OFF に追随する(`Engine deactivated`)。
使い方: check_kanji_role.py --expect=open|closed <スパイクのlog> <awaseのフルデバッグlog>   終了コード: 0=OK / 1=NG / 3=INVALID
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from check_consistency import to_ms  # noqa: E402

WINDOW_MS = 1500


def parse_kanji_rows(path):
    rows = []
    cur = None
    invalid = 0
    for line in open(path, encoding="utf-8").read().splitlines():
        m = re.match(r"\[[\d:.]+Z\] KEY \[[^\]]*\] .*vk=0x19 .*press=([\d:.]+Z)", line)
        if m:
            cur = {"press": to_ms(m.group(1))}
            rows.append(cur)
            continue
        if re.match(r"\[[\d:.]+Z\] KEY ", line):
            cur = None
            continue
        if cur is None:
            continue
        m = re.match(r"\s+前\s*: A\(open=(\d) ", line)
        if m:
            cur["before"] = int(m.group(1))
        m = re.match(r"\s+\+1500ms: A\(open=(\d) ", line)
        if m:
            cur["after"] = int(m.group(1))
        if "[AUTO] フォーカス復帰" in line:
            invalid += 1
    return rows, invalid


def awase_events(path, press, want_closing_write):
    """押下から WINDOW_MS のあいだの、Engine の OFF 化と閉じる書き込みの時刻(ms)を返す。"""
    deactivated = []
    close_writes = []
    for line in open(path, encoding="utf-8", errors="replace").read().splitlines():
        m = re.match(r"\d{4}-\d\d-\d\dT([\d:.]+)Z\s+\w+\s+(.*)", line)
        if not m:
            continue
        t = to_ms(m.group(1))
        if not (press - 50 <= t <= press + WINDOW_MS):
            continue
        body = m.group(2)
        if "Engine deactivated" in body:
            deactivated.append(t)
        elif want_closing_write and "actuation decision" in body and "open=false" in body:
            close_writes.append(t)
    return deactivated, close_writes


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--expect=")]
    expect = next((a.split("=", 1)[1] for a in sys.argv[1:] if a.startswith("--expect=")), "")
    if expect not in ("open", "closed") or len(args) != 2:
        print(__doc__)
        return 2
    rows, invalid = parse_kanji_rows(args[0])
    if invalid:
        print(f"INVALID: 実行中にフォーカスが外れた({invalid}回)。この回は判定に使わない")
        return 3
    rows = [r for r in rows if {"before", "after"} <= r.keys()]
    if not rows:
        print("INVALID: 0x19 の KEY 行(前/+1500ms)が無い(--chord が動かなかった)")
        return 3
    fails = 0
    for i, r in enumerate(rows, 1):
        if r["before"] != 1:
            print(f"{i}: 前 open={r['before']} → INVALID(IME が ON でない)")
            return 3
        deactivated, close_writes = awase_events(args[1], r["press"], expect == "open")
        why = []
        if expect == "open":
            if r["after"] != 1:
                why.append("実IME が閉じた(行は閉じないコマンドのはず)")
            if deactivated:
                why.append(f"Engine が OFF になった({len(deactivated)}件): IME は開いたままなのに belief だけ反転")
            if close_writes:
                why.append(f"閉じる書き込みが出た({len(close_writes)}件)")
        else:
            if r["after"] != 0:
                why.append("実IME が閉じていない(行はトグルのはず)")
            if not deactivated:
                why.append("Engine が OFF に追随していない")
        fails += bool(why)
        print(f"{i}: 前 open={r['before']} +1500ms open={r['after']} Engine OFF={len(deactivated)} 閉じる書き込み={len(close_writes)} → "
              + ("PASS" if not why else "FAIL: " + " / ".join(why)))
    print("結果:", "ALL PASS" if fails == 0 else f"{fails} 件 FAIL")
    return 0 if fails == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
