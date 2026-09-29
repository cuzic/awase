#!/usr/bin/env python3
"""ADR-178 領域A撤去後の観測: reassert/force-on を撤去した現在、drift correction「だけ」で ON 回復が働くか
(`typing_stress --mode=drift-on` のログ)。

手順(1試行): IME を ON にそろえる(`drift_on_pre`)→ ハーネスが実 IME を直接閉じる(`drift_on_close`、awase を
経由しない「ずれ」)→ +500/+1500/+3000ms の `real_ime_open`(`drift_on_check`)→ かな単打を1回打って確定した
テキスト(`drift_on_typed`)。API の開閉表示だけでなく、実際に打った結果で ON/OFF を確認する。

試行の分類:
  invalid      前提が成立しない(pre が True でない / close 直後に閉じていない=ずれを作れていない / 全チェック不能)
  recovered    いずれかのチェックポイントで実 IME が再び開き、かつ打鍵結果が期待どおりのかな
  api_only     実 IME は開いたが打鍵結果が期待と違う(API 表示と実タイピングの食い違い)
  not_recovered 3秒間閉じたまま(drift correction では戻らない)
判定は観測用(CI の expect は 'observe')。撤去した経路の代替として drift correction が十分かの材料にする。
使い方: check_drift_recovery.py [--json out.json] <typing_stress.log> <awase.log>
終了コード: 0=有効試行が全て recovered / 1=recovered でない試行あり / 3=INVALID / 2=使い方の誤り
"""
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from check_typing_stress import parse  # noqa: E402

DRIFT_LOG_PATTERNS = (
    re.compile(r"Blacklist drift correction: apply_ime_open\((\w+)\)"),
    re.compile(r"\[drift\] correction:"),
)


def count_awase_drift_lines(path: str) -> int:
    n = 0
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                if any(p.search(line) for p in DRIFT_LOG_PATTERNS):
                    n += 1
    except OSError:
        pass
    return n


def analyze(recs: list, drift_lines: int) -> dict:
    cfg = next((r for r in recs if r.get("type") == "config"), {})
    aborts = [r["reason"] for r in recs if r.get("type") == "abort"]
    done = any(r.get("type") == "done" for r in recs)

    def by_n(t):
        d = {}
        for r in recs:
            if r.get("type") == t:
                d.setdefault(r["n"], []).append(r)
        return d

    pre, close, checks, typed = by_n("drift_on_pre"), by_n("drift_on_close"), by_n("drift_on_check"), by_n("drift_on_typed")
    ns = sorted(set(pre) | set(close) | set(checks) | set(typed))
    trials = []
    counts = {"recovered": 0, "api_only": 0, "not_recovered": 0, "invalid": 0}
    for n in ns:
        p = pre.get(n, [{}])[0].get("real_ime_open")
        c = close.get(n, [{}])[0]
        cps = sorted(checks.get(n, []), key=lambda r: r["checkpoint_ms"])
        t = typed.get(n, [{}])[0]
        reason = None
        if p is not True:
            reason = f"ON前提が未成立(pre_open={p})"
        elif c.get("real_ime_open") is not False:
            reason = f"ずれを作れていない(close 直後 open={c.get('real_ime_open')}, set_ret={c.get('set_ret')})"
        elif not cps or all(x["real_ime_open"] is None for x in cps):
            reason = "全チェックポイントが読み取り不能"
        elif not t:
            reason = "打鍵確認の記録が無い"
        if reason:
            kind = "invalid"
        else:
            opened = [x["checkpoint_ms"] for x in cps if x["real_ime_open"] is True]
            if not opened:
                kind = "not_recovered"
            elif t.get("ok"):
                kind = "recovered"
            else:
                kind = "api_only"
        counts[kind] += 1
        trials.append({"n": n, "kind": kind, "reason": reason, "checks": cps, "typed": t,
                       "first_open_ms": next((x["checkpoint_ms"] for x in cps if x["real_ime_open"] is True), None)})
    invalid = []
    if aborts:
        invalid.append("中断: " + "; ".join(aborts))
    if not done and not aborts:
        invalid.append("完走マーカー(done)が無い")
    if not trials:
        invalid.append("試行が0件")
    elif counts["invalid"] == len(trials):
        invalid.append(f"全 {len(trials)} 試行が前提未成立でINVALID")
    if invalid:
        verdict = "INVALID"
    elif counts["recovered"] == len(trials) - counts["invalid"]:
        verdict = "RECOVERED"
    elif counts["recovered"] == 0:
        verdict = "NOT_RECOVERED"
    else:
        verdict = "PARTIAL"
    return {"verdict": verdict, "cfg": cfg, "trials": trials, "counts": counts, "invalid": invalid,
            "drift_log_fired": drift_lines}


def summary_line(r: dict) -> str:
    c, k = r["cfg"], r["counts"]
    return (
        f"DRIFT_RECOVERY: verdict={r['verdict']} form={c.get('form', '?')} ime={c.get('ime', '?')} "
        f"trials={len(r['trials'])} recovered={k['recovered']} api_only={k['api_only']} "
        f"not_recovered={k['not_recovered']} invalid_trials={k['invalid']} drift_log_fired={r['drift_log_fired']}"
    )


def main(argv) -> int:
    args = list(argv)
    json_out = None
    if "--json" in args:
        i = args.index("--json")
        if i + 1 >= len(args):
            print(__doc__)
            return 2
        json_out = args[i + 1]
        del args[i:i + 2]
    if len(args) != 2:
        print(__doc__)
        return 2
    try:
        recs = parse(args[0])
    except OSError as e:
        print(f"typing_stress.log を読めない: {e}")
        print("DRIFT_RECOVERY: verdict=INVALID reason=no-log")
        return 3
    r = analyze(recs, count_awase_drift_lines(args[1]))
    cfg = r["cfg"]
    print(f"入力先={cfg.get('form')} IME={cfg.get('ime')} mode={cfg.get('mode')}")
    for t in r["trials"]:
        cps = " ".join(f"+{c['checkpoint_ms']}ms={c['real_ime_open']}" for c in t["checks"])
        typed = t["typed"].get("text") if t["typed"] else None
        tag = f"INVALID({t['reason']})" if t["reason"] else t["kind"]
        print(f"  試行#{t['n']:>2} {cps} 打鍵結果={typed!r}  {tag}")
    for x in r["invalid"]:
        print(f"  INVALID: {x}")
    print(f"awase.log の drift correction 発火行数: {r['drift_log_fired']}(参考値、試行との時刻突合せはしていない)")
    line = summary_line(r)
    print(line)
    if json_out:
        with open(json_out, "w", encoding="utf-8") as f:
            json.dump({"verdict": r["verdict"], "cfg": cfg, "counts": r["counts"], "line": line,
                       "invalid": r["invalid"]}, f, ensure_ascii=False)
    return {"RECOVERED": 0, "PARTIAL": 1, "NOT_RECOVERED": 1, "INVALID": 3}[r["verdict"]]


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
