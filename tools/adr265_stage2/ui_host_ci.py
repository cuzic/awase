"""CI helper: a plain Win32 window with an EDIT child that sends IME toggle keys.

The TIP is loaded into *this* process, so the shared memory block
Local\\mozc-readable-ime-<tid> is created by this process' UI thread. The thread id
is written to the file given as argv[1]; reader.py (a separate process) reads it.
"""
import ctypes
import ctypes.wintypes as wt
import faulthandler
import os
import struct
import sys
import time

u32 = ctypes.windll.user32
k32 = ctypes.windll.kernel32
u32.CreateWindowExW.argtypes = [wt.DWORD, wt.LPCWSTR, wt.LPCWSTR, wt.DWORD, ctypes.c_int, ctypes.c_int,
                                ctypes.c_int, ctypes.c_int, wt.HWND, wt.HMENU, wt.HINSTANCE, wt.LPVOID]
u32.CreateWindowExW.restype = wt.HWND
u32.SetForegroundWindow.argtypes = [wt.HWND]
u32.SetFocus.argtypes = [wt.HWND]
u32.SetFocus.restype = wt.HWND
u32.ShowWindow.argtypes = [wt.HWND, ctypes.c_int]
u32.GetForegroundWindow.restype = wt.HWND
u32.PeekMessageW.argtypes = [ctypes.POINTER(wt.MSG), wt.HWND, wt.UINT, wt.UINT, wt.UINT]
u32.TranslateMessage.argtypes = [ctypes.POINTER(wt.MSG)]
u32.DispatchMessageW.argtypes = [ctypes.POINTER(wt.MSG)]
k32.GetModuleHandleW.restype = wt.HMODULE
k32.GetModuleHandleW.argtypes = [wt.LPCWSTR]
u32.DefWindowProcW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]
u32.DefWindowProcW.restype = ctypes.c_ssize_t
WNDPROC = ctypes.WINFUNCTYPE(ctypes.c_ssize_t, wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM)


class WNDCLASSW(ctypes.Structure):
    _fields_ = [("style", wt.UINT), ("lpfnWndProc", WNDPROC), ("cbClsExtra", ctypes.c_int),
                ("cbWndExtra", ctypes.c_int), ("hInstance", wt.HINSTANCE), ("hIcon", wt.HANDLE),
                ("hCursor", wt.HANDLE), ("hbrBackground", wt.HANDLE), ("lpszMenuName", wt.LPCWSTR),
                ("lpszClassName", wt.LPCWSTR)]


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", wt.WORD), ("wScan", wt.WORD), ("dwFlags", wt.DWORD), ("time", wt.DWORD),
                ("dwExtraInfo", ctypes.c_size_t)]


class INPUT(ctypes.Structure):
    class _U(ctypes.Union):
        _fields_ = [("ki", KEYBDINPUT), ("pad", ctypes.c_byte * 32)]
    _anonymous_ = ("u",)
    _fields_ = [("type", wt.DWORD), ("u", _U)]


def send_vk(vk):
    for flags in (0, 2):
        i = INPUT(type=1)
        i.ki = KEYBDINPUT(vk, 0, flags, 0, 0)
        u32.SendInput(1, ctypes.byref(i), ctypes.sizeof(INPUT))
        time.sleep(0.03)


k32.OpenFileMappingW.restype = wt.HANDLE
k32.OpenFileMappingW.argtypes = [wt.DWORD, wt.BOOL, wt.LPCWSTR]
k32.MapViewOfFile.restype = ctypes.c_void_p
k32.MapViewOfFile.argtypes = [wt.HANDLE, wt.DWORD, wt.DWORD, wt.DWORD, ctypes.c_size_t]
_view = None


def read_active(tid):
    """`active` field (version >= 2) of this process' own published block, or None."""
    if read_state(tid) is None:
        return None
    return struct.unpack("<IIIIIIIIIIQ", ctypes.string_at(_view, 48))[9]


def read_state(tid):
    """(effective_open, effective_mode) from this process' own published block, or None."""
    global _view
    if _view is None:
        h = k32.OpenFileMappingW(4, False, f"Local\\mozc-readable-ime-{tid}")
        if h:
            _view = k32.MapViewOfFile(h, 4, 0, 0, 48)
    if not _view:
        return None
    for _ in range(50):
        a = struct.unpack("<IIIIIIIIIIQ", ctypes.string_at(_view, 48))
        if a[2] % 2 == 0 and a == struct.unpack("<IIIIIIIIIIQ", ctypes.string_at(_view, 48)):
            return (a[3], a[4])
    return None


def pump(seconds):
    end = time.time() + seconds
    msg = wt.MSG()
    while time.time() < end:
        while u32.PeekMessageW(ctypes.byref(msg), None, 0, 0, 1):
            u32.TranslateMessage(ctypes.byref(msg))
            u32.DispatchMessageW(ctypes.byref(msg))
        time.sleep(0.01)


