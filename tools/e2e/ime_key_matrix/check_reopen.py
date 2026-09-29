#!/usr/bin/env python3
"""ADR-203 e2e (c) / BUG-170 の実機確認: 「OFF 前に1語確定 → 物理 OFF → 1秒以内に物理 ON → 即打鍵」
(`typing_stress --mode=reopen` のログ)の判定。

1試行: `reopen_pre`(OFF 前の1語を確定した入力先のテキスト)→ `reopen_on`(物理 OFF→gap→物理 ON、`on_utc` が ON キー押下)→
`reopen_typed`(ON 直後の1語を確定した入力先のテキスト)。awase.log は ON キー押下〜確定後(`on_utc`〜`reopen_typed.utc`)の窓で数える:
  stuck        `[gji-fsm] StartComposition while engine off`(GjiFsm が OffCold に固着したまま候補窓が出た。BUG-170 の起点)
  stale_escape `StaleConfirm` を含む行のうち `escape=true`(途中の語の未確定文字まで VK_ESCAPE で消す。BUG-171)
  reopen_sync  GjiFsm へ ON を同期した遷移(`trigger="Reopen(` / `trigger="ImeOn(`)。ADR-203 (ii) が働いた証拠
  vk_cold / vk_warm  `[vk-send] … prepend_f2_warmup=true|false` の件数(true=cold 経路)
情報として出す(合否には使わない): ON キー押下から最初の `[vk-send]` までの遅延(ms。ADR-203 D2「ON キー単独タップ直後の遅延」の再測定)と、
最初の `[vk-send]` が cold 経路か(BUG-170 の想定は cold)。cold 経路の期待は実測前に決め打たない(tuning-constants の方針)。

試行の分類:
  invalid  前提が成立しない(OFF 前の語が期待どおりでない=ON にそろっていない / 打鍵前にフォーカスが外れた / 記録欠け / 時間窓が逆転)
  pass     ON 直後の語が期待どおりで、stuck=0 かつ stale_escape=0
  fail     ON 直後の語が期待と違う(欠落・リテラル化・入れ替わり)、または stuck>0 / stale_escape>0
verdict: INVALID(有効試行が半数未満・中断・完走マーカー無し) / FAIL(有効試行に fail がある) / PASS。
使い方: check_reopen.py [--json out.json] <typing_stress.log> <awase.log>
終了コード: 0=PASS / 1=FAIL / 3=INVALID / 2=使い方の誤り
"""
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from check_typing_stress import parse, normalize  # noqa: E402

TIME = re.compile(r"T(\d\d:\d\d:\d\d\.\d{3})")
PATTERNS = {
    "stuck": re.compile(r"\[gji-fsm\] StartComposition while engine off"),
    "stale_escape": re.compile(r"StaleConfirm.*escape=true|escape=true.*StaleConfirm"),
    "reopen_sync": re.compile(r'trigger="(Reopen|ImeOn)\('),
}
VK_SEND = re.compile(r"\[vk-send\] .*prepend_f2_warmup=(true|false)")


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


def ms_of(hms: str) -> int:
    h, m, rest = hms.split(":")
    s, ms = rest.split(".")
    return ((int(h) * 60 + int(m)) * 60 + int(s)) * 1000 + int(ms)


def window(lines: list, t0: str, t1: str) -> dict:
    c = {k: 0 for k in PATTERNS}
    vk = []  # (ts, cold)
    for ts, line in lines:
        if not (t0 <= ts <= t1):
            continue
        for k, p in PATTERNS.items():
            if p.search(line):
                c[k] += 1
        m = VK_SEND.search(line)
        if m:
            vk.append((ts, m.group(1) == "true"))
    c["vk_cold"] = sum(1 for _, cold in vk if cold)
    c["vk_warm"] = sum(1 for _, cold in vk if not cold)
    c["first_vk_cold"] = vk[0][1] if vk else None
    c["first_vk_delay_ms"] = ms_of(vk[0][0]) - ms_of(t0) if vk else None
    return c


