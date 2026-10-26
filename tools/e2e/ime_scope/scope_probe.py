#!/usr/bin/env python3
"""ADR-282 の測定: GJI・MS-IME が IME の開閉・変換モードを窓(スレッド)ごとに持つか、全体で共有するか(awase は起動しない)。

窓 A(別プロセス)で IME の状態を作って保持し、別プロセスの窓 B(新しいスレッド)で `ka` を打って結果(`か` か `ka` か)を読む。
A/B の種類: edit(typing_stress の自前 Win32 EDIT。読める窓)、chrome(実 Chrome のテキスト欄。Imm32Unavailable)、wt(Windows Terminal。tsf-native)。
セル(1 セル = 1 回の A→B。各セルの前に『IME OFF にして終了する edit の窓』で状態をそろえる):
  ctl_direct: A なし。B で何もせず ka(B が最初から開いているか)
  ctl_on    : A なし。B で ON キーを 1 回押して ka(B で IME が効くか。ひらがなのはず)
  s1        : A=開×ひらがな。B で何もせず ka        → か なら B は A の『開』を引き継いだ(共有)
  s3        : A=開×半角英数(英数キー)。B で ON キー→ka → ka なら B は A の『半角英数』を引き継いだ(共有)
A は自分で ka を打って状態を自己検証する(hira は か、eisu は ka)。合わないセルは判定に使わない。
使い方: scope_probe.py --dist dist --out out --ime gji|msime [--pairs edit>edit,edit>chrome,...] [--rounds 2]
"""
import argparse
import ctypes
import json
import os
import platform
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "wt"))
import scope_pure as P  # noqa: E402

TS_JSON = "[TS-JSON] "
FORM = {"edit": "edit", "chrome": "chromepage"}
IME_ON_VK = {"gji": 0x16, "msime": 0xF2}


def ts_records(path):
    out = []
    try:
        for line in open(path, encoding="utf-8", errors="replace"):
            i = line.find(TS_JSON)
            if i >= 0:
                try:
                    out.append(json.loads(line[i + len(TS_JSON):]))
                except ValueError:
                    pass
    except OSError:
        pass
    return out


def log_has(path, needle):
    try:
        return needle in open(path, encoding="utf-8", errors="replace").read()
    except OSError:
        return False


def wait_until(fn, timeout, step=0.5):
    t0 = time.time()
    while time.time() - t0 < timeout:
        if fn():
            return True
        time.sleep(step)
    return False


PROCS = []


def kill_all():
    """自分が起動した typing_stress だけを止める(名前での一括 taskkill は、初回の CI でランナーごと止まったので使わない)。
    Chrome・Windows Terminal は、typing_stress の shutdown と wt_vocab.close が閉じる。"""
    for p in PROCS:
        try:
            if p.poll() is None:
                p.kill()
        except OSError:
            pass
    PROCS.clear()
    time.sleep(1.0)


class Ctx:
    def __init__(self, a):
        self.dist = Path(a.dist).resolve()
        self.out = Path(a.out).resolve()
        (self.out / "logs").mkdir(parents=True, exist_ok=True)
        (self.out / "work").mkdir(parents=True, exist_ok=True)
        self.ime = a.ime
        self.layout = str(Path(a.layout).resolve()).replace("\\", "/")
        self.n = 0
        self.results = []

    def next_tag(self, what):
        self.n += 1
        return f"{self.n:03d}-{what}"


def ts_args(ctx, kind, log, extra):
    flag = "--msime" if ctx.ime == "msime" else "--activate-gji"
    return [str(ctx.dist / "typing_stress.exe"), f"--form={FORM[kind]}", "--mode=scope", "--no-awase", flag,
            f"--layout={ctx.layout}", "--settle-read", f"--log={log}"] + extra + (
        [f"--chrome-path={os.environ['SCOPE_CHROME_PATH']}"] if os.environ.get("SCOPE_CHROME_PATH") else [])