def main():
    faulthandler.enable()  # native crashes (access violation) leave a Python traceback on stderr
    t_start = time.time()
    tid_file = sys.argv[1]
    # steps: "VK=open:mode" (hex vk; "*" = any) e.g. "F2=1:1,F1=1:2". The key is re-sent (up to 3
    # times, 8 s each) until the published state matches, so lost keys do not shift the sequence.
    # "VK=same" is a negative check: send the key once and require the published state to stay as it was
    # for 4 s (the key has no binding in the current keymap).
    steps = []
    for item in [x for x in sys.argv[2].split(",") if x]:
        vk_s, _, exp = item.partition("=")
        if exp == "same":
            steps.append((int(vk_s, 16), "same", None))
            continue
        eo, _, em = exp.partition(":")
        steps.append((int(vk_s, 16), None if eo in ("", "*") else int(eo), None if em in ("", "*") else int(em)))
    wndproc = WNDPROC(lambda h, m, w, l: u32.DefWindowProcW(h, m, w, l))
    wc = WNDCLASSW()
    wc.lpfnWndProc = wndproc
    wc.hInstance = k32.GetModuleHandleW(None)
    wc.lpszClassName = "ReadableCiHost"
    u32.RegisterClassW(ctypes.byref(wc))
    hwnd = u32.CreateWindowExW(0, "ReadableCiHost", "readable-ci", 0x10CF0000, 100, 100, 500, 200,
                               None, None, wc.hInstance, None)
    edit = u32.CreateWindowExW(0x200, "EDIT", "", 0x50010000 | 0x0080, 10, 10, 460, 120, hwnd,
                               None, wc.hInstance, None)
    u32.ShowWindow(hwnd, 5)
    u32.SetForegroundWindow(hwnd)
    u32.SetFocus(edit)
    tid = k32.GetCurrentThreadId()
    with open(tid_file, "w") as f:
        f.write(str(tid))
    print(f"ui_host: tid={tid} hwnd={(hwnd or 0):#x} fg={u32.GetForegroundWindow() == hwnd}", flush=True)
    pump(6)  # let the TIP load and publish its initial state
    def matches(st, eo, em):
        return st is not None and (eo is None or st[0] == eo) and (em is None or st[1] == em)

    for n, (vk, eo, em) in enumerate(steps):
        if eo == "same":
            before = read_state(tid)
            u32.SetForegroundWindow(hwnd)
            u32.SetFocus(edit)
            print(f"ui_host: send vk={vk:#x} t_ns={time.time_ns()} try=1 (expect no change)", flush=True)
            send_vk(vk)
            pump(4)
            after = read_state(tid)
            print(f"ui_host: vk={vk:#x} expected=same before={before} after={after} ok={before == after}", flush=True)
            continue
        for attempt in range(1, 4):
            u32.SetForegroundWindow(hwnd)
            u32.SetFocus(edit)
            tag = "send" if attempt == 1 else "again"
            print(f"ui_host: {tag} vk={vk:#x} t_ns={time.time_ns()} try={attempt}", flush=True)
            send_vk(vk)
            deadline = time.time() + 8
            while time.time() < deadline and not matches(read_state(tid), eo, em):
                pump(0.05)
            ok = matches(read_state(tid), eo, em)
            print(f"ui_host: vk={vk:#x} try={attempt} -> state={read_state(tid)} expected=({eo},{em}) ok={ok}",
                  flush=True)
            if ok:
                break
        pump(1)
    if steps and os.environ.get("MOZC_CI_DEACTIVATE") == "1":
        # Deactivation (run LAST, in its own process: switching the layout is global when "a different
        # input method per app window" is off, so every process started afterwards would have no TIP): switching this thread to a layout without a TIP must reset `active`.
        print(f"ui_host: before deactivate: active={read_active(tid)}", flush=True)
        u32.LoadKeyboardLayoutW.restype = ctypes.c_void_p
        u32.ActivateKeyboardLayout.argtypes = [ctypes.c_void_p, wt.UINT]
        hkl = u32.LoadKeyboardLayoutW("00000409", 1)
        u32.ActivateKeyboardLayout(hkl, 0)
        end = time.time() + 8
        while time.time() < end and read_active(tid) != 0:
            pump(0.1)
        print(f"ui_host: deactivate: active={read_active(tid)}", flush=True)
    if not steps:
        idle = float(sys.argv[3]) if len(sys.argv) > 3 else 60  # idle: let an outside driver work on us
        end = time.time() + idle
        while time.time() < end:
            print(f"ui_host: alive t={time.time() - t_start:.0f}s state={read_state(tid)}", flush=True)
            pump(5)
        print("ui_host: idle time over, exiting", flush=True)
    pump(2)


main()
