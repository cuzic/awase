#!/usr/bin/env python3
"""--grid の学習ラウンドのログ(ime_key_matrix_spike.log)を集計する(ADR-191)。

  grid_learn.py <spike.log> [<spike.log> ...]   セル(状態×キー)ごとの結果の分布・決定性・入力中の行方・セットアップ不能を出す
  grid_learn.py --json out.json <spike.log>...   表(セル → 結果の分布)をJSONに書く
  grid_learn.py --graph <spike.log>...           --grid-setup=keys の探索(遷移グラフ): [GRID-SETUP](到達経路/到達不能)と [GRID-EDGE](辺)を出す
  grid_learn.py --diff <keys.json> <imm.json>    keys版とimm版の表のセルごとの差分(どのセルで結果が違うか)

セル = (状態, キー)。状態 = 開閉-変換モード-入力中の段階(例 on-c19-conv-henkan)。
結果 = (押下後の開閉, 変換モード, 入力中(空/未確定あり/確定して空), 観測時点)。既定の観測時点は +1500ms(+400ms は --at=400)。
入力中の行方(押下前が入力中/変換中のセルのみ): 確定=comp が空になり入力欄の末尾が変わった / 破棄=comp が空で入力欄は空のまま / 保持=comp が残った。
"""
import json
import re
import sys
from collections import Counter, defaultdict

sys.path.insert(0, __file__.rsplit("/", 1)[0] if "/" in __file__ else ".")
import effect_learning as e  # noqa: E402  (parse_snap を再利用)

BEGIN = re.compile(r"^\[GRID-BEGIN\] id=(\S+) shard=(\S+) n=(\d+)/(\d+) state=(\S+) key=(\S+) trial=(\d+)")
PRE = re.compile(r"^\[GRID-PRE\] id=(\S+) check=(\d) attempt=(\d) ok=(true|false)")
SKIP = re.compile(r"^\[GRID-SKIP\] id=(\S+)")
KEYTAG = re.compile(r"^\[[\d:.]+Z\] KEY \[GRID id=(\S+) shard=(\S+) state=(\S+) key=(\S+) trial=(\d+)\]")


def comp_kind(s):
    if s is None or s[2] is None:
        return "?"
    return "入力中" if s[2] else "空"


def snap_tuple(line):
    """(open, conv(native bit込みの生値でなく状態タプル), comp有無, comp文字列, tail) を返す。"""
    m = e.SNAP_RE.search(line)
    if not m:
        return None
    st = e.parse_snap(line)
    if st is None:
        return None  # A/B 不一致・読み取り失敗=観測不能。「結果」として表に入れない(偽の非決定セルの原因になる)
    tail = re.search(r'tail=("(?:[^"\\]|\\.)*")', line)
    comp = re.search(r'comp=("(?:[^"\\]|\\.)*"|\?)', line)
    conv_b = re.search(r"B\(open=\S+ conv=(\S+)\)", line)
    conv_a = re.search(r"A\(open=\S+ conv=(\S+)\)", line)
    raw_conv = None
    for mm in (conv_b, conv_a):
        if mm and mm.group(1) != "?":
            raw_conv = mm.group(1)
            break
    return dict(
        open=st[0],
        conv=raw_conv,
        composing=st[2],
        comp=comp.group(1) if comp else "?",
        tail=tail.group(1) if tail else '""',
    )


