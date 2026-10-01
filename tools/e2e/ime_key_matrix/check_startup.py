#!/usr/bin/env python3
"""D1 startup 判定。終了コード: 0=PASS, 1=FAIL, 3=INVALID。"""
import datetime as dt
import json
import re
import sys

from check_typing_stress import parse

TS = re.compile(r"T(\d\d:\d\d:\d\d\.\d{3})")
ALIGN = re.compile(r"\[startup-align\].*desired=(true|false)")
DRIFT = re.compile(r"\[drift\] correction")
REINIT = re.compile(r"\[ime-io\] actuation SendInput kind=kanji_marker vk=\[1A, 16\]")
OBSERVE = re.compile(r"Imm32Unavailable entry without trusted cache: 安全デフォルト ON")
ENGINE = re.compile(r"\[engine-input\]")
START = re.compile(r"Keyboard Layout Emulator starting")


def seconds(s):
    t = dt.datetime.strptime(s, "%H:%M:%S.%f")
    return t.hour * 3600 + t.minute * 60 + t.second + t.microsecond / 1e6


def load_awase(path):
    out = []
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            m = TS.search(line[:40])
            if m:
                out.append((seconds(m.group(1)), line))
    return out


def analyze(recs, lines):
    cfg = next((x for x in recs if x.get("type") == "config"), {})
    pre = next((x for x in recs if x.get("type") == "startup_pre"), None)
    typed = next((x for x in recs if x.get("type") == "startup_typed"), None)
    aborts = [x.get("reason") for x in recs if x.get("type") == "abort"]
    done = any(x.get("type") == "done" for x in recs)
    starts = [t for t, line in lines if START.search(line)]
    invalid = []
    if cfg.get("mode") != "startup": invalid.append("mode=startup でない")
    if not pre or not typed: invalid.append("startup_pre/startup_typed が無い")
    if aborts: invalid.append("中断: " + "; ".join(aborts))
    if not done: invalid.append("完走マーカー(done)が無い")
    if len(starts) != 1: invalid.append(f"awase 起動行が1件でない({len(starts)})")
    if invalid:
        return {"verdict":"INVALID","cfg":cfg,"invalid":invalid}
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
    if aligns != [desired]: failures.append(f"startup-align={aligns!r} (期待 [{desired!r}])")
    if initial == "on":
        if not engine or engine[0] - start > 1.0:
            failures.append("最初の engine-input が起動から1秒以内でない")
    else:
        if reinit: failures.append(f"OFF起動直後の reinit={reinit}")
        if typed.get("open_after_idle") is True: failures.append("3秒アイドル中に IME が開いた")
    return {"verdict":"FAIL" if failures else "PASS","cfg":cfg,"initial":initial,
            "drift":drifts,"align":aligns,"reinit":reinit,"observe":observed,
            "first_engine_ms":None if not engine else round((engine[0]-start)*1000),
            "failures":failures,"invalid":[]}


def summary_line(r):
    c = r.get("cfg", {})
    return (f"STARTUP: verdict={r['verdict']} form={c.get('form','?')} ime={c.get('ime','?')} "
            f"initial={r.get('initial','?')} drift={r.get('drift','?')} align={','.join(r.get('align',[])) or '-'} "
            f"reinit={r.get('reinit','?')} imm32_default_on={r.get('observe','?')} "
            f"first_engine_ms={r.get('first_engine_ms','?')}")


def main(argv):
    args = list(argv); out = None
    if "--json" in args:
        i = args.index("--json"); out = args[i+1]; del args[i:i+2]
    if len(args) != 2: return 2
    try: r = analyze(parse(args[0]), load_awase(args[1]))
    except OSError as e:
        print(f"ログを読めない: {e}"); return 3
    for x in r.get("invalid", []) + r.get("failures", []): print(f"  {r['verdict']}: {x}")
    line = summary_line(r); print(line)
    if out:
        with open(out, "w", encoding="utf-8") as f: json.dump({**r,"line":line}, f, ensure_ascii=False)
    return {"PASS":0,"FAIL":1,"INVALID":3}[r["verdict"]]


if __name__ == "__main__": sys.exit(main(sys.argv[1:]))
