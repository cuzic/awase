#!/usr/bin/env python3
"""環境イベント(最小化/復元・Win32 ダイアログの Tab 移動・仮想デスクトップ切替・画面ロック)を CI の実機で起こす部品。

再現用の語彙の不足を埋めるもの(2026-10-05 の棚卸し「環境イベント」)。BUG-063(仮想デスクトップ切替後)・BUG-091(ダイアログの Tab 直後)・
BUG-023(画面ロック中の修飾キー)のような「キー入力以外の環境の変化」を、awase を起動したまま起こして awase.log の反応を見るために使う。
Python(ctypes)だけで書いてあり、Rust のビルドは要らない。

使い方: python probe.py --events=tab,minrestore,vdesk,lock [--log=probe.log] [--marker=1]
  tab        2 つの EDIT(WS_TABSTOP)を持つ最上位窓(IsDialogMessage で Tab 移動する Win32 ダイアログ相当)で、Tab / Shift+Tab を注入しフォーカスが動くか
  minrestore 窓を最小化→復元して、前面・フォーカスが戻るか
  vdesk      IVirtualDesktopManager で自窓が現在の仮想デスクトップにあるかを読みながら、Win+Ctrl+D(新規作成して切替)→ Win+Ctrl+F4(閉じて戻る)
  lock       rundll32 user32.dll,LockWorkStation で画面ロック。入力デスクトップが Winlogon に移るか(解除はできない。最後に回す)
ログは `[EV-JSON] {...}` の 1 行 1 JSON(ts は awase.log と同じ UTC)。完走マーカーは `=== 完了 ===`。--marker=1 は AWASE_TEST_INJECTION 用の目印を付ける。
"""
import ctypes
import ctypes.wintypes as wt
import json
import subprocess
import sys
import time

MARKER = 0x5350494B
user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
ole32 = ctypes.WinDLL("ole32")

LRESULT = ctypes.c_ssize_t
WNDPROC = ctypes.WINFUNCTYPE(LRESULT, wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM)


class WNDCLASSEXW(ctypes.Structure):
    _fields_ = [("cbSize", wt.UINT), ("style", wt.UINT), ("lpfnWndProc", WNDPROC), ("cbClsExtra", ctypes.c_int),
                ("cbWndExtra", ctypes.c_int), ("hInstance", wt.HINSTANCE), ("hIcon", wt.HICON), ("hCursor", wt.HANDLE),
                ("hbrBackground", wt.HBRUSH), ("lpszMenuName", wt.LPCWSTR), ("lpszClassName", wt.LPCWSTR), ("hIconSm", wt.HICON)]


class GUITHREADINFO(ctypes.Structure):
    _fields_ = [("cbSize", wt.DWORD), ("flags", wt.DWORD), ("hwndActive", wt.HWND), ("hwndFocus", wt.HWND), ("hwndCapture", wt.HWND),
                ("hwndMenuOwner", wt.HWND), ("hwndMoveSize", wt.HWND), ("hwndCaret", wt.HWND), ("rcCaret", wt.RECT)]


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", wt.WORD), ("wScan", wt.WORD), ("dwFlags", wt.DWORD), ("time", wt.DWORD), ("dwExtraInfo", ctypes.c_size_t)]


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [("dx", wt.LONG), ("dy", wt.LONG), ("mouseData", wt.DWORD), ("dwFlags", wt.DWORD), ("time", wt.DWORD), ("dwExtraInfo", ctypes.c_size_t)]


class _U(ctypes.Union):
    _fields_ = [("ki", KEYBDINPUT), ("mi", MOUSEINPUT)]


class INPUT(ctypes.Structure):
    _anonymous_ = ("u",)
    _fields_ = [("type", wt.DWORD), ("u", _U)]


class GUID(ctypes.Structure):
    _fields_ = [("Data1", wt.DWORD), ("Data2", wt.WORD), ("Data3", wt.WORD), ("Data4", ctypes.c_ubyte * 8)]

    def __str__(self):
        return "%08X-%04X-%04X-%s" % (self.Data1, self.Data2, self.Data3, bytes(self.Data4).hex().upper())


def _guid(s):
    import uuid
    u = uuid.UUID(s)
    g = GUID()
    g.Data1, g.Data2, g.Data3 = u.time_low, u.time_mid, u.time_hi_version
    for i, b in enumerate(u.bytes[8:]):
        g.Data4[i] = b
    return g


