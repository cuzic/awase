#!/usr/bin/env python3
"""chrome_probe のログの集計(ADR-199 T1(e))。各ケースの `RESULT PASS/RECOVER/FAIL/INVALID` を数え、内容を表示する。
FAIL があれば rc=1、1件も判定できなければ(全 INVALID・ケース無し)rc=3、それ以外 0。
--strict(ADR-244 の専用構成): INVALID・RECOVER が 1 件でもある、または完走マーカー(=== 全ケース完了 ===)が無ければ rc=1。専用構成は少数のセルの退行(例: 持続トグルを手放さず
次のケースの setup が INVALID になる)を必ず赤にするため、INVALID を黙って数えないようにする。
使い方: check_chrome_probe.py [--strict] <chrome_probe.log>
"""
import re
import sys


def main():
    argv = [a for a in sys.argv[1:] if a != "--strict"]
    strict = len(argv) != len(sys.argv) - 1
    if len(argv) != 1:
        print(__doc__)
        return 2
    try:
        lines = open(argv[0], encoding="utf-8").read().splitlines()
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
    if counts["FAIL"]:
        return 1
    if strict and not any("=== 全ケース完了 ===" in ln for ln in lines):
        print("FAIL(--strict): 完走マーカー(=== 全ケース完了 ===)が無い（途中で止まった実行を緑にしない）")
        return 1
    if strict and (counts["INVALID"] or counts["RECOVER"]):
        print("FAIL(--strict): INVALID または RECOVER がある（専用構成は全セルが 1 回で PASS すること）")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
