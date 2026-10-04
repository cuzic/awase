#!/usr/bin/env python3
"""BUG-103 の実機再現: `[[post_bypass]]` が設定リロード(WM_RELOAD_CONFIG)で反映されるか。

実際の awase.exe(debug、AWASE_TEST_INJECTION=1)を起動し、config_verify_probe の `--mode=caret`(目印付き SendInput で Ctrl+P / Ctrl+N を物理 Ctrl 扱いで注入)で
`[ctrl-bypass] post_bypass armed` が awase.log に出るかを見る(ルールに一致した Ctrl+キーでだけ出る)。
  対照 A: 起動時の config に [[post_bypass]] Ctrl+P があれば armed が出る(検証が成り立つ前提)。
  B: 起動時はルールなし → armed 0 件を確認 → config.toml にルールを足して awase_tray_window へ WM_RELOAD_CONFIG を投げる → もう一度 Ctrl+P を注入。
     armed が出れば「リロードで反映される」(BUG-103 は直っている)、出なければ BUG-103 の再現。
使い方: python tools/e2e/config_verify/reload_post_bypass.py --dist dist --out out
"""
from __future__ import annotations

import argparse
import ctypes
import json
import os
import re
import sys
import time
import subprocess
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import run as cv  # noqa: E402  (tools/e2e/config_verify/run.py の起動・停止・ログ読みを再利用)

ARMED_RE = re.compile(r"\[ctrl-bypass\] post_bypass armed")
RELOAD_RE = re.compile(r"Config reload requested via WM_RELOAD_CONFIG")
RULE = '\n[[post_bypass]]\nkey = "Ctrl+P"\n'
WM_APP = 0x8000
WM_RELOAD_CONFIG = WM_APP + 10


def run_probe(dist: Path, work: Path, idx: int) -> bool:
    probe_log = work / f"probe-{idx}.log"
    pp = subprocess.Popen([str(dist / "config_verify_probe.exe"), "--mode=caret", f"--log={probe_log}"], cwd=work)
    t0, done = time.time(), False
    while time.time() - t0 < 60:
        if probe_log.exists() and "=== 完了 ===" in probe_log.read_text(encoding="utf-8", errors="replace"):
            done = True
            break
        if pp.poll() is not None and not probe_log.exists():
            break
        time.sleep(0.5)
    time.sleep(1.0)
    if pp.poll() is None:
        pp.kill()
    return done


def post_reload() -> bool:
    user32 = ctypes.windll.user32
    hwnd = user32.FindWindowW("awase_tray_window", None)
    if not hwnd:
        return False
    return bool(user32.PostMessageW(hwnd, WM_RELOAD_CONFIG, 0, 0))


def count(lines: list[str], rx: re.Pattern) -> int:
    return sum(1 for l in lines if rx.search(l))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dist", default="dist")
    ap.add_argument("--out", default="out")
    a = ap.parse_args()
    dist, out = Path(a.dist).resolve(), Path(a.out).resolve()
    (out / "logs").mkdir(parents=True, exist_ok=True)
    rows = []

    # 対照 A: 起動時からルールあり
    workA = out / "work" / "ctl-start-with-rule"
    procA = cv.start_awase(dist, workA, cv.CFG_BASE + RULE)
    try:
        cv.wait_stable(workA, procA)
        doneA = run_probe(dist, workA, 0)
    finally:
        cv.stop_awase(procA)
    linesA = cv.read_log(workA)
    (out / "logs" / "ctl-start-with-rule.awase.log").write_text("\n".join(linesA), encoding="utf-8")
    armedA = count(linesA, ARMED_RE)
    rows.append(("対照A: 起動時からルールあり → Ctrl+P で armed", "PASS" if armedA > 0 else "UNVERIFIABLE",
                 f"armed={armedA} probe_done={doneA}(0 なら CI では検証できない環境)"))

    # B: 起動時はルールなし → リロードで足す
    workB = out / "work" / "reload"
    procB = cv.start_awase(dist, workB, cv.CFG_BASE)
    try:
        cv.wait_stable(workB, procB)
        done0 = run_probe(dist, workB, 0)
        armed_before = count(cv.read_log(workB), ARMED_RE)
        (workB / "config.toml").write_text(cv.CFG_BASE + RULE, encoding="utf-8", newline="\n")
        posted = post_reload()
        time.sleep(4.0)
        reloaded = count(cv.read_log(workB), RELOAD_RE)
        done1 = run_probe(dist, workB, 1)
        time.sleep(1.0)
    finally:
        cv.stop_awase(procB)
    linesB = cv.read_log(workB)
    (out / "logs" / "reload.awase.log").write_text("\n".join(linesB), encoding="utf-8")
    armed_after = count(linesB, ARMED_RE) - armed_before
    rows.append(("B-1: ルールなしで起動 → Ctrl+P では armed しない(前提)", "PASS" if armed_before == 0 else "FAIL",
                 f"armed={armed_before} probe_done={done0}"))
    rows.append(("B-2: ルールを足して WM_RELOAD_CONFIG を送るとリロードされる(前提)", "PASS" if (posted and reloaded > 0) else "UNVERIFIABLE",
                 f"posted={posted} reload_log={reloaded}"))
    if armedA == 0 or not (posted and reloaded > 0):
        verdict, ev = "UNVERIFIABLE", "対照が成り立たない"
    elif armed_after > 0:
        verdict, ev = "PASS", "リロード後の Ctrl+P で armed が出た(リロードで反映された=BUG-103 は直っている)"
    else:
        verdict, ev = "FAIL", "リロード後も Ctrl+P で armed が出ない(BUG-103 を再現: 再起動まで反映されない)"
    rows.append(("B-3: リロード後に追加した [[post_bypass]] が効く(BUG-103)", verdict, f"{ev} armed_after={armed_after} probe_done={done1}"))

    (out / "results.json").write_text(json.dumps(rows, ensure_ascii=False, indent=1), encoding="utf-8")
    md = ["| 項目 | 判定 | 根拠 |", "|---|---|---|"] + [f"| {i} | **{s}** | {e} |" for i, s, e in rows]
    (out / "summary.md").write_text("\n".join(md) + "\n", encoding="utf-8")
    print("\n".join(md))
    sp = os.environ.get("GITHUB_STEP_SUMMARY")
    if sp:
        with open(sp, "a", encoding="utf-8") as f:
            f.write("# BUG-103 リロード検証\n\n" + "\n".join(md) + "\n")
    # BUG の再現(FAIL)は見つかった不具合であり、このジョブ自体は成功で終える(観測用)。
    return 0


if __name__ == "__main__":
    sys.exit(main())
