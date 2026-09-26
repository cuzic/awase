#!/usr/bin/env python3
"""文字→親指(押し続ける)のタイムアウト後の IME 開閉の合否判定(ADR-199 T10 決定A、スパイクの `--charthumb`)。

各ラウンドで、文字を親指より先に離して重なり不足にしたまま親指を押し続ける(タイムアウトを越える)。判定は親指の KEY 行の
実IME(A: ImmGetOpenStatus)で行う:
  前      open=1 であること(F2 で IME が ON になっていない回は INVALID)
  +400ms  open=1 であること(親指を押している間に awase が IME を閉じない。閉じていれば FAIL)
  +1500ms open=0 であること(親指を離した後に forced の開閉(`keys.ime_off`)が発火する。閉じなければ FAIL)
使い方: check_charthumb.py [--thumb-vk=1D] <スパイク(--charthumb)のlog>   終了コード: 0=全ラウンドOK / 1=NG / 3=INVALID
"""
import re
import sys


def parse(path, thumb_vk):
    rounds = []
    cur = None
    invalid = 0
    started = False
    for line in open(path, encoding="utf-8").read().splitlines():
        m = re.match(r"\[[\d:.]+Z\] KEY \[[^\]]*\] .*vk=0x([0-9A-Fa-f]+) ", line)
        if m:
            started = True
            cur = {} if int(m.group(1), 16) == thumb_vk else None
            if cur is not None:
                rounds.append(cur)
            continue
        if started and "[AUTO] フォーカス復帰" in line:
            invalid += 1
        if cur is None:
            continue
        m = re.match(r"\s+前\s*: A\(open=(\d) ", line)
        if m:
            cur["before"] = int(m.group(1))
        m = re.match(r"\s+\+(400|1500)ms: A\(open=(\d) ", line)
        if m:
            cur[m.group(1)] = int(m.group(2))
    return rounds, invalid


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--thumb-vk=")]
    thumb_vk = next((int(a.split("=", 1)[1], 16) for a in sys.argv[1:] if a.startswith("--thumb-vk=")), 0x1D)
    if len(args) != 1:
        print(__doc__)
        return 2
    rounds, invalid = parse(args[0], thumb_vk)
    if invalid:
        print(f"INVALID: 実行中にフォーカスが外れた({invalid}回)。この回は判定に使わない")
        return 3
    complete = [r for r in rounds if {"before", "400", "1500"} <= r.keys()]
    if not complete:
        print("INVALID: 親指の KEY 行(前/+400ms/+1500ms)が1件も無い(--charthumb が動かなかった)")
        return 3
    fails = 0
    valid = 0
    print(f"{'ROUND':>5} {'前':<4} {'+400ms(保持中)':<16} {'+1500ms(離した後)':<18} 判定")
    for i, r in enumerate(complete, 1):
        if r["before"] != 1:
            print(f"{i:>5} {r['before']:<4} {r['400']:<16} {r['1500']:<18} INVALID(IME が ON でない)")
            continue
        valid += 1
        why = []
        if r["400"] != 1:
            why.append("親指を押している間に IME が閉じた")
        if r["1500"] != 0:
            why.append("親指を離した後に IME が閉じていない")
        fails += bool(why)
        print(f"{i:>5} {r['before']:<4} {r['400']:<16} {r['1500']:<18} {'PASS' if not why else 'FAIL: ' + ' / '.join(why)}")
    if valid == 0:
        print("INVALID: 全ラウンドで IME が ON でなかった")
        return 3
    print("結果:", "ALL PASS" if fails == 0 else f"{fails} 件 FAIL")
    return 0 if fails == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
