#!/usr/bin/env python3
"""ADR-178 領域A撤去後の観測: 外部から閉じられた IME を、awase が「観測して」「ON へ戻すか」
(`typing_stress --mode=drift-on` のログ)。

手順(1試行): VK_IME_ON で IME を ON にそろえる(`drift_on_pre`、awase の明示意図 ON になる)→ ハーネスが
実 IME を直接閉じる(`drift_on_close`、awase を経由しない「ずれ」)→ +500/+1500/+3000ms の `real_ime_open`
(`drift_on_check`)→ かな単打を1回打って確定したテキスト(`drift_on_typed`)。

**この測定が答えるのは「awase がずれを観測して戻すか」であって「drift correction の判断が戻さない」ではない**。
run 36508587614 の解析(Opus レビュー)で、TsfNative(tsf)ではポーリングが止まり、閉じた後 awase は ImeModel へ開閉を
観測しないことが分かった(`runtime/mod.rs` reschedule_ime_refresh。明示意図があるときも止まる)。そこで awase.log から、
2つの時間窓の中で次を数える。窓は `drift_on_close.utc`〜`drift_on_typed.press_utc`(閉じてから打鍵直前まで)と
`press_utc`〜`drift_on_typed.utc`(打鍵中〜確定後):
  observed  [閉→打鍵直前] `[stage-observe] observer_poll=Some` / `ObserverReported`(ImeModel への開閉観測。0 なら
            drift correction の判断まで届いていない。打鍵時に conv だけを読む送信前チェックは含まない)
  drift     [閉→打鍵直前] `[drift] correction` / `Blacklist drift correction`(drift correction の発火)
  conv_read [打鍵中] `idle-conv-check-diag` / `kind=probe`(打鍵時に awase が conv を読んだ。tsf×MS-IME では conv の NATIVE を
            「ON 確認」と誤認して閉じた IME へ送った、と run 36510380572 で確認)
  reinit    [打鍵中] `giving up` / `GJI reinit` / `VK_IME_ON 送信`(literal 回収→GJI reinit=別経路の書き込み)
  unicode   [打鍵中] `send_keys: mode=Unicode`(Unicode 注入では IME が閉じていても文字が入る。edit は ON キーと無関係に
            常に Unicode 注入。打鍵結果は IME 状態の証拠にならない)
summary の各列は「その現象があった試行数」(行数ではない)。時刻は HH:MM:SS.mmm の文字列比較なので、UTC 日付をまたぐ run では
窓が逆転する。逆転した試行は invalid にする。

試行の分類:
  invalid            前提が成立しない(pre が True でない / on_key が VK_IME_ON(0x16)でない / ON 操作〜close の間の
                     最後の explicit_intent が Some(true) でない / close 直後に閉じていない / 全チェック不能 /
                     打鍵前にフォーカスが外れた / 打鍵記録なし / 時間窓が逆転)
  recovered          最後のチェックポイントで API 上も開いており、実打鍵もかな(打鍵より前に開いていた)
  reopened_by_typing 3秒間は閉じたまま、打鍵後に API 上開いていて実打鍵もかな(打鍵時の別経路=reinit 等が開け直した)
  typed_blind        実打鍵はかなだが Unicode 注入の窓なので IME 状態の証拠にならない
  unexplained        実打鍵はかなで Unicode でもないが、API は最後まで閉・打鍵後も閉(原因不明。旧 api_lies)
  api_only           API 上は開いたが実打鍵が期待と違う
  not_recovered      API 上も閉じたままで、実打鍵も期待と違う(生ローマ字等)
verdict: INVALID(有効試行が半数未満を含む) / RECOVERED(有効試行の全てが recovered) / REOPENED_BY_OTHER_PATH(有効試行の全てが
recovered か reopened_by_typing で、reopened_by_typing がある=drift correction 以外の経路が開け直した) /
NOT_OBSERVED(実打鍵がかなの試行が無く、observed=0 かつ drift=0=戻さないのではなく見ていない) /
NOT_RECOVERED(実打鍵がかなの試行が無く、observed>0 または drift>0) / UNDETERMINED(それ以外。typed_blind 等を含む)。
判定は観測用(CI の expect は 'observe')。

物理 Ctrl+無変換 変種(`typing_stress --drift-off-ctrl-muhenkan`、drift_on_close.method=ctrl_muhenkan): ハーネスが直接閉じる代わりに、
マーカー付き SendInput の Ctrl↓→無変換↓↑→Ctrl↑ で OFF にする(debug awase は物理 Ctrl+無変換として扱う)。ここでの「ずれ」は
「awase の OFF 操作のあとも実 IME が ON のまま」(close 直後の open=True)。試行の分類:
  invalid        前提不成立(上と同じ。さらに awase.log の `[engine-input] vk=0x1D KeyDown` が物理 Ctrl 付き=`mods(c=true` かつ
                 `phys_ctrl=true` でない、または1件も無い。phys_ctrl=false は注入が物理キー扱いされていない=INVALID)
  gap_not_made   close 直後に実 IME が閉じた(OFF 操作が効き、ずれは作れなかった)
  corrected      ずれができ、その後 ON のままではなくなった(最後のチェックポイントで閉、または実打鍵がかなでない)
  not_corrected  ずれができ、最後まで ON のままで実打鍵もかな(drift correction 等で OFF へ戻らない)
verdict: INVALID / GAP_NOT_MADE(ずれが1件も作れない) / CORRECTED(ずれた試行の全てが corrected) / NOT_CORRECTED(全て not_corrected) /
UNDETERMINED。summary 行には既存列(recovered=corrected, not_recovered=not_corrected)に加え method=ctrl_muhenkan gap_made gap_not_made
phys_ctrl_ok を付ける。
使い方: check_drift_recovery.py [--json out.json] <typing_stress.log> <awase.log>
終了コード: 0=RECOVERED / 1=それ以外 / 3=INVALID / 2=使い方の誤り
"""
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from e2e_common import (  # noqa: E402
    DRIFT_RE, OBSERVED_RE, PHYS_CTRL_RE, UNICODE_RE, load_awase_timed as load_awase, ts_json_records as parse,
)