def parse(path):
    trials = {}
    order = []
    alias = {}  # ログ上の試行id → trials のキー(同じidが再び BEGIN したら別の試行として扱う)
    dups = 0
    cur = None
    for line in open(path, encoding="utf-8", errors="replace").read().splitlines():
        m = BEGIN.match(line)
        if m:
            i = m.group(1)
            if i in trials:
                dups += 1
                key = f"{i}#{dups}"
            else:
                key = i
            alias[i] = key
            i = key
            trials[i] = dict(id=i, shard=m.group(2), state=m.group(5), key=m.group(6), trial=int(m.group(7)),
                             pre_checks=[], skipped=False, before=None, after={})
            order.append(i)
            continue
        m = PRE.match(line)
        if m and alias.get(m.group(1)) in trials:
            trials[alias[m.group(1)]]["pre_checks"].append((int(m.group(2)), int(m.group(3)), m.group(4) == "true"))
            continue
        m = SKIP.match(line)
        if m and alias.get(m.group(1)) in trials:
            trials[alias[m.group(1)]]["skipped"] = True
            continue
        m = KEYTAG.match(line)
        if m and alias.get(m.group(1)) in trials:
            cur = trials[alias[m.group(1)]]
            continue
        if line.startswith("["):
            # 別のKEY行や[GRID-..]行=この押下のスナップショットブロックの終わり。--fast のログには +1500ms 行が無く、
            # 次の準備操作のブロックを、この試行の観測として取り込んでしまうため。
            cur = None
            continue
        s = line.strip()
        if cur is not None:
            if s.startswith("前"):
                cur["before"] = snap_tuple(s)
            elif s.startswith("+100ms"):
                cur["after"][100] = snap_tuple(s)
            elif s.startswith("+400ms"):
                cur["after"][400] = snap_tuple(s)
            elif s.startswith("+1500ms"):
                cur["after"][1500] = snap_tuple(s)
                cur = None
    if dups:
        print(f"警告: {path}: 試行idの重複が {dups} 件(複数ランが追記されたログ?)。別の試行として数えた", file=sys.stderr)
    return [trials[i] for i in order]


def log_flags(path):
    """ログが打ち切られていないか([GRID-ABORT])と、除外された試行数([GRID-PRUNE])を返す。"""
    abort, prune = [], []
    for line in open(path, encoding="utf-8", errors="replace"):
        if "[GRID-ABORT]" in line:
            abort.append(line.strip())
        elif "[GRID-PRUNE]" in line:
            prune.append(line.strip())
    return abort, prune


def outcome(t, at):
    a = t["after"].get(at)
    b = t["before"]
    if not a or not b:
        return None
    o = "ON" if a["open"] else "OFF" if a["open"] is not None else "?"
    fate = ""
    if b["composing"]:
        if a["composing"]:
            fate = "/保持"
        elif a["tail"] != b["tail"] and a["tail"] != '""':
            fate = "/確定"
        else:
            fate = "/破棄"
    else:
        fate = "/入力中" if a["composing"] else ""
    return f"{o}/{a['conv']}{fate}"


def graph(paths):
    setup, unreach, edges = {}, [], {}
    for p in paths:
        for line in open(p, encoding="utf-8", errors="replace"):
            m = re.search(r"\[GRID-SETUP\] state=(\S+) path=(\S*)", line)
            if m:
                setup.setdefault(m.group(1), m.group(2))
                continue
            m = re.search(r"\[GRID-SETUP\] state=(\S+) 到達不能", line)
            if m and m.group(1) not in unreach:
                unreach.append(m.group(1))
                continue
            m = re.search(r"\[GRID-EDGE\] from=(\S+) key=(\S+) to=(\S+)", line)
            if m:
                edges.setdefault((m.group(1), m.group(2)), Counter())[m.group(3)] += 1
    print("到達できた状態(リセット=IMMで(開,0x19) からの経路):")
    for st, path in sorted(setup.items()):
        print(f"  {st:12} path={path or '(リセットのまま)'}")
    print("到達不能:", ", ".join(u for u in unreach if u not in setup) or "なし")
    print("遷移(観測した辺):")
    for (frm, key), c in sorted(edges.items()):
        print(f"  {frm:10} --{key}--> " + " / ".join(f"{t}×{n}" for t, n in c.most_common()))