def analyze(recs: list, lines: list) -> dict:
    cfg = next((r for r in recs if r.get("type") == "config"), {})
    aborts = [r["reason"] for r in recs if r.get("type") == "abort"]
    done = any(r.get("type") == "done" for r in recs)

    def by_n(t):
        return {r["n"]: r for r in recs if r.get("type") == t}

    pre, on, typed = by_n("reopen_pre"), by_n("reopen_on"), by_n("reopen_typed")
    trials, counts = [], {"pass": 0, "fail": 0, "invalid": 0}
    delays = []
    for n in sorted(set(pre) | set(on) | set(typed)):
        p, o, t = pre.get(n), on.get(n), typed.get(n)
        reason = None
        if not (p and o and t):
            reason = "記録欠け(pre/on/typed のいずれかが無い)"
        elif not p.get("ok"):
            reason = f"OFF 前の語が期待どおりでない(text={p.get('text')!r})=IME が ON にそろっていない"
        elif t.get("focus_lost"):
            reason = "打鍵前にフォーカスが外れた"
        elif t["utc"] < o["on_utc"]:
            reason = "時間窓が逆転(UTC 日付をまたいだ?)"
        row = {"n": n}
        if reason:
            row.update(status="invalid", why=reason)
            counts["invalid"] += 1
            trials.append(row)
            continue
        w = window(lines, o["on_utc"], t["utc"])
        why = []
        if not t.get("ok"):
            why.append(f"ON 直後の語が期待と違う(expect={t.get('expect')!r} actual={normalize(t.get('text', ''))!r})")
        if w["stuck"]:
            why.append(f"GjiFsm が OffCold に固着({w['stuck']}件)")
        if w["stale_escape"]:
            why.append(f"StaleConfirm の escape=true({w['stale_escape']}件)")
        row.update(w)
        row.update(status="fail" if why else "pass", why="; ".join(why))
        counts[row["status"]] += 1
        if w["first_vk_delay_ms"] is not None:
            delays.append(w["first_vk_delay_ms"])
        trials.append(row)
    valid = counts["pass"] + counts["fail"]
    invalid_run = []
    if aborts:
        invalid_run.append("中断: " + "; ".join(aborts))
    if not done and not aborts:
        invalid_run.append("完走マーカー(done)が無い")
    if not trials:
        invalid_run.append("試行が0件")
    elif valid * 2 < len(trials):
        invalid_run.append(f"有効試行が半数未満({valid}/{len(trials)})")
    if invalid_run:
        verdict = "INVALID"
    elif counts["fail"]:
        verdict = "FAIL"
    else:
        verdict = "PASS"
    delays.sort()
    first_cold = sum(1 for r in trials if r.get("first_vk_cold") is True)
    return {"verdict": verdict, "invalid_reasons": invalid_run, "counts": counts, "trials": trials,
            "form": cfg.get("form"), "ime": cfg.get("ime"), "first_vk_cold": first_cold,
            "delay_p50_ms": delays[len(delays) // 2] if delays else None,
            "delay_max_ms": delays[-1] if delays else None}


def main() -> int:
    args = [a for a in sys.argv[1:] if not a.startswith("--json")]
    json_path = None
    if "--json" in sys.argv[1:]:
        i = sys.argv.index("--json")
        json_path = sys.argv[i + 1]
        args = [a for a in args if a != json_path]
    if len(args) != 2:
        print(__doc__)
        return 2
    res = analyze(parse(args[0]), load_awase(args[1]))
    for r in res["trials"]:
        if r["status"] == "invalid":
            print(f"{r['n']}: INVALID {r['why']}")
        else:
            d = r["first_vk_delay_ms"]
            print(f"{r['n']}: {r['status'].upper()} stuck={r['stuck']} stale_escape={r['stale_escape']} reopen_sync={r['reopen_sync']} "
                  f"vk cold/warm={r['vk_cold']}/{r['vk_warm']} first_vk={'-' if d is None else str(d) + 'ms'}"
                  + (f" ← {r['why']}" if r["why"] else ""))
    for why in res["invalid_reasons"]:
        print("INVALID:", why)
    c = res["counts"]
    print(f"REOPEN: verdict={res['verdict']} form={res['form']} ime={res['ime']} pass={c['pass']} fail={c['fail']} invalid={c['invalid']} "
          f"first_vk_cold={res['first_vk_cold']} delay_p50_ms={res['delay_p50_ms']} delay_max_ms={res['delay_max_ms']}")
    if json_path:
        with open(json_path, "w", encoding="utf-8") as f:
            json.dump(res, f, ensure_ascii=False, indent=1)
    return {"PASS": 0, "FAIL": 1, "INVALID": 3}[res["verdict"]]


if __name__ == "__main__":
    sys.exit(main())
