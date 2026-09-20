#!/usr/bin/env python3
"""`--vkprobe` の結果表(ADR-186): 候補キーごとに、(a) OSが実際に配送したVK/スキャンコード(スパイクのフック観測)、
(b) IME OFFから押したときと、IME ON(かな)から押したときの実IMEの変化(open/conv)を出す。
使い方: vkprobe_report.py <スパイクのlog>   終了コード: 手順が1件も無ければ1、それ以外0(観測用)
"""
import re
import sys

# spike の VKPROBE_CANDIDATES と同じ順(ラベルのみ)
LABELS = [
    "scan 0x70 のみ(ひらがな物理キー)", "VK_KANA 0x15", "VK_DBE_KATAKANA 0xF1", "VK_DBE_HIRAGANA 0xF2",
    "VK_DBE_HIRAGANA 0xF2 (scan=0)", "VK_DBE_ROMAN 0xF5", "VK_DBE_NOROMAN 0xF6", "scan 0x29 のみ(半角/全角物理キー)",
    "VK_DBE_SBCSCHAR 0xF3", "VK_DBE_DBCSCHAR 0xF4", "VK_KANJI 0x19", "scan 0x3A のみ(英数物理キー)",
    "VK_DBE_ALPHANUMERIC 0xF0", "VK_IME_ON 0x16 (scan=0)", "scan 0x79 のみ(変換物理キー)", "scan 0x7B のみ(無変換物理キー)",
    "VK_CONVERT 0x1C",
]


def main():
    events = []  # {vk, scan, before(open,conv), after(open,conv)}
    cur = None
    for line in open(sys.argv[1], encoding="utf-8").read().splitlines():
        m = re.match(r"\[[\d:.]+Z\] KEY \[[^\]]*\] .*?vk=0x([0-9A-Fa-f]+) scan=0x([0-9A-Fa-f]+) press=[\d:.]+Z( \(auto\))?", line)
        if m:
            cur = {"vk": int(m.group(1), 16), "scan": int(m.group(2), 16), "auto": bool(m.group(3))}
            events.append(cur)
            continue
        if cur is None:
            continue
        m = re.match(r"\s+前\s*: A\(open=(\d) conv=0x([0-9A-Fa-f]+)\)", line)
        if m:
            cur["before"] = (int(m.group(1)), int(m.group(2), 16))
        m = re.match(r"\s+\+1500ms: A\(open=(\d) conv=0x([0-9A-Fa-f]+)\)", line)
        if m:
            cur["after"] = (int(m.group(1)), int(m.group(2), 16))
    # 押下のみ(auto)を対象に、[準備OFF, 候補, 準備ON, 候補] の4件ずつ
    keys = [e for e in events if e["auto"]]
    if len(keys) < 4:
        print("FAIL: 記録された押下が足りない")
        return 1

    def fmt(e):
        if "before" not in e or "after" not in e:
            return "記録なし"
        (bo, bc), (ao, ac) = e["before"], e["after"]
        chg = "変化なし" if (bo, bc) == (ao, ac) else "変化"
        return f"open {bo}→{ao} conv 0x{bc:02X}→0x{ac:02X} ({chg})"

    print(f"{'候補':<34} {'OSが配送したVK/scan':<20} {'IME OFFから':<44} IME ON(かな)から")
    for i, label in enumerate(LABELS):
        g = keys[i * 4:(i + 1) * 4]
        if len(g) < 4:
            print(f"{label:<34} (記録なし)")
            continue
        off_press, on_press = g[1], g[3]
        seen = f"vk=0x{off_press['vk']:02X} scan=0x{off_press['scan']:02X}"
        print(f"{label:<34} {seen:<20} {fmt(off_press):<44} {fmt(on_press)}")
    print("結果: 観測")
    return 0


if __name__ == "__main__":
    sys.exit(main())