PATTERNS = {
    "observed": OBSERVED_RE,
    "drift": DRIFT_RE,
    "conv_read": re.compile(r"idle-conv-check-diag|kind=probe"),
    "reinit": re.compile(r"giving up|GJI reinit|VK_IME_ON 送信"),
    "unicode": UNICODE_RE,
}
# 閉→打鍵直前の窓で数えるもの / 打鍵中〜確定後の窓で数えるもの
PRE_KEYS = ("observed", "drift")
TYPING_KEYS = ("conv_read", "reinit", "unicode")
INTENT = re.compile(r"explicit_intent=(\S+)")
MUHENKAN_DOWN = re.compile(r"\[engine-input\] vk=0x1D KeyDown")
PHYS_CTRL = PHYS_CTRL_RE


def window_counts(lines: list, t0, t1, keys) -> dict:
    c = {k: 0 for k in keys}
    if not t0 or not t1:
        return c
    for ts, line in lines:
        if t0 <= ts <= t1:
            for k in keys:
                if PATTERNS[k].search(line):
                    c[k] += 1
    return c


def intent_between(lines: list, lo, hi):
    """lo〜hi(同じ試行の ON 操作〜close)で最後に出た `explicit_intent=` の値。範囲内に無ければ None。"""
    if not lo or not hi:
        return None
    last = None
    for ts, line in lines:
        if ts > hi:
            break
        if ts >= lo:
            m = INTENT.search(line)
            if m:
                last = m.group(1)
    return last


def phys_ctrl_check(lines: list, t0, t1):
    """t0〜t1 の `[engine-input] vk=0x1D KeyDown` が全て物理 Ctrl 付きか。(件数, 物理 Ctrl 付きの件数)。"""
    n = ok = 0
    if t0 and t1:
        for ts, line in lines:
            if t0 <= ts <= t1 and MUHENKAN_DOWN.search(line):
                n += 1
                ok += bool(PHYS_CTRL.search(line))
    return n, ok


