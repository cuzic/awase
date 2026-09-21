#!/usr/bin/env python3
"""3段階のラウンドで、キャリブレーション手法と予測手法の両方を改善するための計測(ユーザー指示 2026-09-21)。

  1. 設定の読み取りラウンド: GJIの設定(config1.db+Mozc公開キーマップ)から静的な表Sを作る。ここでは spec_atok(手書きのATOK写像)で代用
  2. 学習ラウンド: 注入で実IMEを観測して表Lを作る。SとLの食い違いセルが「設定の読み取りの直すべき箇所」
  3. 検証ラウンド: 学習に使っていないランで、表(S/L/合成M=Lの決定セルを優先、無ければS)の開ループ連鎖を採点する

  cycle.py all <学習ログ...> -- <検証ログ...>   1〜3を通しで出す(オフライン)

  cycle.py learn  <A'ログ...>               学習ラウンド: 表の質(網羅・決定性・一段予測)を出す
  cycle.py verify <学習ログ...> -- <検証ログ...>   予測検証ラウンド(オフライン): 学習に使っていないランで開ループ連鎖の
                                              精度と、最初に外れる原因セル(=次に直す箇所)を出す
  ハードウェア確認(awase実機の Engine ずれ)は effect_learning.py --drift (DRIFT_OFF=100/400/1500)。

学習側の指標: C1 網羅(観測したセル数/理論セル数)  C2 決定性(多数派一致率)  C3 一段予測の正答率(leave-one-run-out)
予測側の指標: P1 開ループ連鎖の一致率  P2 最初のずれまでの平均手数  P3 最初に外れる原因セル(状態×キー: 予測→実際)
"""
import sys
from collections import Counter

import effect_learning as e

ALL_KEYS = [k for k in e.NAMES]
STATES = [(o, n, c, True) for o in (False, True) for n in (True, False) for c in (False, True)]


def learn_round(paths):
    runs = [e.parse(p) for p in paths]
    table = e.learn(runs)
    theory = len(STATES) * len(ALL_KEYS)
    tot = det = 0
    nondet = []
    for cell, c in table.items():
        n = sum(c.values())
        tot += n
        det += c.most_common(1)[0][1]
        if c.most_common(1)[0][1] < n:
            nondet.append((cell, c))
    ok = ng = 0
    for i, rows in enumerate(runs):
        tr = e.learn([r for j, r in enumerate(runs) if j != i])
        for vk, before, a400, *_ in rows:
            if before and a400:
                p = e.predict(tr, before, vk)
                if p is None:
                    continue
                ok += p == a400
                ng += p != a400
    print(f"[学習] C1 網羅 {len(table)}/{theory}セル  C2 決定性 {det}/{tot}={det / max(tot, 1):.1%}  "
          f"C3 一段予測(LORO) {ok}/{ok + ng}={ok / max(ok + ng, 1):.1%}")
    for (st, vk), c in nondet:
        print(f"    非決定: {e.fmt(st)} + {e.NAMES[vk]} → " + " / ".join(f"{e.fmt(k)}×{v}" for k, v in c.most_common()))
    return table


def verify_round(train_paths, test_paths):
    table = e.learn([e.parse(p) for p in train_paths])
    agree = steps = 0
    firsts = []
    causes = Counter()
    for p in test_paths:
        rows = e.parse(p)
        belief = prev = None
        div = None
        k = 0
        for vk, before, a400, *_ in rows:
            if not (before and a400):
                belief = prev = None
                continue
            if belief is None or before != prev:
                belief = before
            prev = a400
            nxt = e.predict(table, belief, vk)
            steps += 1
            ok = nxt == a400
            agree += ok
            if not ok and div is None:
                div = k
                causes[(e.fmt(belief), e.NAMES[vk], e.fmt(nxt), e.fmt(a400))] += 1
            belief = nxt if nxt is not None else belief
            k += 1
        firsts.append(div if div is not None else k)
    print(f"[予測] P1 開ループ連鎖の一致 {agree}/{steps}={agree / max(steps, 1):.1%}  "
          f"P2 最初のずれまで平均 {sum(firsts) / max(len(firsts), 1):.1f}手(ラン数{len(firsts)})")
    for (b, k, pr, ac), n in causes.most_common():
        print(f"    P3 最初の外れ: 信念{b} + {k} → 予測{pr} / 実際{ac} ×{n}")


def three_rounds(train_paths, test_paths):
    train = [e.parse(p) for p in train_paths]
    L = e.learn(train)

    def det_L(st, vk):
        c = L.get((st, vk))
        if not c:
            return None
        top, n = c.most_common(1)[0][1], sum(c.values())
        return c.most_common(1)[0][0] if top == n else None  # 非決定は予測しない

    S = e.spec_atok
    M = lambda st, vk: (det_L(st, vk) if det_L(st, vk) is not None else S(st, vk))
    print("== 1. 設定の読み取り(S)と、2. 学習(L)の食い違いセル ==")
    diff = tot = 0
    for (st, vk), c in sorted(L.items(), key=lambda x: (x[0][1], x[0][0])):
        top = c.most_common(1)[0][0]
        s_pred = S(st, vk)
        tot += 1
        if s_pred != top:
            diff += 1
            print(f"    {e.fmt(st)} + {e.NAMES[vk]}: 設定→{e.fmt(s_pred) if s_pred else '未定義'} / 学習→{e.fmt(top)}×{sum(c.values())}")
    print(f"    食い違い {diff}/{tot}セル  (Lが未観測のセルは {len(STATES) * len(ALL_KEYS) - len(L)}、Sだけが埋められる)")
    print("== 3. 検証(学習に使っていないラン。開ループ連鎖、予測なしはbeliefを変えない) ==")
    for name, f in (("S 設定のみ", S), ("L 学習のみ", det_L), ("M 合成", M)):
        agree = steps = 0
        firsts = []
        for p in test_paths:
            rows = e.parse(p)
            belief = prev = None
            div = None
            k = 0
            for vk, before, a400, *_ in rows:
                if not (before and a400):
                    belief = prev = None
                    continue
                if belief is None or before != prev:
                    belief = before
                prev = a400
                nxt = f(belief, vk)
                steps += 1
                ok = nxt == a400
                agree += ok
                if not ok and div is None:
                    div = k
                belief = nxt if nxt is not None else belief  # 予測なし=beliefは変えない(実状態への再同期はしない=観測が無い場合と同じ)
                k += 1
            firsts.append(div if div is not None else k)
        print(f"    {name}: 一致 {agree}/{steps}={agree / max(steps, 1):.1%}  最初のずれまで平均 {sum(firsts) / max(len(firsts), 1):.1f}手")


if __name__ == "__main__":
    a = sys.argv[1:]
    if not a:
        print(__doc__)
    elif a[0] == "learn":
        learn_round(a[1:])
    elif a[0] == "all" and "--" in a:
        i = a.index("--")
        three_rounds(a[1:i], a[i + 1:])
    elif a[0] == "verify" and "--" in a:
        i = a.index("--")
        verify_round(a[1:i], a[i + 1:])
    else:
        print(__doc__)
