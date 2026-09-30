#!/usr/bin/env python3
"""`awase-keymap-learn-win --trace-features` の標準エラーログ(`[feat]` 行)を分析する。

隠れ状態の候補(直前キー・変換中・入力中の文字数・EDITの文字数など)を、学習の状態に
加える前に、実データで「結果の割れを説明するか」を調べる(ADR-210)。

使い方:
    python3 tools/keymap-learn/analyze_features.py learn-stderr.log [...]

出力:
1. 結果が割れるセル((押下前status, キー)で結果が複数)の数と割合。
2. 特徴量ごとの独立評価: **その run の学習中の押下だけ**で(status,キー[,特徴量])→結果の多数派を
   作り、同じ run の検証ウォーク・再測定の押下で採点する(正答率と、予測できた割合)。

注意(ADR-210 のレビュー B1): 学習と検証ウォークの押下を混ぜた leave-one-out で特徴量を選ぶと、
効果を大きく見積もる(文脈キーを足すと実際の採否基準の信頼度が 0.69〜0.82 に落ちた)。
本スクリプトの (2) が実際の採否に近い条件。特徴量の選択に検証ウォークの押下を使わないこと。
訓練と検証の境界は、学習終了時に出る `timing ... stage=training` の行で切る。
"""
import collections
import re
import sys

FEAT = re.compile(
    r"\[feat\] n=(\d+) key=(0x[0-9A-F]+) delivered=(\d) contaminated=(\d) cleared=(\d) "
    r"clear_changed=(\d) (?:late_notify=\d+ )?before_status=(Status \{[^}]*\}) after_status=(Status \{[^}]*\}) "
    r"disp=(\w+) B\[(.*?)\] A\[(.*?)\]"
)


def kv(text):
    return dict(re.findall(r"(\w+)=(\S+)", text))


def status(text):
    m = re.search(r"open: (\w+), mode: (\d+), composing: (\w+)", text)
    return m.groups()


def parse(path):
    """(訓練の行, 検証以降の行) を返す。汚染された押下・未送達の押下・矛盾した観測は除く。"""
    train, walk, seen_training_end = [], [], False
    with open(path, encoding="utf8", errors="replace") as f:
        for line in f:
            if "stage=training" in line:
                seen_training_end = True
                continue
            m = FEAT.match(line)
            if not m:
                continue
            n, key, delivered, contaminated, cleared, _cc, bs, as_, disp, b, _a = m.groups()
            if delivered != "1" or contaminated == "1":
                continue
            before, after = status(bs), status(as_)
            if (after[0] == "false" and after[2] == "true") or (
                before[0] == "false" and before[2] == "true"
            ):
                continue  # 閉と報告されているのに入力中: IMMの矛盾した観測
            row = dict(key=key, before=before, out=after + (disp,), feat=kv(b))
            (walk if seen_training_end else train).append(row)
    return train, walk


def with_prev(rows):
    prev = None
    for r in rows:
        r["prev1"] = prev
        prev = r["key"]


FEATURES = {
    "(なし)": lambda r: None,
    "直前キー": lambda r: r["prev1"],
    "変換中(属性)": lambda r: (int(r["feat"].get("attr_mask", "0x0"), 16) & ~0b10001) != 0
    or int(r["feat"].get("cand", "0")) > 0,
    "入力中の文字数(0/1/2+)": lambda r: min(int(r["feat"].get("comp_len", "0")), 2),
    "EDITの文字あり": lambda r: int(r["feat"].get("text_len", "0")) > 0,
}


def evaluate(train, walk, fn):
    table = collections.defaultdict(collections.Counter)
    for r in train:
        table[(r["before"], r["key"], fn(r))][r["out"]] += 1
    correct = predicted = 0
    for r in walk:
        c = table.get((r["before"], r["key"], fn(r)))
        if not c:
            continue
        predicted += 1
        correct += c.most_common(1)[0][0] == r["out"]
    return correct, predicted


def main(paths):
    for path in paths:
        train, walk = parse(path)
        for rows in (train, walk):
            with_prev(rows)
        cells = collections.defaultdict(set)
        for r in train + walk:
            cells[(r["before"], r["key"])].add(r["out"])
        split = sum(1 for v in cells.values() if len(v) > 1)
        print(f"== {path}")
        print(f"  訓練 {len(train)} 押下 / 検証以降 {len(walk)} 押下、結果が割れるセル {split}/{len(cells)}")
        if not walk:
            print("  (訓練の終わりの目印 `stage=training` が無い、または検証の押下が無い)")
            continue
        for name, fn in FEATURES.items():
            c, p = evaluate(train, walk, fn)
            acc = c / p if p else float("nan")
            print(f"  {name:22s} 正答率={acc:.3f} 予測できた割合={p / len(walk):.3f} ({c}/{p} of {len(walk)})")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    main(sys.argv[1:])
