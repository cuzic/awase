#!/usr/bin/env python3
"""リセット操作(Ctrl+無変換→Ctrl+変換 / Ctrl+変換→Ctrl+無変換)の合否判定(ADR-187、`--resync`)。

`resync(ON)` の押下後は「実IME ON かつ かな(conv の NATIVE ビット) かつ Engine ON」、`resync(OFF)` の押下後は
「実IME OFF かつ Engine OFF」になっていることを要求する(Engine は check_consistency と同じく、700ms後の `k` の扱いで読む)。
それ以外の手順(無変換/変換/ひらがな)は、ずれが起きたか(実IMEとEngineの不一致)を「ずれ N 件」として数えるだけで、判定には使わない。
使い方: check_resync.py <スパイク(--resync)のlog> <awaseのフルデバッグlog>   終了コード: 0=全リセットが成功 / 1=失敗あり / 3=INVALID
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from check_consistency import engine_after, parse_engine, parse_spike  # noqa: E402


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    steps, invalid = parse_spike(sys.argv[1])
    probes, _ = parse_engine(sys.argv[2])
    if invalid:
        print(f"INVALID: 実行中にフォーカスが外れた({invalid}回)。この回は判定に使わない")
        return 3
    if not steps:
        print("FAIL: 手順の記録が1件もない(--resync が動かなかった)")
        return 1
    fails = 0
    drift = 0
    resync_seen = 0
    print(f"{'STEP':>4} {'押下':<14} {'実IME(+1500ms)':<18} {'Engine':<7} 判定")
    for st in steps:
        if "open" not in st:
            print(f"{st['n']:>4} {st['name']:<14} 記録なし")
            continue
        got = engine_after(probes, st["press"])
        real = f"open={st['open']} conv=0x{st['conv']:02X}"
        got_s = "?" if got is None else ("ON" if got else "OFF")
        is_last = st["n"] == st["total"]
        if st["name"].startswith("resync("):
            resync_seen += 1
            want_on = st["name"] == "resync(ON)"
            real_ok = bool(st["open"]) and bool(st["conv"] & 1) if want_on else not st["open"]
            eng_ok = got == want_on
            ok = real_ok and eng_ok
            fails += not ok
            note = "PASS" if ok else f"FAIL: 期待 実IME{'ON(かな)' if want_on else 'OFF'} かつ Engine {'ON' if want_on else 'OFF'}"
        else:
            want = bool(st["open"]) and bool(st["conv"] & 1)
            mismatch = got is not None and got != want and not is_last
            drift += mismatch
            note = "ずれ(実IMEとEngineが不一致)" if mismatch else "-"
        print(f"{st['n']:>4} {st['name']:<14} {real:<18} {got_s:<7} {note}")
    if resync_seen < 4:
        print(f"FAIL: リセット手順が {resync_seen}/4 件しか記録されていない(1打目に Ctrl が付かない等で手順に対応づけられなかった)")
        fails += 1
    print(f"ずれ: {drift} 件(リセットの前提として起きたか。followがある構成では0でもよい)")
    print("結果:", "ALL PASS(リセット成功)" if fails == 0 else f"{fails} 件 FAIL(リセット失敗)")
    return 0 if fails == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