class ProcA:
    """窓 A。start() で状態を作って自己検証し、release() で窓を閉じる。"""

    def __init__(self, ctx, kind, state):
        self.ctx, self.kind, self.state = ctx, kind, state
        self.self_text = None
        self.ok = False
        self.proc = None
        self.hold = None
        self.tag = ctx.next_tag(f"A-{kind}-{state}")

    def start(self):
        c = self.ctx
        if self.kind == "wt":
            return self._start_wt()
        log = str(c.out / "logs" / f"{self.tag}.log")
        self.log = log
        self.hold = str(c.out / "work" / f"{self.tag}.hold")
        if os.path.exists(self.hold):
            os.remove(self.hold)
        self.proc = subprocess.Popen(ts_args(c, self.kind, log, ["--scope-role=a", f"--scope-state={self.state}",
                                                                  f"--scope-hold={self.hold}"]))
        PROCS.append(self.proc)
        wait_until(lambda: log_has(log, "[SCOPE] A-READY") or log_has(log, "=== 完了 ==="), 120)
        recs = [r for r in ts_records(log) if r.get("type") == "scope_a"]
        if recs:
            self.self_text = recs[-1].get("text", "")
            self.ok = True
        return self

    def _start_wt(self):
        import wt_vocab as W
        c = self.ctx
        echo = c.out / "work" / f"{self.tag}.echo.txt"
        self.echo = echo
        hwnd, info = W.launch(W.echo_args(echo, seconds=300))
        self.hwnd = hwnd
        if not hwnd or not W.wait_ready(echo):
            return self
        W.foreground(hwnd)
        time.sleep(1.5)
        set_state_wt(W, c.ime, self.state)
        self.self_text = type_ka_wt(W, echo)
        self.ok = True
        return self

    def release(self):
        if self.kind == "wt":
            try:
                import wt_vocab as W
                if self.hwnd:
                    W.close(self.hwnd)
            except Exception:
                pass
            return
        if self.hold:
            Path(self.hold).write_text("release", encoding="utf-8")
        if self.proc:
            try:
                self.proc.wait(timeout=60)
            except subprocess.TimeoutExpired:
                self.proc.kill()


def set_state_wt(W, ime, state):
    on = IME_ON_VK[ime]
    if state == "off":
        W.press(0x1A, hold_ms=50, scan=0x70)
        time.sleep(0.8)
        return
    W.press(0x1A, hold_ms=50, scan=0x70)
    time.sleep(0.6)
    W.press(on, hold_ms=50, scan=0x70)
    time.sleep(1.5)
    if state == "eisu":
        W.press(0xF0, hold_ms=50, scan=0x3A)
        time.sleep(0.8)


def type_ka_wt(W, echo):
    n0 = len(W.read_rows(echo))
    W.press(0x4B, hold_ms=40)
    time.sleep(0.12)
    W.press(0x41, hold_ms=40)
    time.sleep(0.7)
    W.press(0x0D, hold_ms=50)
    time.sleep(1.2)
    rows = W.read_rows(echo)[n0:]
    return "".join(chr(r[1]) for r in rows if r[1] >= 32 and r[1] != 127)


def run_b(ctx, kind, act):
    tag = ctx.next_tag(f"B-{kind}-{act}")
    if kind == "wt":
        import wt_vocab as W
        echo = ctx.out / "work" / f"{tag}.echo.txt"
        hwnd, info = W.launch(W.echo_args(echo, seconds=120))
        if not hwnd or not W.wait_ready(echo):
            return {"ok": False, "why": f"wt 起動失敗 {info}"}
        W.foreground(hwnd)
        time.sleep(1.5)
        if act == "ime-on":
            W.press(IME_ON_VK[ctx.ime], hold_ms=50, scan=0x70)
            time.sleep(0.8)
        text = type_ka_wt(W, echo)
        W.close(hwnd)
        return {"ok": True, "text": text}
    log = str(ctx.out / "logs" / f"{tag}.log")
    proc = subprocess.Popen(ts_args(ctx, kind, log, ["--scope-role=b", f"--scope-act={act}"]))
    PROCS.append(proc)
    done = wait_until(lambda: log_has(log, "=== 完了 ==="), 150)
    if not done:
        proc.kill()
    recs = [r for r in ts_records(log) if r.get("type") == "scope_b"]
    if not recs:
        return {"ok": False, "why": "scope_b 記録なし(abort 等)"}
    r = recs[-1]
    return {"ok": True, "text": r.get("text", ""), "open_before": r.get("open_before"),
            "open_after_act": r.get("open_after_act")}


def reset(ctx):
    a = ProcA(ctx, "edit", "off").start()
    a.release()
    kill_all()


def run_cell(ctx, a_kind, b_kind, cell, rnd):
    reset(ctx)
    a = None
    rec = {"a": a_kind, "b": b_kind, "cell": cell, "round": rnd}
    if cell in ("s1", "s3"):
        a = ProcA(ctx, a_kind, "hira" if cell == "s1" else "eisu").start()
        want = "hira" if cell == "s1" else "ka"
        rec["a_self"] = P.classify(a.self_text or "")
        rec["a_self_ok"] = a.ok and rec["a_self"] == want
    act = "none" if cell in ("s1", "ctl_direct") else "ime-on"
    b = run_b(ctx, b_kind, act)
    rec["b_ok"] = b.get("ok", False)
    rec["b_class"] = P.classify(b.get("text", "")) if b.get("ok") else "none"
    rec["b_open_before"] = b.get("open_before")
    rec["b_open_after_act"] = b.get("open_after_act")
    if not b.get("ok"):
        rec["why"] = b.get("why")
    if a:
        a.release()
    kill_all()
    ctx.results.append(rec)
    print(json.dumps(rec, ensure_ascii=False), flush=True)
    return rec


