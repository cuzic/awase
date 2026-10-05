"""CI 判定の統計の小さな部品(標準ライブラリだけ)。

e2e の各構成は試行数 n が小さい(5〜10 回)ので、「n 回で 0 件 = 起きない」とは言えない。
Wilson 区間で「k/n」の真の割合がどの範囲にありうるかを併記し、再現しなかった結果の強さを数字で出す。
"""
import math


def wilson_interval(k: int, n: int, z: float = 1.96):
    """二項割合 k/n の Wilson 信頼区間(既定は 95%)。(下限, 上限) を 0..1 で返す。n=0 なら (0.0, 1.0)。"""
    if n <= 0:
        return (0.0, 1.0)
    k = max(0, min(k, n))
    p = k / n
    z2 = z * z
    denom = 1 + z2 / n
    center = (p + z2 / (2 * n)) / denom
    half = z * math.sqrt(p * (1 - p) / n + z2 / (4 * n * n)) / denom
    return (max(0.0, center - half), min(1.0, center + half))


def format_rate(k: int, n: int) -> str:
    """`k/n (95%: 下限〜上限)` を % で整形する。n=0 は `-`。"""
    if n <= 0:
        return "-"
    lo, hi = wilson_interval(k, n)
    return f"{k}/{n} ({lo * 100:.0f}〜{hi * 100:.0f}%)"
