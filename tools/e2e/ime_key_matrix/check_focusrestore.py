#!/usr/bin/env python3
"""typing_stress --mode=focus-restore の `fr_stage` / `fr_trial` を表にする(観測のみ。合否は付けない)。

半角英数の持続トグル(左 Shift 単独タップ)中に別窓 B へフォーカスを移したとき、
`runtime/ime_refresh.rs::ir_notify_focus_changed` → `kp_restore_kana_from_half_width(false)` が
窓 A / 窓 B の実 IME(open・conv)と awase のログのどこに効くかを時点ごとに出す。
- 時点: pre / tap1#k(k 回目のタップの +700) / settled(+300,+1000) / away(+150,+600,+1500) / back(+150,+600,+1500) / tap2(+300,+1000)。
- 各窓を imm(ImmGetContext 系) と imc(既定 IME 窓への WM_IME_CONTROL)の 2 経路で読む。conv は 16 進。bit0=NATIVE(かな)。
- awase.log は試行の [utc0, utc_end] の窓から、復元まわりの行(下の PATTERNS)を時刻つきで出し、種類ごとに数える。
終了コード: 0=観測できた / 3=INVALID(試行 0 件・abort あり・完走マーカー無し)。
使い方: check_focusrestore.py [--json out.json] <typing_stress.log> <awase.log>
"""
import argparse
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from e2e_common import load_awase_timed, ts_json_records  # noqa: E402

# (名前, 正規表現)。awase.log の固定断片(key_pipeline.rs / ime_refresh.rs / output/mod.rs のログ文言)。
PATTERNS = [
    ("force_release", re.compile(r"\[shift-conv-guard\] FocusChanged 中")),
    ("restore_begin", re.compile(r"\[shift-conv-guard\] かな入力へ復元")),
    ("inject_hira", re.compile(r"VK_DBE_HIRAGANA \(scan 付き\) 注入")),
    ("inject_skip", re.compile(r"VK_DBE_HIRAGANA 注入をスキップ")),
    ("retry_abort", re.compile(r"復元リトライ #\d+: .*中断")),
    ("capture_fail", re.compile(r"capture 失敗")),
    ("conv_write_fail", re.compile(r"conv 復元 write #\d+ (失敗|: Aborted)")),
    ("conv_native_ok", re.compile(r"NATIVE 確認")),
    ("conv_giveup", re.compile(r"conv 復元 \d+ 回で NATIVE 未確認")),
    ("gji_exit_skip", re.compile(r"GJI 半角英数トグル .*(スキップ|見送った)")),
    ("toggle_skip", re.compile(r"半角英数トグル復元 write をスキップ")),
    ("focus", re.compile(r"FocusChanged|\[focus-scope\]|focus transition|\[focus-settle\]")),
    ("ime_mode_key", re.compile(r"\[ime-mode\]|\[hook\] IME-mode vk=|VK_DBE_(HIRAGANA|ALPHANUMERIC)")),
    ("send_input", re.compile(r"\[ime-io\] actuation SendInput")),
    ("belief_input_mode", re.compile(r"UserHalfWidthAlnumToggle|input_mode")),
]


def hx(v):
    return "-" if v is None else f"0x{v:x}"


def win(w):
    o = {True: "O", False: "C", None: "?"}
    return f"imm={o[w.get('imm_open')]}/{hx(w.get('imm_conv'))} imc={o[w.get('imc_open')]}/{hx(w.get('imc_conv'))}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--json")
    ap.add_argument("ts_log")
    ap.add_argument("awase_log", nargs="?")
    a = ap.parse_args()
    recs = ts_json_records(a.ts_log)
    stages = [r for r in recs if r.get("type") == "fr_stage"]
    trials = [r for r in recs if r.get("type") == "fr_trial"]
    aborts = [r for r in recs if r.get("type") == "abort"]
    cfg = next((r for r in recs if r.get("type") == "fr_config"), {})
    print(f"FOCUSRESTORE: config={json.dumps(cfg, ensure_ascii=False)} trials={len(trials)} stages={len(stages)} aborts={len(aborts)}")
    for ab in aborts:
        print(f"FOCUSRESTORE: ABORT {ab.get('reason')}")
    awase = load_awase_timed(a.awase_log) if a.awase_log else []
    print(f"FOCUSRESTORE: awase.log の時刻付き行={len(awase)}")
    out = []
    entered_n = 0
    for t in trials:
        n = t["n"]
        if not t.get("entered", True):
            print(f"\n=== trial n={n} 半角英数に入れなかった(taps={t.get('taps')}、この試行は捨てる) ===")
            for s in (s for s in stages if s["n"] == n):
                print(f"  {s['stage']:7} +{s['at_ms']:<5} {s['utc']} fg={s['fg']:5} A[{win(s['A'])}]")
            continue
        entered_n += 1
        print(f"\n=== trial n={n} taps={t.get('taps')} control={t.get('control')} away_ok={t.get('away_ok')} back_ok={t.get('back_ok')} typed={t['typed']['text']!r} (expect {t['typed']['expect']!r}) ===")
        print(f"  utc: start={t['utc']} tap1={t['utc_tap1']} away={t['utc_away']} back={t['utc_back']} type={t['utc_type']} tap2={t['utc_tap2']} end={t['utc_end']}")
        for s in (s for s in stages if s["n"] == n):
            print(f"  {s['stage']:7} +{s['at_ms']:<5} {s['utc']} fg={s['fg']:5} A[{win(s['A'])}]  B[{win(s['B'])}]")
        win_lines = [(ts, ln.rstrip()) for ts, ln in awase if t["utc"] <= ts <= t["utc_end"]]
        counts = {}
        picked = []
        for ts, ln in win_lines:
            for name, rx in PATTERNS:
                if rx.search(ln):
                    counts[name] = counts.get(name, 0) + 1
                    picked.append((ts, name, ln))
                    break
        print(f"  awase.log 窓内の行={len(win_lines)} 種類別={json.dumps(counts, ensure_ascii=False)}")
        for ts, name, ln in picked[:90]:
            print(f"    {ts} [{name}] {ln[ln.find(ts) + 12:][:230]}")
        out.append({"n": n, "trial": t, "stages": [s for s in stages if s["n"] == n], "awase_counts": counts})
    if a.json:
        with open(a.json, "w", encoding="utf-8") as f:
            json.dump(out, f, ensure_ascii=False, indent=1)
    done = any("=== 完了 ===" in ln for ln in open(a.ts_log, encoding="utf-8", errors="replace"))
    if not entered_n or aborts or not done:
        print(f"FOCUSRESTORE: INVALID (trials={len(trials)} entered={entered_n} aborts={len(aborts)} done={done})")
        return 3
    print("FOCUSRESTORE: 観測のみ(合否なし)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
