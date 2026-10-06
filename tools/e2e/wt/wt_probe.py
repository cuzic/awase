#!/usr/bin/env python3
"""Windows Terminal を CI の実機で制御できるかを測り(可否表)、BUG-113 の再現手順を実行する(Windows 専用)。

相:
  V  vocab    awase なし。wt 起動・前面化・打鍵・echo 読み戻し・UIA 読み取り・新規タブ/分割/閉じる・窓を閉じる、の可否
  I  ime      awase(debug、AWASE_TEST_INJECTION=1)+ GJI。IME ON にして Engine 越しに打ち、シェルが受け取った文字を分類
  B  bug113   awase+GJI で Windows Terminal に物理半角/全角(0xF3/0xF4、マーカー付き注入)を N 回押し、余計な「@」(U+0040)を数える。
              (docs/known-bugs/BUG-113.md の再現手順。Engine 有効のまま 1 回押す → 「@」が 1 文字出る)
  N  bug113-noawase  B と同じ操作を awase なしで(対照。「@」が出るなら awase 起因ではない)
  S  bug113-scan    B の変種: 実機の半角/全角の scan code(0x29)を付け、回数を --presses2(既定 30)・間隔 0.6 秒に増やす
  H  bug121         Ctrl+無変換(keys.ime_off 既定)を 20 回(BUG-121: 実 IME と belief がずれた直後に稀に「@」)。1 回ごとに外から IME を ON に戻してずれを作る
  D  bug114-keys   BUG-114 の起動時経路: 端末を先に開いて前面にしてから awase を起動する(起動時のフォーカススコープが
                   Windows Terminal=TsfNative)。フォーカスを動かさずに IME OFF キー → 1.8 秒待つ → 1.2 秒間隔で文字キーを 4 回
                   (awase の idle conv check を通す)→ 10 秒待つ。awase.log を wt_pure.judge_bug114 で判定する(合否を付ける唯一の相)
  E  bug114-forceopen  D の変種: IME OFF キーの後、実 IME を WM_IME_CONTROL(IMC_SETOPENSTATUS,1)で外から開いてから打つ
使い方: python wt_probe.py --dist dist --out out [--phases V,I,B,N] [--presses 10]
出力: out/results.json, out/summary.md, out/logs/*, out/shots/*.png
判定は付けない(観測と可否表)。実機で何が起きたかの事実だけを残す。例外は相 D/E(BUG-114 の判定、終了コード 0=PASS・1=FAIL・3=INVALID)。
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import wt_pure as P  # noqa: E402
import wt_vocab as W  # noqa: E402


def now():
    return time.strftime("%H:%M:%S")


def rec(results, **kw):
    kw["at"] = now()
    results.append(kw)
    print(json.dumps(kw, ensure_ascii=False), flush=True)
    return kw


# ---------------------------------------------------------------- awase

def start_awase(dist: Path, work: Path, repo: Path):
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    shutil.copy(dist / "awase.exe", work / "awase.exe")
    if (repo / "layout").exists():
        shutil.copytree(repo / "layout", work / "layout")
    (work / "config.toml").write_text('[general]\nlayouts_dir = "layout"\ndefault_layout = "nicola_keytop.yab"\n',
                                      encoding="utf-8", newline="\n")
    env = dict(os.environ, RUST_LOG="debug", AWASE_TEST_INJECTION="1")
    p = subprocess.Popen([str(work / "awase.exe")], cwd=work, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    log = work / "awase.log"
    t0, last_size, last_change = time.time(), -1, time.time()
    while time.time() - t0 < 40:
        if p.poll() is not None:
            break
        size = log.stat().st_size if log.exists() else 0
        if size != last_size:
            last_size, last_change = size, time.time()
        if time.time() - t0 >= 5 and size > 0 and time.time() - last_change >= 2.0:
            break
        time.sleep(0.5)
    return p


def stop_awase(p):
    if p.poll() is not None:
        return f"awase は既に終了(rc={p.returncode})"
    subprocess.run(["taskkill", "/PID", str(p.pid)], capture_output=True)
    try:
        p.wait(timeout=8)
        return f"正常終了(rc={p.returncode})"
    except subprocess.TimeoutExpired:
        p.kill()
        return "WM_CLOSE で終わらず kill"


def awase_lines(work: Path):
    f = work / "awase.log"
    return f.read_text(encoding="utf-8", errors="replace").splitlines() if f.exists() else []


# ---------------------------------------------------------------- 共通の準備

def open_terminal(results, out: Path, tag: str, seconds=240):
    """wt を echo 付きで起動して前面化する。(hwnd, echo パス) を返す。失敗は (None, echo)。"""
    echo = out / "work" / f"{tag}.echo.txt"
    echo.parent.mkdir(parents=True, exist_ok=True)
    if echo.exists():
        echo.unlink()
    pre = W.windows()
    hwnd, info = W.launch(W.echo_args(echo, seconds))
    r = rec(results, type="launch", tag=tag, ok=hwnd is not None, preexisting_windows=len(pre), **{k: v for k, v in info.items() if k != "windows_after"})
    if hwnd is None:
        return None, echo
    ready = W.wait_ready(echo, 40)
    fg = W.foreground(hwnd)
    rec(results, type="ready", tag=tag, echo_ready=ready, foreground=fg, foreground_hwnd=W.foreground_hwnd(), hwnd=hwnd)
    W.screenshot(str(out / "shots" / f"{tag}-opened.png"))
    return hwnd, echo


def finish_terminal(results, tag, hwnd):
    W.close(hwnd)
    time.sleep(2.0)
    left = [w for w in W.windows() if w["hwnd"] == hwnd]
    rec(results, type="close", tag=tag, closed=not left)


# ---------------------------------------------------------------- 相 V

def phase_vocab(results, out: Path):
    tag = "V"
    hwnd, echo = open_terminal(results, out, tag)
    if hwnd is None:
        return
    # 1) 打鍵(ASCII、scan code 付き VK)
    t0 = int(time.time() * 1000)
    n = W.type_vks([0x41, 0x42, 0x43], 80)  # a b c
    time.sleep(1.5)
    rows = W.read_rows(echo)
    rec(results, type="send_vk", tag=tag, sent=n, received=P.text_of(rows), counts=P.classify(rows))
    # 2) Unicode 直接送信(IME を通さない)
    n = W.type_unicode("あい")
    time.sleep(1.5)
    rows2 = W.read_rows(echo)[len(rows):]
    rec(results, type="send_unicode", tag=tag, sent=n, received=P.text_of(rows2), counts=P.classify(rows2))
    # 3) UIA で画面を読む
    txt = W.uia_text(hwnd)
    rec(results, type="uia_text", tag=tag, textpattern_found="TEXTPATTERN_COUNT=0" not in txt and "TEXTPATTERN class" in txt,
        has_abc="abc" in txt, has_kana="あい" in txt, excerpt=txt[:600])
    # 4) タブ・分割・ペインを閉じる(UIA のタブ数と echo への到達で確かめる)
    tabs0 = W.uia_text(hwnd, count_tabs=True)
    W.shortcut_new_tab()
    time.sleep(3.0)
    tabs1 = W.uia_text(hwnd, count_tabs=True)
    rec(results, type="new_tab", tag=tag, before=tabs0, after=tabs1)
    W.screenshot(str(out / "shots" / f"{tag}-newtab.png"))
    # 新規タブは既定のシェルなので echo には届かない=フォーカスが新しいタブへ移った証拠。閉じて echo に戻るかを見る
    before = len(W.read_rows(echo))
    W.press(0x58, 30)  # x
    time.sleep(1.0)
    reached_new_tab = len(W.read_rows(echo)) - before
    W.shortcut_close_pane()
    time.sleep(2.0)
    W.press(0x59, 30)  # y
    time.sleep(1.0)
    reached_after_close = len(W.read_rows(echo)) - before
    rec(results, type="tab_focus", tag=tag, echo_got_while_on_new_tab=reached_new_tab, echo_total_after_close=reached_after_close,
        tabs_after_close=W.uia_text(hwnd, count_tabs=True))
    W.shortcut_split_pane()
    time.sleep(2.5)
    before = len(W.read_rows(echo))
    W.press(0x5A, 30)  # z
    time.sleep(1.0)
    got_in_split = len(W.read_rows(echo)) - before
    W.screenshot(str(out / "shots" / f"{tag}-split.png"))
    W.shortcut_close_pane()
    time.sleep(2.0)
    W.press(0x51, 30)  # q
    time.sleep(1.0)
    rec(results, type="split_pane", tag=tag, echo_got_while_in_split=got_in_split, echo_got_after_close=len(W.read_rows(echo)) - before)
    finish_terminal(results, tag, hwnd)


# ---------------------------------------------------------------- 相 I / B / N

KEYS_NICOLA = [0x4B, 0x41, 0x54, 0x41]  # k a t a(Engine 有効なら NICOLA の配列でかなになる)


def with_awase(results, out: Path, dist: Path, repo: Path, tag: str, use_awase: bool, body):
    proc, work = None, out / "work" / tag
    if use_awase:
        proc = start_awase(dist, work, repo)
        rec(results, type="awase_start", tag=tag, running=proc.poll() is None, log_lines=len(awase_lines(work)))
    hwnd, echo = open_terminal(results, out, tag)
    try:
        if hwnd is not None:
            body(results, out, tag, hwnd, echo, work)
    finally:
        if hwnd is not None:
            finish_terminal(results, tag, hwnd)
        if proc:
            rec(results, type="awase_stop", tag=tag, result=stop_awase(proc))
            lines = awase_lines(work)
            (out / "logs").mkdir(parents=True, exist_ok=True)
            (out / "logs" / f"{tag}.awase.log").write_text("\n".join(lines), encoding="utf-8")


def ime_on(results, tag):
    n = W.press(W.VK["HIRAGANA"], 60, marker=True)  # ひらがな(IME ON)
    time.sleep(1.5)
    rec(results, type="ime_on_key", tag=tag, sent=n)


def body_ime(results, out, tag, hwnd, echo, work):
    ime_on(results, tag)
    n = W.type_vks(KEYS_NICOLA, 90, marker=True)
    time.sleep(1.5)
    W.screenshot(str(out / "shots" / f"{tag}-typed.png"))
    W.press(W.VK["RETURN"], 40, marker=True)
    time.sleep(1.5)
    rows = W.read_rows(echo)
    rec(results, type="ime_typing", tag=tag, sent=n, received=P.text_of(rows), counts=P.classify(rows))


def make_body_bug113(presses, scan=None, gap=1.2):
    def body(results, out, tag, hwnd, echo, work):
        ime_on(results, tag)
        base = len(W.read_rows(echo))
        rec(results, type="bug113_begin", tag=tag, presses=presses)
        for i in range(presses):
            vk = W.VK["SBCSCHAR"] if i % 2 == 0 else W.VK["DBCSCHAR"]  # 物理半角/全角は押すたびに 0xF3/0xF4 が交互に届く(BUG-113)
            t = int(time.time() * 1000)
            n = W.press(vk, 60, marker=True, scan=scan)
            time.sleep(gap)
            rows = W.read_rows(echo)[base:]
            rec(results, type="bug113_press", tag=tag, i=i + 1, vk=f"0x{vk:02X}", scan=scan, sent=n, received_so_far=P.text_of(rows), at_so_far=P.classify(rows)["at"], t=t)
        W.screenshot(str(out / "shots" / f"{tag}-after.png"))
        W.press(W.VK["RETURN"], 40, marker=True)
        time.sleep(1.5)
        rows = W.read_rows(echo)[base:]
        rec(results, type="bug113_result", tag=tag, presses=presses, received=P.text_of(rows), counts=P.classify(rows))
    return body


def make_body_bug121(presses):
    def body(results, out, tag, hwnd, echo, work):
        ime_on(results, tag)
        base = len(W.read_rows(echo))
        rec(results, type="bug121_begin", tag=tag, presses=presses)
        for i in range(presses):
            if i % 2 == 0:
                W.press(W.VK["HIRAGANA"], 40, marker=True)  # 外から IME を ON に戻す(awase の OFF の後にずれを作る)
                time.sleep(0.6)
            n = W.chord([W.VK["CTRL"]], W.VK["NONCONVERT"], 50, marker=True)  # Ctrl+無変換(keys.ime_off 既定)
            time.sleep(1.0)
            rows = W.read_rows(echo)[base:]
            rec(results, type="bug121_press", tag=tag, i=i + 1, sent=n, received_so_far=P.text_of(rows), at_so_far=P.classify(rows)["at"])
        W.screenshot(str(out / "shots" / f"{tag}-after.png"))
        W.press(W.VK["RETURN"], 40, marker=True)
        time.sleep(1.5)
        rows = W.read_rows(echo)[base:]
        rec(results, type="bug121_result", tag=tag, presses=presses, received=P.text_of(rows), counts=P.classify(rows))
    return body


# ---------------------------------------------------------------- 相 D / E(BUG-114)

def phase_bug114(results, out: Path, dist: Path, repo: Path, tag: str, force_open: bool):
    """端末を先に開いて前面にしてから awase を起動し(起動時スコープ=Windows Terminal)、フォーカスを動かさずに
    desired=閉 と実 IME をずらして drift 補正を起こす。別窓へ移ると FocusChanged で app_policy が作り直され、起動時経路を見られない。"""
    work = out / "work" / tag
    hwnd, echo = open_terminal(results, out, tag)
    if hwnd is None:
        return
    proc = start_awase(dist, work, repo)
    rec(results, type="awase_start", tag=tag, running=proc.poll() is None, log_lines=len(awase_lines(work)),
        foreground_is_terminal=W.foreground_hwnd() == hwnd)
    try:
        W.foreground(hwnd)
        W.press(W.VK["IME_OFF"], 50, marker=True)
        time.sleep(1.8)  # 明示 IME 操作直後は idle conv check が止まる(EXPLICIT_IME_SUPPRESS_MS=1500ms)
        open_after_off = W.ime_control(hwnd, 0x0005)
        set_ret = W.ime_control(hwnd, 0x0006, 1) if force_open else None
        time.sleep(0.3)
        open_before_keys = W.ime_control(hwnd, 0x0005)
        for _ in range(4):
            W.press(W.VK["A"], 60, marker=True)  # TYPING_IDLE_MS(500ms)を超える間隔で打つ
            time.sleep(1.2)
        time.sleep(10.0)
        rec(results, type="bug114_drive", tag=tag, force_open=force_open, set_ret=set_ret, open_after_off=open_after_off,
            open_before_keys=open_before_keys, open_end=W.ime_control(hwnd, 0x0005), foreground_is_terminal=W.foreground_hwnd() == hwnd)
    finally:
        finish_terminal(results, tag, hwnd)
        rec(results, type="awase_stop", tag=tag, result=stop_awase(proc))
        lines = awase_lines(work)
        (out / "logs").mkdir(parents=True, exist_ok=True)
        (out / "logs" / f"{tag}.awase.log").write_text("\n".join(lines), encoding="utf-8")
        rec(results, type="bug114_result", tag=tag, **P.judge_bug114(lines))


# ---------------------------------------------------------------- summary

def md(results):
    o = ["# Windows Terminal 制御語彙の実機probe", ""]
    o += ["| 相 | 項目 | 結果 | 補足 |", "|---|---|---|---|"]
    for r in results:
        t, tag = r.get("type"), r.get("tag", "")
        if t == "launch":
            o.append(f"| {tag} | wt 起動 | {r.get('ok')} | wt.exe={r.get('wt_exe')} 既存窓={r.get('preexisting_windows')} {r.get('launch_s', '')}s {r.get('error', '')} |")
        elif t == "ready":
            o.append(f"| {tag} | echo 準備・前面化 | echo={r.get('echo_ready')} fg={r.get('foreground')} | 前面 hwnd={r.get('foreground_hwnd')} / 対象={r.get('hwnd')} |")
        elif t == "send_vk":
            o.append(f"| {tag} | VK 打鍵→echo | 受信 `{r.get('received')}` | 送信入力数={r.get('sent')} |")
        elif t == "send_unicode":
            o.append(f"| {tag} | Unicode 直接送信→echo | 受信 `{r.get('received')}` | {r.get('counts')} |")
        elif t == "uia_text":
            o.append(f"| {tag} | UIA TextPattern | found={r.get('textpattern_found')} abc={r.get('has_abc')} かな={r.get('has_kana')} | |")
        elif t == "new_tab":
            o.append(f"| {tag} | 新規タブ(UIA タブ数) | {r.get('before')} → {r.get('after')} | |")
        elif t == "tab_focus":
            o.append(f"| {tag} | タブ切替で入力先が移る | 新タブ中の echo 受信={r.get('echo_got_while_on_new_tab')} 閉じた後の累計={r.get('echo_total_after_close')} | 閉じた後のタブ数={r.get('tabs_after_close')} |")
        elif t == "split_pane":
            o.append(f"| {tag} | 分割ペイン | 分割中の echo 受信={r.get('echo_got_while_in_split')} 閉じた後の累計={r.get('echo_got_after_close')} | |")
        elif t == "close":
            o.append(f"| {tag} | 窓を閉じる | {r.get('closed')} | |")
        elif t == "ime_typing":
            o.append(f"| {tag} | IME ON で打鍵→確定 | 受信 `{r.get('received')}` | {r.get('counts')} |")
        elif t == "bug121_result":
            c = r.get("counts", {})
            o.append(f"| {tag} | **BUG-121**: Ctrl+無変換を {r.get('presses')} 回 | 「@」={c.get('at')} 件 | 受信 `{r.get('received')}` / {c} |")
        elif t == "bug114_result":
            o.append(f"| {tag} | **BUG-114**: 起動時スコープのまま drift 補正 | {r.get('verdict')} | {r.get('counts')} {r.get('failures')} {r.get('invalid')} |")
        elif t == "bug113_result":
            c = r.get("counts", {})
            o.append(f"| {tag} | **BUG-113**: 半角/全角を {r.get('presses')} 回 | 「@」={c.get('at')} 件 | 受信 `{r.get('received')}` / {c} |")
    o.append("")
    o.append("`@` の件数は、物理半角/全角(0xF3/0xF4 を交互)を押した後にシェルが受け取った U+0040 の数。awase なし(相 N)が 0 で awase あり(相 B)が >0 なら awase 起因。")
    return "\n".join(o)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dist", default="dist")
    ap.add_argument("--out", default="out")
    ap.add_argument("--phases", default="V,I,B,N")
    ap.add_argument("--presses", type=int, default=10)
    ap.add_argument("--presses2", type=int, default=30)
    a = ap.parse_args()
    dist, out = Path(a.dist).resolve(), Path(a.out).resolve()
    repo = HERE.parents[2]
    (out / "shots").mkdir(parents=True, exist_ok=True)
    results: list = []
    rec(results, type="env", wt_exe=W.wt_exe(), windows=W.windows(), phases=a.phases)
    for ph in a.phases.split(","):
        try:
            if ph == "V":
                phase_vocab(results, out)
            elif ph == "I":
                with_awase(results, out, dist, repo, "I-ime", True, body_ime)
            elif ph == "B":
                with_awase(results, out, dist, repo, "B-bug113", True, make_body_bug113(a.presses))
            elif ph == "S":
                with_awase(results, out, dist, repo, "S-bug113-scan29", True, make_body_bug113(a.presses2, scan=0x29, gap=0.6))
            elif ph == "H":
                with_awase(results, out, dist, repo, "H-bug121", True, make_body_bug121(20))
            elif ph == "D":
                phase_bug114(results, out, dist, repo, "D-bug114-keys", False)
            elif ph == "E":
                phase_bug114(results, out, dist, repo, "E-bug114-forceopen", True)
            elif ph == "N":
                with_awase(results, out, dist, repo, "N-bug113-noawase", False, make_body_bug113(a.presses))
        except Exception as e:  # 1 相の失敗で全体を止めない
            import traceback
            rec(results, type="phase_error", phase=ph, error=f"{e!r}", tb=traceback.format_exc()[-800:])
        (out / "results.json").write_text(json.dumps(results, ensure_ascii=False, indent=1), encoding="utf-8")
        (out / "summary.md").write_text(md(results), encoding="utf-8")
    print(md(results))
    verdicts = [r["verdict"] for r in results if r.get("type") == "bug114_result"]
    if not verdicts:
        return 0
    return 1 if "FAIL" in verdicts else 3 if "INVALID" in verdicts else 0


if __name__ == "__main__":
    sys.exit(main())