user32.CreateWindowExW.restype = wt.HWND
user32.CreateWindowExW.argtypes = [wt.DWORD, wt.LPCWSTR, wt.LPCWSTR, wt.DWORD, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
                                   wt.HWND, wt.HMENU, wt.HINSTANCE, wt.LPVOID]
user32.DefWindowProcW.restype = LRESULT
user32.DefWindowProcW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]
user32.RegisterClassExW.argtypes = [ctypes.POINTER(WNDCLASSEXW)]
user32.PeekMessageW.argtypes = [ctypes.POINTER(wt.MSG), wt.HWND, wt.UINT, wt.UINT, wt.UINT]
user32.IsDialogMessageW.argtypes = [wt.HWND, ctypes.POINTER(wt.MSG)]
user32.TranslateMessage.argtypes = [ctypes.POINTER(wt.MSG)]
user32.DispatchMessageW.argtypes = [ctypes.POINTER(wt.MSG)]
user32.GetForegroundWindow.restype = wt.HWND
user32.SetForegroundWindow.argtypes = [wt.HWND]
user32.BringWindowToTop.argtypes = [wt.HWND]
user32.SetFocus.argtypes = [wt.HWND]
user32.ShowWindow.argtypes = [wt.HWND, ctypes.c_int]
user32.IsIconic.argtypes = [wt.HWND]
user32.GetGUIThreadInfo.argtypes = [wt.DWORD, ctypes.POINTER(GUITHREADINFO)]
user32.GetWindowThreadProcessId.argtypes = [wt.HWND, ctypes.POINTER(wt.DWORD)]
user32.AttachThreadInput.argtypes = [wt.DWORD, wt.DWORD, wt.BOOL]
user32.SendInput.argtypes = [wt.UINT, ctypes.POINTER(INPUT), ctypes.c_int]
user32.GetClassNameW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.OpenInputDesktop.restype = wt.HANDLE
user32.OpenInputDesktop.argtypes = [wt.DWORD, wt.BOOL, wt.DWORD]
user32.CloseDesktop.argtypes = [wt.HANDLE]
user32.GetUserObjectInformationW.argtypes = [wt.HANDLE, ctypes.c_int, wt.LPVOID, wt.DWORD, ctypes.POINTER(wt.DWORD)]
kernel32.GetModuleHandleW.restype = wt.HMODULE
kernel32.GetCurrentThreadId.restype = wt.DWORD

VK = dict(TAB=0x09, SHIFT=0x10, CTRL=0x11, LWIN=0x5B, D=0x44, LEFT=0x25, F4=0x73)
WS_OVERLAPPEDWINDOW, WS_VISIBLE, WS_CHILD, WS_BORDER, WS_TABSTOP = 0x00CF0000, 0x10000000, 0x40000000, 0x00800000, 0x00010000
WS_EX_CONTROLPARENT = 0x00010000
SW_MINIMIZE, SW_RESTORE = 6, 9
LOG_PATH = "env_events_probe.log"
EXTRA = 0
TOP = None
EDITS = []
_wndproc_ref = None
_vdm = None


def ts():
    t = time.time()
    return time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(t)) + ".%03dZ" % int((t % 1) * 1000)


def emit(**kw):
    line = "[EV-JSON] " + json.dumps({"ts": ts(), **kw}, ensure_ascii=False)
    print(line, flush=True)
    with open(LOG_PATH, "a", encoding="utf-8") as f:
        f.write(line + "\n")


def pump(ms):
    """メッセージを読みながら待つ。Tab は IsDialogMessage に渡す(ダイアログ相当の移動)。"""
    end = time.time() + ms / 1000
    msg = wt.MSG()
    while time.time() < end:
        while user32.PeekMessageW(ctypes.byref(msg), None, 0, 0, 1):
            if not (TOP and user32.IsDialogMessageW(TOP, ctypes.byref(msg))):
                user32.TranslateMessage(ctypes.byref(msg))
                user32.DispatchMessageW(ctypes.byref(msg))
        time.sleep(0.01)


