#!/usr/bin/env python3
"""compartment_notify_probe(ADR-193)のログから、キー→TSF compartment 変更通知の遅延を集計する(ADR-191)。

  notify_latency.py <compartment_notify_probe.log> [<log2> ...]

出力: キーごとの(最初の通知までの遅延, 最後の通知までの遅延, POLL検出までの遅延, 通知の種類と順序)と、
全体の P50/P95/max、通知が来なかったキーの割合、複数回通知が来たキーの「最後の通知までの時間」、
POLL(--poll)が変化を検出するまでの遅延と通知との比較、OPENCLOSE と CONVERSION の順序。
"""
import re
import sys
from collections import Counter

LINE = re.compile(r"^\+\s*(\d+)ms\s+(\S+)\s+(\S+)\s*(?:=\s*(-?\d+))?")
KEYNAMES = {
    "0x16": "IME_ON", "0x1A": "IME_OFF", "0xF2": "ひらがな", "0xF1": "カタカナ", "0xF0": "英数",
    "0x1D": "無変換", "0x1C": "変換", "0xF3": "半角全角", "0x1B": "Esc", "0x4B": "k", "0x41": "a",
    "0x20": "Space", "0x0D": "Enter", "0x19": "漢字",
}


def parse(path):
    ev = []
    for line in open(path, encoding="utf-8", errors="replace"):
        m = LINE.match(line)
        if m:
            at, kind, name, val = m.groups()
            ev.append((int(at), kind, name, None if val is None else int(val)))
    return ev


def pct(xs, p):
    if not xs:
        return None
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round(p / 100 * (len(xs) - 1))))]


def per_key(ev):
    """[(key名, 押下時刻, [(delay, name, value)…NOTIFY], [(delay, name, value)…POLL])]"""
    rows = []
    idx = [i for i, e in enumerate(ev) if e[1] == "KEY"]
    for n, i in enumerate(idx):
        end = idx[n + 1] if n + 1 < len(idx) else len(ev)
        t0 = ev[i][0]
        notif = [(e[0] - t0, e[2], e[3]) for e in ev[i + 1:end] if e[1] == "NOTIFY"]
        poll = [(e[0] - t0, e[2], e[3]) for e in ev[i + 1:end] if e[1] == "POLL"]
        rows.append((ev[i][2], t0, notif, poll))
    return rows


def report(path):
    ev = parse(path)
    rows = per_key(ev)
    print(f"== {path}: キー{len(rows)}件、NOTIFY {sum(1 for e in ev if e[1]=='NOTIFY')}件、POLL {sum(1 for e in ev if e[1]=='POLL')}件")
    first, last, pfirst, plast, quiet = [], [], [], [], []
    no_notify, no_notify_but_poll = [], []
    order = Counter()
    for key, t0, notif, poll in rows:
        nm = KEYNAMES.get(key, key)
        f = notif[0][0] if notif else None
        l = notif[-1][0] if notif else None
        pf = poll[0][0] if poll else None
        pl = poll[-1][0] if poll else None
        kinds = ",".join(f"{n}@{d}" for d, n, _ in notif) or "なし"
        fs = "なし" if f is None else f"{f}ms"
        ls = "なし" if l is None else f"{l}ms"
        pfs = "なし" if pf is None else f"{pf}ms"
        pls = "なし" if pl is None else f"{pl}ms"
        print(f"  {nm:8} 通知: 最初{fs} 最後{ls} ({kinds})  POLL: 最初{pfs} 最後{pls}")
        if notif:
            first.append(f)
            last.append(l)
            if len(notif) > 1:
                quiet.append(l - f)
            names = [n for _, n, _ in notif]
            if "OPENCLOSE" in names and "CONVERSION" in names:
                order["OPENCLOSE→CONVERSION" if names.index("OPENCLOSE") < names.index("CONVERSION") else "CONVERSION→OPENCLOSE"] += 1
        else:
            no_notify.append(nm)
            if poll:
                no_notify_but_poll.append(nm)
        if poll:
            pfirst.append(pf)
            plast.append(pl)
    n = max(len(rows), 1)
    if not rows:
        print("エラー: KEY 行が0件(プローブが ABORT した/キーを1本も注入しなかった)。遅延は測れていない(「通知なし0/0=0%」ではない)", file=sys.stderr)
        sys.exit(2)
    print(f"\n[通知] 最初の通知までの遅延 ms: P50={pct(first,50)} P95={pct(first,95)} max={max(first) if first else None} (通知ありのキー{len(first)}件)")
    print(f"[通知] 最後の通知までの遅延 ms: P50={pct(last,50)} P95={pct(last,95)} max={max(last) if last else None}")
    print(f"[通知] 複数回来たキー{len(quiet)}件の「最初→最後」の幅 ms: P50={pct(quiet,50)} max={max(quiet) if quiet else None}")
    print(f"[通知] 通知が来なかったキー {len(no_notify)}/{len(rows)}={len(no_notify)/n:.0%}: {sorted(set(no_notify))}"
          f"  うちPOLLだけが変化を検出したキー: {sorted(set(no_notify_but_poll))}")
    print(f"[POLL] 変化の最初の検出 ms: P50={pct(pfirst,50)} P95={pct(pfirst,95)} max={max(pfirst) if pfirst else None}"
          f"  最後の検出 P95={pct(plast,95)}")
    print(f"[順序] {dict(order)}")
    return first, last, len(no_notify), len(rows)


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(2)
    allf, alll, nn, nk = [], [], 0, 0
    for p in sys.argv[1:]:
        f, l, a, b = report(p)
        allf += f
        alll += l
        nn += a
        nk += b
        print()
    if len(sys.argv) > 2:
        print(f"[全体] 最初の通知 P50={pct(allf,50)} P95={pct(allf,95)} P99={pct(allf,99)} max={max(allf) if allf else None}、"
              f"最後の通知 P95={pct(alll,95)} max={max(alll) if alll else None}、通知なし {nn}/{nk}")
