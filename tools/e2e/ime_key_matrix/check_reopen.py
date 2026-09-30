#!/usr/bin/env python3
"""ADR-203 e2e (c) / BUG-170 の実機確認: 「OFF 前に1語確定 → 物理 OFF → 1秒以内に物理 ON → 即打鍵」
(`typing_stress --mode=reopen` のログ)の判定。

1試行: `reopen_pre`(OFF 前の1語を確定した入力先のテキスト、`trial_utc`=試行の先頭)→ `reopen_on`(物理 OFF→gap→物理 ON。`off_utc`=OFF 押下、
`on_utc`=ON 押下、`open_before_on`=ON 直前の実 IME の開閉)→ `reopen_typed`(ON 直後の1語を確定した入力先のテキスト、`press_utc`=打鍵)。
awase.log は次の窓で数える(いずれも実ログの書式で照合する。書式はテストの入力にそのまま使っている):
  OFF 前の窓  [trial_utc, off_utc): 試行の先頭の turn_ime_on(OFF→ON)も「確定済みの語の後の開き直し」なので、ここでの固着も数える(review M5)
  ON 後の窓   [on_utc, reopen_typed.utc]
数えるもの:
  stuck         `[gji-fsm] StartComposition while engine off`(GjiFsm が OffCold に固着したまま候補窓が出た。BUG-170 の起点)
  stale_escape  `per-VK[i/n] stale confirm 検出 … escape=true`(途中の語の未確定文字まで VK_ESCAPE で消す。BUG-171)
  flush_escape  `[raw-tsf-literal] flush escape=true`(literal 回収が実際に ESC を送った)
  reopen_belief `trigger="Reopen(BeliefSync:`(ADR-203 (ii): 物理 ON キーで GjiFsm を OffCold→ON へ同期)
  imeon_belief  `trigger="ImeOn(BeliefSync:`(ADR-203 (i): 送信時の belief 起点の ON 同期)
  imeon_other   上記以外の `trigger="ImeOn(`(awase 自身の ON 操作など。(i)(ii) の証拠にはならない)
  unicode       `send_keys: mode=Unicode`(Unicode 注入では IME が閉じていても文字が入る。IME 状態の証拠にならない)
  vk_cold / vk_warm  `[vk-send] … prepend_f2_warmup=true|false`(true=cold 経路)
情報として出す: ON キー押下(press_utc)から最初の `[vk-send]` までの ms(NICOLA の同時打鍵待ちと打鍵の押下時間を含む)と、
その `[vk-send]` から `per-VK: 全 N VK 確認済み → セッション確認` までの ms(ADR-203 D2 の遅延の定義)。

試行の分類:
  invalid  前提が成立しない(記録欠け / フォーカス喪失 / 時間窓の逆転 / OFF 前の語が期待どおりでない(OFF 前の窓に固着が無いとき) /
           物理 OFF が効いていない(`open_before_on`=true))
  blind    ON 後の窓に Unicode 注入がある(IME 状態の証拠にならない。有効試行に数えない)
  pass     ON 後の語が期待どおり、固着・ESC なし、ON 後に実 IME が閉じていない、(--require-cold のとき)最初の語が cold 経路
  fail     上のどれかを満たさない
verdict: INVALID(有効試行が半数未満・中断・完走マーカー無し) / FAIL(有効試行に fail がある) / PASS。
--require-cold: ADR-203 (c) の PASS 条件「ON 後の最初の語が cold 経路(prepend_f2_warmup=true)」を合否に含める(GJI の TSF 系構成用)。
使い方: check_reopen.py [--require-cold] [--json out.json] <typing_stress.log> <awase.log>
終了コード: 0=PASS / 1=FAIL / 3=INVALID / 2=使い方の誤り
"""
import argparse
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from check_typing_stress import parse, normalize  # noqa: E402

