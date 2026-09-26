#!/usr/bin/env python3
"""--kana-drop の結果集計。実IMEを conv=0x09(ROMAN 無し)へ落としたあと、awase の conv 自動書き込みが
ROMAN(0x10)を戻したかを [KANA-RESULT] 行で数える。判定はしない(rc=0)。結果行が無ければ rc=3(INVALID)。

使い方: check_kana_drop.py <spike.log>
"""
import re
import sys
from collections import defaultdict

ROMAN = 0x10


def main() -> int:
    rows = defaultdict(list)
    for line in open(sys.argv[1], encoding="utf-8", errors="replace"):
        m = re.search(r"\[KANA-RESULT\] round=(\d+) variant=(\S+) open=(\S+) conv=(\S+)", line)
        if m:
            conv = None if m.group(4) == "none" else int(m.group(4), 16)
            rows[m.group(2)].append((m.group(3), conv))
    if not rows:
        print("kana-drop: [KANA-RESULT] が1件も無い(INVALID)")
        return 3
    print("| variant | 回数 | ROMAN 復元 | ROMAN 無しのまま | 読めない | 最終 conv の内訳 |")
    print("|---|---|---|---|---|---|")
    for v, rs in rows.items():
        ok = sum(1 for _, c in rs if c is not None and c & ROMAN)
        ng = sum(1 for _, c in rs if c is not None and not c & ROMAN)
        un = sum(1 for _, c in rs if c is None)
        cnt = defaultdict(int)
        for o, c in rs:
            cnt[f"open={o},conv={'none' if c is None else f'0x{c:X}'}"] += 1
        print(f"| {v} | {len(rs)} | {ok} | {ng} | {un} | {dict(cnt)} |")
    return 0


if __name__ == "__main__":
    sys.exit(main())
