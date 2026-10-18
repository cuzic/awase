"""ADR-265 stage 2, physical-key measurement (Windows only, standard library only).

A PERSON presses the IME mode keys on the physical keyboard; this script only records:
  * key   : presses of the IME-mode keys listed in MODE_VKS (vk, scan code, down/up, injected?, time). A low-level keyboard
            hook sees every key but records NOTHING for any other key (not even that it was pressed) and always passes the
            key on. No characters are ever recorded.
  * shm   : every change of the shared block (open, mode, active, version) of the thread that has the keyboard focus in the
            target process, read directly (the same way the spec describes).
The comparison with awase's belief is made offline (analyze_physical.py) from awase.log's [mozc-shm] lines.

usage: drive_physical.py --pids 123 --seconds 420 --out physical.jsonl
Needs drive_measure.py and drive_scenarios.py in the same directory. Not verified on a Windows machine yet.
"""
import argparse
import ctypes
import ctypes.wintypes as wt
import json
import sys
import threading
import time

import drive_measure as dm
import drive_scenarios as ds

u32 = dm.u32
k32 = dm.k32

# The ONLY keys that are recorded: Kanji/halfwidth-fullwidth, Convert, Nonconvert, Kana, the DBE_* mode keys (Eisu,
# Katakana, Hiragana, Hankaku/Zenkaku, ...), and Caps Lock (Shift+Caps is Eisu on a JIS keyboard).
MODE_VKS = {0x15, 0x16, 0x17, 0x18, 0x19, 0x1C, 0x1D, 0x14, 0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6}
WM_KEYDOWN, WM_SYSKEYDOWN = 0x0100, 0x0104
LRESULT = ctypes.c_ssize_t
HOOKPROC = ctypes.WINFUNCTYPE(LRESULT, ctypes.c_int, wt.WPARAM, wt.LPARAM)


class KBDLLHOOKSTRUCT(ctypes.Structure):
    _fields_ = [("vkCode", wt.DWORD), ("scanCode", wt.DWORD), ("flags", wt.DWORD), ("time", wt.DWORD),
                ("dwExtraInfo", ctypes.c_size_t)]


u32.SetWindowsHookExW.argtypes = [ctypes.c_int, HOOKPROC, wt.HINSTANCE, wt.DWORD]
u32.SetWindowsHookExW.restype = wt.HHOOK
u32.CallNextHookEx.argtypes = [wt.HHOOK, ctypes.c_int, wt.WPARAM, wt.LPARAM]
u32.CallNextHookEx.restype = LRESULT
u32.UnhookWindowsHookEx.argtypes = [wt.HHOOK]
u32.GetMessageW.argtypes = [ctypes.POINTER(wt.MSG), wt.HWND, wt.UINT, wt.UINT]
u32.PostThreadMessageW.argtypes = [wt.DWORD, wt.UINT, wt.WPARAM, wt.LPARAM]
k32.GetModuleHandleW.restype = wt.HMODULE
k32.GetModuleHandleW.argtypes = [wt.LPCWSTR]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pids", required=True)
    ap.add_argument("--seconds", type=float, default=420)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    pids = {int(x) for x in args.pids.split(",") if x}
    out = open(args.out, "w", encoding="utf-8")
    lock = threading.Lock()

    def emit(**kw):
        kw["t_utc"] = dm.utc_iso(time.time_ns())
        line = json.dumps(kw, ensure_ascii=False)
        with lock:
            out.write(line + "\n")
            out.flush()

    emit(event="start", mode_vks=sorted(f"{v:#04x}" for v in MODE_VKS), seconds=args.seconds)
    h, clicks = ds.focus_text(pids)
    if not h:
        emit(event="abort", reason="no text focus in the target", clicks=clicks)
        return 2
    emit(event="ready", clicks=clicks)

    hook_tid = {}

    def hook_thread():
        def cb(n_code, w_param, l_param):
            if n_code == 0:
                k = ctypes.cast(l_param, ctypes.POINTER(KBDLLHOOKSTRUCT)).contents
                if k.vkCode in MODE_VKS:  # every other key is neither recorded nor counted
                    emit(event="key", vk=f"{k.vkCode:#04x}", scan=k.scanCode, down=w_param in (WM_KEYDOWN, WM_SYSKEYDOWN),
                         injected=bool(k.flags & 0x10))
            return u32.CallNextHookEx(None, n_code, w_param, l_param)
        proc = HOOKPROC(cb)
        hook = u32.SetWindowsHookExW(13, proc, k32.GetModuleHandleW(None), 0)  # WH_KEYBOARD_LL
        hook_tid["tid"] = k32.GetCurrentThreadId()
        hook_tid["ok"] = bool(hook)
        msg = wt.MSG()
        while hook and u32.GetMessageW(ctypes.byref(msg), None, 0, 0) > 0:
            pass
        if hook:
            u32.UnhookWindowsHookEx(hook)

    t = threading.Thread(target=hook_thread, daemon=True)
    t.start()
    for _ in range(100):
        if "ok" in hook_tid:
            break
        time.sleep(0.05)
    emit(event="hook", ok=hook_tid.get("ok"))
    if not hook_tid.get("ok"):
        return 3

    last = None
    end = time.time() + args.seconds
    while time.time() < end:
        _, ftid, fpid, fcls, how = dm.focus_info()
        blk = dm.read_block(ftid, fpid) if fpid in pids else None
        if blk is None:
            sig = (ftid, fpid, None)
        elif "eff_open" in blk:
            sig = (ftid, fpid, blk["eff_open"], blk["eff_mode"], blk["tsf_open"], blk["tsf_mode"])
        else:  # a block the reader must not trust (inactive, wrong process, ...)
            sig = (ftid, fpid, "rejected", blk.get("active"), blk.get("version"))
        if sig != last:
            last = sig
            emit(event="shm", focus_tid=ftid, focus_pid=fpid, focus_class=fcls, in_target=fpid in pids, block=blk)
        time.sleep(0.025)
    emit(event="done")
    if hook_tid.get("tid"):
        u32.PostThreadMessageW(hook_tid["tid"], 0x0012, 0, 0)  # WM_QUIT: ends the hook thread
    return 0


if __name__ == "__main__":
    sys.exit(main())
