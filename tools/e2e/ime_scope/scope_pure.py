"""ADR-282 の IME 状態の保持範囲の測定(scope_probe.py)のうち OS に依存しない部分: 打鍵結果の分類と、セルの判定。Linux の unittest で検査できる。"""
import unicodedata


def classify(text: str) -> str:
    """`ka` を打って Enter で確定した結果の分類: hira(かな)/ ka(英字のまま)/ none(何も出ない)/ other。"""
    t = "".join(text.split())
    if not t:
        return "none"
    if any(0x3040 <= ord(c) <= 0x30FF for c in t):
        return "hira"
    if unicodedata.normalize("NFKC", t).lower() == "ka":
        return "ka"
    return "other"


def majority(values: list):
    """(最頻値, 一致数, 総数)。空なら (None, 0, 0)。同数のときは 'mixed'。"""
    if not values:
        return None, 0, 0
    counts = {}
    for v in values:
        counts[v] = counts.get(v, 0) + 1
    best = max(counts.values())
    top = [k for k, n in counts.items() if n == best]
    return (top[0] if len(top) == 1 else "mixed"), best, len(values)


def judge_open(ctl_direct: str, s1: str) -> str:
    """開閉の保持範囲。s1 = A を『開×ひらがな』にしたあと B で何もせず ka。ctl_direct = A なしの B で何もせず ka。
    shared-open: B が A の開を引き継いだ(ctl は ka で s1 は hira) / independent: B は閉のまま(ctl・s1 とも ka) /
    inconclusive: B が最初から開(ctl が hira)など、区別できない。"""
    if ctl_direct == "hira":
        return "inconclusive(B が A なしでも開)"
    if ctl_direct != "ka":
        return f"inconclusive(ctl={ctl_direct})"
    if s1 == "hira":
        return "shared-open"
    if s1 == "ka":
        return "independent"
    return f"inconclusive(s1={s1})"


def judge_conv(ctl_on: str, s3: str) -> str:
    """変換モードの保持範囲。s3 = A を『開×半角英数』にしたあと B で ON キーを 1 回押して ka。ctl_on = A なしで B が ON キー→ka(hira のはず)。
    shared-conv: B が A の半角英数を引き継いだ(ctl は hira で s3 は ka) / independent: B は ON でひらがな(ctl・s3 とも hira)。"""
    if ctl_on != "hira":
        return f"inconclusive(ctl_on={ctl_on}: B で ON キーが効いていない)"
    if s3 == "ka":
        return "shared-conv"
    if s3 == "hira":
        return "independent"
    return f"inconclusive(s3={s3})"
