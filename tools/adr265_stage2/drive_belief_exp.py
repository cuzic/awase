"""ADR-265 belief experiments E1-E4 (Windows only, standard library only). Protocol: docs/tasks/adr265-belief-experiments.md.

  e1 : a PERSON picks "direct input" / "hiragana" from the taskbar indicator menu at each beep (high beep = pick direct input,
       low double beep = pick hiragana). This script only records every change of the shared block of the Notepad text thread.
  e2 : IMM-style writes (WM_IME_CONTROL / IMC_SETOPENSTATUS to ImmGetDefaultIMEWnd, the same kind of write awase's ImmCross does).
  e3 : key -> shared-block delay (injected F2 / Kanji), optional CPU load (--load N busy processes).
  e4 : an Edge window with a text field and a password field; records the block in each field and the typed result (window title).

usage: drive_belief_exp.py e1|e2|e3|e4 --pids 123 --out x.jsonl [--rounds N] [--load N]
Needs drive_measure.py and drive_scenarios.py in the same directory. Not verified on a Windows machine yet.
"""
import argparse
import ctypes
import ctypes.wintypes as wt
import json
import subprocess
import sys
import threading
import time

import drive_measure as dm
import drive_scenarios as ds

u32 = dm.u32
imm = ctypes.windll.imm32
imm.ImmGetDefaultIMEWnd.argtypes = [wt.HWND]
imm.ImmGetDefaultIMEWnd.restype = wt.HWND
u32.SendMessageTimeoutW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM, wt.UINT, wt.UINT, ctypes.c_void_p]
u32.SendMessageTimeoutW.restype = ctypes.c_ssize_t
u32.GetWindowTextW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
WM_IME_CONTROL, IMC_SETOPENSTATUS = 0x0283, 0x0006


class Out:
    def __init__(self, path):
        self.f = open(path, "w", encoding="utf-8")
        self.lock = threading.Lock()

    def emit(self, **kw):
        kw["t_utc"] = dm.utc_iso(time.time_ns())
        kw["t_ns"] = time.perf_counter_ns()
        with self.lock:
            self.f.write(json.dumps(kw, ensure_ascii=False) + "\n")
            self.f.flush()


def sig_of(blk):
    if blk is None:
        return None
    if "eff_open" in blk:
        return (blk["eff_open"], blk["eff_mode"], blk["tsf_open"], blk["tsf_mode"], blk["seq"])
    return ("rejected", blk.get("active"), blk.get("version"))


def watcher(out, tid, pid, stop):
    """Record every change of the block of a FIXED thread (so a menu taking the focus does not change what is read)."""
    last = object()
    while not stop.is_set():
        blk = dm.read_block(tid, pid)
        s = sig_of(blk)
        if s != last:
            last = s
            out.emit(event="shm", block=blk)
        time.sleep(0.01)


def text_thread(pids):
    h, clicks = ds.focus_text(pids)
    if not h:
        return None
    _, ftid, fpid, fcls, how = dm.focus_info()
    return h, ftid, fpid, fcls


def ensure_on(pids, out):
    for _ in range(6):
        _, ftid, fpid, _, _ = dm.focus_info()
        blk = dm.read_block(ftid, fpid) if fpid in pids else None
        if blk and blk.get("eff_open") == 1:
            return True
        dm.send_vk(0xF2)
        time.sleep(1.5)
    return False


def e1(args, out):
    pids = {int(x) for x in args.pids.split(",") if x}
    tt = text_thread(pids)
    if not tt:
        out.emit(event="abort", reason="no text focus")
        return 2
    h, tid, pid, cls = tt
    out.emit(event="ready", tid=tid, pid=pid, cls=cls)
    if not ensure_on(pids, out):
        out.emit(event="abort", reason="could not turn the IME on")
        return 4
    stop = threading.Event()
    t = threading.Thread(target=watcher, args=(out, tid, pid, stop), daemon=True)
    t.start()
    import winsound
    time.sleep(2.0)
    for i in range(args.rounds * 2):
        want_open = 0 if i % 2 == 0 else 1  # i even: close from the menu; odd: open (hiragana)
        out.emit(event="cue", step=i, want_open=want_open)
        if want_open == 0:
            winsound.Beep(1400, 500)
        else:
            winsound.Beep(600, 250)
            time.sleep(0.15)
            winsound.Beep(600, 250)
        time.sleep(15.0)
    stop.set()
    out.emit(event="done", scenario="e1")
    return 0


def imm_set_open(ime_wnd, open_):
    res = ctypes.c_size_t(0)
    t0 = time.perf_counter_ns()
    ok = u32.SendMessageTimeoutW(ime_wnd, WM_IME_CONTROL, IMC_SETOPENSTATUS, 1 if open_ else 0, 2, 1000, ctypes.byref(res))
    return ok, res.value, t0, time.perf_counter_ns()


