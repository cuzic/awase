#!/usr/bin/env python3
"""D1 startup 判定。終了コード: 0=PASS/OBSERVE, 1=FAIL, 3=INVALID。

オプション:
  --gate      startup_typed.real_ime_open_before_type が false(打鍵直前に IME が閉)または None(読めない)の回を INVALID(前提不成立)にする。
              これで FAIL が出れば『IME が開いているのに期待と違う』=awase 側の欠陥。
  --evidence  awase.log の [msime-ready]・send_keys: mode=Vk・literal detect の件数と先頭数行を出す。
`--no-awase` で走った回(config.no_awase)は awase 判定をせず、打鍵結果の分類(kana/ka/raw/other)だけを OBSERVE として出す。
"""
import json
import re
import sys

from e2e_common import hms_to_seconds as seconds, ts_json_records as parse

TS = re.compile(r"T(\d\d:\d\d:\d\d\.\d{3})")
ALIGN = re.compile(r"\[startup-align\].*desired=(true|false)")
DRIFT = re.compile(r"\[drift\] correction")
REINIT = re.compile(r"\[ime-io\] actuation SendInput kind=kanji_marker vk=\[1A, 16\]")
OBSERVE = re.compile(r"Imm32Unavailable entry without trusted cache: 安全デフォルト ON")
ENGINE = re.compile(r"\[engine-input\]")
START = re.compile(r"Keyboard Layout Emulator starting")
EVIDENCE = (("msime_ready", re.compile(r"\[msime-ready\]")), ("send_keys_vk", re.compile(r"send_keys: mode=Vk")),
            ("literal_detect", re.compile(r"literal detect")))


def load_awase(path):
    out = []
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            m = TS.search(line[:40])
            if m:
                out.append((seconds(m.group(1)), line))
    return out


def classify_text(typed):
    """打鍵結果の text を kana(期待のかな) / ka(ローマ字リテラル) / raw(NICOLA の生キー文字=awase が Engine を止めた) / other に分ける。"""
    t = (typed.get("text") or "").strip()
    if t == (typed.get("expect") or "").strip(): return "kana"
    if t == "ka": return "ka"
    if t and t == typed.get("raw_char"): return "raw"
    return "other"


def open_label(v):
    return {True: "open", False: "closed"}.get(v, "unknown")


def evidence(lines):
    out = {}
    for key, rx in EVIDENCE:
        hit = [l.rstrip() for _, l in lines if rx.search(l)]
        out[key] = {"count": len(hit), "head": hit[:3]}
    return out


