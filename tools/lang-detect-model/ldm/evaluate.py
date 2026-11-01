"""検出器の規則(累積の対数尤度比が閾値に届いたら確定)での評価。

モデルは各打鍵の増分 `inc[N, RUN_CAP]`(詰めた所は 0)を返す。確定は『累積が -en_th 以下になった最初の打鍵』で NotJapanese。
指標は固定の誤判定率での検出率で比べる(尺度の違うモデルを同じ軸に載せるため):
- 誤判定率 FPR = 日本語のエピソードのうち NotJapanese に確定した割合(元別)
- 検出率 TPR = 非日本語のエピソード(ラベル別)のうち NotJapanese に確定した割合、と平均の打鍵数
"""
from __future__ import annotations

import numpy as np

from .data import LABELS, SOURCES, Episodes


def first_hit(inc: np.ndarray, length: np.ndarray, th: float) -> tuple[np.ndarray, np.ndarray]:
    """累積が -th 以下になった最初の打鍵(1 始まり)と、確定したか。"""
    cum = np.cumsum(inc, axis=1)
    mask = np.arange(inc.shape[1])[None, :] < length[:, None]
    hit = (cum <= -th) & mask
    decided = hit.any(axis=1)
    presses = np.where(decided, hit.argmax(axis=1) + 1, length)
    return decided, presses


def sweep(ep: Episodes, inc: np.ndarray, ths: np.ndarray) -> dict[float, dict]:
    out = {}
    for th in ths:
        decided, presses = first_hit(inc, ep.length, float(th))
        row = {}
        for si, src in enumerate(SOURCES):
            m = (ep.label == 0) & (ep.source == si)
            if m.any():
                row[f"fpr_ja_{src}"] = float(decided[m].mean())
        row["fpr_ja_all"] = float(decided[ep.label == 0].mean())
        for li, lab in enumerate(LABELS[1:], start=1):
            m = ep.label == li
            row[f"tpr_{lab}"] = float(decided[m].mean())
            row[f"press_{lab}"] = float(presses[m & decided].mean()) if (m & decided).any() else float("nan")
        out[float(th)] = row
    return out


def tpr_at_fpr(table: dict[float, dict], target: float, key: str = "fpr_ja_all") -> dict:
    """FPR が target 以下になる最小の閾値の行(閾値が大きいほど FPR は小さい)。"""
    ok = [(th, r) for th, r in sorted(table.items()) if r[key] <= target]
    if not ok:
        return {}
    th, row = ok[0]
    return {"threshold": th, **row}
