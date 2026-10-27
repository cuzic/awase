#!/usr/bin/env python3
"""`typing_stress --mode=immread`(BUG-219 の調査、ci/imm-read-matrix 専用)のログを集計する。

入力先の窓 × IME ごとに、IMM で読んだ IME の開閉(prod=awase の prefetch と同じ読み、raw=クラス名の門前払いを外した素の読み)が、
最後の打鍵で決めた実状態(truth: open=「か」/closed=`ka`)と一致するかを数える。判定はしない(観測のみ)。
INVALID(終了コード 3)は「実行できなかった」だけ: abort・完走マーカー無し・immread 行が 0 件。それ以外は終了コード 0。
使い方: check_immread.py [--json out.json] <typing_stress.log>
出力の最後の行: `IMMREAD: verdict=OBSERVE|INVALID form=… ime=… class=… tsf_native=… trials=… prod=… raw=…`
  prod / raw は `open[on/off/none]closed[on/off/none]`(読みの回数。truth が unknown の試行は別に `unknown=N`)。
"""
import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from e2e_common import ts_json_records as parse  # noqa: E402


def tally(rows: list, key: str) -> dict:
    out = {}
    for truth in ("open", "closed"):
        t = {"on": 0, "off": 0, "none": 0}
        for r in rows:
            if r.get("truth") == truth:
                for k in t:
                    t[k] += int(r.get(key, {}).get(k, 0))
        out[truth] = t
    return out


def fmt(t: dict) -> str:
    return "".join(
        f"{truth}[{t[truth]['on']}/{t[truth]['off']}/{t[truth]['none']}]" for truth in ("open", "closed")
    )


def analyze(recs: list, done: bool) -> dict:
    rows = [r for r in recs if r.get("type") == "immread"]
    cfg = next((r for r in recs if r.get("type") == "config"), {})
    reasons = [r.get("reason", "") for r in recs if r.get("type") == "abort"]
    if not done:
        reasons.append("完走マーカー(=== 完了 ===)が無い")
    if not rows:
        reasons.append("immread 行が 0 件")
    classes = sorted({r.get("class", "") for r in rows if r.get("class")})
    convs = {
        truth: sorted({c for r in rows if r.get("truth") == truth for c in r.get("prod_convs", [])})
        for truth in ("open", "closed", "unknown")
    }
    return {
        "verdict": "INVALID" if reasons else "OBSERVE",
        "invalid_reasons": reasons,
        "form": cfg.get("form"),
        "ime": cfg.get("ime"),
        "classes": classes,
        "tsf_native": any(r.get("tsf_native") for r in rows),
        "trials": {
            "open": sum(1 for r in rows if r.get("truth") == "open"),
            "closed": sum(1 for r in rows if r.get("truth") == "closed"),
            "unknown": sum(1 for r in rows if r.get("truth") == "unknown"),
        },
        "prod": tally(rows, "prod"),
        "raw": tally(rows, "raw"),
        "ext_prod": tally(rows, "ext_prod"),
        "ext_raw": tally(rows, "ext_raw"),
        "external_reader": any(r.get("external") for r in recs if r.get("type") == "immread_reader"),
        "convs": convs,
        "unknown_texts": [r.get("text") for r in rows if r.get("truth") == "unknown"][:5],
    }


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(add_help=False)
    ap.add_argument("--json", dest="json_path")
    ap.add_argument("log", nargs="*")
    a = ap.parse_args(argv)
    if len(a.log) != 1:
        print(__doc__)
        return 2
    try:
        done = "=== 完了 ===" in open(a.log[0], encoding="utf-8", errors="replace").read()
        recs = parse(a.log[0])
    except OSError:
        done, recs = False, []
    res = analyze(recs, done)
    if a.json_path:
        with open(a.json_path, "w", encoding="utf-8") as f:
            json.dump(res, f, ensure_ascii=False, indent=1)
    for r in res["invalid_reasons"]:
        print(f"invalid: {r}")
    t = res["trials"]
    print(
        f"IMMREAD: verdict={res['verdict']} form={res['form']} ime={res['ime']} class={','.join(res['classes'])} "
        f"tsf_native={res['tsf_native']} trials=open:{t['open']},closed:{t['closed']},unknown:{t['unknown']} "
        f"prod={fmt(res['prod'])} raw={fmt(res['raw'])} "
        f"ext_prod={fmt(res['ext_prod'])} ext_raw={fmt(res['ext_raw'])} external_reader={res['external_reader']} "
        f"convs(prod)={res['convs']}"
    )
    return 3 if res["verdict"] == "INVALID" else 0


if __name__ == "__main__":
    sys.exit(main())