TIME = re.compile(r"T(\d\d:\d\d:\d\d\.\d{3})")
PATTERNS = {
    "stuck": re.compile(r"\[gji-fsm\] StartComposition while engine off"),
    "stale_escape": re.compile(r"stale confirm 検出.*escape=true"),
    "flush_escape": re.compile(r"\[raw-tsf-literal\] flush escape=true"),
    "reopen_belief": re.compile(r'trigger="Reopen\(BeliefSync:'),
    "imeon_belief": re.compile(r'trigger="ImeOn\(BeliefSync:'),
    "imeon_other": re.compile(r'trigger="ImeOn\((?!BeliefSync:)'),
    "unicode": re.compile(r"send_keys: mode=Unicode"),
}
BAD = ("stuck", "stale_escape", "flush_escape")
VK_SEND = re.compile(r"\[vk-send\] .*prepend_f2_warmup=(true|false)")
CONFIRM = re.compile(r"per-VK: 全 \d+ VK 確認済み → セッション確認")


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


def count(lines: list, t0: str, t1: str) -> dict:
    c = {k: 0 for k in PATTERNS}
    for ts, line in lines:
        if t0 <= ts < t1:
            for k, p in PATTERNS.items():
                if p.search(line):
                    c[k] += 1
    return c


def post_window(lines: list, on_utc: str, press_utc: str, end_utc: str) -> dict:
    c = count(lines, on_utc, end_utc + "\x7f")  # 終端を含める(末尾の文字列比較用の番兵)
    vk = [(ts, "prepend_f2_warmup=true" in line) for ts, line in lines if on_utc <= ts <= end_utc and VK_SEND.search(line)]
    c["vk_cold"] = sum(1 for _, cold in vk if cold)
    c["vk_warm"] = sum(1 for _, cold in vk if not cold)
    c["first_vk_cold"] = vk[0][1] if vk else None
    after = [v for v in vk if v[0] >= press_utc]
    first = after[0] if after else (vk[0] if vk else None)
    c["vk_after_press_ms"] = ms_of(first[0]) - ms_of(press_utc) if first else None
    confirm = [ts for ts, line in lines if first and first[0] <= ts <= end_utc and CONFIRM.search(line)]
    c["confirm_ms"] = ms_of(confirm[0]) - ms_of(first[0]) if confirm else None
    return c