def key(vk, down):
    i = INPUT(type=1)
    i.ki = KEYBDINPUT(vk, 0, 0 if down else 2, 0, EXTRA)
    return user32.SendInput(1, ctypes.byref(i), ctypes.sizeof(INPUT)) == 1


def chord(*vks):
    """vks の順に押し、逆順に離す。全部の注入が通れば True。"""
    ok = True
    for v in vks:
        ok &= key(v, True)
        time.sleep(0.04)
    time.sleep(0.08)
    for v in reversed(vks):
        ok &= key(v, False)
        time.sleep(0.04)
    return ok


def cls_of(h):
    if not h:
        return None
    b = ctypes.create_unicode_buffer(128)
    user32.GetClassNameW(h, b, 128)
    return b.value


def vdm():
    """IVirtualDesktopManager(公開 API)。作れなければ None。vtable: 3=IsWindowOnCurrentVirtualDesktop 4=GetWindowDesktopId。"""
    global _vdm
    if _vdm is not None:
        return _vdm or None
    ole32.CoInitialize(None)
    clsid = _guid("AA509086-5CA9-4C25-8F95-589D3C07B48A")
    iid = _guid("A5CD92FF-29BE-454C-8D04-D82879FB3F1B")
    p = ctypes.c_void_p()
    hr = ole32.CoCreateInstance(ctypes.byref(clsid), None, 23, ctypes.byref(iid), ctypes.byref(p))  # CLSCTX_ALL
    if hr != 0 or not p.value:
        emit(type="vdm_create", ok=False, hr=hr & 0xFFFFFFFF)
        _vdm = False
        return None
    vt = ctypes.cast(ctypes.cast(p, ctypes.POINTER(ctypes.c_void_p))[0], ctypes.POINTER(ctypes.c_void_p))
    _vdm = (p, vt)
    return _vdm


def on_current_vd(h):
    m = vdm()
    if not m:
        return None
    p, vt = m
    f = ctypes.WINFUNCTYPE(ctypes.HRESULT, ctypes.c_void_p, wt.HWND, ctypes.POINTER(wt.BOOL))(vt[3])
    r = wt.BOOL()
    try:
        f(p, h, ctypes.byref(r))
    except OSError as e:
        return f"err:{e}"
    return bool(r.value)


def desktop_id(h):
    m = vdm()
    if not m:
        return None
    p, vt = m
    f = ctypes.WINFUNCTYPE(ctypes.HRESULT, ctypes.c_void_p, wt.HWND, ctypes.POINTER(GUID))(vt[4])
    g = GUID()
    try:
        f(p, h, ctypes.byref(g))
    except OSError as e:
        return f"err:{e}"
    return str(g)


def input_desktop():
    """入力デスクトップの名前。開けなければ (None, エラー番号)(別デスクトップ=ロック中など)。"""
    h = user32.OpenInputDesktop(0, False, 0x0100)
    if not h:
        return None, ctypes.get_last_error()
    buf = ctypes.create_unicode_buffer(256)
    need = wt.DWORD()
    user32.GetUserObjectInformationW(h, 2, buf, 512, ctypes.byref(need))
    user32.CloseDesktop(h)
    return buf.value, 0


def sample():
    fg = user32.GetForegroundWindow()
    gi = GUITHREADINFO(cbSize=ctypes.sizeof(GUITHREADINFO))
    user32.GetGUIThreadInfo(0, ctypes.byref(gi))
    focus = gi.hwndFocus
    idx = next((i for i, e in enumerate(EDITS) if e == focus), -1) if focus else None
    return dict(fg_is_top=bool(fg) and fg == TOP, fg_class=cls_of(fg), focus_idx=idx, iconic=bool(user32.IsIconic(TOP)),
                on_current_vd=on_current_vd(TOP), input_desktop=input_desktop()[0])


def front():
    fg = user32.GetForegroundWindow()
    ftid = user32.GetWindowThreadProcessId(fg, None) if fg else 0
    me = kernel32.GetCurrentThreadId()
    att = ftid and ftid != me and user32.AttachThreadInput(me, ftid, True)
    user32.BringWindowToTop(TOP)
    user32.SetForegroundWindow(TOP)
    user32.SetFocus(EDITS[0])
    if att:
        user32.AttachThreadInput(me, ftid, False)
    pump(300)


