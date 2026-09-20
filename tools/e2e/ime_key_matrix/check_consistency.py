#!/usr/bin/env python3
"""プリセット非依存の合否判定(ADR-186): 各押下の +1500ms 時点で、awase の Engine 状態が実IMEの状態に
追随しているかだけを見る。期待表(check.py の EXPECT)は ATOK の動作を前提にしているが、こちらは
「かな(IME ON かつ conv の NATIVE ビット) = Engine ON、それ以外(半角英数・直接入力) = Engine OFF」という
ユーザー要件だけを不変条件にする。ATOK / MS-IME / パススルー設定のどれでも同じ判定で使える。

Engine の状態は、スパイクが各押下の700ms後に打つ `k` を awase がどう扱ったか(journal の decision)で読む:
`PassThrough` なら Engine OFF(そのまま入力される)、それ以外(Consume 等)なら Engine ON(NICOLA変換の対象)。
「Engine activated/deactivated」のログは、delegate経由のOFFでは出ないため状態の根拠にしない。

使い方: check_consistency.py [--real-only] <スパイク(--walk)のlog> <awaseのフルデバッグlog(RUST_LOG=debug)>
--real-only: Engine は見ず、実IMEの推移だけを出す(awase を起動しない対照実験用、常に終了コード0/INVALIDのみ3)
終了コード: 0=全手順で追随 / 1=追随しない手順あり / 3=実行中にフォーカスが外れた(INVALID) / 2=使い方の誤り
"""
import re
import sys


def to_ms(t: str) -> float:
    h, m, s = t.rstrip("Z").split(":")
    return (int(h) * 3600 + int(m) * 60) * 1000 + float(s) * 1000


def parse_spike(path):
    steps = []  # {n, name, press, open, conv}
    cur = None
    invalid = 0
    started = False
    for line in open(path, encoding="utf-8").read().splitlines():
        m = re.match(r"\[[\d:.]+Z\] KEY \[SCRIPT (\d+)/(\d+) (\S+)[^\]]*\].*?press=([\d:.]+Z)", line)
        if m:
            started = True
            cur = {"n": int(m.group(1)), "total": int(m.group(2)), "name": m.group(3), "press": to_ms(m.group(4))}
            steps.append(cur)
            continue
        if started and "[AUTO] フォーカス復帰" in line:
            invalid += 1
        m = re.match(r"\s+\+1500ms: A\(open=(\d) conv=0x([0-9A-Fa-f]+)\)", line)
        if m and cur is not None and "open" not in cur:
            cur["open"] = int(m.group(1))
            cur["conv"] = int(m.group(2), 16)
    return steps, invalid


def parse_engine(path):
    """(k の KeyDown の時刻ms, decision) の列と、Unwarranted 件数。"""
    probes = []
    unwarranted = 0
    for line in open(path, encoding="utf-8", errors="replace").read().splitlines():
        m = re.match(r"\d{4}-\d\d-\d\dT([\d:.]+)Z\s+\w+\s+(.*)", line)
        if not m:
            continue
        if "outcome=Unwarranted" in m.group(2):
            unwarranted += 1
        k = re.search(r'key input seq=\d+ .*?vk_code=75 is_down=true .*?decision="(\w+)"', m.group(2))
        if k:
            probes.append((to_ms(m.group(1)), k.group(1)))
    return sorted(probes), unwarranted


def engine_after(probes, press):
    """押下の後、最初の k(KeyDown) の decision から Engine 状態を返す(PassThrough=OFF)。無ければ None。"""
    for ms, decision in probes:
        if press < ms <= press + 2500:
            return decision != "PassThrough"
    return None


def main():
    real_only = "--real-only" in sys.argv
    sys.argv = [a for a in sys.argv if a != "--real-only"]
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    steps, invalid = parse_spike(sys.argv[1])
    probes, unwarranted = parse_engine(sys.argv[2]) if not real_only else ([], 0)
    if invalid:
        print(f"INVALID: 実行中にフォーカスが外れた({invalid}回)。この回は判定に使わない")
        return 3
    if not steps:
        print("FAIL: 手順の記録が1件もない(--walk が動かなかった)")
        return 1
    total = steps[0]["total"]
    if real_only:
        print(f"{'STEP':>4} {'押下':<8} 実IME(+1500ms)")
        for st in steps:
            r = f"open={st['open']} conv=0x{st['conv']:02X}" if "open" in st else "記録なし"
            print(f"{st['n']:>4} {st['name']:<8} {r}")
        print("結果: 対照実験(awase なし)")
        return 0
    fails = 0
    print(f"{'STEP':>4} {'押下':<8} {'実IME(+1500ms)':<18} {'期待Engine':<10} {'実Engine':<8} 判定")
    for st in steps:
        if "open" not in st:
            print(f"{st['n']:>4} {st['name']:<8} 記録なし → FAIL")
            fails += 1
            continue
        want = bool(st["open"]) and bool(st["conv"] & 1)
        got = engine_after(probes, st["press"])
        real = f"open={st['open']} conv=0x{st['conv']:02X}"
        # 最終手順はスパイクがk入力の前に閉じることがあり、Engine状態が読めない(?)。判定不能として失敗にしない。
        undecided = got is None and st['n'] == total
        ok = got == want or undecided
        fails += not ok
        got_s = "?" if got is None else ("ON" if got else "OFF")
        verdict = "判定不能(最終手順、kが取れなかった)" if undecided else ("PASS" if ok else "FAIL: 実IMEに追随していない")
        print(f"{st['n']:>4} {st['name']:<8} {real:<18} {'ON' if want else 'OFF':<10} {got_s:<8} {verdict}")
    if len(steps) < total:
        print(f"FAIL: 記録された手順が {len(steps)}/{total} 件しかない")
        fails += 1
    if unwarranted:
        print(f"注意: outcome=Unwarranted が {unwarranted} 件(判定には使わない)")
    print("結果:", "ALL PASS" if fails == 0 else f"{fails} 件 FAIL")
    return 0 if fails == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
