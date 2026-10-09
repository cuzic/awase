#!/usr/bin/env python3
"""typing_stress --mode=focus-restore の `fr_stage` / `fr_trial` を表にする(観測のみ。合否は付けない)。

半角英数の持続トグル(左 Shift 単独タップ)中に別窓 B へフォーカスを移したとき、
`runtime/ime_refresh.rs::ir_notify_focus_changed` → `kp_restore_kana_from_half_width(false)` が
窓 A / 窓 B の実 IME(open・conv)と awase のログのどこに効くかを時点ごとに出す。
- 時点: pre / tap1#k(k 回目のタップの +700) / settled(+300,+1000) / away(+150,+600,+1500) / back(+150,+600,+1500) / tap2(+300,+1000)。
- 各窓を imm(ImmGetContext 系) と imc(既定 IME 窓への WM_IME_CONTROL)の 2 経路で読む。conv は 16 進。bit0=NATIVE(かな)。
- awase.log は試行の [utc0, utc_end] の窓から、復元まわりの行(下の PATTERNS)を時刻つきで出し、種類ごとに数える。
終了コード: 0=観測できた(--strict では全試行が合格) / 1=--strict で不合格あり / 3=INVALID(試行 0 件・abort あり・完走マーカー無し)。
使い方: check_focusrestore.py [--strict] [--json out.json] <typing_stress.log> <awase.log>

--strict の判定(ADR-245。fr_config の scenario / expect / b_open で決まる。bimeoff・control・sameproc は判定せず観測のみ):
- 共通: 窓の移動と戻りが成立(away_ok / back_ok、alttab は alttab_ok)。離脱の間(utc_away〜utc_back)に VK_DBE_HIRAGANA 注入と
  「かな入力へ復元」が無い(離脱では IME へ送らない)。窓 B の conv が離脱前後で変わらない。戻りの後に「復元 write をスキップ」「復元リトライ中断」が出ない。
  b_open なら B の最初の打鍵が期待のかな(B に ObservedEisu が持ち越されない)。
- expect=resume(MS-IME 本体・GJI の MS-IME プリセット): 戻って最初の打鍵で A がかな(typed の本文が期待のかな、typed 時点の A の conv に NATIVE)。
  lshift: 最初の左 Shift タップの後に NATIVE。shiftchar: Shift+文字の後の conv にカタカナ(bit1)が無い。
- expect=rebuild(GJI の ATOK プリセット。F2 が純粋なトグルなので注入せずトグルを立て直す): 戻って最初の打鍵は素通し(本文がキーの英字、conv に NATIVE なし)、
  次の左 Shift タップ(tap2)でかなに戻る(NATIVE)。lshift は resume と同じ(タップが Exit で F2 を送る)。
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


NATIVE = 1
KATAKANA = 2
OBSERVE_ONLY = {"bimeoff"}


def stage_of(stages, n, name):
    """試行 n の name 段の最後の記録(無ければ None)。"""
    c = [s for s in stages if s["n"] == n and s["stage"] == name]
    return c[-1] if c else None


def conv_of(st, win_key="A"):
    if not st:
        return None
    w = st[win_key]
    v = w.get("imm_conv")
    return v if v is not None else w.get("imc_conv")


def verdict(cfg, t, stages, awase):
    """strict の判定。(理由のリスト)を返す。空なら合格。"""
    n = t["n"]
    scn = cfg.get("scenario", "basic")
    expect = cfg.get("expect", "resume")
    why = []
    if t.get("control") or cfg.get("sameproc") or scn in OBSERVE_ONLY:
        return None
    if not t.get("away_ok"):
        why.append("窓 B/C へ移せなかった(away_ok=false)")
    if not t.get("back_ok"):
        why.append("A へ戻せなかった(back_ok=false)")
    if scn == "alttab" and not t.get("alttab_ok", True):
        why.append("Alt+Tab で A へ戻らなかった(alttab_ok=false)")
    leave = [ln for ts, ln in awase if t["utc_away"] <= ts < t["utc_back"]]
    for name, rx in PATTERNS:
        if name in ("inject_hira", "restore_begin"):
            k = sum(1 for ln in leave if rx.search(ln))
            if k:
                why.append(f"離脱の間に {name} が {k} 件(離脱では IME へ送らない)")
    after = [ln for ts, ln in awase if t["utc_back"] <= ts <= t["utc_end"]]
    for name, rx in PATTERNS:
        if name in ("toggle_skip", "retry_abort"):
            k = sum(1 for ln in after if rx.search(ln))
            if k:
                why.append(f"戻りの後に {name} が {k} 件")
    pre = stage_of(stages, n, "pre")
    away_last = [s for s in stages if s["n"] == n and s["stage"] == "away"]
    if pre and away_last and scn != "abc":
        b0 = pre["B"].get("imc_conv")
        b1 = away_last[-1]["B"].get("imc_conv")
        if b0 is not None and b1 is not None and b0 != b1:
            why.append(f"窓 B の conv が離脱前後で変わった({hx(b0)}→{hx(b1)})")
    typed = t["typed"]
    text = typed["text"].strip()
    if cfg.get("b_open") and t.get("b_typed") is not None:
        if t["b_typed"].strip() != typed["expect"]:
            why.append(f"B の最初の打鍵が {t['b_typed'].strip()!r}(期待 {typed['expect']!r}。B に英数が持ち越された疑い)")
    typed_st = stage_of(stages, n, "typed")
    typed_conv = conv_of(typed_st)
    tap2 = [s for s in stages if s["n"] == n and s["stage"] == "tap2"]
    tap2_conv = conv_of(tap2[-1]) if tap2 else None
    raw = chr(typed["vk"]).lower() if typed.get("vk") and 0x41 <= typed["vk"] <= 0x5A else None
    if scn == "lshift":
        ft = conv_of(stage_of(stages, n, "firsttap"))
        if ft is None or not ft & NATIVE:
            why.append(f"最初の左 Shift タップの後に NATIVE が無い(conv={hx(ft)})")
    if scn == "shiftchar":
        sc = conv_of(stage_of(stages, n, "shiftchar"))
        if sc is not None and sc & KATAKANA:
            why.append(f"Shift+文字の後にカタカナになった(conv={hx(sc)}、本文 {t.get('first_text')!r})")
    if expect == "rebuild" and scn != "lshift":
        if raw is None or text.lower() != raw:
            why.append(f"戻って最初の打鍵が素通しでない(本文 {text!r}、期待 {raw!r})")
        if typed_conv is not None and typed_conv & NATIVE:
            why.append(f"トグルが維持されていない(typed 時点の conv={hx(typed_conv)})")
        if tap2_conv is None or not tap2_conv & NATIVE:
            why.append(f"次の左 Shift タップでかなに戻らなかった(tap2 の conv={hx(tap2_conv)})")
    else:
        if text != typed["expect"]:
            why.append(f"戻って最初の打鍵がかなにならない(本文 {text!r}、期待 {typed['expect']!r})")
        if typed_conv is None or not typed_conv & NATIVE:
            why.append(f"typed 時点の A の conv に NATIVE が無い(conv={hx(typed_conv)})")
    return why


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--json")
    ap.add_argument("--strict", action="store_true")
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
    verdicts = []
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
        v = verdict(cfg, t, stages, awase)
        if v is None:
            print("  FOCUSRESTORE_VERDICT: 観測のみ")
        else:
            verdicts.append((n, v))
            print(f"  FOCUSRESTORE_VERDICT: {'FAIL' if v else 'PASS'} scenario={cfg.get('scenario', 'basic')} expect={cfg.get('expect', 'resume')}")
            for r in v:
                print(f"    - {r}")
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
    failed = [n for n, v in verdicts if v]
    print(f"FOCUSRESTORE: 判定した試行={len(verdicts)} 不合格={len(failed)} {failed}")
    if a.strict:
        if not verdicts:
            print("FOCUSRESTORE: INVALID (--strict なのに判定した試行が 0 件)")
            return 3
        return 1 if failed else 0
    return 0


if __name__ == "__main__":
    sys.exit(main())
