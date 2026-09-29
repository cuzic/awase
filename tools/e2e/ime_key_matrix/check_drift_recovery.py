#!/usr/bin/env python3
"""ADR-178 領域A撤去後の観測: 外部から閉じられた IME を、awase が「観測して」「ON へ戻すか」
(`typing_stress --mode=drift-on` のログ)。

手順(1試行): VK_IME_ON で IME を ON にそろえる(`drift_on_pre`、awase の明示意図 ON になる)→ ハーネスが
実 IME を直接閉じる(`drift_on_close`、awase を経由しない「ずれ」)→ +500/+1500/+3000ms の `real_ime_open`
(`drift_on_check`)→ かな単打を1回打って確定したテキスト(`drift_on_typed`)。

**この測定が答えるのは「awase がずれを検知して戻すか」であって「drift correction の判断が戻さない」ではない**。
run 36508587614 の解析(Opus レビュー)で、TsfNative(tsf)では `reschedule_ime_refresh` がポーリングを予約せず、
閉じた後 awase は一度も IME を読まないことが分かった。そこで awase.log から、閉じてから打鍵までの時間窓(試行の
`drift_on_close.utc`〜`drift_on_typed.utc`)内の次を数える:
  observed  `[stage-observe] observer_poll=Some` / `ObserverReported`(awase が IME 状態を新たに観測した回数。0 なら
            drift correction の判断まで届いていない=「戻さない」とは言えない)
  drift     `[drift] correction` / `Blacklist drift correction`(drift correction が発火した回数)
  reinit    `giving up` / `GJI reinit` / `VK_IME_ON 送信`(打鍵時の literal 回収→GJI reinit=別経路の書き込みで開け直す)
  unicode   `send_keys: mode=Unicode`(Unicode 注入では IME が閉じていても文字が入る。打鍵結果は IME 状態の証拠にならない)

試行の分類:
  invalid            前提が成立しない(pre が True でない / close 直後に閉じていない / 全チェック不能 /
                     打鍵前にフォーカスが外れた / 打鍵記録なし)
  recovered          最後のチェックポイントで API 上も開いており、実打鍵もかな(打鍵より前に開いていた)
  reopened_by_typing 3秒間は閉じたまま、打鍵後に API 上開いていて実打鍵もかな(打鍵時の別経路=reinit 等が開け直した)
  typed_blind        実打鍵はかなだが Unicode 注入の窓なので IME 状態の証拠にならない(閉じたまま/開いたまま問わず)
  api_only           API 上は開いたが実打鍵が期待と違う
  not_recovered      API 上も閉じたままで、実打鍵も期待と違う(生ローマ字等)
verdict: 有効試行の全てが recovered=RECOVERED / 実打鍵がかなの有効試行が1件も無い=NOT_RECOVERED / それ以外=UNDETERMINED
(reopened_by_typing・typed_blind・api_only が混じる場合を含む)。observed=0 のときは NOT_RECOVERED でも
「awase は観測していない」ことが本質で、drift correction の判断を評価したことにはならない(summary の observed 列で読む)。
判定は観測用(CI の expect は 'observe')。
使い方: check_drift_recovery.py [--json out.json] <typing_stress.log> <awase.log>
終了コード: 0=RECOVERED / 1=それ以外 / 3=INVALID / 2=使い方の誤り
"""
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from check_typing_stress import parse  # noqa: E402

TIME = re.compile(r"T(\d\d:\d\d:\d\d\.\d{3})")
PATTERNS = {
    "observed": re.compile(r"\[stage-observe\] observer_poll=Some|ObserverReported"),
    "drift": re.compile(r"\[drift\] correction:|Blacklist drift correction: apply_ime_open"),
    "reinit": re.compile(r"giving up|GJI reinit|VK_IME_ON 送信"),
    "unicode": re.compile(r"send_keys: mode=Unicode"),
}
INTENT = re.compile(r"explicit_intent=(\S+)")


def load_awase(path: str) -> list:
    """awase.log を (HH:MM:SS.mmm, 行) の列にする。読めなければ空。"""
    out = []
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                m = TIME.search(line[:40])
                if m:
                    out.append((m.group(1), line))
    except OSError:
        pass
    return out


def window_counts(lines: list, t0, t1) -> dict:
    c = {k: 0 for k in PATTERNS}
    if not t0 or not t1:
        return c
    for ts, line in lines:
        if t0 <= ts <= t1:
            for k, pat in PATTERNS.items():
                if pat.search(line):
                    c[k] += 1
    return c


def intent_before(lines: list, t0):
    """t0 より前で最後に出た `explicit_intent=` の値(無ければ None)。"""
    last = None
    for ts, line in lines:
        if t0 and ts > t0:
            break
        m = INTENT.search(line)
        if m:
            last = m.group(1)
    return last


