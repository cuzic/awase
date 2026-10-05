#!/usr/bin/env python3
"""環境イベント(probe.py)を windows-latest で起こし、①起こせたか ②awase 起動中の awase.log の反応 を out/ に残す。

相 A: awase なしで tab / minrestore / vdesk(部品が実機で動くかの可否表)
相 B: awase(debug、AWASE_TEST_INJECTION=1)を起動して同じイベント。各イベントの前後の awase.log を抜粋
相 C: awase を起動して画面ロック(最後。解除できないので他の相より後)
使い方: python tools/e2e/env_events/run.py --dist dist --out out   (dist/awase.exe が要る)
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "config_verify"))
import run as cv  # noqa: E402  (awase の起動・停止・ログ読み)

HERE = Path(__file__).resolve().parent
TS_RE = re.compile(r"^(\d{4}-\d\d-\d\dT[\d:]+\.\d{3})")
EV_RE = re.compile(r"\[EV-JSON\] (\{.*\})")


def run_probe(events: str, log: Path, marker: bool, timeout: int) -> tuple[bool, list[dict]]:
    if log.exists():
        log.unlink()
    cmd = [sys.executable, str(HERE / "probe.py"), f"--events={events}", f"--log={log}"] + (["--marker=1"] if marker else [])
    p = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    try:
        p.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        p.kill()
    err = p.stderr.read().decode("utf-8", errors="replace") if p.stderr else ""
    text = log.read_text(encoding="utf-8", errors="replace") if log.exists() else ""
    recs = [json.loads(m.group(1)) for m in map(EV_RE.search, text.splitlines()) if m]
    if err.strip():
        recs.append({"type": "probe_stderr", "text": err.strip()[-1500:]})
    return "=== 完了 ===" in text, recs


def windows_of(recs: list[dict]) -> dict[str, tuple[str, str]]:
    """イベント名 -> (event_begin の ts, event_end の ts)。"""
    w: dict[str, list[str]] = {}
    for r in recs:
        if r.get("type") in ("event_begin", "event_end"):
            w.setdefault(r["name"], ["", ""])[0 if r["type"] == "event_begin" else 1] = r["ts"]
    return {k: (v[0], v[1] or v[0]) for k, v in w.items()}


def plus_seconds(ts: str, sec: float) -> str:
    """`2026-10-05T01:02:03.456Z` に sec 秒足す(同じ書式で返す)。"""
    import datetime as dt
    t = dt.datetime.strptime(ts[:23], "%Y-%m-%dT%H:%M:%S.%f") + dt.timedelta(seconds=sec)
    return t.strftime("%Y-%m-%dT%H:%M:%S.") + "%03d" % (t.microsecond // 1000)


def excerpt(lines: list[str], t0: str, t1: str, limit: int = 80) -> list[str]:
    out = []
    for l in lines:
        m = TS_RE.match(l)
        if m and t0[:23] <= m.group(1) <= t1[:23]:
            out.append(l[:260])
    return out[:limit]


def phase(name: str, dist: Path, out: Path, events: str, with_awase: bool, timeout: int) -> dict:
    info: dict = {"phase": name, "events": events, "awase": with_awase}
    work = out / "work" / name
    proc = None
    if with_awase:
        proc = cv.start_awase(dist, work, cv.CFG_BASE)
        cv.wait_stable(work, proc)
    else:
        work.mkdir(parents=True, exist_ok=True)
    try:
        done, recs = run_probe(events, work / "probe.log", with_awase, timeout)
    finally:
        if proc:
            info["awase_stop"] = cv.stop_awase(proc)
    info["probe_done"], info["records"] = done, recs
    if with_awase:
        lines = cv.read_log(work)
        (out / "logs").mkdir(parents=True, exist_ok=True)
        (out / "logs" / f"{name}.awase.log").write_text("\n".join(lines), encoding="utf-8")
        ex = {}
        for ev, (t0, t1) in windows_of(recs).items():
            ex[ev] = excerpt(lines, t0, plus_seconds(t1, 3.0))  # イベント終了の 3 秒後まで(遅れて出る反応を拾う)
        info["awase_excerpt"] = ex
    return info


def md(results: list[dict]) -> str:
    o = ["# 環境イベント実機probe", ""]
    o += ["| 相 | イベント | 起こせたか(happened) | 補足 |", "|---|---|---|---|"]
    for r in results:
        for rec in r["records"]:
            if rec.get("type") in ("tab", "minrestore", "vdesk", "lock"):
                note = ""
                if rec["type"] == "vdesk":
                    note = f"create後 on_current_vd={rec.get('after_create', {}).get('on_current_vd')} / {rec.get('reason', '')}"
                if rec["type"] == "lock":
                    note = "入力デスクトップ推移=" + ",".join(dict.fromkeys(rec.get("input_desktop_over_time", [])))
                o.append(f"| {r['phase']} | {rec['type']} | {rec.get('happened')} | {note} |")
            if rec.get("type") in ("event_error", "probe_stderr"):
                o.append(f"| {r['phase']} | {rec.get('name', 'probe')} | ERROR | {str(rec.get('error') or rec.get('text'))[:200]} |")
        if not r["probe_done"]:
            o.append(f"| {r['phase']} | (完走せず) | - | probe が `=== 完了 ===` に達しなかった |")
    o.append("")
    for r in results:
        for ev, lines in r.get("awase_excerpt", {}).items():
            o += [f"## {r['phase']} / {ev}: イベント中〜3 秒後の awase.log({len(lines)} 行、先頭 80)", "", "```"] + lines[:40] + ["```", ""]
    return "\n".join(o)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dist", default="dist")
    ap.add_argument("--out", default="out")
    ap.add_argument("--phases", default="A,B,C")
    a = ap.parse_args()
    dist, out = Path(a.dist).resolve(), Path(a.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    plan = {"A": ("A-noawase", "tab,minrestore,vdesk", False, 150), "B": ("B-awase", "tab,minrestore,vdesk", True, 150),
            "C": ("C-awase-lock", "lock", True, 90)}
    results: list[dict] = []
    for k in a.phases.split(","):
        name, events, aw, to = plan[k]
        results.append(phase(name, dist, out, events, aw, to))
        (out / "results.json").write_text(json.dumps(results, ensure_ascii=False, indent=1), encoding="utf-8")
        (out / "summary.md").write_text(md(results), encoding="utf-8")
        print(md(results)[:3000], flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