def analyze(recs: list, lines: list, require_cold: bool = False) -> dict:
    cfg = next((r for r in recs if r.get("type") == "config"), {})
    aborts = [r["reason"] for r in recs if r.get("type") == "abort"]
    done = any(r.get("type") == "done" for r in recs)

    def by_n(t):
        return {r["n"]: r for r in recs if r.get("type") == t}

    pre, on, typed = by_n("reopen_pre"), by_n("reopen_on"), by_n("reopen_typed")
    trials, counts = [], {"pass": 0, "fail": 0, "invalid": 0, "blind": 0}
    presses, confirms = [], []
    for n in sorted(set(pre) | set(on) | set(typed)):
        p, o, t = pre.get(n), on.get(n), typed.get(n)
        row = {"n": n}

        def finish(status, why):
            row.update(status=status, why=why)
            counts[status] += 1
            trials.append(row)

        if not (p and o and t):
            finish("invalid", "記録欠け(pre/on/typed のいずれかが無い)")
            continue
        if t.get("focus_lost"):
            finish("invalid", "打鍵前にフォーカスが外れた")
            continue
        if not (p["trial_utc"] <= o["off_utc"] <= o["on_utc"] <= t["utc"]):
            finish("invalid", "時間窓が逆転(UTC 日付をまたいだ?)")
            continue
        # M5: OFF 前の窓(試行の先頭の開き直し〜OFF 押下)の固着・ESC は、前提不成立に逃がさず fail にする。
        w0 = count(lines, p["trial_utc"], o["off_utc"])
        bad0 = {k: w0[k] for k in BAD if w0[k]}
        if not p.get("ok") and not bad0:
            finish("invalid", f"OFF 前の語が期待どおりでない(text={p.get('text')!r})=IME が ON にそろっていない")
            continue
        if o.get("open_before_on") is True and not bad0:
            finish("invalid", "物理 OFF が効いていない(ON キー直前も実 IME が開いたまま)")
            continue
        w = post_window(lines, o["on_utc"], t["press_utc"], t["utc"])
        row.update(w)
        row["pre_window"] = w0
        if w["unicode"] and not bad0:
            finish("blind", f"ON 後の窓に Unicode 注入がある({w['unicode']}件)。IME 状態の証拠にならない")
            continue
        why = []
        if bad0:
            why.append("OFF 前の窓で " + ", ".join(f"{k}={v}" for k, v in bad0.items()))
        if not t.get("ok"):
            why.append(f"ON 直後の語が期待と違う(expect={t.get('expect')!r} actual={normalize(t.get('text', ''))!r})")
        bad1 = {k: w[k] for k in BAD if w[k]}
        if bad1:
            why.append("ON 後の窓で " + ", ".join(f"{k}={v}" for k, v in bad1.items()))
        if t.get("real_ime_open") is False:
            why.append("ON 後も実 IME が閉じている")
        if require_cold:
            if w["first_vk_cold"] is None:
                why.append("ON 後の語の [vk-send] が無い(cold 経路を確認できない)")
            elif w["first_vk_cold"] is False:
                why.append("ON 後の最初の語が warm 経路(ADR-203 (c) は cold を要求)")
        finish("fail" if why else "pass", "; ".join(why))
        if w["vk_after_press_ms"] is not None:
            presses.append(w["vk_after_press_ms"])
        if w["confirm_ms"] is not None:
            confirms.append(w["confirm_ms"])
    valid = counts["pass"] + counts["fail"]
    invalid_run = []
    if aborts:
        invalid_run.append("中断: " + "; ".join(aborts))
    if not done and not aborts:
        invalid_run.append("完走マーカー(done)が無い")
    if not trials:
        invalid_run.append("試行が0件")
    elif valid * 2 < len(trials):
        invalid_run.append(f"有効試行が半数未満({valid}/{len(trials)}。blind={counts['blind']} invalid={counts['invalid']})")
    verdict = "INVALID" if invalid_run else ("FAIL" if counts["fail"] else "PASS")
    presses.sort()
    confirms.sort()
    med = lambda v: v[len(v) // 2] if v else None  # noqa: E731
    return {"verdict": verdict, "invalid_reasons": invalid_run, "counts": counts, "trials": trials,
            "form": cfg.get("form"), "ime": cfg.get("ime"), "require_cold": require_cold,
            "first_vk_cold": sum(1 for r in trials if r.get("first_vk_cold") is True),
            "vk_after_press_p50_ms": med(presses), "vk_after_press_max_ms": presses[-1] if presses else None,
            "confirm_p50_ms": med(confirms), "confirm_max_ms": confirms[-1] if confirms else None}


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(add_help=False)
    ap.add_argument("--require-cold", action="store_true")
    ap.add_argument("--json", dest="json_path")
    ap.add_argument("logs", nargs="*")
    a = ap.parse_args(argv)
    if len(a.logs) != 2:
        print(__doc__)
        return 2
    res = analyze(parse(a.logs[0]), load_awase(a.logs[1]), a.require_cold)
    for r in res["trials"]:
        if r["status"] in ("invalid", "blind"):
            print(f"{r['n']}: {r['status'].upper()} {r['why']}")
        else:
            print(f"{r['n']}: {r['status'].upper()} stuck={r['stuck']} stale_escape={r['stale_escape']} flush_escape={r['flush_escape']} "
                  f"reopen_belief={r['reopen_belief']} imeon_belief={r['imeon_belief']} imeon_other={r['imeon_other']} "
                  f"vk cold/warm={r['vk_cold']}/{r['vk_warm']} press→vk={r['vk_after_press_ms']}ms confirm={r['confirm_ms']}ms"
                  + (f" ← {r['why']}" if r["why"] else ""))
    for why in res["invalid_reasons"]:
        print("INVALID:", why)
    c = res["counts"]
    print(f"REOPEN: verdict={res['verdict']} form={res['form']} ime={res['ime']} pass={c['pass']} fail={c['fail']} invalid={c['invalid']} "
          f"blind={c['blind']} require_cold={res['require_cold']} first_vk_cold={res['first_vk_cold']} "
          f"press_vk_p50_ms={res['vk_after_press_p50_ms']} press_vk_max_ms={res['vk_after_press_max_ms']} "
          f"confirm_p50_ms={res['confirm_p50_ms']} confirm_max_ms={res['confirm_max_ms']}")
    if a.json_path:
        with open(a.json_path, "w", encoding="utf-8") as f:
            json.dump(res, f, ensure_ascii=False, indent=1)
    return {"PASS": 0, "FAIL": 1, "INVALID": 3}[res["verdict"]]


if __name__ == "__main__":
    sys.exit(main())
