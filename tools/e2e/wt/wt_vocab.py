"""Windows Terminal を CI の実機で制御するための語彙(Python + ctypes だけ。Windows 専用)。

再現用の語彙の不足を埋めるもの(2026-10-05 の棚卸し「アプリの種類」)。BUG-113/121/142(Windows Terminal + GJI、物理半角/全角キーで
「@」が出る・IME ON/Engine ON に固着)を、実機で起こすための部品:

  windows()            最上位の Windows Terminal 窓(class CASCADIA_HOSTING_WINDOW_CLASS)の一覧
  launch(args)         wt.exe を起動して新しい窓の hwnd を返す(`wt -w new …`)
  foreground(hwnd)     前面化(AttachThreadInput の定番の回避策)
  close(hwnd)          WM_CLOSE
  press(vk)/chord(…)   SendInput(scan code 付き。marker=True で AWASE_TEST_INJECTION の debug awase が物理キー扱い)
  type_unicode(s)      KEYEVENTF_UNICODE(IME を通さず文字を直接送る。可否の対照用)
  ime_control(hwnd, …) 既定 IME 窓へ WM_IME_CONTROL(awase を通さず実 IME の開閉を読む/外から変える)
  shortcut_*           新規タブ・分割・ペイン/タブを閉じる(wt の既定のキー操作)
  uia_text(hwnd)       UIA の TextPattern で画面のテキストを読む(wt_uia.ps1)
  screenshot(path)     画面全体の PNG(実機で何が見えていたかの証拠)
入力の受け口は wt_echo.ps1(Windows Terminal の中で動く記録器)。受け取ったキーの文字コードを書き出す。
"""
import ctypes
import ctypes.wintypes as wt
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

MARKER = 0x5350494B  # hook::TEST_INJECTION_MARKER(`AWASE_TEST_INJECTION=1` の debug ビルド awase が物理キー扱い)
WT_CLASS = "CASCADIA_HOSTING_WINDOW_CLASS"
HERE = Path(__file__).resolve().parent

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, KEYEVENTF_SCANCODE = 0x1, 0x2, 0x4, 0x8
WM_CLOSE = 0x0010
SW_RESTORE = 9

VK = dict(SHIFT=0x10, CTRL=0x11, ALT=0x12, RETURN=0x0D, ESC=0x1B, SPACE=0x20, TAB=0x09, T=0x54, W=0x57, D=0x44, A=0x41,
          OEM_PLUS=0xBB, HIRAGANA=0xF2, SBCSCHAR=0xF3, DBCSCHAR=0xF4, ALNUM=0xF0, NONCONVERT=0x1D, CONVERT=0x1C, KANJI=0x19,
          IME_ON=0x16, IME_OFF=0x1A)


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", wt.WORD), ("wScan", wt.WORD), ("dwFlags", wt.DWORD), ("time", wt.DWORD), ("dwExtraInfo", ctypes.c_size_t)]


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [("dx", wt.LONG), ("dy", wt.LONG), ("mouseData", wt.DWORD), ("dwFlags", wt.DWORD), ("time", wt.DWORD), ("dwExtraInfo", ctypes.c_size_t)]


class _U(ctypes.Union):
    _fields_ = [("ki", KEYBDINPUT), ("mi", MOUSEINPUT)]


class INPUT(ctypes.Structure):
    _anonymous_ = ("u",)
    _fields_ = [("type", wt.DWORD), ("u", _U)]


user32.SendInput.argtypes = [wt.UINT, ctypes.POINTER(INPUT), ctypes.c_int]
user32.MapVirtualKeyW.restype = wt.UINT
user32.GetForegroundWindow.restype = wt.HWND
user32.GetClassNameW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.GetWindowTextW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.GetWindowThreadProcessId.argtypes = [wt.HWND, ctypes.POINTER(wt.DWORD)]
user32.PostMessageW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]
EnumProc = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)

# 拡張キー扱いにする VK(物理半角/全角などは拡張ではない)。矢印・Ins/Del 等は使わないので空。
_EXTENDED = set()


# ---------------------------------------------------------------- 窓

def _cls(h):
    b = ctypes.create_unicode_buffer(256)
    user32.GetClassNameW(h, b, 256)
    return b.value


def _title(h):
    b = ctypes.create_unicode_buffer(512)
    user32.GetWindowTextW(h, b, 512)
    return b.value


