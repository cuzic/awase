#!/usr/bin/env python3
"""ADR-191 09-T4(BUG-020)の観測: 明示意図の OFF 直後、drift correction が実 IME を不適切に ON へ
戻す/固定するかを見る(`typing_stress --mode=drift` のログ)。

手順(1試行): IME を ON にそろえる(`drift_pre`)→ `off_vk` を単発で押す → +100/+400/+1500ms の
`real_ime_open`(`ImmGetOpenStatus`)を `drift_check` として記録。期待は「OFF 後は速やかに閉じ、
以後も閉じたまま」。1試行につき2種類の失敗を区別する:
  reverted     いったん閉じた後、再び開いた(drift correction が意図せず ON へ戻した、BUG-020型の候補)
  never_closed どのチェックポイントでも一度も閉じなかった(OFF 自体が効いていない。reverted とは
               閉→開の遷移が無い点で違うが、放置すると verdict=PASS のまま隠れるため同じく FAIL に数える)
まだ「(BUG-020を)作れた」と断定はしない(他要因〈IME側の遅延等〉の可能性は残る)。

判定は現時点では下限の観測用(exit codeは目安、CI の expect は 'observe' で判定には使わない)。
使い方: check_drift_correction.py [--json out.json] <typing_stress.log> <awase.log>
終了コード: 0=全試行で OFF 後 ON への復帰・OFF不発なし / 1=いずれかあり / 3=INVALID(実行できなかった) / 2=使い方の誤り
"""
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from check_typing_stress import parse  # noqa: E402  (typing_stress.log の [TS-JSON] 行パーサを共有)

DRIFT_LOG_PATTERN = re.compile(r"Blacklist drift correction: apply_ime_open\((\w+)\) → (\S+)")


def parse_awase_drift_lines(path: str) -> list:
    """awase.log から `Blacklist drift correction` 行を抜き出す(発火の有無・回数のみ、時刻突合せはしない)。"""
    out = []
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                m = DRIFT_LOG_PATTERN.search(line)
                if m:
                    out.append({"desired": m.group(1), "outcome": m.group(2)})
    except OSError:
        pass
    return out


def analyze(recs: list, drift_log_lines: list) -> dict:
    cfg = next((r for r in recs if r.get("type") == "config"), {})
    aborts = [r["reason"] for r in recs if r.get("type") == "abort"]
    done = any(r.get("type") == "done" for r in recs)
    pre = {r["n"]: r for r in recs if r.get("type") == "drift_pre"}
    checks = [r for r in recs if r.get("type") == "drift_check"]
    by_trial = {}
    for r in checks:
        by_trial.setdefault(r["n"], []).append(r)
    trials = []
    reverted = 0
    never_closed = 0
    for n in sorted(by_trial):
        cps = sorted(by_trial[n], key=lambda r: r["checkpoint_ms"])
        # OFF後、いったん閉じた(False)後にもう一度開いた(True)チェックポイントがあれば「復帰」とみなす。
        # 逆に、どのチェックポイントでも一度も閉じなかった(OFFがそもそも効かなかった)場合は別に数える
        # ("復帰"と違い、閉→開の遷移が無いので trial_reverted では検出できない。放置すると、OFF が
        # 全く効かない壊れ方〈まさに BUG-020 型を含む〉が verdict=PASS のまま隠れる)。
        seen_closed = False
        trial_reverted = False
        for cp in cps:
            if cp["real_ime_open"] is False:
                seen_closed = True
            elif cp["real_ime_open"] is True and seen_closed:
                trial_reverted = True
        trial_never_closed = bool(cps) and not seen_closed
        if trial_reverted:
            reverted += 1
        if trial_never_closed:
            never_closed += 1
        trials.append({
            "n": n, "pre": pre.get(n), "checkpoints": cps,
            "reverted": trial_reverted, "never_closed": trial_never_closed,
        })
    invalid = []
    if aborts:
        invalid.append("中断: " + "; ".join(aborts))
    if not done and not aborts:
        invalid.append("完走マーカー(done)が無い")
    if not trials:
        invalid.append("試行が0件")
    if invalid:
        verdict = "INVALID"
    elif reverted or never_closed:
        verdict = "FAIL"
    else:
        verdict = "PASS"
    return {
        "verdict": verdict, "cfg": cfg, "trials": trials, "invalid": invalid,
        "n_trials": len(trials), "n_reverted": reverted, "n_never_closed": never_closed,
        "drift_log_fired": len(drift_log_lines),
    }


def summary_line(r: dict) -> str:
    c = r["cfg"]
    return (
        f"DRIFT_CORRECTION: verdict={r['verdict']} form={c.get('form', '?')} ime={c.get('ime', '?')} "
        f"trials={r['n_trials']} reverted_to_on={r['n_reverted']} never_closed={r['n_never_closed']} "
        f"drift_log_fired={r['drift_log_fired']}"
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
        print("DRIFT_CORRECTION: verdict=INVALID reason=no-log")
        return 3
    drift_log_lines = parse_awase_drift_lines(args[1])
    r = analyze(recs, drift_log_lines)
    cfg = r["cfg"]
    print(f"入力先={cfg.get('form')} IME={cfg.get('ime')} mode={cfg.get('mode')}")
    for t in r["trials"]:
        pre_open = t["pre"].get("real_ime_open") if t["pre"] else None
        cps = " ".join(f"+{c['checkpoint_ms']}ms={c['real_ime_open']}" for c in t["checkpoints"])
        if t["reverted"]:
            tag = "復帰あり(BUG-020型の候補)"
        elif t["never_closed"]:
            tag = "一度も閉じなかった(OFFが効いていない)"
        else:
            tag = "-"
        print(f"  試行#{t['n']:>2} OFF前={pre_open} {cps}  {tag}")
    for x in r["invalid"]:
        print(f"  INVALID: {x}")
    print(f"awase.log の drift correction 発火行数: {r['drift_log_fired']}(参考値、試行との時刻突合せはしていない)")
    line = summary_line(r)
    print(line)
    if json_out:
        with open(json_out, "w", encoding="utf-8") as f:
            json.dump({"verdict": r["verdict"], "cfg": cfg, "n_trials": r["n_trials"],
                       "n_reverted": r["n_reverted"], "n_never_closed": r["n_never_closed"],
                       "drift_log_fired": r["drift_log_fired"],
                       "line": line, "invalid": r["invalid"]}, f, ensure_ascii=False)
    return {"PASS": 0, "FAIL": 1, "INVALID": 3}[r["verdict"]]


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
