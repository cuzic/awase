#!/usr/bin/env python3
"""トレイ「IME 制御の学習キャッシュをクリア」の実機確認(BUG-108, PR #353)。windows-latest 用。

実際の awase.exe を cache.toml に誤学習エントリを仕込んだ状態で起動し、トレイウィンドウ
(`awase_tray_window`)へメニュー選択と同じ `WM_COMMAND`(IDM_CLEAR_IMM_CACHE=52)を送って、
awase.log と cache.toml の結果で判定する。

  A: 正常系   [imm_capability] が空になる/[injection_mode] は残る/学習表 JSON は無傷/
              ログに「cleared: N entries (persisted=true)」(N>=仕込み件数)
  B: 壊れた cache.toml  上書きしない(バイト不変)/ログに persisted=false

使い方: python tools/e2e/tray_cache_clear/run.py --dist dist --out out   (dist/awase.exe が前提)
終了コード: 全 PASS で 0、FAIL があれば 1。
"""
from __future__ import annotations

import argparse
import ctypes
import json
import os
import re
import shutil
import subprocess
import sys
import time
import tomllib
from ctypes import wintypes
from pathlib import Path

TRAY_CLASS = "awase_tray_window"
WM_COMMAND = 0x0111
IDM_CLEAR_IMM_CACHE = 52

SEEDED = {"fake_a.exe": {"Edit": "unavailable"}, "fake_b.exe": {"Chrome_WidgetWin_1": "broken", "Edit": "works"}}
SEEDED_COUNT = 3
GOOD_CACHE = (
    '[imm_capability."fake_a.exe"]\nEdit = "unavailable"\n\n'
    '[imm_capability."fake_b.exe"]\nChrome_WidgetWin_1 = "broken"\nEdit = "works"\n\n'
    '[injection_mode]\n"Some.Class" = "tsf"\n'
)
BROKEN_CACHE = '[imm_capability."fake_a.exe"\nEdit = "unavailable"\nthis is not toml\n'
LEARN_TABLE = '{"sentinel": "must-survive-clear"}\n'
CLEARED_RE = re.compile(r"IMM capability cache cleared: (\d+) entries \(persisted=(true|false)\)")


def start_awase(dist: Path, work: Path, cache_text: str) -> subprocess.Popen:
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    shutil.copy(dist / "awase.exe", work / "awase.exe")
    (work / "config.toml").write_text("", encoding="utf-8", newline="\n")
    (work / "cache.toml").write_text(cache_text, encoding="utf-8", newline="\n")
    (work / "keymap-learn-table.json").write_text(LEARN_TABLE, encoding="utf-8", newline="\n")
    env = dict(os.environ, RUST_LOG="debug", AWASE_TEST_INJECTION="1")
    return subprocess.Popen([str(work / "awase.exe")], cwd=work, env=env,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def wait_for_tray(proc: subprocess.Popen, timeout: float = 40.0) -> int:
    user32 = ctypes.windll.user32
    user32.FindWindowW.restype = wintypes.HWND
    t0 = time.time()
    while time.time() - t0 < timeout:
        if proc.poll() is not None:
            return 0
        hwnd = user32.FindWindowW(TRAY_CLASS, None)
        if hwnd:
            return int(hwnd)
        time.sleep(0.5)
    return 0


def send_clear(hwnd: int) -> None:
    # メニュー選択確定と同じ WM_COMMAND。SendMessageW は awase 側の処理完了まで戻らない。
    send = ctypes.windll.user32.SendMessageW
    send.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
    send.restype = wintypes.LPARAM
    send(wintypes.HWND(hwnd), WM_COMMAND, IDM_CLEAR_IMM_CACHE, 0)


def read_log(work: Path) -> list[str]:
    p = work / "awase.log"
    return p.read_text(encoding="utf-8", errors="replace").splitlines() if p.exists() else []


def wait_for_cleared(work: Path, skip_lines: int = 0, timeout: float = 15.0):
    """送信前の行(skip_lines 行)は見ない。起動時に同形式のログが出ても古い件数を拾わない。"""
    t0 = time.time()
    while time.time() - t0 < timeout:
        for line in read_log(work)[skip_lines:]:
            m = CLEARED_RE.search(line)
            if m:
                return int(m.group(1)), m.group(2) == "true"
        time.sleep(0.5)
    return None


def stop_awase(proc: subprocess.Popen) -> None:
    if proc.poll() is None:
        subprocess.run(["taskkill", "/PID", str(proc.pid)], capture_output=True)
        try:
            proc.wait(timeout=8)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=8)


