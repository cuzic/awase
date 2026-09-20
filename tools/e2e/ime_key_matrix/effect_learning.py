#!/usr/bin/env python3
"""IMEキーの「効果」を学習した表で、実IME状態を予測できるか（モードずれを推定側で防げるか）の評価。

入力: ime_key_matrix_spike の `--walk=N --seed=S` のログ（複数可。1ファイル=1シード=1ラン）。
状態 = (open, native, comp)。open は A/B 観測の一致、native は conv の NATIVE ビット、
comp は未確定文字列の有無。観測時点は押下 +400ms（settle 済み）。

評価:
  1. 決定性: (状態, キー) セルごとに次状態が一意か。非決定セルがあれば、表だけでは
     ずれを防げない（別の観測が要る）。
  2. 一段予測（closed-loop）: 押下前の実状態から次状態を当てる。学習=他のラン、検証=そのラン。
  3. 開ループ予測（open-loop）: 最初の実状態だけ与え、以後は観測せず予測を連鎖。
     これが「awase の belief が読み取りなしでどれだけ実状態に追随できるか」の直接の指標。
  4. ベースライン: 変化なし / 素朴な静的モデル。
  5. 観測タイミング: +400ms と +1500ms で状態が違う押下の割合（settle ノイズ）。

使い方:
  effect_learning.py <walk.log> [<walk2.log> ...]            表の学習と予測精度(awase停止/起動どちらのログでも)
  effect_learning.py --drift <walk.log> <awase.log>          awase起動時: Engine の ON/OFF と実IME状態のずれ
  effect_learning.py --compare <A.log> <B.log>               A(awaseは素通し) と B(awaseを通す) の表の差分
    A = awase起動・AWASE_TEST_INJECTIONなし(注入キーは外部注入として素通し)=IME単体の効果
    B = awase起動・AWASE_TEST_INJECTION=1(注入キーを物理キー扱い)=awase+IMEの最終結果
    差分 = awase自身がIME状態に与えた影響(awase起因のずれ)。
"""
import re
import sys
from collections import Counter, defaultdict

KEY_RE = re.compile(
    r"^\[[\d:.]+Z\] KEY .*? vk=0x([0-9A-Fa-f]+) .*?press=([\d:.]+)Z \((auto)\)"
)
SNAP_RE = re.compile(
    r"A\(open=(\S+) conv=(\S+)\) B\(open=(\S+) conv=(\S+)\).*?comp=(\"[^\"]*\"|\?)"
)
NAMES = {
    0x1D: "無変換", 0x1C: "変換", 0xF2: "ひらがな", 0xF3: "半角/全角",
    0x4B: "k", 0x41: "a", 0x0D: "Enter", 0x1B: "Esc",
}


def parse_snap(text):
    m = SNAP_RE.search(text)
    if not m:
        return None
    ao, ac, bo, bc, comp = m.groups()

    def b(x):
        return {"1": True, "0": False}.get(x)

    def h(x):
        try:
            return int(x, 16)
        except ValueError:
            return None

    a_open, b_open = b(ao), b(bo)
    if a_open is not None and b_open is not None and a_open != b_open:
        return None  # A/B 不一致は観測不能として除外
    open_ = a_open if a_open is not None else b_open
    conv = h(bc) if h(bc) is not None else h(ac)
    if open_ is None or conv is None or comp == "?":
        return None
    return (open_, conv & 1, comp != '""', (conv & 0x10) != 0)


def parse(path):
    """→ [(vk, before, after400, after1500)]  (どれか観測不能なら None)"""
    rows, cur = [], None
    for line in open(path, encoding="utf-8").read().splitlines():
        m = KEY_RE.match(line)
        if m:
            cur = {"vk": int(m.group(1), 16), "press": to_ms(m.group(2))}
            rows.append(cur)
            continue
        if cur is None:
            continue
        s = line.strip()
        if s.startswith("前"):
            cur["before"] = parse_snap(s)
        elif s.startswith("+400ms"):
            cur["a400"] = parse_snap(s)
        elif s.startswith("+1500ms"):
            cur["a1500"] = parse_snap(s)
    return [
        (r["vk"], r.get("before"), r.get("a400"), r.get("a1500"), r["press"])
        for r in rows
        if r["vk"] in NAMES
    ]


def to_ms(t):
    h, m, sec = t.split(":")
    return (int(h) * 3600 + int(m) * 60) * 1000 + float(sec) * 1000


