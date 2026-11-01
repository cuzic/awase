"""episodes.tsv を numpy の配列にする。"""
from __future__ import annotations

from dataclasses import dataclass

import numpy as np

from .common import CACHE, GRID, RUN_CAP

KEYS = "".join(GRID)  # 47 キー(JIS の QWERTY 格子)
KEY_INDEX = {c: i for i, c in enumerate(KEYS)}
N_KEYS = len(KEYS)
PAD = N_KEYS  # 詰め物
LABELS = ["ja", "en", "other", "code"]
SOURCES = ["wiki", "tatoeba", "repo"]
STARTS = ["sentence", "word", "mid"]


@dataclass
class Episodes:
    keys: np.ndarray  # [N, RUN_CAP] int16、PAD で詰める
    length: np.ndarray  # [N]
    label: np.ndarray  # [N] LABELS の添字
    source: np.ndarray  # [N] SOURCES の添字
    start: np.ndarray  # [N] STARTS の添字
    split: np.ndarray  # [N] 0=train 1=dev 2=test

    def select(self, mask: np.ndarray) -> "Episodes":
        return Episodes(*(getattr(self, f)[mask] for f in ("keys", "length", "label", "source", "start", "split")))

    def __len__(self) -> int:
        return len(self.length)


def load(path=None) -> Episodes:
    path = path or CACHE / "data" / "episodes.tsv"
    keys, length, label, source, start, split = [], [], [], [], [], []
    sp_idx = {"train": 0, "dev": 1, "test": 2}
    for line in open(path, encoding="utf-8"):
        sp, src, lab, st, ks = line.rstrip("\n").split("\t")
        ks = ks[:RUN_CAP]
        if not ks or any(c not in KEY_INDEX for c in ks):
            continue
        row = [KEY_INDEX[c] for c in ks] + [PAD] * (RUN_CAP - len(ks))
        keys.append(row)
        length.append(len(ks))
        label.append(LABELS.index(lab))
        source.append(SOURCES.index(src))
        start.append(STARTS.index(st))
        split.append(sp_idx[sp])
    return Episodes(
        np.array(keys, dtype=np.int16), np.array(length, dtype=np.int16), np.array(label, dtype=np.int8),
        np.array(source, dtype=np.int8), np.array(start, dtype=np.int8), np.array(split, dtype=np.int8),
    )