def usable(rows):
    return [r["b_class"] for r in rows if r["b_ok"] and r.get("a_self_ok", True)]


def summarize(ctx, pairs):
    rs = ctx.results
    lines = []
    for a_kind, b_kind in pairs:
        def cls(cell, a=None):
            return usable([r for r in rs if r["b"] == b_kind and r["cell"] == cell and (a is None or r["a"] == a)])
        cd, co = P.majority(cls("ctl_direct")), P.majority(cls("ctl_on"))
        s1, s3 = P.majority(cls("s1", a_kind)), P.majority(cls("s3", a_kind))
        n_s1 = len([r for r in rs if r["a"] == a_kind and r["b"] == b_kind and r["cell"] == "s1"])
        n_s3 = len([r for r in rs if r["a"] == a_kind and r["b"] == b_kind and r["cell"] == "s3"])
        o = P.judge_open(cd[0] or "none", s1[0] or "none")
        v = P.judge_conv(co[0] or "none", s3[0] or "none")
        lines.append({
            "ime": ctx.ime, "pair": f"{a_kind}>{b_kind}", "open": o, "conv": v,
            "ctl_direct": cd, "ctl_on": co, "s1": s1, "s3": s3, "s1_runs": n_s1, "s3_runs": n_s3})
    return lines


def env_info():
    info = {"platform": platform.platform(), "python": sys.version.split()[0]}
    try:
        b = ctypes.c_int(-1)
        ok = ctypes.windll.user32.SystemParametersInfoW(0x104E, 0, ctypes.byref(b), 0)
        info["SPI_GETTHREADLOCALINPUTSETTINGS"] = b.value if ok else f"失敗({ok})"
    except Exception as e:  # noqa: BLE001
        info["SPI_GETTHREADLOCALINPUTSETTINGS"] = f"取得不可 {e!r}"
    try:
        r = subprocess.run(["reg", "query", r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion", "/v", "ProductName"],
                           capture_output=True, text=True)
        info["ProductName"] = r.stdout.strip().splitlines()[-1].strip() if r.stdout.strip() else ""
        r = subprocess.run(["reg", "query", r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion", "/v", "CurrentBuildNumber"],
                           capture_output=True, text=True)
        info["Build"] = r.stdout.strip().splitlines()[-1].strip() if r.stdout.strip() else ""
    except Exception as e:  # noqa: BLE001
        info["ProductName"] = f"取得不可 {e!r}"
    return info


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dist", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--ime", choices=["gji", "msime"], required=True)
    ap.add_argument("--pairs", default="edit>edit,edit>chrome,chrome>edit,edit>wt,wt>edit")
    ap.add_argument("--rounds", type=int, default=2)
    ap.add_argument("--layout", default="layout/nicola_keytop.yab")
    ap.add_argument("--deadline-min", type=float, default=50)
    a = ap.parse_args()
    ctx = Ctx(a)
    pairs = [tuple(p.split(">")) for p in a.pairs.split(",") if p]
    t_end = time.time() + a.deadline_min * 60
    env = env_info()
    print("ENV", json.dumps(env, ensure_ascii=False), flush=True)
    kill_all()
    b_kinds = []
    for _, b in pairs:
        if b not in b_kinds:
            b_kinds.append(b)
    for rnd in range(a.rounds):
        for b in b_kinds:
            for cell in ("ctl_direct", "ctl_on"):
                if time.time() < t_end:
                    run_cell(ctx, "edit", b, cell, rnd)
        for a_kind, b_kind in pairs:
            for cell in ("s1", "s3"):
                if time.time() < t_end:
                    run_cell(ctx, a_kind, b_kind, cell, rnd)
    summary = summarize(ctx, pairs)
    for s in summary:
        print(f"SCOPE: ime={s['ime']} pair={s['pair']} open={s['open']} conv={s['conv']} "
              f"ctl_direct={s['ctl_direct']} ctl_on={s['ctl_on']} s1={s['s1']}(runs {s['s1_runs']}) s3={s['s3']}(runs {s['s3_runs']})")
    (ctx.out / "results.json").write_text(
        json.dumps({"env": env, "runs": ctx.results, "summary": summary}, ensure_ascii=False, indent=1), encoding="utf-8")
    md = ["# IME scope probe", "", f"ime={a.ime}", "", "```", json.dumps(env, ensure_ascii=False, indent=1), "```", "",
          "| pair | open | conv | ctl_direct | ctl_on | s1 | s3 |", "|---|---|---|---|---|---|---|"]
    for s in summary:
        md.append(f"| {s['pair']} | {s['open']} | {s['conv']} | {s['ctl_direct']} | {s['ctl_on']} | {s['s1']} | {s['s3']} |")
    (ctx.out / "summary.md").write_text("\n".join(md), encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
