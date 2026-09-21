#!/usr/bin/env python3
"""格子から生成した予測表を、独立したランダムwalk(学習に使っていない、キーで到達した状態)で採点する(ADR-191 検証ラウンド)。

  score_walk.py <grid.json> <walk spike.log>...   一段予測の正答率と、不一致セルの一覧

表の引き方は予測側(key_effect_table.rs)と同じ: セル = "<開閉>-c<変換モード>-<入力中の段階>|<キー>"。段階は
入力中の文字列が空なら none、あれば typing(直前のキーが 変換/無変換 なら conv-henkan / conv-muhenkan。Spaceは注入しないので無し)。
閉状態の変換モードは読み取りが不安定なので、閉のセルは変換モード非依存(開閉だけ比較)。
正答 = 押下後(+1500ms)の 開閉 と(開なら)変換モード、入力中の有無(保持=あり/それ以外=なし)が表の多数派結果と一致。
"""
import json
import re
import sys
from collections import Counter

KEYS = {0x1D: "muhenkan", 0x1C: "henkan", 0xF2: "hiragana", 0xF3: "hankaku-zenkaku", 0xF4: "hankaku-zenkaku",
        0x1B: "esc", 0x0D: "enter", 0xF1: "katakana", 0xF0: "eisu"}
KEY_RE = re.compile(r"^\[[\d:.]+Z\] KEY .*? vk=0x([0-9A-Fa-f]+) ")
SNAP_RE = re.compile(r"A\(open=(\S+) conv=(\S+)\) B\(open=(\S+) conv=(\S+)\).*?comp=(\"[^\"]*\"|\?)")


def snap(line):
    m = SNAP_RE.search(line)
    if not m:
        return None
    ao, ac, bo, bc, comp = m.groups()
    o = {"1": True, "0": False}
    a_open, b_open = o.get(ao), o.get(bo)
    if a_open is not None and b_open is not None and a_open != b_open:
        return None
    op = a_open if a_open is not None else b_open

    def h(x):
        try:
            return int(x, 16)
        except ValueError:
            return None
    conv = h(bc) if h(bc) is not None else h(ac)
    if op is None or conv is None or comp == "?":
        return None
    return (op, conv, comp != '""')


def parse(path):
    rows, cur = [], None
    for line in open(path, encoding="utf-8", errors="replace").read().splitlines():
        m = KEY_RE.match(line)
        if m:
            cur = {"vk": int(m.group(1), 16)}
            rows.append(cur)
            continue
        if cur is None:
            continue
        s = line.strip()
        if s.startswith("前"):
            cur["b"] = snap(s)
        elif s.startswith("+1500ms"):
            cur["a"] = snap(s)
    return rows


def majority(dist):
    return max(dist.items(), key=lambda x: x[1])[0]


def score(table, paths):
    ok = ng = unseen = 0
    bad = Counter()
    for p in paths:
        prev = None
        for r in parse(p):
            vk, b, a = r["vk"], r.get("b"), r.get("a")
            pk = prev
            prev = vk
            if vk not in KEYS or not b or not a:
                continue
            key = KEYS[vk]
            op, conv, comp = b
            if not op:
                # 予測器(key_effect_table.rs)と同じ引き方: 閉状態は変換モードを問わず、段階は none だけ
                # (predict() が !open のとき stage を None に固定する)。全convでセルが一意かつ開閉の結果が一致するときだけ予測がある
                # (gen_key_effect_table.py::finalize が閉セルを畳む条件と同じ。食い違えば「予測なし」)。
                cands = [v for k, v in table.items() if k.startswith("off-") and k.endswith("-none|" + key)]
                opens = {majority(v).startswith("ON") for v in cands if len(v) == 1}
                if not cands or any(len(v) != 1 for v in cands) or len(opens) != 1:
                    unseen += 1
                    continue
                exp_open = next(iter(opens))
                good = a[0] == exp_open
            else:
                stage = "none"
                if comp:
                    stage = {0x1C: "conv-henkan", 0x1D: "conv-muhenkan"}.get(pk, "typing")
                cell = f"on-c{conv:02X}-{stage}|{key}"
                res = table.get(cell)
                if res is None or len(res) != 1:
                    unseen += 1
                    continue
                m = majority(res).split("/")
                if len(m) < 2 or m[1] in ("None", "?"):
                    unseen += 1  # 観測不能を結果として持つセル(旧版の grid_learn が作った表)は採点しない
                    continue
                good = a[0] == (m[0] == "ON") and (not a[0] or a[1] == int(m[1], 16)) and (a[2] == (len(m) > 2 and m[2] == "保持"))
                if not a[0]:
                    good = m[0] == "OFF"
            if good:
                ok += 1
            else:
                ng += 1
                bad[(f"{'on' if op else 'off'}-c{conv:02X}", "入力中" if comp else "-", KEYS[vk], majority(res),
                     f"{'ON' if a[0] else 'OFF'}/0x{a[1]:02X}{'/入力中' if a[2] else ''}")] += 1
    return ok, ng, unseen, bad


if __name__ == "__main__":
    table = json.load(open(sys.argv[1], encoding="utf-8"))
    ok, ng, unseen, bad = score(table, sys.argv[2:])
    if ok + ng + unseen == 0:
        print("エラー: 採点できた押下が0件(ログに +1500ms の観測が無い?。--fast/--snap100 のログは採点できない)", file=sys.stderr)
        sys.exit(2)
    print(f"一段予測: 一致 {ok} / 不一致 {ng} / 表に無い・非決定 {unseen}  正答率 {ok / max(ok + ng, 1):.1%}")
    for (st, comp, k, pred, act), n in bad.most_common():
        print(f"   不一致: {st} {comp} + {k}: 表={pred} 実際={act} ×{n}")
