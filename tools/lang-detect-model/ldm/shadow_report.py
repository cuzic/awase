"""実機の `awase.log`(`[lang-detect]` の debug 行)から、今の判定器と shadow(LR)の食い違いを数える。

    uv run python -m ldm.shadow_report path/to/awase.log [awase.log.old ...]

debug の行に `lr_llr=` と `lr_not_japanese_at=` が出るのは、shadow を入れた版(`docs/tasks/lang-detect-lr-shadow.md`)から。
読むのは行の数値だけで、打鍵の内容は含まれない。
"""
from __future__ import annotations

import re
import sys
from collections import Counter

LINE = re.compile(
    r'\[lang-detect\] seq=\d+.*?verdict="(?P<verdict>[\w-]+)".*?stop="(?P<stop>[\w-]+)" presses=(?P<presses>\d+) llr=(?P<llr>-?[\d.]+)'
    r'.*?action="(?P<action>[\w-]+)"'
    r'(?:.*?lr_llr=Some\((?P<lr_llr>-?[\d.]+)\))?'
    r'(?:.*?lr_not_japanese_at=(?P<at>Some\((?P<at_n>\d+)\)|None))?'
)


def parse(lines):
    for ln in lines:
        m = LINE.search(ln)
        if m and m.group("lr_llr") is not None:
            yield {
                "verdict": m.group("verdict"),
                "stop": m.group("stop"),
                "presses": int(m.group("presses")),
                "llr": float(m.group("llr")),
                "action": m.group("action"),
                "lr_llr": float(m.group("lr_llr")),
                "lr_at": int(m.group("at_n")) if m.group("at_n") else None,
            }


def summarize(rows) -> str:
    rows = list(rows)
    c = Counter((r["verdict"], "lr:not-japanese" if r["lr_at"] else "lr:none") for r in rows)
    out = [f"episodes with shadow: {len(rows)}", "現在の判定 × shadow:"]
    for (v, s), n in sorted(c.items()):
        out.append(f"  {v:13} {s:16} {n}")
    fired = [r for r in rows if r["verdict"] == "not-japanese" and r["action"] != "none"]
    saved = [r for r in fired if not r["lr_at"]]
    out.append(f"現在が英数に入った(action != none): {len(fired)} 件、うち shadow は確定しなかった: {len(saved)} 件")
    extra = [r for r in rows if r["verdict"] != "not-japanese" and r["lr_at"]]
    out.append(f"現在は確定せず shadow が英数と確定した: {len(extra)} 件(英文なら取りこぼしの改善、日本語なら shadow の誤発火)")
    return "\n".join(out)


if __name__ == "__main__":
    lines = []
    for p in sys.argv[1:]:
        with open(p, encoding="utf-8", errors="replace") as f:
            lines += f.readlines()
    print(summarize(parse(lines)))