def analyze(recs, lines, gate=False, with_evidence=False):
    cfg = next((x for x in recs if x.get("type") == "config"), {})
    pre = next((x for x in recs if x.get("type") == "startup_pre"), None)
    typed = next((x for x in recs if x.get("type") == "startup_typed"), None)
    aborts = [x.get("reason") for x in recs if x.get("type") == "abort"]
    done = any(x.get("type") == "done" for x in recs)
    starts = [t for t, line in lines if START.search(line)]
    no_awase = bool(cfg.get("no_awase"))
    invalid = []
    if cfg.get("mode") != "startup": invalid.append("mode=startup でない")
    if not pre or not typed: invalid.append("startup_pre/startup_typed が無い")
    if aborts: invalid.append("中断: " + "; ".join(aborts))
    if not done: invalid.append("完走マーカー(done)が無い")
    if not no_awase and len(starts) != 1: invalid.append(f"awase 起動行が1件でない({len(starts)})")
    if invalid:
        return {"verdict":"INVALID","cfg":cfg,"invalid":invalid}
    open_before = typed.get("real_ime_open_before_type")
    text_class = classify_text(typed)
    extra = {"open_before":open_before,"text_class":text_class,"no_awase":no_awase}
    if with_evidence: extra["evidence"] = evidence(lines)
    if no_awase:
        # awase なしの対照: 生キー k,a が『か』になるか(IME が開いている)・`ka` になるか(閉)を観測するだけで合否にしない。
        return {"verdict":"OBSERVE","cfg":cfg,"initial":pre.get("initial"),"drift":0,"align":[],"reinit":0,"observe":0,
                "first_engine_ms":None,"failures":[],"invalid":[],"align_gated":False,**extra}
    if gate and open_before is not True:
        why = "打鍵直前に IME が閉(前提不成立)" if open_before is False else "打鍵直前の IME 開閉を読めない(前提を確認できない)"
        return {"verdict":"INVALID","cfg":cfg,"initial":pre.get("initial"),"invalid":[why],"failures":[],**extra}
    start = starts[0]
    in30 = [(t, l) for t, l in lines if start <= t <= start + 30]
    aligns = [m.group(1) for _, l in in30 if (m := ALIGN.search(l))]
    drifts = sum(bool(DRIFT.search(l)) for _, l in in30)
    reinit = sum(bool(REINIT.search(l)) for _, l in in30 if _ <= start + 3.0)
    observed = sum(bool(OBSERVE.search(l)) for _, l in in30)
    engine = [t for t, l in lines if ENGINE.search(l) and t >= start]
    initial = pre.get("initial")
    failures = []
    if not typed.get("ok"): failures.append("打鍵結果が期待文字列と不一致")
    if drifts: failures.append(f"起動後30秒の drift={drifts}")
    desired = "true" if initial == "on" else "false"
    # 実 Chrome(form=chromepage)は Imm32Unavailable で観測が来ず `[startup-align]`(最初の成功観測へ揃えた)が出ないのが正常。
    # そこでは startup-align の有無を合否に入れず、観察項目として align 列に記録するだけにする。
    chrome = cfg.get("form") == "chromepage"
    if not chrome and aligns != [desired]: failures.append(f"startup-align={aligns!r} (期待 [{desired!r}])")
    late_first_key = initial == "on" and (not engine or engine[0] - start > 2.0)
    if initial == "on":
        pass
    else:
        if reinit: failures.append(f"OFF起動直後の reinit={reinit}")
        if typed.get("open_after_idle") is True: failures.append("3秒アイドル中に IME が開いた")
    # ON 起動の前提は「起動から2秒以内に最初の打鍵が入る」こと(起動直後の先同期は最大約0.5秒。手順書は1秒だが、CI の runner では
    # ハーネスの起動検知が 0.8〜5.7 秒揺れ、1秒だと半数が INVALID になった)。ハーネスの起動検知・前面化の遅れで
    # 2秒を超えたら、BUG-163 の『起動直後の最初の打鍵』を試せていないので FAIL でなく INVALID(前提不成立)にする。
    if late_first_key:
        return {"verdict":"INVALID","cfg":cfg,"initial":initial,"invalid":["最初の engine-input が起動から2秒以内でない(前提不成立)"],
                "first_engine_ms":None if not engine else round((engine[0]-start)*1000),
                "drift":drifts,"align":aligns,"reinit":reinit,"observe":observed,"failures":failures,"align_gated":not chrome,**extra}
    return {"verdict":"FAIL" if failures else "PASS","cfg":cfg,"initial":initial,
            "drift":drifts,"align":aligns,"reinit":reinit,"observe":observed,
            "first_engine_ms":None if not engine else round((engine[0]-start)*1000),
            "failures":failures,"invalid":[],"align_gated":not chrome,**extra}


def summary_line(r):
    c = r.get("cfg", {})
    return (f"STARTUP: verdict={r['verdict']} form={c.get('form','?')} ime={c.get('ime','?')} "
            f"initial={r.get('initial','?')} drift={r.get('drift','?')} align={','.join(r.get('align',[])) or '-'} "
            f"reinit={r.get('reinit','?')} imm32_default_on={r.get('observe','?')} "
            f"first_engine_ms={r.get('first_engine_ms','?')} open_before={open_label(r.get('open_before'))} "
            f"text_class={r.get('text_class','?')}")


def main(argv):
    args = list(argv); out = None
    if "--json" in args:
        i = args.index("--json"); out = args[i+1]; del args[i:i+2]
    gate = "--gate" in args; args = [a for a in args if a != "--gate"]
    ev = "--evidence" in args; args = [a for a in args if a != "--evidence"]
    if len(args) != 2: return 2
    try: recs = parse(args[0])
    except OSError as e:
        print(f"ログを読めない: {e}"); return 3
    cfg = next((x for x in recs if x.get("type") == "config"), {})
    try: lines = load_awase(args[1])
    except OSError as e:
        # --no-awase の対照には awase.log が無い。それ以外は従来どおり読めなければ INVALID。
        if not cfg.get("no_awase"):
            print(f"ログを読めない: {e}"); return 3
        lines = []
    r = analyze(recs, lines, gate=gate, with_evidence=ev)
    for x in r.get("invalid", []) + r.get("failures", []): print(f"  {r['verdict']}: {x}")
    line = summary_line(r); print(line)
    for key, v in r.get("evidence", {}).items():
        print(f"EVIDENCE: {key}={v['count']}")
        for h in v["head"]: print(f"  {h[:200]}")
    if out:
        with open(out, "w", encoding="utf-8") as f: json.dump({**r,"line":line}, f, ensure_ascii=False)
    return {"PASS":0,"OBSERVE":0,"FAIL":1,"INVALID":3}[r["verdict"]]


if __name__ == "__main__": sys.exit(main(sys.argv[1:]))
