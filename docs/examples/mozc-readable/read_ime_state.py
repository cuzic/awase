"""mozc-readable の共有メモリから、いまの IME の開閉・入力モードを読んで表示する最小例。

仕様: docs/mozc-readable-shared-memory.md
動作: Windows のみ。標準ライブラリだけ(ctypes)。mozc-readable をインストールして、
      Mozc Readable を入力方式にした窓へ入力してから動かす。
      実行: uv run read_ime_state.py   (停止は Ctrl+C)

**未検証**: この例は Linux 上で構文だけ確認したもので、Windows 実機では動かしていない。
AppContainer(UWP など)の窓は読めない(`NtOpenSection` が要る。仕様書 4-4 章)。version 2 のビルドが前提。
動かして食い違いがあれば、仕様書(と実装)が正。
"""
import ctypes
import struct
import sys
import time
from ctypes import wintypes

MAGIC = 0x5A52444D
STRUCT = struct.Struct("<10IQ")  # u32 x10 + u64 = 48 バイト(TipSharedState)
assert STRUCT.size == 48
MODES = {0: "直接入力", 1: "ひらがな", 2: "全角カタカナ", 3: "半角英数", 4: "全角英数", 5: "半角カタカナ"}

FILE_MAP_READ = 0x0004
SYNCHRONIZE = 0x00100000

k32 = ctypes.WinDLL("kernel32", use_last_error=True)
u32 = ctypes.WinDLL("user32", use_last_error=True)
k32.OpenFileMappingW.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.LPCWSTR]
k32.OpenFileMappingW.restype = wintypes.HANDLE
k32.MapViewOfFile.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD, ctypes.c_size_t]
k32.MapViewOfFile.restype = ctypes.c_void_p
k32.UnmapViewOfFile.argtypes = [ctypes.c_void_p]
k32.CloseHandle.argtypes = [wintypes.HANDLE]
k32.OpenEventW.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.LPCWSTR]
k32.OpenEventW.restype = wintypes.HANDLE
k32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
u32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
u32.GetWindowThreadProcessId.restype = wintypes.DWORD


class GUITHREADINFO(ctypes.Structure):
    _fields_ = [
        ("cbSize", wintypes.DWORD), ("flags", wintypes.DWORD),
        ("hwndActive", wintypes.HWND), ("hwndFocus", wintypes.HWND),
        ("hwndCapture", wintypes.HWND), ("hwndMenuOwner", wintypes.HWND),
        ("hwndMoveSize", wintypes.HWND), ("hwndCaret", wintypes.HWND),
        ("rcCaret", wintypes.RECT),
    ]


def focus_thread():
    """キーボードフォーカスを持つ窓のスレッド ID とプロセス ID。前面窓のスレッドとは限らない。"""
    info = GUITHREADINFO()
    info.cbSize = ctypes.sizeof(info)
    if not u32.GetGUIThreadInfo(0, ctypes.byref(info)) or not info.hwndFocus:
        return None
    pid = wintypes.DWORD(0)
    tid = u32.GetWindowThreadProcessId(info.hwndFocus, ctypes.byref(pid))
    return (tid, pid.value) if tid else None


def read_state(tid, pid):
    """条件を満たしたときだけ辞書を返す。読めない・信用できないときは None(= 「不明」として扱う)。"""
    mapping = k32.OpenFileMappingW(FILE_MAP_READ, False, f"Local\\mozc-readable-ime-{tid}")
    if not mapping:
        return None  # その窓のスレッドでは mozc-readable が動いていない(または名前空間が違う)
    view = k32.MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, STRUCT.size)
    try:
        if not view:
            return None
        for _ in range(8):  # seqlock: 奇数は書き込み中。前後の seq が一致したときだけ採る
            seq1 = struct.unpack("<I", ctypes.string_at(view + 8, 4))[0]
            if seq1 % 2:
                continue
            raw = ctypes.string_at(view, STRUCT.size)
            seq2 = struct.unpack("<I", ctypes.string_at(view + 8, 4))[0]
            if seq1 != seq2:
                continue
            magic, version, seq, eo, em, to, tm, btid, bpid, active, tick = STRUCT.unpack(raw)
            if magic != MAGIC or seq == 0:  # seq==0: 作った直後でまだ書かれていない
                return None
            if bpid != pid:  # スレッド ID は再利用される。別プロセスの古いブロックを読まない
                return None
            if version < 2 or not active:  # v1 は「いま使われているか」が分からない。active=0 は他の IME に切り替え済み
                return None
            return {"version": version, "open": bool(to), "mode": MODES.get(tm, f"?{tm}"),
                    "effective_open": bool(eo), "effective_mode": MODES.get(em, f"?{em}")}
        return None
    finally:
        if view:
            k32.UnmapViewOfFile(view)
        k32.CloseHandle(mapping)  # 持ち続けない: 書き出し側が終わっても古いブロックが残る


def main():
    # 変化の合図(pulse)。取りこぼすので、起きるたびに読み直す。無くても 1 秒ごとに読む。
    ev = k32.OpenEventW(SYNCHRONIZE, False, "Local\\mozc-readable-ime-changed")
    last = object()
    while True:
        ft = focus_thread()
        st = read_state(*ft) if ft else None
        if st != last:
            last = st
            if st is None:
                print("不明(この窓では読めない)")
            else:
                print(f"{'開' if st['open'] else '閉'} / {st['mode']}")
        if not ev:  # TIP がまだ一度も書いていないと開けない。開けるまで毎回やり直す
            ev = k32.OpenEventW(SYNCHRONIZE, False, "Local\\mozc-readable-ime-changed")
        if ev:
            k32.WaitForSingleObject(ev, 1000)
        else:
            time.sleep(0.2)


if __name__ == "__main__":
    if sys.platform != "win32":
        sys.exit("Windows のみ")
    main()