def windows():
    """最上位の Windows Terminal 窓: [{hwnd, pid, title, visible, iconic}]。"""
    out = []

    def cb(h, _):
        if _cls(h) == WT_CLASS:
            pid = wt.DWORD()
            user32.GetWindowThreadProcessId(h, ctypes.byref(pid))
            out.append(dict(hwnd=int(h), pid=int(pid.value), title=_title(h), visible=bool(user32.IsWindowVisible(h)),
                            iconic=bool(user32.IsIconic(h))))
        return True

    user32.EnumWindows(EnumProc(cb), 0)
    return out


def ime_control(hwnd, cmd, value=0):
    """対象窓の既定 IME 窓へ WM_IME_CONTROL(cmd, value) を短いタイムアウト付きで送り、戻り値を返す。送れなければ None。
    cmd: 0x0005=IMC_GETOPENSTATUS、0x0006=IMC_SETOPENSTATUS(Windows Terminal で外から閉じる操作が効くことは ADR-227 D0-5 で実測済み)。"""
    imm32 = ctypes.WinDLL("imm32", use_last_error=True)
    imm32.ImmGetDefaultIMEWnd.restype = wt.HWND
    imm32.ImmGetDefaultIMEWnd.argtypes = [wt.HWND]
    ime_wnd = imm32.ImmGetDefaultIMEWnd(wt.HWND(hwnd))
    if not ime_wnd:
        return None
    send = user32.SendMessageTimeoutW
    send.restype = ctypes.c_ssize_t
    send.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM, wt.UINT, wt.UINT, ctypes.POINTER(ctypes.c_size_t)]
    result = ctypes.c_size_t(0)
    ok = send(ime_wnd, 0x0283, cmd, value, 0x0002, 500, ctypes.byref(result))  # WM_IME_CONTROL, SMTO_ABORTIFHUNG
    return int(result.value) if ok else None


def foreground_hwnd():
    return int(user32.GetForegroundWindow() or 0)


def foreground(hwnd, tries=5):
    """前面化して、実際に前面になったかを返す。AttachThreadInput で前景ロックを回避する。"""
    h = wt.HWND(hwnd)
    for _ in range(tries):
        if user32.IsIconic(h):
            user32.ShowWindow(h, SW_RESTORE)
        fg = user32.GetForegroundWindow()
        ft = user32.GetWindowThreadProcessId(fg, None) if fg else 0
        me = kernel32.GetCurrentThreadId()
        att = bool(ft and ft != me and user32.AttachThreadInput(me, ft, True))
        user32.BringWindowToTop(h)
        user32.SetForegroundWindow(h)
        if att:
            user32.AttachThreadInput(me, ft, False)
        time.sleep(0.3)
        if foreground_hwnd() == hwnd:
            return True
    return foreground_hwnd() == hwnd


def close(hwnd):
    user32.PostMessageW(wt.HWND(hwnd), WM_CLOSE, 0, 0)


def wt_exe():
    """wt.exe のパス(app execution alias)。無ければ None。"""
    return shutil.which("wt.exe") or shutil.which("wt")


def launch(args, timeout=20.0, new_window=True):
    """wt.exe を起動し、新しく現れた Windows Terminal 窓の hwnd(無ければ None)と記録を返す。

    args は wt に渡すコマンド(例: ["pwsh", "-NoProfile", "-File", "x.ps1"])。`wt -w new new-tab <args>` の形にする。
    """
    before = {w["hwnd"] for w in windows()}
    exe = wt_exe()
    info = {"wt_exe": exe}
    if not exe:
        return None, {**info, "error": "wt.exe が PATH に無い"}
    cmd = [exe] + (["-w", "new"] if new_window else []) + ["new-tab"] + list(args)
    info["cmd"] = cmd
    try:
        subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except OSError as e:
        return None, {**info, "error": f"起動失敗: {e}"}
    t0 = time.time()
    while time.time() - t0 < timeout:
        new = [w for w in windows() if w["hwnd"] not in before]
        if new:
            info["launch_s"] = round(time.time() - t0, 2)
            return new[0]["hwnd"], info
        time.sleep(0.25)
    return None, {**info, "error": "新しい窓が現れなかった", "windows_after": windows()}


# ---------------------------------------------------------------- キー入力

def _ki(vk=0, scan=0, flags=0, marker=False):
    i = INPUT()
    i.type = 1
    i.ki = KEYBDINPUT(vk, scan, flags, 0, MARKER if marker else 0)
    return i


