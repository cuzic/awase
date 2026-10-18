"""ADR-265 stage 2 measurement driver (Windows only, standard library only).

Sends IME mode keys to ONE target process (selected by PID, never by window class, so a user's own
browser windows are never touched) and records, per key, the operation-derived expectation next to the
shared-memory state that this driver reads directly from the focus thread. awase's own reading is taken
later from awase.log ([mozc-shm] lines) and compared offline by timestamp.

usage: drive_measure.py --pids 123,456 --label notepad --cycles 25 --out drive_notepad.jsonl

One cycle (every key is sent exactly once, no retry; expectations come from the known previous state):
  F2 -> (1,1) hiragana,  F1 -> (1,2) full katakana,  F2 -> (1,1),  F0 -> (1,3) half alnum,
  F2 -> (1,1),  0x19 (Kanji) -> (0,*) IME off.
Warm-up (not counted, retries allowed): turn the IME on, then off, so the cycles start from a known state
and the cold first key of a fresh process (about 8 s) is out of the way.

Exit codes: 0 done, 2 target window never became foreground, 3 no shared block for the focus thread,
4 warm-up failed, 5 the target lost the foreground during the run.
Not verified on a Windows machine yet (syntax-checked on Linux only).
"""
import argparse
import ctypes
import ctypes.wintypes as wt
import json
import struct
import sys
import time

u32 = ctypes.windll.user32
k32 = ctypes.windll.kernel32
u32.GetForegroundWindow.restype = wt.HWND
u32.GetWindowThreadProcessId.argtypes = [wt.HWND, ctypes.POINTER(wt.DWORD)]
u32.GetWindowThreadProcessId.restype = wt.DWORD
u32.GetClassNameW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
u32.IsWindowVisible.argtypes = [wt.HWND]
u32.GetWindow.argtypes = [wt.HWND, wt.UINT]
u32.GetWindow.restype = wt.HWND
u32.SetForegroundWindow.argtypes = [wt.HWND]
u32.ShowWindow.argtypes = [wt.HWND, ctypes.c_int]
u32.AttachThreadInput.argtypes = [wt.DWORD, wt.DWORD, wt.BOOL]
u32.GetWindowRect.argtypes = [wt.HWND, ctypes.POINTER(wt.RECT)]
u32.GetGUIThreadInfo.argtypes = [wt.DWORD, ctypes.c_void_p]
k32.OpenFileMappingW.restype = wt.HANDLE
k32.OpenFileMappingW.argtypes = [wt.DWORD, wt.BOOL, wt.LPCWSTR]
k32.MapViewOfFile.restype = ctypes.c_void_p
k32.MapViewOfFile.argtypes = [wt.HANDLE, wt.DWORD, wt.DWORD, wt.DWORD, ctypes.c_size_t]
k32.UnmapViewOfFile.argtypes = [ctypes.c_void_p]
k32.CloseHandle.argtypes = [wt.HANDLE]
ENUMPROC = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)

MAGIC = 0x5A52444D
FMT = "<IIIIIIIIIIQ"  # TipSharedState, 48 bytes (docs: readable/docs/shared-memory.md in mozc-readable)
assert struct.calcsize(FMT) == 48
CYCLE = [(0xF2, 1, 1), (0xF1, 1, 2), (0xF2, 1, 1), (0xF0, 1, 3), (0xF2, 1, 1), (0x19, 0, None)]
SETTLE_S = 1.5  # awase reads about every 500 ms; the shared block updates within about 61 ms of a key


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", wt.WORD), ("wScan", wt.WORD), ("dwFlags", wt.DWORD), ("time", wt.DWORD),
                ("dwExtraInfo", ctypes.c_size_t)]


class INPUT(ctypes.Structure):
    class _U(ctypes.Union):
        _fields_ = [("ki", KEYBDINPUT), ("pad", ctypes.c_byte * 32)]
    _anonymous_ = ("u",)
    _fields_ = [("type", wt.DWORD), ("u", _U)]


def send_vk(vk):
    for flags in (0, 2):  # key down, key up
        i = INPUT(type=1)
        i.ki = KEYBDINPUT(vk, 0, flags, 0, 0)
        u32.SendInput(1, ctypes.byref(i), ctypes.sizeof(INPUT))
        time.sleep(0.03)


def cls_of(hwnd):
    buf = ctypes.create_unicode_buffer(256)
    u32.GetClassNameW(hwnd, buf, 256)
    return buf.value


def tid_pid(hwnd):
    pid = wt.DWORD()
    tid = u32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    return tid, pid.value


