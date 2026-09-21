#!/usr/bin/env python3
"""開閉トグルキー(半角/全角 0xF3/0xF4 等)の合否判定(ADR-189、`--hz`)。

各押下で、実IMEの開閉が直前の状態から**反転**すること(VKの種類に関わらず、開なら閉・閉なら開)と、
Engine が実IMEに追随していること(check_consistency と同じ、700ms後の `k` の扱い)を要求する。
使い方: check_toggle.py [--real-only] <スパイク(--hz)のlog> [<awaseのフルデバッグlog>]   終了コード: 0=全手順OK / 1=NG / 3=INVALID
  --real-only: awase を起動しない対照(実IME単体)。Engine の判定を省く(awase log は不要)。

「反転」の基準は、**その押下の直前の実IME開閉**(スパイクログの「前」行)である。スパイクは手順の間に、Engine 追随の確認用の
`k`(と、直接入力のとき IME を開く 半角/全角0xF4 + ESC の準備押下)を挟むので、前の手順の押下後の状態と、この押下の直前の状態は
一致するとは限らない(実機でF3の直前に準備の0xF4でIMEが開かれ、F3で閉じた。前手順の後(閉)と比べると「不変」に見えて誤FAILしていた)。
「前」行が無いログでは従来どおり、前の手順の押下後の状態と比べる。
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from check_consistency import engine_after, next_press, parse_engine, parse_spike  # noqa: E402


def main():
    args = [a for a in sys.argv[1:] if a != "--real-only"]
    real_only = len(args) != len(sys.argv) - 1
    if len(args) != (1 if real_only else 2):
        print(__doc__)
        return 2
    steps, invalid = parse_spike(args[0])
    probes, _ = parse_engine(args[1]) if not real_only else ([], 0)
    used = set()  # 1つの k を複数の手順に数えない
    if invalid:
        print(f"INVALID: 実行中にフォーカスが外れた({invalid}回)。この回は判定に使わない")
        return 3
    if not steps:
        print("FAIL: 手順の記録が1件もない(--hz が動かなかった)")
        return 1
    fails = 0
    prev_open = 0  # スパイクは開始時に VK_IME_OFF を注入し、直接入力から始める
    print(f"{'STEP':>4} {'押下':<10} {'実IME(+1500ms)':<18} {'反転':<6} {'Engine':<7} 判定")
    for st in steps:
        if "open" not in st:
            print(f"{st['n']:>4} {st['name']:<10} 記録なし → FAIL")
            fails += 1
            continue
        flipped = st["open"] != st.get("before_open", prev_open)
        want_engine = bool(st["open"]) and bool(st["conv"] & 1)
        got = want_engine if real_only else engine_after(probes, st["press"], used, next_press(steps, st))
        undecided = got is None and st["n"] == st["total"]  # 最終手順は k が取れないことがある(判定不能、失敗にはしない)
        eng_ok = got == want_engine or undecided
        ok = flipped and eng_ok
        fails += not ok
        real = f"open={st['open']} conv=0x{st['conv']:02X}"
        got_s = "?" if got is None else ("ON" if got else "OFF")
        why = [] if ok else (["開閉が反転していない"] if not flipped else []) + (["Engineが実IMEに追随していない"] if not eng_ok else [])
        print(f"{st['n']:>4} {st['name']:<10} {real:<18} {'○' if flipped else '×':<6} {got_s:<7} {('判定不能(最終手順)' if ok and undecided else 'PASS') if ok else 'FAIL: ' + ' / '.join(why)}")
        prev_open = st["open"]
    if len(steps) < steps[0]["total"]:
        print(f"FAIL: 記録された手順が {len(steps)}/{steps[0]['total']} 件しかない")
        fails += 1
    print("結果:", "ALL PASS" if fails == 0 else f"{fails} 件 FAIL")
    return 0 if fails == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