def analyze_ctrl(recs: list, lines: list) -> dict:
    """物理 Ctrl+無変換で OFF にした変種(docstring 参照)。"""
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
    counts = {"gap_not_made": 0, "corrected": 0, "not_corrected": 0, "invalid": 0}
    seen = {k: 0 for k in PATTERNS}
    trials, phys_ok = [], 0
    for n in ns:
        pr = pre.get(n, [{}])[0]
        c = close.get(n, [{}])[0]
        cps = sorted(checks.get(n, []), key=lambda r: r.get("checkpoint_ms", 0))
        t = typed.get(n, [{}])[0]
        t_close, t_press, t_end = c.get("utc"), t.get("press_utc"), t.get("utc")
        it = intent_between(lines, pr.get("on_utc"), t_close)
        w = window_counts(lines, t_close, t_press, PRE_KEYS)
        nmu, nok = phys_ctrl_check(lines, t_close, t_press)
        reason = None
        if pr.get("real_ime_open") is not True:
            reason = f"ON前提が未成立(pre_open={pr.get('real_ime_open')})"
        elif pr.get("on_key") != "0x16":
            reason = f"VK_IME_ON 以外で ON にした(on_key={pr.get('on_key')})"
        elif it != "Some(true)":
            reason = f"明示意図 ON を確認できない(ON操作〜close の最後の explicit_intent={it})"
        elif c.get("method") != "ctrl_muhenkan":
            reason = f"物理 Ctrl+無変換で OFF にしていない(method={c.get('method')})"
        elif c.get("real_ime_open") is None:
            reason = "close 直後の実 IME 状態を読めない"
        elif not cps or all(x.get("real_ime_open") is None for x in cps):
            reason = "全チェックポイントが読み取り不能"
        elif not t:
            reason = "打鍵確認の記録が無い"
        elif t.get("focus_lost"):
            reason = "打鍵前にフォーカスが外れた"
        elif not (t_close and t_press and t_end) or not (t_close <= t_press <= t_end):
            reason = "時間窓が不正(UTC 日付またぎ、または時刻の欠落)"
        elif nmu == 0:
            reason = "awase.log に Ctrl+無変換の engine-input が無い(注入が awase に届かない)"
        elif nok != nmu:
            reason = f"物理 Ctrl 扱いでない(vk=0x1D KeyDown {nmu} 件中 mods(c=true …) phys_ctrl=true は {nok} 件。前提不成立)"
        last = next((x.get("real_ime_open") for x in reversed(cps) if x.get("real_ime_open") is not None), None)
        if reason:
            kind = "invalid"
        else:
            phys_ok += 1
            if c.get("real_ime_open") is False:
                kind = "gap_not_made"
            else:
                for k in PRE_KEYS:
                    seen[k] += w[k] > 0
                kind = "corrected" if (last is False or not t.get("ok")) else "not_corrected"
        counts[kind] += 1
        trials.append({"n": n, "kind": kind, "reason": reason, "checks": cps, "typed": t, "window": w,
                       "intent": it, "on_key": pr.get("on_key"), "close_open": c.get("real_ime_open")})
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
    if not invalid and valid * 2 < len(trials):
        invalid.append(f"有効試行が半数未満({valid}/{len(trials)})")
    gap_made = counts["corrected"] + counts["not_corrected"]
    if invalid:
        verdict = "INVALID"
    elif gap_made == 0:
        verdict = "GAP_NOT_MADE"
    elif counts["not_corrected"] == 0:
        verdict = "CORRECTED"
    elif counts["corrected"] == 0:
        verdict = "NOT_CORRECTED"
    else:
        verdict = "UNDETERMINED"
    return {"verdict": verdict, "cfg": cfg, "trials": trials, "invalid": invalid, "seen": seen,
            "ctrl": True, "gap_made": gap_made, "gap_not_made": counts["gap_not_made"], "phys_ctrl_ok": phys_ok,
            "counts": {"recovered": counts["corrected"], "reopened_by_typing": 0, "typed_blind": 0, "unexplained": 0,
                       "api_only": 0, "not_recovered": counts["not_corrected"], "invalid": counts["invalid"]},
            "intent_true": sum(1 for x in trials if x["intent"] == "Some(true)")}


def analyze(recs: list, lines: list) -> dict:
    if any(r.get("type") == "drift_on_close" and r.get("method") == "ctrl_muhenkan" for r in recs):
        return analyze_ctrl(recs, lines)
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
    counts = {"recovered": 0, "reopened_by_typing": 0, "typed_blind": 0, "unexplained": 0, "api_only": 0,
              "not_recovered": 0, "invalid": 0}
    seen = {k: 0 for k in PATTERNS}  # その現象があった有効試行の数
    intent_true = 0
    for n in ns:
        pr = pre.get(n, [{}])[0]
        c = close.get(n, [{}])[0]
        cps = sorted(checks.get(n, []), key=lambda r: r.get("checkpoint_ms", 0))
        t = typed.get(n, [{}])[0]
        t_close, t_press, t_end = c.get("utc"), t.get("press_utc"), t.get("utc")
        w = window_counts(lines, t_close, t_press, PRE_KEYS)
        w.update(window_counts(lines, t_press, t_end, TYPING_KEYS))
        it = intent_between(lines, pr.get("on_utc"), t_close)
        reason = None
        if pr.get("real_ime_open") is not True:
            reason = f"ON前提が未成立(pre_open={pr.get('real_ime_open')})"
        elif pr.get("on_key") != "0x16":
            reason = f"VK_IME_ON 以外で ON にした(on_key={pr.get('on_key')}。明示意図が消えうる)"
        elif it != "Some(true)":
            reason = f"明示意図 ON を確認できない(ON操作〜close の最後の explicit_intent={it})"
        elif c.get("real_ime_open") is not False:
            reason = f"ずれを作れていない(close 直後 open={c.get('real_ime_open')}, set_ret={c.get('set_ret')})"
        elif not cps or all(x.get("real_ime_open") is None for x in cps):
            reason = "全チェックポイントが読み取り不能"
        elif not t:
            reason = "打鍵確認の記録が無い"
        elif t.get("focus_lost"):
            reason = "打鍵前にフォーカスが外れた"
        elif not (t_close and t_press and t_end) or not (t_close <= t_press <= t_end):
            reason = "時間窓が不正(UTC 日付またぎ、または時刻の欠落)"
        last = next((x.get("real_ime_open") for x in reversed(cps) if x.get("real_ime_open") is not None), None)
        if reason:
            kind = "invalid"
        else:
            intent_true += 1
            for k in PATTERNS:
                if w[k] > 0:
                    seen[k] += 1
            if not t.get("ok"):
                kind = "api_only" if last is True else "not_recovered"
            elif w["unicode"] > 0:
                kind = "typed_blind"
            elif last is True:
                kind = "recovered"
            elif t.get("real_ime_open") is True:
                kind = "reopened_by_typing"
            else:
                kind = "unexplained"
        counts[kind] += 1
        trials.append({"n": n, "kind": kind, "reason": reason, "checks": cps, "typed": t, "window": w,
                       "intent": it, "on_key": pr.get("on_key")})
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
    typed_ok = counts["recovered"] + counts["reopened_by_typing"] + counts["typed_blind"] + counts["unexplained"]
    if not invalid and valid * 2 < len(trials):
        invalid.append(f"有効試行が半数未満({valid}/{len(trials)})")
    if invalid:
        verdict = "INVALID"
    elif counts["recovered"] == valid:
        verdict = "RECOVERED"
    elif counts["recovered"] + counts["reopened_by_typing"] == valid:
        verdict = "REOPENED_BY_OTHER_PATH"
    elif typed_ok == 0:
        verdict = "NOT_OBSERVED" if seen["observed"] == 0 and seen["drift"] == 0 else "NOT_RECOVERED"
    else:
        verdict = "UNDETERMINED"
    return {"verdict": verdict, "cfg": cfg, "trials": trials, "counts": counts, "invalid": invalid,
            "seen": seen, "intent_true": intent_true}


