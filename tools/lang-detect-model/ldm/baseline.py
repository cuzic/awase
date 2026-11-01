"""今の判定器(`src/lang_detect.rs` の `KanaModel::with_models(TrigramInterp, LatinBackground::embedded())`)の Python 移植。

比較の基準にするためのもの。Rust 側の実測値と一致することを `tests/test_baseline.py` で固定する
(2026-11-01 の誤発火 `ccndie` の各打鍵の増分: -4.95, -2.77, +4.57, -0.90, -6.82, -3.73。CI のプローブ #922 の出力)。
"""
from __future__ import annotations

import gzip
import math
from collections import defaultdict

from .common import GRID, ROOT, UNSHIFTED

TRIGRAM_WEIGHT = 0.7
KEY_KANA = {key: ch for ch, key in UNSHIFTED.items() if "ぁ" <= ch <= "ん"}
# `key_count`: 未シフト面で定義済みのキー数(仮名・記号。無 = 未定義を除く)。yab の 無 を除いた数。
_FACE_DEFINED = 44


def _load_ngram():
    bi, tri = {}, {}
    with gzip.open(ROOT / "data" / "ngram_hiragana.csv.gz", "rt", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            kind, chars, score = line.split(",")
            if kind == "2" and len(chars) == 2:
                bi[(chars[0], chars[1])] = 10 ** float(score)
            elif kind == "3" and len(chars) == 3:
                tri[(chars[0], chars[1], chars[2])] = 10 ** float(score)
    return bi, tri


def _load_latin():
    text = (ROOT / "data" / "latin_bigram.txt").read_text(encoding="utf-8")
    lines = [ln for ln in text.splitlines() if ln and not ln.startswith("#")]
    chars = lines[0].removeprefix("chars=")
    n = len(chars)

    def row(line):
        return [-int(line[i * 2 : i * 2 + 2], 16) / 16.0 for i in range(n)]

    uni = row(lines[1].removeprefix("uni="))
    bi = [row(lines[2 + i]) for i in range(n)]
    return {c: i for i, c in enumerate(chars)}, uni, bi


class Baseline:
    def __init__(self) -> None:
        bi, tri = _load_ngram()
        self.freq = bi
        self.left: dict[str, float] = defaultdict(float)
        self.listed: dict[str, int] = defaultdict(int)
        self.vocab: set[str] = set()
        for (a, b), f in bi.items():
            self.left[a] += f
            self.listed[a] += 1
            self.vocab |= {a, b}
        self.total = sum(self.left.values())
        self.floor = min(bi.values()) * 0.1
        self.tri = tri
        self.prefix: dict[tuple[str, str], list[float]] = defaultdict(lambda: [0.0, 0])
        for (a, b, _c), f in tri.items():
            p = self.prefix[(a, b)]
            p[0] += f
            p[1] += 1
        self.floor3 = min(tri.values()) * 0.1
        self.lidx, self.luni, self.lbi = _load_latin()

    def _ln_ja2(self, prev, token):
        v = len(self.vocab)
        if prev is not None and prev in self.vocab:
            den = (v - self.listed.get(prev, 0)) * self.floor + self.left.get(prev, 0.0)
            num = self.freq.get((prev, token), self.floor)
            return math.log(num / den)
        left = self.left.get(token, 0.0)
        return math.log((left + self.floor) / (v * self.floor + self.total))

    def _interp(self, a, b, token, ln_p2):
        pre = self.prefix.get((a, b))
        if pre is None:
            return ln_p2
        den = (len(self.vocab) - pre[1]) * self.floor3 + pre[0]
        q3 = self.tri.get((a, b, token), self.floor3) / den
        return math.log(TRIGRAM_WEIGHT * q3 + (1 - TRIGRAM_WEIGHT) * math.exp(ln_p2))

    def increments(self, keys: str) -> list[float]:
        """各打鍵の対数尤度比の増分(`KanaModel::step`)。"""
        out, prev, prev2, prev_key = [], None, None, None
        for k in keys:
            token = KEY_KANA.get(k)
            if token is None or token not in self.vocab:
                out.append(0.0)
                prev = prev2 = prev_key = None
                continue
            i = self.lidx.get(k)
            if i is None:
                out.append(0.0)
                prev = prev2 = prev_key = None
                continue
            bg = self.luni[i] if prev_key is None else self.lbi[prev_key][i]
            ja = self._ln_ja2(prev, token)
            if prev2 is not None and prev is not None:
                ja = self._interp(prev2, prev, token, ja)
            out.append(ja - bg)
            prev2, prev, prev_key = prev, token, i
        return out
