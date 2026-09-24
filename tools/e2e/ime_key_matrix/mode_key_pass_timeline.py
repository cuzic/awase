#!/usr/bin/env python3
"""通過マーク窓(ADR-187)のタイムライン集計(レビュー文書 A-2 の実機測定)。

  mode_key_pass_timeline.py <awase.log> [<awase2.log> ...]

awase.log(RUST_LOG=debug)から、無変換/変換の生キー通過(`[mode-key-follow] mode key PassThrough`)ごとに、
窓(既定300ms)内の `IME snapshot`(成功/空振り)と `[mode-key-follow]` 判定を時系列に並べ、A-2 の前提を数える:
  - 最初の成功観測が「処理前の古い値」だったか(後の成功観測と値が違う)
  - 最初の成功の後に、窓内の再読み取りが空振り(ime_on=None)・観測ゼロだったか
  - 「A-2成立候補」= 最初の成功が古い値 かつ 窓内の残りに成功観測が無い
"""
import re
import sys
from datetime import datetime

TS = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?)Z?\s")
WINDOW_MS = 300


def ts_ms(line):
    m = TS.match(line)
    if not m:
        return None
    s = m.group(1)
    if "." in s:
        head, frac = s.split(".")
        s = head + "." + frac[:6].ljust(6, "0")
        fmt = "%Y-%m-%dT%H:%M:%S.%f"
    else:
        fmt = "%Y-%m-%dT%H:%M:%S"
    return datetime.strptime(s, fmt).timestamp() * 1000


SNAP = re.compile(r"IME snapshot: .*ime_on=(\S+) romaji=\S+ conv=(\S+)")
PASS = re.compile(r"mode key PassThrough\(vk=(0x[0-9A-Fa-f]+)\)")


def parse(path):
    presses = []
    cur = None
    for line in open(path, encoding="utf-8", errors="replace"):
        t = ts_ms(line)
        if t is None:
            continue
        m = PASS.search(line)
        if m:
            cur = {"vk": m.group(1), "t0": t, "ev": []}
            presses.append(cur)
            continue
        if cur is None:
            continue
        dt = t - cur["t0"]
        if dt > 1500:
            continue
        m = SNAP.search(line)
        if m:
            on, conv = m.groups()
            ok = on != "None"
            cur["ev"].append((dt, "snap", f"on={on} conv={conv}", ok))
        elif "IME detection timed out" in line:
            cur["ev"].append((dt, "timeout", "", False))
        elif "[mode-key-follow]" in line and "IME refresh scheduled" not in line:
            cur["ev"].append((dt, "follow", line.split("[mode-key-follow]", 1)[1].strip(), None))
    return presses


def pct(xs, p):
    if not xs:
        return None
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round(p / 100 * (len(xs) - 1))))]


def analyze(path):
    presses = parse(path)
    print(f"== {path}: 通過 {len(presses)} 件")
    first_ok, change_at, last_stale = [], [], []
    reads = ok_reads = 0
    stale_first = a2 = no_success = 0
    late_only = 0
    for p in presses:
        in_win = [e for e in p["ev"] if e[0] < WINDOW_MS]
        snaps = [e for e in in_win if e[1] == "snap"]
        oks = [e for e in snaps if e[3]]
        print(f"  {p['vk']} @{p['t0']:.0f}")
        for e in p["ev"]:
            print(f"     +{e[0]:6.1f}ms {e[1]:7s} {e[2]}")
        if not oks:
            no_success += 1
            # 窓後に成功があるか
            if any(e[1] == "snap" and e[3] for e in p["ev"] if e[0] >= WINDOW_MS):
                late_only += 1
            continue
        reads += len(snaps)
        ok_reads += len(oks)
        first_ok.append(oks[0][0])
        vals = [e[2] for e in oks]
        final_all = [e[2] for e in p["ev"] if e[1] == "snap" and e[3]][-1]
        if vals[0] != final_all:
            stale_first += 1
            change_at.append(next(e[0] for e in oks if e[2] == final_all))
            last_stale.append(max(e[0] for e in oks if e[2] != final_all))
            if len(oks) == 1:
                a2 += 1
    n = len(presses)
    print(f"  -- 窓内に成功観測なし: {no_success}/{n}(うち窓後に成功: {late_only})")
    print(f"  -- 最初の成功観測が古い値(後の値と違う): {stale_first}/{n}")
    print(f"  -- A-2成立候補(最初の成功が古い値かつ窓内の成功がその1回だけ): {a2}/{n}")
    print(f"  -- 最初の成功観測までの遅延 P50/P95/max: {pct(first_ok,50)} / {pct(first_ok,95)} / {max(first_ok) if first_ok else None}")
    print(f"  -- 最後に古い値を読んだ時刻 P50/P95/max: {pct(last_stale,50)} / {pct(last_stale,95)} / {max(last_stale) if last_stale else None}")
    print(f"  -- 窓内の読み取り成功率(成功観測を持つ押下のみ): {ok_reads}/{reads}")
    print(f"  -- 値が変わって見えた時刻 P50/P95/max: {pct(change_at,50)} / {pct(change_at,95)} / {max(change_at) if change_at else None}")


if __name__ == "__main__":
    for path in sys.argv[1:]:
        analyze(path)
