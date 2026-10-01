#!/usr/bin/env python3
"""chrome_probe のログの集計(ADR-199 T1(e))。各ケースの `RESULT PASS/RECOVER/FAIL/INVALID` を数え、内容を表示する。
FAIL があれば rc=1、1件も判定できなければ(全 INVALID・ケース無し)rc=3、それ以外 0。
使い方: check_chrome_probe.py <chrome_probe.log>
"""
import re
import sys


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    try:
        lines = open(sys.argv[1], encoding="utf-8").read().splitlines()
    except OSError:
        print("INVALID: chrome_probe.log が無い(起動できなかった)")
        return 3
    counts = {"PASS": 0, "RECOVER": 0, "FAIL": 0, "INVALID": 0}
    for line in lines:
        line = re.sub(r"^\[[\d:.]+Z\] ", "", line)  # 行頭の時刻を落とす
        if re.match(r"\[CASE |PROBE |RESULT |SUMMARY |SETTLE |前面化|chrome=", line):
            print(line)
        m = re.match(r"RESULT (PASS|RECOVER|FAIL|INVALID)", line)
        if m:
            counts[m.group(1)] += 1
    print("集計:", counts)
    if counts["PASS"] + counts["RECOVER"] + counts["FAIL"] == 0:
        print("INVALID: 判定できたケースが無い")
        return 3
    return 1 if counts["FAIL"] else 0


if __name__ == "__main__":
    sys.exit(main())
