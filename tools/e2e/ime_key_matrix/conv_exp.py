"""隠れ状態「変換中(Conversion)」を打鍵履歴から追跡すると、非決定セルが決定的になるかの検証。"""
import sys
from collections import Counter, defaultdict
import effect_learning as e

def step_flag(flag, before, vk, variant):
    if variant == "none" or before is None: return False
    if vk == 0x1C and before[2] and before[0]: return True          # 変換(入力中) → 変換中
    if variant == "keep_muh" and vk == 0x1D and flag: return True   # 無変換は変換中を保持
    return False

def aug_rows(rows, variant):
    out, flag = [], False
    for vk, b, a, a15, press in rows:
        if not (b and a): flag = False; out.append((vk, None, None)); continue
        out.append((vk, (b, flag), (a, step_flag(flag, b, vk, variant))))
        flag = step_flag(flag, b, vk, variant)
    return out

def run(paths, variant):
    tab = defaultdict(Counter)
    for p in paths:
        for vk, b, a in aug_rows(e.parse(p), variant):
            if b: tab[(b, vk)][a[0]] += 1   # 次状態は(観測状態)だけを予測対象にする
    tot = det = 0; nd = []
    for k, c in tab.items():
        n = sum(c.values()); top = c.most_common(1)[0][1]; tot += n; det += top
        if top < n: nd.append((k, c))
    if tot == 0:
        print(f"[{variant}] エラー: 使える観測が0件(押下前と +400ms の状態が無い。--fast/--snap100 のログは対象外)", file=sys.stderr)
        sys.exit(2)
    print(f"[{variant}] セル={len(tab)} 決定性 {det}/{tot}={det/tot:.1%} 非決定セル={len(nd)}")
    for ((st, fl), vk), c in nd:
        print(f"    {e.fmt(st)}{'[変換中]' if fl else ''} + {e.NAMES[vk]} → " + " / ".join(f"{e.fmt(k)}×{v}" for k, v in c.most_common()))

if __name__ == "__main__":
    for v in ("none", "clear", "keep_muh"):
        run(sys.argv[1:], v)
