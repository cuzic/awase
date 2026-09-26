#!/usr/bin/env python3
"""設定の実機確認(ADR-201)。実際の awase.exe を config を変えて順に起動し、awase.log と操作結果で判定する。

使い方(windows-latest): python tools/e2e/config_verify/run.py --dist dist --out out
  dist/ に awase.exe と config_verify_probe.exe がある前提。out/ に結果(results.json, summary.md, logs/)を書く。

判定は PASS / FAIL / UNVERIFIABLE。FAIL は「期待と違った挙動」(見つかった不具合)、UNVERIFIABLE は「CI の環境では確かめられない」。
日本語を含む設定は Python(UTF-8)から TOML ファイルとして書く(PowerShell スクリプトには直書きしない)。
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

WARN_RE = re.compile(r"\bstartup: ")
NOTE_RE = re.compile(r"\bstartup note: ")


# ---------------------------------------------------------------- 設定(構成)

def hotkey_cfg(h: str) -> str:
    return f'[general]\nengine_toggle_hotkey = "{h}"\n'


A1_HOTKEYS = {
    "a1-vk": "Ctrl+Shift+VK_F12",  # GUI の書き方(BUG-167)
    "a1-plain": "Ctrl+Shift+F12",  # 手書き
    "a1-jp": "Ctrl+Shift+変換",  # 日本語名
    "a1-lower": "ctrl+shift+f12",  # 小文字
}

CFG_A2 = """\
[general]
muhenkan_solo_tap_dedicated_fn_key = "F18"

[keys]
engine_on = ["Ctrl+F12"]

[keys.ime_detect]
on = ["F13"]

[[post_bypass]]
key = "Ctrl+J"
"""

CFG_A3 = """\
[general]
no_such_option = 1
apply_calibrated_mode_keys = true

[[keymap]]
from = "Ctrl+VK_I"
to = ["VK_TAB"]

[[keymapz]]
from = "Ctrl+VK_K"
to = ["VK_TAB"]
"""

CFG_A4 = """\
[general]
engine_toggle_hotkey = "Ctrl+Shift+NoSuchKey"

[keys]
engine_on = ["Ctrl+NoSuchKey"]
"""

CFG_C = """\
[[keymaps]]
from = "Ctrl+VK_P"
to = ["VK_UP"]