def wait_change(tid, pid, seq0, timeout_s):
    end = time.perf_counter() + timeout_s
    while time.perf_counter() < end:
        blk = dm.read_block(tid, pid)
        if blk and "seq" in blk and blk["seq"] != seq0:
            return blk, time.perf_counter_ns()
        time.sleep(0.002)
    return None, time.perf_counter_ns()


def e2(args, out):
    pids = {int(x) for x in args.pids.split(",") if x}
    tt = text_thread(pids)
    if not tt:
        out.emit(event="abort", reason="no text focus")
        return 2
    h, tid, pid, cls = tt
    if not ensure_on(pids, out):
        out.emit(event="abort", reason="could not turn the IME on")
        return 4
    ime_wnd = imm.ImmGetDefaultIMEWnd(h)
    out.emit(event="ready", tid=tid, pid=pid, cls=cls, hwnd=int(h or 0), ime_wnd=int(ime_wnd or 0))
    if not ime_wnd:
        out.emit(event="abort", reason="no default IME window")
        return 5
    time.sleep(1.0)
    for r in range(args.rounds):
        for want in (0, 1):
            before = dm.read_block(tid, pid)
            seq0 = before.get("seq") if before else None
            ok, res, t0, t1 = imm_set_open(ime_wnd, want)
            blk, t2 = wait_change(tid, pid, seq0, 2.0)
            out.emit(event="imm_write", round=r, want_open=want, send_ok=bool(ok), result=res,
                     before=before, after=blk, changed=blk is not None,
                     send_ms=(t1 - t0) / 1e6, delay_ms=(t2 - t1) / 1e6 if blk else None)
            time.sleep(1.0)
    out.emit(event="done", scenario="e2")
    return 0


def e3(args, out):
    pids = {int(x) for x in args.pids.split(",") if x}
    tt = text_thread(pids)
    if not tt:
        out.emit(event="abort", reason="no text focus")
        return 2
    h, tid, pid, cls = tt
    if not ensure_on(pids, out):
        out.emit(event="abort", reason="could not turn the IME on")
        return 4
    procs = [subprocess.Popen([sys.executable, "-c", "while True: pass"]) for _ in range(args.load)]
    out.emit(event="ready", tid=tid, pid=pid, load=args.load)
    time.sleep(2.0)
    try:
        for i in range(args.rounds):
            vk, want = (0x19, 0) if i % 2 == 0 else (0xF2, 1)  # alternate Kanji (off) and F2 (on)
            before = dm.read_block(tid, pid)
            seq0 = before.get("seq") if before else None
            t0 = time.perf_counter_ns()
            dm.send_vk(vk)
            t1 = time.perf_counter_ns()
            blk, t2 = wait_change(tid, pid, seq0, 2.0)
            out.emit(event="key", i=i, vk=f"{vk:#04x}", want_open=want, load=args.load, changed=blk is not None,
                     eff_open=blk.get("eff_open") if blk else None, from_key_down_ms=(t2 - t0) / 1e6 if blk else None,
                     from_send_done_ms=(t2 - t1) / 1e6 if blk else None)
            time.sleep(0.8)
    finally:
        for p in procs:
            p.terminate()
    out.emit(event="done", scenario="e3")
    return 0


def title_of(hwnd):
    buf = ctypes.create_unicode_buffer(512)
    u32.GetWindowTextW(hwnd, buf, 512)
    return buf.value


def e4(args, out):
    pids = {int(x) for x in args.pids.split(",") if x}
    h, clicks = ds.focus_text(pids)
    if not h:
        out.emit(event="abort", reason="no focus in the Edge window", clicks=clicks)
        return 2
    out.emit(event="ready", clicks=clicks, title=title_of(h))
    # fields: the text field has the focus first (autofocus). Tab moves to the password field.
    for field, key_to_move in (("text", None), ("password", 0x09), ("text", 0x09)):
        if key_to_move:
            dm.send_vk(key_to_move)
            time.sleep(1.0)
        _, ftid, fpid, fcls, how = dm.focus_info()
        for state_vk, label in ((0xF2, "ime-on"), (0x19, "ime-off")):
            dm.send_vk(state_vk)
            time.sleep(1.5)
            blk = dm.read_block(ftid, fpid) if fpid in pids else None
            before_title = title_of(h)
            for vk in (0x41, 0x4B):  # 'a' 'k' typed through the IME (hiragana: "あ" + pending k)
                dm.send_vk(vk)
                time.sleep(0.2)
            time.sleep(0.8)
            out.emit(event="field", field=field, ime=label, block=blk, title_before=before_title, title_after=title_of(h))
    out.emit(event="done", scenario="e4")
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("exp", choices=["e1", "e2", "e3", "e4"])
    ap.add_argument("--pids", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--rounds", type=int, default=10)
    ap.add_argument("--load", type=int, default=0)
    args = ap.parse_args()
    out = Out(args.out)
    out.emit(event="start", exp=args.exp)
    sys.exit({"e1": e1, "e2": e2, "e3": e3, "e4": e4}[args.exp](args, out))


if __name__ == "__main__":
    main()