def summary_line(r: dict) -> str:
    c, k, w = r["cfg"], r["counts"], r["seen"]
    extra = ""
    if r.get("ctrl"):
        extra = (f" method=ctrl_muhenkan gap_made={r['gap_made']} gap_not_made={r['gap_not_made']} "
                 f"phys_ctrl_ok={r['phys_ctrl_ok']}")
    return (
        f"DRIFT_RECOVERY: verdict={r['verdict']} form={c.get('form', '?')} ime={c.get('ime', '?')} "
        f"trials={len(r['trials'])} recovered={k['recovered']} reopened_by_typing={k['reopened_by_typing']} "
        f"typed_blind={k['typed_blind']} unexplained={k['unexplained']} api_only={k['api_only']} "
        f"not_recovered={k['not_recovered']} invalid_trials={k['invalid']} observed={w['observed']} "
        f"drift={w['drift']} conv_read={w['conv_read']} reinit={w['reinit']} unicode={w['unicode']} "
        f"intent_true={r['intent_true']}" + extra
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
              "unexplained=0 api_only=0 not_recovered=0 invalid_trials=0 observed=0 drift=0 conv_read=0 reinit=0 unicode=0 "
              "intent_true=0")
        return 3
    r = analyze(recs, load_awase(args[1]))
    cfg = r["cfg"]
    print(f"入力先={cfg.get('form')} IME={cfg.get('ime')} mode={cfg.get('mode')}")
    for t in r["trials"]:
        cps = " ".join(f"+{c['checkpoint_ms']}ms={c.get('real_ime_open')}" for c in t["checks"])
        typed = t["typed"].get("text") if t["typed"] else None
        tag = f"INVALID({t['reason']})" if t["reason"] else t["kind"]
        w = t["window"]
        print(f"  試行#{t['n']:>2} on_key={t['on_key']} intent={t['intent']} {cps} 打鍵結果={typed!r} "
              f"[観測={w['observed']} drift={w['drift']} | conv_read={w.get('conv_read', '-')} reinit={w.get('reinit', '-')} unicode={w.get('unicode', '-')}]  {tag}")
    for x in r["invalid"]:
        print(f"  INVALID: {x}")
    line = summary_line(r)
    print(line)
    if json_out:
        with open(json_out, "w", encoding="utf-8") as f:
            json.dump({"verdict": r["verdict"], "cfg": cfg, "counts": r["counts"], "seen": r["seen"],
                       "line": line, "invalid": r["invalid"]}, f, ensure_ascii=False)
    return {"CORRECTED": 0, "GAP_NOT_MADE": 1, "NOT_CORRECTED": 1, "RECOVERED": 0, "REOPENED_BY_OTHER_PATH": 1, "UNDETERMINED": 1, "NOT_RECOVERED": 1, "NOT_OBSERVED": 1, "INVALID": 3}[r["verdict"]]


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
