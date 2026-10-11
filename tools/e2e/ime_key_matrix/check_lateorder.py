#!/usr/bin/env python3
"""`typing_stress --mode=lateorder`(ADR-255 段階3b)のログを判定する。

burst(速い連打で `foo bar` を N 回): 本文が期待(`foo bar` × N)と一致すれば PASS。入れ替わり(`fo obar` 等)・欠落・重複は FAIL。
hold(押したまま自動リピート相当の KeyDown 6 回→離す→x): 本文が ` x` なら PASS。空白が複数ならリピート漏れ、`x` の後ろに空白があれば押されっぱなし。
INVALID(終了コード 3)は「実行できなかった」だけ: abort・完走マーカー無し・lateorder 行が 0 件。FAIL は終了コード 1。
使い方: check_lateorder.py [--json out.json] <typing_stress.log>
出力の最後の行: `LATEORDER: verdict=PASS|FAIL|INVALID key=… interval=… burst=<ok>/<n> hold=<ok>/<n>`
"""
import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from e2e_common import ts_json_records as parse  # noqa: E402


def analyze(recs: list, done: bool) -> dict:
    rows = [r for r in recs if r.get("type") == "lateorder"]
    cfg = next((r for r in recs if r.get("type") == "lateorder_config"), {})
    reasons = [r.get("reason", "") for r in recs if r.get("type") == "abort"]
    if not done:
        reasons.append("完走マーカー(=== 完了 ===)が無い")
    if not rows:
        reasons.append("lateorder 行が 0 件")
    stat = {"burst": [0, 0], "hold": [0, 0]}
    bad = []
    for r in rows:
        k = r.get("kind")
        if k not in stat:
            continue
        stat[k][1] += 1
        if r.get("text") == r.get("expect"):
            stat[k][0] += 1
        else:
            bad.append({x: r.get(x) for x in ("n", "kind", "expect", "text")})
    ok = not reasons and all(a == b for a, b in stat.values())
    return {
        "verdict": "INVALID" if reasons else ("PASS" if ok else "FAIL"),
        "invalid_reasons": reasons, "key": cfg.get("key"), "interval": cfg.get("interval_ms"),
        "burst": stat["burst"], "hold": stat["hold"], "mismatches": bad[:10],
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
    for m in res["mismatches"]:
        print(f"  mismatch: {m}")
    for why in res["invalid_reasons"]:
        print("INVALID:", why)
    print(f"LATEORDER: verdict={res['verdict']} key={res['key']} interval={res['interval']} "
          f"burst={res['burst'][0]}/{res['burst'][1]} hold={res['hold'][0]}/{res['hold'][1]}")
    if a.json_path:
        with open(a.json_path, "w", encoding="utf-8") as f:
            json.dump(res, f, ensure_ascii=False, indent=1)
    return {"INVALID": 3, "FAIL": 1}.get(res["verdict"], 0)


if __name__ == "__main__":
    sys.exit(main())