[[keymaps]]
from = "Ctrl+VK_N"
to = ["VK_DOWN"]
"""

CFG_BASE = "[general]\n"


# ---------------------------------------------------------------- 起動・停止

def start_awase(dist: Path, work: Path, config: str) -> subprocess.Popen:
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    shutil.copy(dist / "awase.exe", work / "awase.exe")
    (work / "config.toml").write_text(config, encoding="utf-8", newline="\n")
    # CI ランナーの初回クロスプロセス IMM プローブが遅く「IMM 不可」と誤学習されるのを防ぐ(e2e-ime.yml と同じ)。
    (work / "cache.toml").write_text(
        '[imm_capability."config_verify_probe.exe"]\nEdit = "works"\n', encoding="utf-8", newline="\n"
    )
    env = dict(os.environ, RUST_LOG="debug", AWASE_TEST_INJECTION="1")
    return subprocess.Popen([str(work / "awase.exe")], cwd=work, env=env,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def wait_stable(work: Path, proc: subprocess.Popen, min_s: float = 5.0, max_s: float = 40.0) -> None:
    """awase.log が現れ、しばらく増えなくなるまで待つ(起動直後の初期化を待つ)。"""
    t0 = time.time()
    last_size, last_change = -1, time.time()
    log = work / "awase.log"
    while time.time() - t0 < max_s:
        if proc.poll() is not None:
            return
        size = log.stat().st_size if log.exists() else 0
        if size != last_size:
            last_size, last_change = size, time.time()
        if time.time() - t0 >= min_s and size > 0 and time.time() - last_change >= 2.0:
            return
        time.sleep(0.5)


def stop_awase(proc: subprocess.Popen) -> str:
    """WM_CLOSE(taskkill /PID、/f なし)で正常終了させる(BufWriter の未フラッシュ分を残すため)。効かなければ kill。"""
    if proc.poll() is not None:
        return f"awase は既に終了(rc={proc.returncode})"
    subprocess.run(["taskkill", "/PID", str(proc.pid)], capture_output=True)
    try:
        proc.wait(timeout=8)
        return f"正常終了(rc={proc.returncode})"
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=8)
        return "WM_CLOSE で終わらず kill"


def read_log(work: Path) -> list[str]:
    p = work / "awase.log"
    if not p.exists():
        return []
    return p.read_text(encoding="utf-8", errors="replace").splitlines()


def run_case(dist: Path, out: Path, name: str, config: str, probe_args: list[list[str]] | None = None) -> dict:
    """1構成=1起動。probe_args の各要素は config_verify_probe.exe の引数(順に実行)。"""
    work = out / "work" / name
    proc = start_awase(dist, work, config)
    info: dict = {"name": name, "probe": []}
    try:
        wait_stable(work, proc)
        for args in probe_args or []:
            probe_log = work / f"probe-{len(info['probe'])}.log"
            pp = subprocess.Popen([str(dist / "config_verify_probe.exe"), *args, f"--log={probe_log}"], cwd=work)
            t0 = time.time()
            done = False
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
            rows = []
            if probe_log.exists():
                for line in probe_log.read_text(encoding="utf-8", errors="replace").splitlines():
                    if line.startswith("[CV-JSON] "):
                        rows.append(json.loads(line[len("[CV-JSON] "):]))
            info["probe"].append({"args": args, "done": done, "rows": rows})
    finally:
        info["stop"] = stop_awase(proc)
    info["lines"] = read_log(work)
    (out / "logs").mkdir(parents=True, exist_ok=True)
    (out / "logs" / f"{name}.awase.log").write_text("\n".join(info["lines"]), encoding="utf-8")
    for i, _ in enumerate(info["probe"]):
        pl = work / f"probe-{i}.log"
        if pl.exists():
            shutil.copy(pl, out / "logs" / f"{name}.probe-{i}.log")
    return info


# ---------------------------------------------------------------- 判定(純粋関数)

def strip_prefix(line: str, rx: re.Pattern) -> str:
    m = rx.search(line)
    return line[m.end():].strip() if m else line


def warnings_of(lines: list[str]) -> list[str]:
    return [strip_prefix(l, WARN_RE) for l in lines if WARN_RE.search(l)]


def notes_of(lines: list[str]) -> list[str]:
    return [strip_prefix(l, NOTE_RE) for l in lines if NOTE_RE.search(l)]


def new_warnings(lines: list[str], baseline: list[str]) -> list[str]:
    base = set(baseline)
    return [w for w in warnings_of(lines) if w not in base]


def short(s: str, n: int = 160) -> str:
    s = s.replace("|", "\\|")
    return s if len(s) <= n else s[: n - 1] + "…"


def R(item: str, status: str, evidence: str) -> dict:
    return {"item": item, "status": status, "evidence": evidence}


def judge_hotkey_registered(info: dict, hotkey: str, baseline: list[str], item: str) -> dict:
    lines = info["lines"]
    reg = [l for l in lines if f"Toggle hotkey registered: {hotkey}" in l]
    bad = [l for l in lines if "Invalid toggle hotkey format" in l or "Failed to register toggle hotkey" in l]
    nw = new_warnings(lines, baseline)
    if reg and not bad and not nw:
        return R(item, "PASS", f"`Toggle hotkey registered: {hotkey}` あり、Invalid/Failed なし、新規警告なし")
    parts = []
    if not reg:
        parts.append("`Toggle hotkey registered` が無い")
    if bad:
        parts.append("失敗ログ: " + short(bad[0]))
    if nw:
        parts.append("新規警告: " + short("; ".join(nw)))
    return R(item, "FAIL", " / ".join(parts))


def judge_a2(info: dict, baseline: list[str]) -> list[dict]:
    lines = info["lines"]
    nw = new_warnings(lines, baseline)
    notes = notes_of(lines)
    out = []
    if not nw:
        out.append(R("A-2 受理(警告なし)", "PASS", "F18 / Ctrl+F12 / F13 / Ctrl+J のいずれも新規警告なし"))
    else:
        out.append(R("A-2 受理(警告なし)", "FAIL", "新規警告: " + short("; ".join(nw), 400)))
    eff = [n for n in notes if "以前は無視されていた設定" in n]
    if eff:
        missing = [t for t in ("F18", "F13", "Ctrl+F12", "Ctrl+J") if t not in eff[0]]
        m = re.search(r"(\d+) 件", eff[0])
        if missing:
            out.append(R("A-2 「有効になりました」note", "FAIL",
                         f"note はあるが {missing} が列挙されていない: {short(eff[0], 300)}"))
        else:
            out.append(R("A-2 「有効になりました」note", "PASS",
                         f"info の note あり({m.group(1) if m else '?'} 件): {short(eff[0], 300)}"))
    else:
        out.append(R("A-2 「有効になりました」note", "FAIL", "「以前は無視されていた設定」の note が出ていない"))
    return out


def judge_a3(info: dict, baseline: list[str]) -> list[dict]:
    lines = info["lines"]
    notes = notes_of(lines)
    nw = new_warnings(lines, baseline)
    out = []
    km = [n for n in notes if "[[keymap]]" in n and "[[keymaps]] として読みました" in n]
    out.append(R("A-3 `[[keymap]]` 合流 note", "PASS" if km else "FAIL",
                 short(km[0], 200) if km else "「[[keymap]] N 件を [[keymaps]] として読みました」が note に無い"))
    unk = [n for n in notes if "no_such_option" in n]
    unk2 = [n for n in notes if "keymapz" in n]
    ok_unk = bool(unk)
    ok_sug = bool(unk2) and "間違いではありませんか" in unk2[0]
    ev = []
    ev.append("no_such_option: " + (short(unk[0], 140) if unk else "note 無し"))
    ev.append("keymapz: " + (short(unk2[0], 160) if unk2 else "note 無し"))
    out.append(R("A-3 未知キーの note(近い名前つき)", "PASS" if ok_unk and ok_sug else "FAIL", " / ".join(ev)))
    leaked = [w for w in nw if "no_such_option" in w or "keymapz" in w or "[[keymap]]" in w]
    if nw:
        out.append(R("A-3 未知キー・旧表記が警告(トレイ)にならない", "FAIL",
                     "新規の警告あり(=トレイ通知の対象): " + short("; ".join(nw), 300)))
    else:
        out.append(R("A-3 未知キー・旧表記が警告(トレイ)にならない", "PASS",
                     "startup 警告は baseline と同じ(=警告件数0ならトレイ通知は出ない。トレイ自体は観測していない)"))
    rem = [l for l in lines if "apply_calibrated_mode_keys" in l]
    out.append(R("A-3 撤去済みキーは警告しない", "PASS" if not rem else "FAIL",
                 "ログに `apply_calibrated_mode_keys` の言及なし" if not rem else short(rem[0], 200)))
    return out


def judge_a4(info: dict, baseline: list[str]) -> dict:
    nw = new_warnings(info["lines"], baseline)
    hk = [w for w in nw if "engine_toggle_hotkey" in w and "NoSuchKey" in w]
    ek = [w for w in nw if "NoSuchKey" in w and "engine_toggle_hotkey" not in w]
    if hk and ek:
        return R("A-4 存在しないキー名は警告", "PASS", "警告2件(トレイ通知の対象): " + short(hk[0], 110) + " / " + short(ek[0], 110))
    return R("A-4 存在しないキー名は警告", "FAIL",
             f"toggle_hotkey 警告={'あり' if hk else '無し'}、keys.engine_on 警告={'あり' if ek else '無し'}。新規警告: " + short("; ".join(nw), 300))


def probe_rows(info: dict, typ: str) -> list[dict]:
    return [r for p in info["probe"] for r in p["rows"] if r.get("type") == typ]


def judge_hotkey_injection(infos: list[tuple[str, dict]]) -> dict:
    """infos: [(ラベル, info)]。各 info は Ctrl+Shift+F12 を注入した1起動。"""
    ev, toggled_any, control_any = [], False, False
    for label, info in infos:
        toggled = [l for l in info["lines"] if "Engine user_enabled toggled:" in l]
        reg = any("Toggle hotkey registered" in l for l in info["lines"])
        ctl = probe_rows(info, "control_hotkey")
        cf = bool(ctl and ctl[0].get("fired"))
        sent = probe_rows(info, "awase_hotkey_injected")
        so = bool(sent and sent[0].get("sent_ok"))
        toggled_any |= bool(toggled)
        control_any |= cf
        ev.append(f"{label}: 登録={'済' if reg else '無し'} 注入={'OK' if so else 'NG/未実行'} 対照(自前RegisterHotKey)={'発火' if cf else '不発'} "
                  f"toggled={len(toggled)}件" + (f"({short(toggled[0], 100)})" if toggled else ""))
    if toggled_any:
        return R("B ホットキーが効く", "PASS", " ; ".join(ev))
    if control_any:
        return R("B ホットキーが効く", "FAIL", "対照の RegisterHotKey は注入で発火したのに awase は toggle しない。 " + " ; ".join(ev))
    return R("B ホットキーが効く", "UNVERIFIABLE", "注入では対照の RegisterHotKey も発火せず、CI では確かめられない。 " + " ; ".join(ev))


def judge_caret(info: dict, name: str) -> list[dict]:
    rows = {r["label"]: r for r in probe_rows(info, "trial")}
    foc = probe_rows(info, "focus") + probe_rows(info, "focus_retry")
    if not rows:
        return [R(name, "UNVERIFIABLE", "probe の結果が無い(起動失敗・フォーカス失敗?)")]
    focused = all(r.get("focused") for r in rows.values())
    ctl_ok = all(rows.get(k, {}).get("as_expected") for k in ("control_up", "control_down"))

    def desc(k: str) -> str:
        r = rows.get(k)
        return f"{k}: 行{r['line_before']}→{r['line_after']}(期待{r['expect_line']})" if r else f"{k}: 結果なし"

    ev = " ; ".join(desc(k) for k in ("control_up", "control_down", "ctrl_p_up", "ctrl_n_down", "after_up"))
    if not focused or not ctl_ok:
        return [R(name, "UNVERIFIABLE",
                  f"ハーネス自体が成立しない(フォーカス={'OK' if focused else '外れ'}、素の Up/Down の対照={'OK' if ctl_ok else 'NG'})。{ev}")]
    return [R(name + " 対照(素の↑↓)", "PASS", "素の VK_UP/VK_DOWN の注入でキャレットの行が動いた(ハーネスは成立)"),
            *judge_ctrl_np(rows, name, ev)]


def judge_ctrl_np(rows: dict, name: str, ev: str) -> list[dict]:
    p, n = rows.get("ctrl_p_up", {}), rows.get("ctrl_n_down", {})
    if name.startswith("C-0"):  # keymap なしの対照: 動かないのが正しい
        moved = p.get("moved") or n.get("moved")
        return [R(name, "PASS" if not moved else "FAIL", ev)]
    if p.get("as_expected") and n.get("as_expected"):
        return [R(name, "PASS", ev)]
    return [R(name, "FAIL", ev)]


# ---------------------------------------------------------------- 実行

def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dist", default="dist")
    ap.add_argument("--out", default="out")
    ap.add_argument("--only", default="", help="カンマ区切りの構成名(空=全部)")
    a = ap.parse_args()
    dist, out = Path(a.dist).resolve(), Path(a.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    only = {s for s in a.only.split(",") if s}
    want = lambda n: not only or n in only  # noqa: E731
    results: list[dict] = []
    infos: dict[str, dict] = {}

    def go(name: str, cfg: str, probes: list[list[str]] | None = None) -> dict:
        print(f"::group::{name}", flush=True)
        info = run_case(dist, out, name, cfg, probes)
        infos[name] = info
        print(f"stop: {info['stop']}, log lines: {len(info['lines'])}", flush=True)
        print("\n".join(l for l in info["lines"] if WARN_RE.search(l) or NOTE_RE.search(l) or "otkey" in l), flush=True)
        print("::endgroup::", flush=True)
        return info

    base = go("a0-baseline", CFG_BASE)
    baseline = warnings_of(base["lines"])
    results.append(R("(参考) baseline の startup 警告", "INFO", f"{len(baseline)} 件: " + short("; ".join(baseline), 300)))

    for name, hk in A1_HOTKEYS.items():
        if want(name):
            info = go(name, hotkey_cfg(hk))
            results.append(judge_hotkey_registered(info, hk, baseline, f"A-1 {hk}"))
    if want("a2"):
        results.extend(judge_a2(go("a2", CFG_A2), baseline))
    if want("a3"):
        results.extend(judge_a3(go("a3", CFG_A3), baseline))
    if want("a4"):
        results.append(judge_a4(go("a4", CFG_A4), baseline))
    if want("b"):
        bs = []
        for label, marker in (("素のSendInput", "0"), ("目印付き", "1")):
            bs.append((label, go(f"b-marker{marker}", hotkey_cfg("Ctrl+Shift+VK_F12"), [["--mode=hotkey", f"--marker={marker}"]])))
        results.append(judge_hotkey_injection(bs))
    if want("c"):
        results.extend(judge_caret(go("c0-nokeymap", CFG_BASE, [["--mode=caret"]]), "C-0 keymap なし(Ctrl+P/N は動かない)"))
        results.extend(judge_caret(go("c-keymaps", CFG_C, [["--mode=caret"]]), "C Ctrl+P→↑ / Ctrl+N→↓"))

    (out / "results.json").write_text(json.dumps(results, ensure_ascii=False, indent=1), encoding="utf-8")
    md = ["| 項目 | 判定 | 根拠 |", "|---|---|---|"]
    for r in results:
        md.append(f"| {r['item']} | **{r['status']}** | {r['evidence']} |")
    summary = "\n".join(md) + "\n"
    (out / "summary.md").write_text(summary, encoding="utf-8")
    print(summary)
    sp = os.environ.get("GITHUB_STEP_SUMMARY")
    if sp:
        with open(sp, "a", encoding="utf-8") as f:
            f.write("## config-verify (ADR-201 実機確認)\n\n" + summary)
    # FAIL があっても検証結果として報告する(ジョブは成功で終える。判定は summary と artifact を見る)。
    return 0


if __name__ == "__main__":
    sys.exit(main())