def _send(items):
    arr = (INPUT * len(items))(*items)
    return int(user32.SendInput(len(items), arr, ctypes.sizeof(INPUT)))


def key(vk, up=False, marker=False, scan=None):
    """1 回の KeyDown または KeyUp(scan code 付き)。scan を渡すとそれを使う(実機の半角/全角は 0x29 など)。"""
    if scan is None:
        scan = user32.MapVirtualKeyW(vk, 0)
    flags = (KEYEVENTF_KEYUP if up else 0) | (KEYEVENTF_EXTENDEDKEY if vk in _EXTENDED else 0)
    return _send([_ki(vk, scan, flags, marker)])


def press(vk, hold_ms=40, marker=False, scan=None):
    """Down → hold_ms → Up。送れた入力の数(2 なら成功)。"""
    n = key(vk, False, marker, scan)
    time.sleep(hold_ms / 1000)
    return n + key(vk, True, marker, scan)


def chord(mods, vk, hold_ms=40, marker=False):
    """修飾キー(VK のリスト)を押したまま vk を 1 回押す。"""
    for m in mods:
        key(m, False, marker)
    time.sleep(0.03)
    n = press(vk, hold_ms, marker)
    time.sleep(0.03)
    for m in reversed(mods):
        key(m, True, marker)
    return n


def type_vks(vks, interval_ms=60, marker=False):
    n = 0
    for v in vks:
        n += press(v, 30, marker)
        time.sleep(interval_ms / 1000)
    return n


def type_unicode(s, interval_ms=30):
    """IME を通さず文字を直接送る(KEYEVENTF_UNICODE)。入力先が文字を受け取れるかの対照。"""
    n = 0
    for ch in s:
        c = ord(ch)
        n += _send([_ki(0, c, KEYEVENTF_UNICODE), _ki(0, c, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)])
        time.sleep(interval_ms / 1000)
    return n


# wt の既定のキー操作
def shortcut_new_tab():
    return chord([VK["CTRL"], VK["SHIFT"]], VK["T"])


def shortcut_split_pane():
    """分割(Alt+Shift+Plus)。新しいペインにフォーカスが移る。"""
    return chord([VK["ALT"], VK["SHIFT"]], VK["OEM_PLUS"])


def shortcut_close_pane():
    return chord([VK["CTRL"], VK["SHIFT"]], VK["W"])


# ---------------------------------------------------------------- 読み取り・証拠

def uia_text(hwnd, count_tabs=False, timeout=60):
    cmd = ["pwsh", "-NoProfile", "-File", str(HERE / "wt_uia.ps1"), "-Hwnd", str(hwnd)] + (["-CountTabs"] if count_tabs else [])
    try:
        r = subprocess.run(cmd, capture_output=True, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as e:
        return f"ERROR {e}"
    return (r.stdout.decode("utf-8", errors="replace") + r.stderr.decode("utf-8", errors="replace")).strip()


def screenshot(path):
    ps = ("Add-Type -AssemblyName System.Windows.Forms,System.Drawing;"
          "$b=[System.Windows.Forms.SystemInformation]::VirtualScreen;"
          "$bmp=New-Object System.Drawing.Bitmap $b.Width,$b.Height;"
          "$g=[System.Drawing.Graphics]::FromImage($bmp);$g.CopyFromScreen($b.Left,$b.Top,0,0,$bmp.Size);"
          f"$bmp.Save('{path}',[System.Drawing.Imaging.ImageFormat]::Png)")
    try:
        subprocess.run(["pwsh", "-NoProfile", "-Command", ps], capture_output=True, timeout=60)
    except (OSError, subprocess.TimeoutExpired):
        return False
    return os.path.exists(path)


def echo_args(out_path, seconds=180):
    """wt の中で wt_echo.ps1 を動かす引数。"""
    return ["pwsh", "-NoProfile", "-File", str(HERE / "wt_echo.ps1"), "-Out", str(out_path), "-Seconds", str(seconds)]


def wait_ready(out_path, timeout=30.0):
    from wt_pure import parse_echo
    t0 = time.time()
    while time.time() - t0 < timeout:
        try:
            if parse_echo(Path(out_path).read_text(encoding="utf-8", errors="replace"))[0]:
                return True
        except OSError:
            pass
        time.sleep(0.3)
    return False


def read_rows(out_path):
    from wt_pure import parse_echo
    try:
        return parse_echo(Path(out_path).read_text(encoding="utf-8", errors="replace"))[1]
    except OSError:
        return []