def parse_engine(path):
    """awase ログ → [(ms, 'activated'|'deactivated')]"""
    ev = []
    for line in open(path, encoding="utf-8", errors="replace").read().splitlines():
        m = re.match(r"\d{4}-\d\d-\d\dT([\d:.]+)Z\s+\w+\s+.*Engine (activated|deactivated)", line)
        if m:
            ev.append((to_ms(m.group(1)), m.group(2)))
    return ev


def drift(spike_log, awase_log):
    """押下 +400ms 時点の Engine 状態と、実IME状態から期待される Engine 状態(open かつ かな)を比べる。"""
    rows = parse(spike_log)
    ev = parse_engine(awase_log)
    if not ev:
        print("awase ログに Engine activated/deactivated が無い(RUST_LOG=debug か確認)")
        return
    tot = bad = 0
    by = Counter()
    detail = []
    for vk, before, a400, _, press in rows:
        if not a400 or press + 400 < ev[0][0]:
            continue
        eng = [k for ms, k in ev if ms <= press + 400]
        eng_on = eng[-1] == "activated"
        want = a400[0] and a400[1] == 1
        tot += 1
        if eng_on != want:
            bad += 1
            by[(NAMES[vk], fmt(before), fmt(a400), eng_on)] += 1
            detail.append((press, NAMES[vk], fmt(before), fmt(a400), eng_on))
    print(f"押下 {tot} 件中、Engine と実IMEのずれ {bad} 件 ({bad / max(tot, 1):.1%})")
    for (k, b, a, e), n in by.most_common():
        print(f"  {k:6} {b:16} → 実{a:16} でEngine={'ON' if e else 'OFF'}(期待と逆) ×{n}")


def compare(a_log, b_log):
    ta, tb = learn([parse(a_log)]), learn([parse(b_log)])
    keys = sorted(set(ta) | set(tb), key=lambda x: (x[1], x[0]))
    same = diff = only = 0
    print(f"{'状態':16} {'キー':6} {'A(素通し)':30} {'B(awase経由)':30}")
    for k in keys:
        ca, cb = ta.get(k), tb.get(k)
        pa = ca.most_common(1)[0][0] if ca else None
        pb = cb.most_common(1)[0][0] if cb else None
        if pa is None or pb is None:
            only += 1
            mark = "片側のみ"
        elif pa == pb:
            same += 1
            continue
        else:
            diff += 1
            mark = "★差分"
        print(f"{fmt(k[0]):16} {NAMES[k[1]]:6} {fmt(pa):30} {fmt(pb):30} {mark}")
    print(f"一致 {same} / 差分 {diff} / 片側のみ {only}  (差分=awaseがIME状態に与えた影響)")


def fmt(st):
    if st is None:
        return "?"
    o, n, c, r = st
    return ("ON" if o else "OFF") + ("/かな" if n else "/英数") + ("/入力中" if c else "") + ("" if r else "/非ローマ字")


def learn(runs):
    t = defaultdict(Counter)
    for rows in runs:
        for vk, before, a400, *_ in rows:
            if before and a400:
                t[(before, vk)][a400] += 1
    return t


def predict(table, st, vk):
    c = table.get((st, vk))
    return c.most_common(1)[0][0] if c else None


def naive_static(st, vk):
    """公開Mozc既定に近い素朴モデル: 無変換/変換=ON、ひらがな=ON+かな、半角/全角=開閉トグル、
    k/a=入力中(ONのとき)、Enter/Esc=未確定を消す。conv は開閉をまたいで保存しない(=ON時にかな)。"""
    o, n, c, r = st
    if vk in (0x1D, 0x1C):
        return (True, n if o else 1, c, r)
    if vk == 0xF2:
        return (True, 1, c, r)
    if vk == 0xF3:
        return (not o, n, False if o else c, r)
    if vk in (0x4B, 0x41):
        return (o, n, True if o else c, r)
    if vk in (0x0D, 0x1B):
        return (o, n, False, r)
    return st


