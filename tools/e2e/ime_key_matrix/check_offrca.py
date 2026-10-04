#!/usr/bin/env python3
"""chrome_probe --offrca の `OFFRCA {json}` 行を集計する(既定は判定せず、観測の表を出す。rc: 試行0件=3、それ以外 0)。

`--expect-closed`(ADR-221、常設の回帰): 全セルで前提が成立した試行(made)の全てで、**実打鍵が ASCII になる(typed_closed)**ことを要求する。
API の読み戻し(api_closed)は参考値で判定に使わない(修正自身が IMC(OFF) を書くので証拠にならず、GJI のように
API だけ閉で打鍵はかなのままという既知の失敗を見逃す)。made が 0 のセルがあれば INVALID(3)、打鍵が閉でない試行があれば FAIL(1)。

使い方: check_offrca.py [--json out.json] chrome_probe.log
各セル(`action:prep`)について、閉じるまでの時間の分布(打鍵せずの API ポーリング)・打鍵結果との一致・
閉じなかった試行の ladder(別手段)の成否を、先頭試行(n=0)とそれ以降に分けて出す。
"""
import json
import sys


def load(path):
    rows = []
    for line in open(path, encoding="utf-8", errors="replace"):
        i = line.find("OFFRCA {")
        if i < 0:
            continue
        try:
            rows.append(json.loads(line[i + 7 :]))
        except ValueError:
            pass
    return rows


def summarize(trials):
    made = [t for t in trials if t.get("prep_ok") and t.get("api_pre") is True and not t.get("focus_lost")]
    closed = [t for t in made if t.get("closed_ms") is not None]
    never = [t for t in made if t.get("closed_ms") is None]
    ms = sorted(t["closed_ms"] for t in closed)
    # 2回目の打鍵(古い composition の確定が混ざらない)があればそれを実際のモードの証拠にする。
    ev = "typed2_open" if any("typed2_open" in t for t in made) else "typed_open"
    typed_closed = [t for t in made if t.get(ev) is False]
    typed_open = [t for t in made if t.get(ev) is True]
    lad = {}
    for t in never:
        for s in t.get("ladder", []):
            d = lad.setdefault(s["step"], [0, 0])
            d[1] += 1
            if s.get("closed_ms") is not None:
                d[0] += 1
    return dict(
        n=len(trials),
        made=len(made),
        api_closed=len(closed),
        api_never=len(never),
        closed_ms_min=ms[0] if ms else None,
        closed_ms_med=ms[len(ms) // 2] if ms else None,
        closed_ms_max=ms[-1] if ms else None,
        typed_closed=len(typed_closed),
        typed_open=len(typed_open),
        ladder={k: f"{v[0]}/{v[1]}" for k, v in lad.items()},
        then=_then(made),
    )


def _then(made):
    """`--or-then` の ON 側の結果(かな=かな入力できた / literal=ASCII のまま)を数える。"""
    d = {}
    for t in made:
        th = t.get("then")
        if not th:
            continue
        k = th.get("class", "?")
        d[k] = d.get(k, 0) + 1
    return d


def main():
    args = [a for a in sys.argv[1:]]
    out = None
    expect_closed = "--expect-closed" in args
    if expect_closed:
        args.remove("--expect-closed")
    if "--json" in args:
        k = args.index("--json")
        out = args[k + 1]
        del args[k : k + 2]
    rows = load(args[0])
    if not rows:
        print("OFFRCA: 試行0件(INVALID)")
        return 3
    cells = {}
    for r in rows:
        cells.setdefault(r["cell"], []).append(r)
    res = {}
    for cell, ts in cells.items():
        first = [t for t in ts if t["n"] == 0]
        rest = [t for t in ts if t["n"] > 0]
        res[cell] = dict(all=summarize(ts), first=summarize(first), rest=summarize(rest))
        a, f, r = res[cell]["all"], res[cell]["first"], res[cell]["rest"]
        print(
            f"OFFRCA_CELL: cell={cell} awase={ts[0].get('awase')} n={a['n']} made={a['made']} "
            f"api_closed={a['api_closed']} api_never={a['api_never']} closed_ms(min/med/max)={a['closed_ms_min']}/{a['closed_ms_med']}/{a['closed_ms_max']} "
            f"typed_closed={a['typed_closed']} typed_open={a['typed_open']} "
            f"first(api_closed/made)={f['api_closed']}/{f['made']} rest(api_closed/made)={r['api_closed']}/{r['made']} ladder={a['ladder']} then={a['then']}"
        )
    # 失敗(閉じなかった)試行と成功試行の page_events の代表例(IME がキーを処理したか・composition の終了を見る)。
    for cell, ts in cells.items():
        for label, sel in (("closed", [t for t in ts if t.get("closed_ms") is not None]), ("never", [t for t in ts if t.get("closed_ms") is None])):
            if sel:
                t = sel[-1]
                print(f"OFFRCA_SAMPLE: cell={cell} {label} n={t['n']} conv={t.get('conv_pre')}->{t.get('conv_end')} events={t.get('page_events')} text_post={t.get('text_post')!r} typed1={t.get('typed')} typed2={t.get('typed2')}/{t.get('typed2_text')!r}")
    if out:
        json.dump(res, open(out, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    if expect_closed:
        invalid = [c for c, v in res.items() if v["all"]["made"] == 0]
        bad = [c for c, v in res.items() if v["all"]["made"] > 0 and v["all"]["typed_closed"] != v["all"]["made"]]
        if invalid:
            print(f"OFFRCA_VERDICT: INVALID(前提が成立した試行が 0 のセル: {invalid})")
            return 3
        if bad:
            print(f"OFFRCA_VERDICT: FAIL(打鍵が ASCII にならなかった試行があるセル: {bad})")
            return 1
        print(f"OFFRCA_VERDICT: PASS(全 {len(res)} セルで made 全試行の打鍵が ASCII になった)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
