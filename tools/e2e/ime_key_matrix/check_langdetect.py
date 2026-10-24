#!/usr/bin/env python3
"""`typing_stress --mode=langdetect`(ADR-280 C0 の配線確認)のログを判定する。

`[auto_detect] eisu_threshold`(`--eisu` で期待値を渡す)が実際の awase.exe の判定器の閾値に届いたかを、次の 2 つで見る。
 1. awase.log の起動時の観測行 `[lang-detect] config: ... en_threshold_effective=<値>` が `max(eisu_threshold, en_threshold)` と一致する。
 2. 固定の合成列(B 境界 `ayre`: llr -14.9978〈nicola_keytop では -14.734〉、E 英数 `build`)の判定(`[lang-detect]` の journal 行の verdict)が期待どおり。
    B は閾値 14 なら not-japanese、16 なら undecided。E は not-japanese。J(親指+k)は CI では Japanese にならない(無変換が excluded-key)ので、記録だけ表示して合否に含めない。
列の打鍵時刻(typing_stress.log の langdetect 行の start/end)で awase.log を切り出す。本文は読まない。
INVALID(終了コード 3)は「実行できなかった」だけ: abort・完走マーカー無し・config 行/試行の記録が無い。FAIL は終了コード 1。
使い方: check_langdetect.py --eisu 14|16 [--json out.json] <typing_stress.log> <awase.log>
出力の最後の行: `LANGDETECT: verdict=PASS|FAIL|INVALID eisu=… effective=… B=<ok>/<n> E=<ok>/<n> J=<ok>/<n>(J は情報のみ)`
"""
import argparse
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from e2e_common import hms_to_ms, load_awase_timed, ts_json_records as parse  # noqa: E402

CONFIG_RE = re.compile(r"\[lang-detect\] config: .*?en_threshold_effective=(-?[\d.]+) eisu_threshold=(-?[\d.]+)")
FIELD_RE = {
    "verdict": re.compile(r'verdict="?([a-z-]+)"?'),
    "llr": re.compile(r"\bllr=(-?[\d.]+)"),
    "presses": re.compile(r"\bpresses=(\d+)"),
    "action": re.compile(r'\baction="?([a-z-]+)"?'),
    "stop": re.compile(r'\bstop="?([a-z-]+)"?'),
    "evidence": re.compile(r'\bevidence="?([a-z-]+)"?'),
}


def expected_verdict(kind: str, eisu: float) -> str:
    if kind == "J":
        return "japanese"
    if kind == "E":
        return "not-japanese"
    return "not-japanese" if eisu <= 14.5 else "undecided"  # B: llr -14.9978


def episodes(timed, start: str, end: str, slack_ms: int = 300) -> list:
    lo, hi = hms_to_ms(start) - slack_ms, hms_to_ms(end) + slack_ms
    out = []
    for t, line in timed:
        if "[lang-detect]" not in line or "verdict" not in line:
            continue
        ms = hms_to_ms(t)
        if lo <= ms <= hi:
            rec = {k: (m.group(1) if (m := rx.search(line)) else None) for k, rx in FIELD_RE.items()}
            out.append(rec)
    return out


def analyze(recs: list, timed: list, eisu: float, done: bool) -> dict:
    rows = [r for r in recs if r.get("type") == "langdetect"]
    reasons = [r.get("reason", "") for r in recs if r.get("type") == "abort"]
    if not done:
        reasons.append("完走マーカー(=== 完了 ===)が無い")
    if not rows:
        reasons.append("langdetect 行が 0 件")
    cfgs = [m for _, line in timed if (m := CONFIG_RE.search(line))]
    effective = float(cfgs[-1].group(1)) if cfgs else None
    if effective is None:
        reasons.append("awase.log に `[lang-detect] config:` の行が無い(auto_detect が有効になっていない)")
    stat = {"B": [0, 0], "E": [0, 0], "J": [0, 0]}
    detail = []
    for r in rows:
        kind = r.get("kind")
        if kind not in stat:
            continue
        stat[kind][1] += 1
        eps = episodes(timed, r["start"], r["end"])
        want = expected_verdict(kind, eisu)
        got = eps[0] if eps else {}
        ok = bool(eps) and got.get("verdict") == want
        if ok:
            stat[kind][0] += 1
        detail.append({"n": r.get("n"), "kind": kind, "want": want, "episodes": eps, "ok": ok})
    for kind, (_ok, n) in stat.items():
        if n == 0 and rows and kind != "J":
            reasons.append(f"{kind} の試行が 0 件")
    wired = effective is not None and abs(effective - max(eisu, 8.0)) < 1e-6
    # J は情報のみ(合否に含めない): 無変換が IME キー扱いで excluded-key となりエピソードが Japanese に届かない(2026-10-24 の CI 実測)。
    ok_all = wired and all(stat[k][0] == stat[k][1] for k in ("B", "E"))
    return {
        "verdict": "INVALID" if reasons else ("PASS" if ok_all else "FAIL"),
        "invalid_reasons": reasons, "eisu": eisu, "effective": effective, "wired": wired,
        "B": stat["B"], "E": stat["E"], "J": stat["J"], "detail": detail,
    }


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(add_help=False)
    ap.add_argument("--eisu", type=float)
    ap.add_argument("--json", dest="json_path")
    ap.add_argument("log", nargs="*")
    a = ap.parse_args(argv)
    if len(a.log) != 2 or a.eisu is None:
        print(__doc__)
        return 2
    try:
        done = "=== 完了 ===" in open(a.log[0], encoding="utf-8", errors="replace").read()
        recs = parse(a.log[0])
    except OSError:
        done, recs = False, []
    res = analyze(recs, load_awase_timed(a.log[1]), a.eisu, done)
    for d in res["detail"]:
        print(f"  {d['kind']}#{d['n']}: want={d['want']} ok={d['ok']} episodes={d['episodes']}")
    for why in res["invalid_reasons"]:
        print("INVALID:", why)
    print(f"LANGDETECT: verdict={res['verdict']} eisu={res['eisu']} effective={res['effective']} "
          f"B={res['B'][0]}/{res['B'][1]} E={res['E'][0]}/{res['E'][1]} J={res['J'][0]}/{res['J'][1]}")
    if a.json_path:
        with open(a.json_path, "w", encoding="utf-8") as f:
            json.dump(res, f, ensure_ascii=False, indent=1)
    return {"INVALID": 3, "FAIL": 1}.get(res["verdict"], 0)


if __name__ == "__main__":
    sys.exit(main())