def windows_of(pids):
    """Visible, un-owned, reasonably large top-level windows owned by one of `pids`, biggest first."""
    out = []

    def cb(h, _):
        if not u32.IsWindowVisible(h) or u32.GetWindow(h, 4):  # 4 = GW_OWNER
            return True
        if tid_pid(h)[1] in pids:
            r = wt.RECT()
            u32.GetWindowRect(h, ctypes.byref(r))
            area = (r.right - r.left) * (r.bottom - r.top)
            if area > 10000:
                out.append((area, h))
        return True
    u32.EnumWindows(ENUMPROC(cb), 0)
    return [h for _, h in sorted(out, reverse=True)]


def force_foreground(hwnd):
    fg = u32.GetForegroundWindow()
    cur = k32.GetCurrentThreadId()
    fg_tid = tid_pid(fg)[0] if fg else 0
    if fg_tid and fg_tid != cur:
        u32.AttachThreadInput(cur, fg_tid, True)
    u32.ShowWindow(hwnd, 9)
    u32.SetForegroundWindow(hwnd)
    if fg_tid and fg_tid != cur:
        u32.AttachThreadInput(cur, fg_tid, False)


def click_inside(hwnd):
    """Click into the client area of OUR target window so a text input has the focus."""
    r = wt.RECT()
    u32.GetWindowRect(hwnd, ctypes.byref(r))
    u32.SetCursorPos((r.left + r.right) // 2, (r.top + r.bottom) // 2 + 40)
    u32.mouse_event(2, 0, 0, 0, 0)
    u32.mouse_event(4, 0, 0, 0, 0)


def read_block(tid, expect_pid):
    """Read the focus thread's block the way the spec says (open, map, seqlock, unmap, close).

    Returns a dict, or None when it cannot be read or must not be trusted (no mapping, bad magic, seq 0 or odd,
    version < 2, active != 1, wrong process)."""
    h = k32.OpenFileMappingW(4, False, f"Local\\mozc-readable-ime-{tid}")
    if not h:
        return None
    v = k32.MapViewOfFile(h, 4, 0, 0, 48)
    try:
        if not v:
            return None
        for _ in range(50):
            a = struct.unpack(FMT, ctypes.string_at(v, 48))
            b = struct.unpack(FMT, ctypes.string_at(v, 48))
            if a[2] % 2 == 0 and a == b:
                break
        else:
            return None
        magic, version, seq, eo, em, to, tm, btid, bpid, active, tick = a
        if magic != MAGIC or seq == 0 or version < 2 or active != 1 or bpid != expect_pid:
            return {"rejected": True, "magic": magic == MAGIC, "seq": seq, "version": version, "active": active,
                    "block_pid": bpid}
        return {"eff_open": eo, "eff_mode": em, "tsf_open": to, "tsf_mode": tm, "version": version,
                "seq": seq, "block_tid": btid, "block_pid": bpid}
    finally:
        if v:
            k32.UnmapViewOfFile(v)
        k32.CloseHandle(h)


def fg_info():
    hwnd = u32.GetForegroundWindow()
    if not hwnd:
        return 0, 0, 0, ""
    tid, pid = tid_pid(hwnd)
    return hwnd, tid, pid, cls_of(hwnd)


class GUITHREADINFO(ctypes.Structure):
    _fields_ = [("cbSize", wt.DWORD), ("flags", wt.DWORD), ("hwndActive", wt.HWND), ("hwndFocus", wt.HWND),
                ("hwndCapture", wt.HWND), ("hwndMenuOwner", wt.HWND), ("hwndMoveSize", wt.HWND),
                ("hwndCaret", wt.HWND), ("rcCaret", wt.RECT)]


def focus_info():
    """The window that has the keyboard focus (GetGUIThreadInfo of the foreground thread) and its thread.

    TSF state is per thread of the FOCUS window; for XAML apps (modern Notepad) that is not the thread of the
    top-level window. Falls back to hwndActive, then to the foreground window."""
    info = GUITHREADINFO()
    info.cbSize = ctypes.sizeof(info)
    if u32.GetGUIThreadInfo(0, ctypes.byref(info)):
        h = info.hwndFocus or info.hwndActive
        if h:
            tid, pid = tid_pid(h)
            return h, tid, pid, cls_of(h), ("focus" if info.hwndFocus else "active")
    hwnd, tid, pid, cls = fg_info()
    return hwnd, tid, pid, cls, "foreground"


def matches(blk, eo, em):
    return bool(blk) and "eff_open" in blk and (eo is None or blk["eff_open"] == eo) and (
        em is None or blk["eff_mode"] == em)


def utc_iso(ns):
    t = time.gmtime(ns // 1_000_000_000)
    return time.strftime("%Y-%m-%dT%H:%M:%S", t) + f".{ns % 1_000_000_000:09d}Z"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pids", required=True)
    ap.add_argument("--label", required=True)
    ap.add_argument("--cycles", type=int, default=25)
    ap.add_argument("--out", required=True)
    ap.add_argument("--ready", type=float, default=30)
    ap.add_argument("--click", action="store_true",
                    help="click into the middle of the window (off by default: on a large window the click "
                         "lands outside the page's text input and removes the focus from it)")
    args = ap.parse_args()
    pids = {int(x) for x in args.pids.split(",") if x}
    out = open(args.out, "w", encoding="utf-8")

    def emit(**kw):
        kw["t_utc"] = utc_iso(time.time_ns())
        out.write(json.dumps(kw, ensure_ascii=False) + "\n")
        out.flush()
        print(json.dumps(kw, ensure_ascii=False), flush=True)

    # 1. bring the target (and only the target) to the foreground
    target = None
    end = time.time() + args.ready
    while time.time() < end:
        ws = windows_of(pids)
        if ws:
            target = ws[0]
            force_foreground(target)
            time.sleep(0.3)
            if fg_info()[2] in pids:
                break
        time.sleep(0.5)
    hwnd, tid, pid, cls = fg_info()
    emit(event="start", label=args.label, pids=sorted(pids), fg_hwnd=hwnd, fg_tid=tid, fg_pid=pid, fg_class=cls,
         target_found=bool(target))
    if not target or pid not in pids:
        emit(event="abort", reason="target never became foreground")
        sys.exit(2)

    # 2. the shared block of the focus thread must exist (the TIP is loaded there)
    blk = None
    end = time.time() + args.ready
    while time.time() < end:
        if args.click:
            click_inside(target)
        fh, ftid, fpid, fcls, how = focus_info()
        if fpid in pids:
            blk = read_block(ftid, fpid)
            if blk is not None:
                break
        time.sleep(1)
    emit(event="block", focus_tid=ftid, focus_pid=fpid, focus_class=fcls, focus_from=how, block=blk)
    if blk is None:
        sys.exit(3)

    # 3. warm-up with retries (not counted): on, then off
    def drive_to(vk, eo, em, what):
        for attempt in range(1, 5):
            send_vk(vk)
            deadline = time.time() + 10
            while time.time() < deadline:
                fh, ftid, fpid, fcls, how = focus_info()
                if fpid not in pids:
                    emit(event="warmup", step=what, attempt=attempt, ok=False, reason="focus left the target",
                         focus_pid=fpid, focus_class=fcls, focus_from=how)
                    return False
                blk = read_block(ftid, fpid)
                if matches(blk, eo, em):
                    emit(event="warmup", step=what, attempt=attempt, ok=True)
                    return True
                time.sleep(0.05)
            emit(event="warmup", step=what, attempt=attempt, ok=False, reason="state did not reach the expectation",
                 last_block=blk, focus_tid=ftid, focus_class=fcls, focus_from=how)
        return False
    if not (drive_to(0xF2, 1, 1, "on") and drive_to(0x19, 0, None, "off")):
        sys.exit(4)
    time.sleep(2)

    # 4. counted cycles: every key exactly once
    n = 0
    for cycle in range(args.cycles):
        for step, (vk, eo, em) in enumerate(CYCLE):
            hwnd, tid, pid, cls = fg_info()
            if pid not in pids:
                emit(event="abort", reason="target lost the foreground", cycle=cycle, step=step, fg_pid=pid,
                     fg_class=cls)
                sys.exit(5)
            t_send = time.time_ns()
            send_vk(vk)
            time.sleep(SETTLE_S)
            hwnd2, tid2, pid2, cls2 = fg_info()
            fh, ftid, fpid, fcls, how = focus_info()
            blk = read_block(ftid, fpid) if fpid in pids else None
            n += 1
            emit(event="step", label=args.label, seq_no=n, cycle=cycle, step=step, vk=f"{vk:#04x}",
                 t_send_utc=utc_iso(t_send), expected_open=eo, expected_mode=em, fg_tid=tid2, fg_pid=pid2,
                 fg_class=cls2, focus_tid=ftid, focus_pid=fpid, focus_class=fcls, focus_from=how, driver_block=blk, driver_matches_expected=matches(blk, eo, em))
    emit(event="done", label=args.label, steps=n)
    sys.exit(0)


if __name__ == "__main__":
    main()