def diff(keys_json, imm_json):
    a = json.load(open(keys_json, encoding="utf-8"))
    b = json.load(open(imm_json, encoding="utf-8"))
    def top(c):
        return max(c.items(), key=lambda x: x[1])[0]
    only_k = [k for k in a if k not in b]
    only_i = [k for k in b if k not in a]
    both = [k for k in a if k in b]
    diffs = [(k, top(a[k]), top(b[k])) for k in both if top(a[k]) != top(b[k])]
    print(f"共通セル {len(both)}、結果が違う {len(diffs)}、keys版のみ {len(only_k)}、imm版のみ {len(only_i)}")
    if not both:
        print("警告: 共通セルが0件なので比較できていない(「差分0」ではない)。観測時点(--at)や入力ログを確認すること", file=sys.stderr)
        sys.exit(2)
    for k, x, y in sorted(diffs, key=lambda d: (d[0].split('|')[1], d[0])):
        print(f"  {k}: keys={x} / imm={y}")


def main(argv):
    if argv and argv[0] == "--graph":
        graph(argv[1:])
        return
    if argv and argv[0] == "--diff":
        diff(argv[1], argv[2])
        return
    at = None  # 既定: ログにある観測時点のうち最も遅いもの(--fast は +400ms、--snap100 は +100ms、通常は +1500ms)
    out_json = None
    paths = []
    it = iter(argv)
    for a in it:
        if a == "--json":
            out_json = next(it)
        elif a.startswith("--at="):
            at = int(a[5:])
        else:
            paths.append(a)
    rows = [t for p in paths for t in parse(p)]
    for p in paths:
        abort, prune = log_flags(p)
        if abort:
            print(f"警告: {p}: [GRID-ABORT] で打ち切られたログ(表は途中までの少数セル): {abort[0]}", file=sys.stderr)
        if prune:
            print(f"注: {p}: {prune[0]}", file=sys.stderr)
    avail = sorted({k for t in rows for k in t["after"] if t["after"][k]})
    if at is None:
        at = avail[-1] if avail else 1500
        if avail:
            print(f"(観測時点 +{at}ms を使う。ログにある時点: {avail})")
    if not avail or at not in avail:
        print(f"エラー: 観測時点 +{at}ms の観測が0件(ログにある時点: {avail or 'なし'})。--fast のログは --at=400、--snap100 は --at=100", file=sys.stderr)
        sys.exit(2)
    setup_ok = [t for t in rows if not t["skipped"] and t["after"].get(at)]
    skipped = [t for t in rows if t["skipped"]]
    print(f"試行 {len(rows)} 件: 記録あり {len(setup_ok)}、セットアップ不能 {len(skipped)}、押下の観測なし {len(rows) - len(setup_ok) - len(skipped)}")
    cells = defaultdict(Counter)
    for t in setup_ok:
        r = outcome(t, at)
        if r:
            cells[(t["state"], t["key"])][r] += 1
    tot = det = 0
    nondet = []
    for k, c in cells.items():
        n = sum(c.values())
        tot += n
        det += c.most_common(1)[0][1]
        if c.most_common(1)[0][1] < n:
            nondet.append((k, c))
    print(f"セル {len(cells)}、決定性(多数派一致) {det}/{tot}={det / max(tot, 1):.1%}、非決定セル {len(nondet)}  (観測時点 +{at}ms)")
    for (st, key), c in sorted(nondet):
        print(f"  非決定: {st} + {key}: " + " / ".join(f"{k}×{v}" for k, v in c.most_common()))
    sk = Counter((t["state"]) for t in skipped)
    if sk:
        print("セットアップ不能の状態(データ):")
        for st, n in sk.most_common():
            print(f"  {st}: {n}件")
    if out_json:
        json.dump({f"{s}|{k}": dict(c) for (s, k), c in sorted(cells.items())}, open(out_json, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
        print(f"表を {out_json} に書いた")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(__doc__)
    else:
        main(sys.argv[1:])