def analyze(recs: list, lines: list) -> dict:
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
    counts = {"recovered": 0, "reopened_by_typing": 0, "typed_blind": 0, "api_only": 0, "not_recovered": 0, "invalid": 0}
    win = {k: 0 for k in PATTERNS}
    intent_true = 0
    for n in ns:
        pr = pre.get(n, [{}])[0]
        c = close.get(n, [{}])[0]
        cps = sorted(checks.get(n, []), key=lambda r: r.get("checkpoint_ms", 0))
        t = typed.get(n, [{}])[0]
        w = window_counts(lines, c.get("utc"), t.get("utc"))
        it = intent_before(lines, c.get("utc"))
        if it == "Some(true)":
            intent_true += 1
        for k in win:
            win[k] += w[k]
        reason = None
        if pr.get("real_ime_open") is not True:
            reason = f"ON前提が未成立(pre_open={pr.get('real_ime_open')})"
        elif c.get("real_ime_open") is not False:
            reason = f"ずれを作れていない(close 直後 open={c.get('real_ime_open')}, set_ret={c.get('set_ret')})"
        elif not cps or all(x.get("real_ime_open") is None for x in cps):
            reason = "全チェックポイントが読み取り不能"
        elif not t:
            reason = "打鍵確認の記録が無い"
        elif t.get("focus_lost"):
            reason = "打鍵前にフォーカスが外れた"
        last = next((x.get("real_ime_open") for x in reversed(cps) if x.get("real_ime_open") is not None), None)
        if reason:
            kind = "invalid"
        elif not t.get("ok"):
            kind = "api_only" if last is True else "not_recovered"
        elif w["unicode"] > 0:
            kind = "typed_blind"
        elif last is True:
            kind = "recovered"
        elif t.get("real_ime_open") is True:
            kind = "reopened_by_typing"
        else:
            kind = "typed_blind"  # 打鍵はかなだが API は最後まで閉(状態の証拠にならない)
        counts[kind] += 1
        trials.append({"n": n, "kind": kind, "reason": reason, "checks": cps, "typed": t, "window": w,
                       "intent_before_close": it, "on_key": pr.get("on_key")})
    invalid = []
    if aborts:
        invalid.append("中断: " + "; ".join(aborts))
    if not done and not aborts:
        invalid.append("完走マーカー(done)が無い")
    if not trials:
        invalid.append("試行が0件")
    elif counts["invalid"] == len(trials):
        invalid.append(f"全 {len(trials)} 試行が前提未成立でINVALID")
    valid = len(trials) - counts["invalid"]
    typed_ok = counts["recovered"] + counts["reopened_by_typing"] + counts["typed_blind"]
    if invalid:
        verdict = "INVALID"
    elif counts["recovered"] == valid:
        verdict = "RECOVERED"
    elif typed_ok == 0:
        verdict = "NOT_RECOVERED"
    else:
        verdict = "UNDETERMINED"
    return {"verdict": verdict, "cfg": cfg, "trials": trials, "counts": counts, "invalid": invalid,
            "window": win, "intent_true": intent_true}


def summary_line(r: dict) -> str:
    c, k, w = r["cfg"], r["counts"], r["window"]
    return (
        f"DRIFT_RECOVERY: verdict={r['verdict']} form={c.get('form', '?')} ime={c.get('ime', '?')} "
        f"trials={len(r['trials'])} recovered={k['recovered']} reopened_by_typing={k['reopened_by_typing']} "
        f"typed_blind={k['typed_blind']} api_only={k['api_only']} not_recovered={k['not_recovered']} "
        f"invalid_trials={k['invalid']} observed={w['observed']} drift={w['drift']} reinit={w['reinit']} "
        f"unicode={w['unicode']} intent_true={r['intent_true']}"
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
        print("DRIFT_RECOVERY: verdict=INVALID form=? ime=? trials=0 recovered=0 reopened_by_typing=0 typed_blind=0 "
              "api_only=0 not_recovered=0 invalid_trials=0 observed=0 drift=0 reinit=0 unicode=0 intent_true=0")
        return 3
    r = analyze(recs, load_awase(args[1]))
    cfg = r["cfg"]
    print(f"入力先={cfg.get('form')} IME={cfg.get('ime')} mode={cfg.get('mode')}")
    for t in r["trials"]:
        cps = " ".join(f"+{c['checkpoint_ms']}ms={c.get('real_ime_open')}" for c in t["checks"])
        typed = t["typed"].get("text") if t["typed"] else None
        tag = f"INVALID({t['reason']})" if t["reason"] else t["kind"]
        w = t["window"]
        print(f"  試行#{t['n']:>2} on_key={t['on_key']} intent={t['intent_before_close']} {cps} 打鍵結果={typed!r} "
              f"窓内[観測={w['observed']} drift={w['drift']} reinit={w['reinit']} unicode={w['unicode']}]  {tag}")
    for x in r["invalid"]:
        print(f"  INVALID: {x}")
    line = summary_line(r)
    print(line)
    if json_out:
        with open(json_out, "w", encoding="utf-8") as f:
            json.dump({"verdict": r["verdict"], "cfg": cfg, "counts": r["counts"], "window": r["window"],
                       "line": line, "invalid": r["invalid"]}, f, ensure_ascii=False)
    return {"RECOVERED": 0, "UNDETERMINED": 1, "NOT_RECOVERED": 1, "INVALID": 3}[r["verdict"]]


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