def main(paths):
    runs = [parse(p) for p in paths]
    for p, r in zip(paths, runs):
        usable = sum(1 for _, b, a, *_ in r if b and a)
        print(f"{p}: 押下{len(r)}件、状態が取れた{usable}件")
    table = learn(runs)

    print("\n== 1. 決定性（全ランを学習に使った表） ==")
    total = det = 0
    nondet = []
    for (st, vk), c in sorted(table.items(), key=lambda x: (x[0][1], x[0][0])):
        n = sum(c.values())
        total += n
        top = c.most_common(1)[0][1]
        det += top
        if top < n:
            nondet.append((st, vk, c))
        print(f"  {fmt(st):16} + {NAMES[vk]:6} → " + " / ".join(f"{fmt(k)}×{v}" for k, v in c.most_common()))
    print(f"  セル数={len(table)}、多数派一致率={det}/{total}={det / max(total, 1):.1%}、非決定セル={len(nondet)}")

    print("\n== 2. 一段予測（leave-one-run-out。学習=他ラン、検証=そのラン） ==")
    res = Counter()
    for i, rows in enumerate(runs):
        tr = learn([r for j, r in enumerate(runs) if j != i]) if len(runs) > 1 else table
        for vk, before, a400, *_ in rows:
            if not (before and a400):
                continue
            res["n"] += 1
            p = predict(tr, before, vk)
            res["learned_unseen" if p is None else "learned_ok" if p == a400 else "learned_ng"] += 1
            res["nochange_ok"] += before == a400
            res["naive_ok"] += naive_static(before, vk) == a400
    n = max(res["n"], 1)
    print(f"  検証押下 {res['n']} 件")
    print(f"  学習表   : 正答 {res['learned_ok']} / 誤答 {res['learned_ng']} / 未学習セル {res['learned_unseen']}"
          f"  (既知セルの正答率 {res['learned_ok'] / max(res['learned_ok'] + res['learned_ng'], 1):.1%})")
    print(f"  変化なし : 正答率 {res['nochange_ok'] / n:.1%}")
    print(f"  素朴静的 : 正答率 {res['naive_ok'] / n:.1%}")

    print("\n== 3. 開ループ予測（最初の実状態のみ与え、以後は読み取らず連鎖） ==")
    for name, f in (
        ("学習表", lambda tr: (lambda st, vk: predict(tr, st, vk))),
        ("素朴静的", lambda tr: naive_static),
        ("変化なし", lambda tr: (lambda st, vk: st)),
    ):
        steps = agree = 0
        first_div = []
        for i, rows in enumerate(runs):
            tr = learn([r for j, r in enumerate(runs) if j != i]) if len(runs) > 1 else table
            g = f(tr)
            belief = None
            prev_actual = None
            div_at = None
            k = 0
            for vk, before, a400, *_ in rows:
                if not (before and a400):
                    belief = prev_actual = None  # 観測欠損で連鎖を切る
                    continue
                # 直前の実状態と押下前状態が食い違う=読み取り外で状態が変わった(連鎖は成立しない)ので再同期。
                if belief is None or before != prev_actual:
                    belief = before
                prev_actual = a400
                nxt = g(belief, vk)
                steps += 1
                ok = nxt == a400
                agree += ok
                if not ok and div_at is None:
                    div_at = k
                belief = nxt if nxt is not None else a400  # 未学習は実状態へ再同期
                k += 1
            first_div.append(div_at if div_at is not None else k)
        print(f"  {name:6}: belief==実状態の割合 {agree}/{steps}={agree / max(steps, 1):.1%}、"
              f"最初のずれまでの平均手数 {sum(first_div) / max(len(first_div), 1):.1f}")

    print("\n== 4. 観測タイミング（+400ms と +1500ms で状態が違う押下） ==")
    diff = tot = 0
    by_key = Counter()
    for rows in runs:
        for vk, _, a400, a1500, *_ in rows:
            if a400 and a1500:
                tot += 1
                if a400 != a1500:
                    diff += 1
                    by_key[NAMES[vk]] += 1
    print(f"  {diff}/{tot} = {diff / max(tot, 1):.1%}  内訳: {dict(by_key)}")

    if nondet:
        print("\n== 非決定セルの詳細（ずれの直接の原因候補） ==")
        for st, vk, c in nondet:
            print(f"  {fmt(st)} + {NAMES[vk]}: " + " / ".join(f"{fmt(k)}×{v}" for k, v in c.most_common()))


if __name__ == "__main__":
    if len(sys.argv) == 4 and sys.argv[1] == "--drift":
        drift(sys.argv[2], sys.argv[3])
        sys.exit(0)
    if len(sys.argv) == 4 and sys.argv[1] == "--compare":
        compare(sys.argv[2], sys.argv[3])
        sys.exit(0)
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(2)
    main(sys.argv[1:])
