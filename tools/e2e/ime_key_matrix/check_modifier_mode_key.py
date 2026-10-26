#!/usr/bin/env python3
"""修飾キーを押したままのひらがなキー(0xF2)の判定(ADR-281、BUG-221。スパイクの `--chord=<修飾VK>,F2 --chord-prep=1A`)。

2 つのことを見る。
  1. 実 IME(スパイクの KEY 行): 閉じた状態から、修飾付きの F2 で開くか(M1/M1'/M2。GJI/Mozc がこの 4 キーの修飾を捨てるかの実機確認)。
  2. awase(フルデバッグ log、awase を起動した構成のみ): F2 の KeyDown が物理の修飾付きで届き(`mods(c=…`)、予測(`[key-effect-predict] vk=0xF2 open=Some(true)`)と
     Engine の活性化(`Engine activated`)が F2 の押下から WINDOW_MS のあいだに出ること(ADR-281 の A′)。

--mod=none|ctrl|shift|alt    押しっぱなしにした修飾(none=対照。修飾でないキーを押したまま)。awase の `mods(` と照らす。
--expect=observe             判定せずに表示するだけ(実 IME の挙動の測定用。INVALID 以外は rc=0)。
--expect=predict             awase のログで予測と Engine の活性化が出ていることを要求する(出なければ FAIL=rc=1)。実 IME が開いていることも要求する。
使い方: check_modifier_mode_key.py --mod=ctrl --expect=predict <スパイクのlog> [awaseのフルデバッグlog]   終了コード: 0=OK / 1=NG / 3=INVALID
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from check_consistency import to_ms  # noqa: E402

WINDOW_MS = 1500
TAP_VK = "0xF2"
# awase の `[engine-input]` 行の `mods(c=… s=… a=… w=…)` で、押しっぱなしにした修飾が立っていること。
MOD_FLAG = {"ctrl": "c", "shift": "s", "alt": "a"}


def parse_rows(path):
    """スパイクの KEY 行(vk=0xF2)ごとに、押下時刻・前後の実 IME の開閉・フォーカス喪失を集める。"""
    rows = []
    cur = None
    invalid = 0
    for line in open(path, encoding="utf-8", errors="replace").read().splitlines():
        m = re.match(r"\[[\d:.]+Z\] KEY \[[^\]]*\] .*vk=" + TAP_VK + r" .*press=([\d:.]+Z)", line, re.I)
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


def awase_events(path, press):
    """F2 の押下の前後(-50ms〜+WINDOW_MS)の、awase のログから拾う事実。"""
    ev = {"keydown_mods": None, "predict_open": None, "activated": False, "flip_true": False}
    if not path or not os.path.exists(path):
        return None
    for line in open(path, encoding="utf-8", errors="replace").read().splitlines():
        m = re.match(r"\d{4}-\d\d-\d\dT([\d:.]+)Z\s+\w+\s+(.*)", line)
        if not m:
            continue
        t = to_ms(m.group(1))
        if not (press - 50 <= t <= press + WINDOW_MS):
            continue
        body = m.group(2)
        if re.search(r"\[engine-input\] vk=0x[fF]2 KeyDown", body) and ev["keydown_mods"] is None:
            mm = re.search(r"mods\(c=(\w+) s=(\w+) a=(\w+) w=(\w+)\)", body)
            if mm:
                ev["keydown_mods"] = dict(zip("csaw", (v == "true" for v in mm.groups())))
        if "[key-effect-predict] vk=0xF2" in body:
            mo = re.search(r"open=(Some\(true\)|Some\(false\)|None|true|false)", body)
            ev["predict_open"] = mo.group(1) if mo else "?"
        if "Engine activated" in body:
            ev["activated"] = True
        if "[effective-open-flip] → true" in body:
            ev["flip_true"] = True
    return ev


def main():
    flags = [a for a in sys.argv[1:] if a.startswith("--")]
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    mod = next((a.split("=", 1)[1] for a in flags if a.startswith("--mod=")), "")
    expect = next((a.split("=", 1)[1] for a in flags if a.startswith("--expect=")), "")
    if mod not in ("none", "ctrl", "shift", "alt") or expect not in ("observe", "predict") or not 1 <= len(args) <= 2:
        print(__doc__)
        return 2
    rows, invalid = parse_rows(args[0])
    if invalid:
        print(f"INVALID: 実行中にフォーカスが外れた({invalid}回)。この回は判定に使わない")
        return 3
    rows = [r for r in rows if {"before", "after"} <= r.keys()]
    if not rows:
        print(f"INVALID: {TAP_VK} の KEY 行(前/+1500ms)が無い(--chord が動かなかった)")
        return 3
    awase_log = args[1] if len(args) == 2 else None
    fails = 0
    for i, r in enumerate(rows, 1):
        if r["before"] != 0:
            print(f"{i}: 前 open={r['before']} → INVALID(押す前に IME が閉じていない)")
            return 3
        ev = awase_events(awase_log, r["press"])
        why = []
        line = f"MODKEY: mod={mod} 前 open={r['before']} +1500ms open={r['after']}"
        if ev is not None:
            mods = ev["keydown_mods"]
            line += (f" | awase: mods={mods} predict_open={ev['predict_open']} "
                     f"flip_true={ev['flip_true']} Engine activated={ev['activated']}")
            if expect == "predict":
                flag = MOD_FLAG.get(mod)
                if mods is None:
                    why.append("awase のログに F2 の KeyDown(engine-input)が無い(注入が届いていない)")
                elif flag is not None and not mods.get(flag):
                    why.append(f"F2 の KeyDown の mods に {flag}=true が無い(物理の修飾として届いていない)")
                if r["after"] != 1:
                    why.append("実 IME が開いていない(修飾付きの F2 で開かないなら予測してはいけない)")
                if ev["predict_open"] not in ("Some(true)",):
                    why.append(f"F2 の予測が open=Some(true) でない({ev['predict_open']})")
                if not ev["flip_true"] and not ev["activated"]:
                    why.append("belief が ON に追随していない(effective-open-flip → true も Engine activated も無い)")
        elif expect == "predict":
            why.append("awase のログが無い")
        fails += bool(why)
        print(f"{i}: {line} → " + ("PASS" if not why else "FAIL: " + " / ".join(why)))
    print("結果:", "ALL PASS" if fails == 0 else f"{fails} 件 FAIL")
    return 0 if fails == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