def make_form():
    global TOP, _wndproc_ref
    _wndproc_ref = WNDPROC(lambda h, m, w, l: user32.DefWindowProcW(h, m, w, l))
    inst = kernel32.GetModuleHandleW(None)
    wc = WNDCLASSEXW(cbSize=ctypes.sizeof(WNDCLASSEXW), lpfnWndProc=_wndproc_ref, hInstance=inst, lpszClassName="EnvEventsTop")
    user32.RegisterClassExW(ctypes.byref(wc))
    TOP = user32.CreateWindowExW(WS_EX_CONTROLPARENT, "EnvEventsTop", "env events", WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                                 100, 100, 520, 220, None, None, inst, None)
    for i in range(2):
        EDITS.append(user32.CreateWindowExW(0, "EDIT", "", WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP,
                                            10, 10 + i * 50, 440, 30, TOP, None, inst, None))
    pump(300)
    front()


# ---------------------------------------------------------------- イベント

def ev_tab():
    front()
    before = sample()
    ok1 = chord(VK["TAB"])
    pump(400)
    after = sample()
    ok2 = chord(VK["SHIFT"], VK["TAB"])
    pump(400)
    back = sample()
    emit(type="tab", injected=ok1 and ok2, before=before, after_tab=after, after_shift_tab=back,
         happened=(before["focus_idx"] == 0 and after["focus_idx"] == 1 and back["focus_idx"] == 0))


def ev_minrestore():
    front()
    before = sample()
    user32.ShowWindow(TOP, SW_MINIMIZE)
    pump(800)
    mini = sample()
    user32.ShowWindow(TOP, SW_RESTORE)
    pump(800)
    restored = sample()
    front()
    refocused = sample()
    emit(type="minrestore", before=before, minimized=mini, restored=restored, refocused=refocused,
         happened=bool(mini["iconic"]) and not refocused["iconic"])


def ev_vdesk():
    front()
    m = vdm()
    before = sample()
    if not m:
        emit(type="vdesk", happened=False, reason="IVirtualDesktopManager を作れない", before=before)
        return
    id0 = desktop_id(TOP)
    ok = chord(VK["LWIN"], VK["CTRL"], VK["D"])  # 新規デスクトップを作って切替
    pump(2000)
    created = sample()
    ok2 = chord(VK["LWIN"], VK["CTRL"], VK["F4"])  # 今のデスクトップを閉じて戻る
    pump(2000)
    closed = sample()
    front()
    final = sample()
    emit(type="vdesk", desktop_id=id0, injected=ok and ok2, before=before, after_create=created, after_close=closed, final=final,
         happened=(before["on_current_vd"] is True and created["on_current_vd"] is False))


def ev_lock():
    before = sample()
    emit(type="lock_start", before=before)
    subprocess.run(["rundll32.exe", "user32.dll,LockWorkStation"], check=False)
    seen = []
    for _ in range(10):
        pump(1000)
        name, err = input_desktop()
        seen.append(name if name is not None else f"open_failed:{err}")
    inj = key(0x10, True)  # 修飾キーを押したままにはしない: 直後に離す
    key(0x10, False)
    after = sample()
    emit(type="lock", input_desktop_over_time=seen, send_input_ok_while_locked=inj, after=after,
         happened=any(s != "Default" for s in seen))


EVENTS = dict(tab=ev_tab, minrestore=ev_minrestore, vdesk=ev_vdesk, lock=ev_lock)


def main():
    global LOG_PATH, EXTRA
    args = dict(a.lstrip("-").split("=", 1) for a in sys.argv[1:] if "=" in a)
    LOG_PATH = args.get("log", LOG_PATH)
    EXTRA = MARKER if args.get("marker") == "1" else 0
    evs = [e for e in args.get("events", "tab,minrestore,vdesk").split(",") if e]
    bad = [e for e in evs if e not in EVENTS]
    if bad:
        print(__doc__)
        return 2
    make_form()
    for e in evs:
        emit(type="event_begin", name=e)
        try:
            EVENTS[e]()
        except Exception as ex:  # 部品の不具合を結果に残す(落ちて何も残らないのを避ける)
            emit(type="event_error", name=e, error=repr(ex))
        emit(type="event_end", name=e)
        pump(500)
    with open(LOG_PATH, "a", encoding="utf-8") as f:
        f.write("=== 完了 ===\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