def run_case(dist: Path, out: Path, name: str, cache_text: str) -> dict:
    work = out / "work" / name
    proc = start_awase(dist, work, cache_text)
    info: dict = {"name": name, "tray_found": False, "cleared": None}
    try:
        hwnd = wait_for_tray(proc)
        info["tray_found"] = bool(hwnd)
        if hwnd:
            time.sleep(3.0)  # 起動直後の初期化(自身の cache.toml 書込み等)が落ち着くのを待つ
            before = len(read_log(work))
            send_clear(hwnd)
            info["cleared"] = wait_for_cleared(work, before)
    finally:
        stop_awase(proc)
    info["cache_after"] = (work / "cache.toml").read_text(encoding="utf-8", errors="replace")
    info["learn_after"] = (work / "keymap-learn-table.json").read_text(encoding="utf-8", errors="replace")
    (out / "logs").mkdir(parents=True, exist_ok=True)
    (out / "logs" / f"{name}.awase.log").write_text("\n".join(read_log(work)), encoding="utf-8")
    shutil.copy(work / "cache.toml", out / "logs" / f"{name}.cache.after.toml")
    return info


def judge_a(info: dict) -> list[str]:
    errs = []
    if not info["tray_found"]:
        return ["トレイウィンドウが見つからない(awase が起動していない)"]
    if info["cleared"] is None:
        return ["クリアのログが出ない(WM_COMMAND が処理されていない=BUG-108 の no-op 再発の疑い)"]
    removed, persisted = info["cleared"]
    if removed < SEEDED_COUNT:
        errs.append(f"消した件数 {removed} が仕込み {SEEDED_COUNT} 件より少ない")
    if not persisted:
        errs.append("persisted=false(cache.toml に反映されていない)")
    try:
        after = tomllib.loads(info["cache_after"])
    except tomllib.TOMLDecodeError as e:
        return errs + [f"クリア後の cache.toml が壊れている: {e}"]
    imm = after.get("imm_capability", {})
    for proc_name in SEEDED:
        if proc_name in imm:
            errs.append(f"[imm_capability] に {proc_name} が残っている")
    if after.get("injection_mode", {}).get("Some.Class") != "tsf":
        errs.append("[injection_mode] が消えた/変わった(他セクションは残す約束)")
    if info["learn_after"] != LEARN_TABLE:
        errs.append("学習表 keymap-learn-table.json が変わった(触れない約束)")
    return errs


def judge_b(info: dict) -> list[str]:
    errs = []
    if not info["tray_found"]:
        return ["トレイウィンドウが見つからない(awase が起動していない)"]
    if info["cleared"] is None:
        return ["クリアのログが出ない"]
    if info["cleared"][1]:
        errs.append("壊れた cache.toml なのに persisted=true")
    if info["cache_after"] != BROKEN_CACHE:
        errs.append("壊れた cache.toml を上書きした(ADR-198 決定5違反: 手編集の内容が失われる)")
    if info["learn_after"] != LEARN_TABLE:
        errs.append("学習表が変わった")
    return errs


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dist", required=True, type=Path)
    ap.add_argument("--out", required=True, type=Path)
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)

    cases = [("a-clear", GOOD_CACHE, judge_a), ("b-broken-cache", BROKEN_CACHE, judge_b)]
    results, failed = [], False
    for name, cache_text, judge in cases:
        info = run_case(args.dist, args.out, name, cache_text)
        errs = judge(info)
        failed |= bool(errs)
        results.append({"name": name, "verdict": "FAIL" if errs else "PASS", "errors": errs,
                        "cleared": info["cleared"]})
    (args.out / "results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf-8")
    lines = ["# tray-cache-clear (BUG-108)", "", "| 構成 | 判定 | 詳細 |", "|---|---|---|"]
    for r in results:
        detail = "; ".join(r["errors"]) or "cleared={}".format(r["cleared"])
        lines.append(f"| {r['name']} | {r['verdict']} | {detail} |")
    summary = "\n".join(lines) + "\n"
    (args.out / "summary.md").write_text(summary, encoding="utf-8")
    print(summary)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as f:
            f.write(summary)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
