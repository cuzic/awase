#!/usr/bin/env python3
"""`typing_stress --mode=preedit` のログから、未確定文字(composition)の読み取り手段が使えるかを判定する。

1 試行の phase(手順で真値が決まる): before=未確定なし / composing・composing2=未確定あり(かな 1〜2 打、確定前) /
after=未確定なし(終端キー enter=確定・esc=取り消し。`--preedit-end=none` のときは判定しない)。
手段ごとに各 phase の読み値を真値と比べる:
  right        真値どおり(composing なら `composing:<文字列>`、なしなら `none`)
  wrong        真値と逆(本当は未確定があるのに none、など)。この手段は判定に使えない
  unavailable  読めなかった(`nopattern` / `err:*` / `nohimc`)
手段: uia(UIA TextEditPattern::GetActiveComposition)/ imm(ImmGetCompositionStringW、自プロセスの窓のみ)。
text(入力先の読み戻し)は composing 中に本文へ出ていないこと(leak=0)の確認用で、合否には入れない。value(UIA ValuePattern)は参考。
method の verdict: READS(全 phase right)/ PARTIAL(right があり、wrong は無い。unavailable か一部)/ UNUSABLE(wrong がある、または right が 0)。
合否は出さない(観測用)。INVALID(終了コード 3)は「実行できなかった」だけ: abort・完走マーカー無し・preedit 行が 0 件。
使い方: check_preedit.py [--json out.json] <typing_stress.log>
出力の最後の行: `PREEDIT: verdict=OBSERVED|INVALID form=… ime=… end=… uia=<verdict> imm=<verdict> n=<試行数> text_leak=<件数>`
"""
import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from e2e_common import ts_json_records as parse  # noqa: E402

UNAVAILABLE = ("nopattern", "nohimc")
METHODS = ("uia", "imm")


def truth(phase: str, end: str):
    """phase の真値: 'composing' / 'none' / None(判定しない)。"""
    if phase in ("composing", "composing2"):
        return "composing"
    if phase == "before":
        return "none"
    if phase == "after":
        return None if end == "none" else "none"
    return None


def classify(reading: str, want: str) -> str:
    if reading in UNAVAILABLE or reading.startswith("err:"):
        return "unavailable"
    got = "composing" if reading.startswith("composing:") else "none"
    return "right" if got == want else "wrong"


def method_verdict(c: dict) -> str:
    if c["wrong"] > 0 or c["right"] == 0:
        return "UNUSABLE"
    return "READS" if c["unavailable"] == 0 else "PARTIAL"


def analyze(recs: list, done: bool) -> dict:
    cfg = next((r for r in recs if r.get("type") == "config"), {})
    pcfg = next((r for r in recs if r.get("type") == "preedit_config"), {})
    end = pcfg.get("end", "enter")
    reads = [r for r in recs if r.get("type") == "preedit"]
    aborts = [r.get("reason", "") for r in recs if r.get("type") == "abort"]
    counts = {m: {"right": 0, "wrong": 0, "unavailable": 0} for m in METHODS}
    by_phase = {}
    leak = 0
    for r in reads:
        want = truth(r["phase"], end)
        if want is None:
            continue
        for m in METHODS:
            k = classify(r.get(m, "unavailable"), want)
            counts[m][k] += 1
            by_phase.setdefault((m, r["phase"]), {}).setdefault(k, 0)
            by_phase[(m, r["phase"])][k] += 1
        if r["phase"] in ("composing", "composing2") and str(r.get("text", "")).strip():
            leak += 1
    reasons = list(aborts)
    if not done:
        reasons.append("完走マーカー(=== 完了 ===)が無い")
    if not reads:
        reasons.append("preedit 行が 0 件")
    return {
        "verdict": "INVALID" if reasons else "OBSERVED",
        "invalid_reasons": reasons,
        "form": cfg.get("form"),
        "ime": cfg.get("ime"),
        "end": end,
        "n": len({r["n"] for r in reads}),
        "counts": counts,
        "methods": {m: method_verdict(counts[m]) for m in METHODS},
        "by_phase": {f"{m}/{p}": v for (m, p), v in by_phase.items()},
        "text_leak": leak,
        "samples": [{k: r.get(k) for k in ("n", "phase", "uia", "imm", "value", "text")} for r in reads[:8]],
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
    except OSError:
        done = False
    try:
        recs = parse(a.log[0])
    except OSError:
        recs = []
    res = analyze(recs, done)
    for m in METHODS:
        c = res["counts"][m]
        print(f"PREEDIT_METHOD: method={m} verdict={res['methods'][m]} right={c['right']} wrong={c['wrong']} unavailable={c['unavailable']}")
    for k, v in sorted(res["by_phase"].items()):
        print(f"  {k}: {v}")
    for s in res["samples"]:
        print(f"  sample: {s}")
    for why in res["invalid_reasons"]:
        print("INVALID:", why)
    print(f"PREEDIT: verdict={res['verdict']} form={res['form']} ime={res['ime']} end={res['end']} "
          f"uia={res['methods']['uia']} imm={res['methods']['imm']} n={res['n']} text_leak={res['text_leak']}")
    if a.json_path:
        with open(a.json_path, "w", encoding="utf-8") as f:
            json.dump(res, f, ensure_ascii=False, indent=1)
    return 3 if res["verdict"] == "INVALID" else 0


if __name__ == "__main__":
    sys.exit(main())
