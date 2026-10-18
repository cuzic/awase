#!/usr/bin/env python3
"""ADR-265 段階 2: awase が mozc-readable の共有メモリを読む処理の「危ない場面」を、GitHub Actions の Windows で測る。

実機の測定(docs/tasks/adr265-stage2-measurement-result.md)が測っていない場面を、CI で測る:
  focus        フォーカスを窓(別プロセス)の間で、短い滞在で何度も移した直後の読み
  threads      同じプロセスの別スレッドの窓の間のフォーカス移動(別スレッドの状態を採らないか)
  appcontainer AppContainer の窓と、通常の窓の間のフォーカス移動(NtOpenSection の経路)
  stale        IME を別のもの(TIP の無いレイアウト)へ切り替えたあとの古い共有メモリを採らないか(active=0)

真値は操作から決める(窓ごとの IME 状態は、こちらが設定した値)。awase の `[mozc-shm]` の記録
(awase.log の info 行、`thread_id` と `tsf_open`)を、窓ごとの真値とフォーカスの時系列に、時刻で突き合わせる。

サブコマンド:
  host     窓を 1 つ以上作って、IME を指定の状態にして、生きている(別プロセスとして起動される)
  drive    host を起動し、フォーカスを移して、events.jsonl を書く
  analyze  awase.log と events.jsonl を突き合わせて、食い違いを数える(Linux でも動く)
  selftest analyze の判定の自己テスト(Linux でも動く)

Windows 専用の部分(ctypes の user32 など)は、使うときまで読み込まない。
"""
import argparse
import json
import os
import random
import re
import subprocess
import sys
import time

# ---------------------------------------------------------------------------------------------
# 突き合わせ(OS 非依存)
# ---------------------------------------------------------------------------------------------

GRACE_S = 0.9          # awase は約 500ms ごとに読む。状態やフォーカスの変化から、記録が追いつくまでの許容
LOG_RE = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?)Z\s+INFO\s+awase::mozc_shm:\s+\[mozc-shm\]\s+(\{.*\})\s*$")


def parse_log_time(s):
    """awase.log の行頭の時刻(UTC、RFC3339、Z なし部分)を epoch 秒にする。"""
    import calendar
    import datetime
    date, _, frac = s.partition(".")
    t = datetime.datetime.strptime(date, "%Y-%m-%dT%H:%M:%S")
    return calendar.timegm(t.timetuple()) + (float("0." + frac) if frac else 0.0)


def parse_records(lines):
    """awase.log の行から [mozc-shm] の記録を取り出す: [(epoch, dict)]。"""
    out = []
    for line in lines:
        m = LOG_RE.match(line.rstrip("\n"))
        if not m:
            continue
        try:
            out.append((parse_log_time(m.group(1)), json.loads(m.group(2))))
        except (ValueError, json.JSONDecodeError):
            continue
    return out


def build_timelines(events):
    """events.jsonl から、スレッドごとの真値の時系列と、フォーカスの時系列を作る。

    truth    {"ev":"truth","t":..,"tid":..,"open":0|1}      その時刻からこのスレッドの IME は open
    inactive {"ev":"inactive","t":..,"tid":..}              その時刻から TIP が無い(共有メモリは古いまま残りうる)
    focus    {"ev":"focus","t":..,"tid":..}                 その時刻にこのスレッドの窓を前面にした
    """
    truth = {}
    focus = []
    for e in events:
        ev = e.get("ev")
        if ev == "truth":
            truth.setdefault(e["tid"], []).append((e["t"], bool(e["open"])))
        elif ev == "inactive":
            truth.setdefault(e["tid"], []).append((e["t"], None))
        elif ev == "focus":
            focus.append((e["t"], e["tid"]))
    for v in truth.values():
        v.sort(key=lambda x: x[0])
    focus.sort(key=lambda x: x[0])
    return truth, focus


def allowed_states(timeline, t, grace=GRACE_S):
    """時刻 t の記録が持ってよい状態の集合(True / False / None=inactive)。直近 GRACE_S 秒内の変化を許す。

    真値が決まる前(timeline が空、または最初の項目より前)は None を返す(判定しない)。
    """
    if not timeline or timeline[0][0] > t:
        return None
    allowed = set()
    last_before = None
    have_before = False
    for te, val in timeline:
        if te <= t - grace:
            last_before = val
            have_before = True
        elif te <= t:
            allowed.add(val)
    if have_before:
        allowed.add(last_before)
    return allowed


def allowed_focus(focus, t, grace=GRACE_S):
    """時刻 t の記録が読んでよいスレッドの集合: 直近 grace 秒の間に前面だったことのあるスレッド。

    awase が検出するフォーカスは、実際の切り替えより遅れうる(非同期の読み取りを待つ)。切り替えが速いと、その遅れは
    切り替え 1 回分を超えるので、「直前のフォーカス先 1 つ」ではなく、窓 [t - grace, t] と重なる区間のフォーカス先を
    すべて許す。フォーカスがまだ決まっていなければ None。
    """
    if not focus or focus[0][0] > t:
        return None
    allowed = set()
    for k, (tf, tid) in enumerate(focus):
        if tf > t:
            break
        end = focus[k + 1][0] if k + 1 < len(focus) else float("inf")
        if end > t - grace:
            allowed.add(tid)
    return allowed


INTERVAL_LEAD_S = 1.0    # 最初のフォーカス移動の直前から
INTERVAL_TAIL_S = 3.0    # 最後のイベント(フォーカス移動・inactive)の後、awase の最後の tick が記録を出す分まで


def measurement_interval(events):
    """測定区間: 最初のフォーカス移動の少し前から、最後のイベントの少し後まで。なければ None。

    それより前(窓の準備中)に awase が別の窓を読んだ記録は、この区間の外(真値の無いスレッドの読みが出てよい)。
    """
    ts = [e["t"] for e in events if e.get("ev") in ("focus", "inactive")]
    if not ts:
        return None
    return (min(ts) - INTERVAL_LEAD_S, max(ts) + INTERVAL_TAIL_S)


def analyze(records, events, min_accepted=1, require_inactive=False, focus_grace=GRACE_S, state_grace=GRACE_S):
    """記録と events を突き合わせて集計する。dict を返す(ok が False なら食い違いがある)。"""
    truth, focus = build_timelines(events)
    known = set(truth)
    interval = measurement_interval(events)
    stats = {
        "records": len(records),
        "accepted": 0,
        "accepted_known": 0,
        "mismatch_state": [],      # 受け入れた読みの開閉が真値と違う
        "wrong_thread": [],        # フォーカスのスレッドでも、直前のフォーカス先でもないスレッドを受け入れた
        "unknown_accepted": [],    # 真値の無いスレッドの読みを受け入れた(測定区間の内外を問わず)
        "unknown_accepted_in_interval": [],   # そのうち、測定区間の内側(窓の準備中ではない)。食い違いに数える
        "stale_accepted": [],      # inactive になったあとに受け入れた
        "rejected_inactive": 0,
        "verdicts": {},
        "routes": {},
    }
    for t, r in records:
        v = r.get("verdict")
        stats["verdicts"][v] = stats["verdicts"].get(v, 0) + 1
        stats["routes"][r.get("route")] = stats["routes"].get(r.get("route"), 0) + 1
        tid = r.get("thread_id")
        if v == "rejected" and r.get("reason") == "inactive" and tid in known:
            stats["rejected_inactive"] += 1
        if v != "accepted":
            continue
        stats["accepted"] += 1
        if tid not in known:
            item = {"t": t, "tid": tid}
            stats["unknown_accepted"].append(item)
            if interval and interval[0] <= t <= interval[1]:
                stats["unknown_accepted_in_interval"].append(item)
            continue
        stats["accepted_known"] += 1
        al = allowed_states(truth[tid], t, state_grace)
        if al is not None:
            non_none = {x for x in al if x is not None}
            if not non_none:
                # inactive になってから十分たったのに受け入れた = 古い共有メモリを採った
                stats["stale_accepted"].append({"t": t, "tid": tid, "tsf_open": r.get("tsf_open")})
            elif bool(r.get("tsf_open")) not in non_none:
                stats["mismatch_state"].append({"t": t, "tid": tid, "tsf_open": r.get("tsf_open"),
                                                "allowed": sorted(str(x) for x in al)})
        af = allowed_focus(focus, t, focus_grace)
        if af is not None and tid not in af:
            stats["wrong_thread"].append({"t": t, "tid": tid, "allowed_focus": sorted(af)})
    # 真値の無いスレッドの受け入れは、測定区間の外(窓の準備中に、ランナーの別の窓を読んだ)なら食い違いに数えず、
    # 区間の内側なら数える(測定中に、こちらが前面にしていない窓を読んだ=フォーカスの取り違えの疑い)。
    us = sorted(r["read_us"] for _t, r in records if isinstance(r.get("read_us"), (int, float)))
    if us:
        def pct(q):
            return us[min(len(us) - 1, int(len(us) * q))]
        stats["read_us"] = {"n": len(us), "p50": pct(0.5), "p95": pct(0.95), "p99": pct(0.99), "max": us[-1]}
    problems = (len(stats["mismatch_state"]) + len(stats["wrong_thread"]) + len(stats["stale_accepted"])
                + len(stats["unknown_accepted_in_interval"]))
    stats["interval"] = interval
    ok = problems == 0 and stats["accepted_known"] >= min_accepted
    if require_inactive and stats["rejected_inactive"] == 0:
        ok = False
        stats["note"] = "inactive になったのに、rejected(inactive) の記録が 1 件も無い"
    stats["problems"] = problems
    stats["ok"] = ok
    return stats


def print_summary(stats):
    print("== summary")
    print(f"  records            : {stats['records']}")
    print(f"  verdicts           : {stats['verdicts']}")
    print(f"  routes             : {stats['routes']}")
    print(f"  accepted           : {stats['accepted']} (known threads: {stats['accepted_known']})")
    print(f"  rejected(inactive) : {stats['rejected_inactive']}")
    print(f"  mismatch (state)   : {len(stats['mismatch_state'])}")
    print(f"  wrong thread       : {len(stats['wrong_thread'])}")
    outside = len(stats["unknown_accepted"]) - len(stats["unknown_accepted_in_interval"])
    print(f"  unknown accepted   : in the measurement interval: {len(stats['unknown_accepted_in_interval'])}"
          f" / outside it (window setup): {outside}")
    print(f"  stale accepted     : {len(stats['stale_accepted'])}")
    if stats.get("read_us"):
        print(f"  read_us            : {stats['read_us']}")
    for key in ("mismatch_state", "wrong_thread", "unknown_accepted_in_interval", "stale_accepted"):
        for item in stats[key][:10]:
            print(f"    {key}: {item}")
    if stats.get("note"):
        print(f"  note: {stats['note']}")
    print(f"  => {'OK' if stats['ok'] else 'NG'}")


def cmd_analyze(args):
    with open(args.awase_log, encoding="utf-8", errors="replace") as f:
        records = parse_records(f)
    events = []
    with open(args.events, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                events.append(json.loads(line))
    stats = analyze(records, events, min_accepted=args.min_accepted, require_inactive=args.require_inactive,
                    focus_grace=args.focus_grace, state_grace=args.state_grace)
    print_summary(stats)
    if args.summary_json:
        with open(args.summary_json, "w", encoding="utf-8") as f:
            json.dump(stats, f, ensure_ascii=False, indent=1)
    return 0 if stats["ok"] else 1


def cmd_selftest(_args):
    """analyze の判定が、意図どおりに食い違いを数えることを、合成データで確かめる(Windows 不要)。"""
    def rec(t, tid, open_, verdict="accepted", reason=None):
        d = {"verdict": verdict, "thread_id": tid, "route": "local", "tsf_open": open_ if verdict == "accepted" else None}
        if reason:
            d["reason"] = reason
        return (t, d)

    base = 1000.0
    events = [
        {"ev": "truth", "t": base, "tid": 1, "open": 1},
        {"ev": "truth", "t": base, "tid": 2, "open": 0},
        {"ev": "focus", "t": base + 1, "tid": 1},
        {"ev": "focus", "t": base + 5, "tid": 2},
        {"ev": "focus", "t": base + 9, "tid": 1},
    ]
    good = [rec(base + 1.2, 1, True), rec(base + 5.3, 2, False), rec(base + 9.4, 1, True)]
    s = analyze(good, events, min_accepted=3)
    assert s["ok"] and s["accepted_known"] == 3, s

    # 開閉の食い違い
    s = analyze([rec(base + 2, 1, False)], events)
    assert not s["ok"] and len(s["mismatch_state"]) == 1, s

    # フォーカス先でも直前のフォーカス先でもないスレッドを受け入れた(2 を読むべきでない時刻に 2)
    s = analyze([rec(base + 3, 2, False)], events)
    assert not s["ok"] and len(s["wrong_thread"]) == 1, s

    # 切り替えの直後(GRACE_S 以内)は、直前のフォーカス先の読みを許す
    s = analyze([rec(base + 5.4, 1, True)], events)
    assert s["ok"], s
    # 切り替えから十分後は許さない
    s = analyze([rec(base + 6.5, 1, True)], events)
    assert not s["ok"] and len(s["wrong_thread"]) == 1, s

    # 速い切り替え: 2 回前のフォーカス先でも、grace の範囲内なら許す(awase のフォーカス検出の遅れ)。範囲外は許さない
    fast = [{"ev": "truth", "t": base, "tid": 1, "open": 1}, {"ev": "truth", "t": base, "tid": 2, "open": 0},
            {"ev": "truth", "t": base, "tid": 3, "open": 1},
            {"ev": "focus", "t": base + 1.0, "tid": 1}, {"ev": "focus", "t": base + 1.3, "tid": 2},
            {"ev": "focus", "t": base + 1.6, "tid": 3}]
    s = analyze([rec(base + 1.8, 1, True)], fast, focus_grace=1.5)   # 1 のフォーカスは 0.3 秒前まで続いた
    assert s["ok"], s
    s = analyze([rec(base + 1.8, 1, True)], fast)                    # 既定の grace(0.9)でも、窓 [0.9, 1.8] と重なる
    assert s["ok"], s
    s = analyze([rec(base + 3.2, 1, True)], fast, focus_grace=1.5)   # 1 のフォーカスは 1.3 秒前に終わり、窓 [1.7, 3.2] の外
    assert not s["ok"] and len(s["wrong_thread"]) == 1, s

    # 真値の無いスレッドの読み: 測定区間の外(窓の準備中)なら数えるだけ、区間の内側なら食い違い
    s = analyze([rec(base - 20, 99, True), rec(base + 2.1, 1, True)], events)
    assert s["ok"] and len(s["unknown_accepted"]) == 1 and not s["unknown_accepted_in_interval"], s
    s = analyze([rec(base + 2, 99, True), rec(base + 2.1, 1, True)], events)
    assert not s["ok"] and len(s["unknown_accepted_in_interval"]) == 1, s

    # rejected / failed は数えない(食い違いにならない)
    s = analyze([rec(base + 2, 1, None, "rejected", "inactive"), rec(base + 2.1, 1, None, "failed", "not-found")],
                events, min_accepted=0)
    assert s["ok"] and s["rejected_inactive"] == 1, s

    # 受け入れた件数が足りなければ NG
    s = analyze([], events, min_accepted=1)
    assert not s["ok"], s

    # inactive: 切り替えの直後(GRACE_S 以内)の accepted は許し、十分後の accepted は古い共有メモリとして NG
    ev2 = [{"ev": "truth", "t": base, "tid": 1, "open": 1}, {"ev": "inactive", "t": base + 10, "tid": 1},
           {"ev": "focus", "t": base, "tid": 1}]
    s = analyze([rec(base + 10.4, 1, True)], ev2)
    assert s["ok"], s
    s = analyze([rec(base + 12, 1, True)], ev2)
    assert not s["ok"] and len(s["stale_accepted"]) == 1, s
    s = analyze([rec(base + 12, 1, None, "rejected", "inactive"), rec(base + 3, 1, True)], ev2, require_inactive=True)
    assert s["ok"] and s["rejected_inactive"] == 1, s
    s = analyze([rec(base + 3, 1, True)], ev2, require_inactive=True)
    assert not s["ok"], s

    # ログ行の解析
    line = ('2026-10-18T06:30:31.250000Z  INFO awase::mozc_shm: [mozc-shm] '
            '{"trigger":"focus","route":"local","verdict":"accepted","thread_id":7,"tsf_open":true}')
    recs = parse_records([line, "noise", ""])
    assert len(recs) == 1 and recs[0][1]["thread_id"] == 7, recs
    assert abs(recs[0][0] - 1792305031.25) < 1e-6, recs[0][0]
    print("selftest OK")
    return 0


# ---------------------------------------------------------------------------------------------
# Windows 側(窓を作る・キーを送る・フォーカスを移す)
# ---------------------------------------------------------------------------------------------

def win32():
    """user32 / kernel32 の ctypes を準備して、道具をまとめた名前空間を返す(Windows でだけ呼ぶ)。"""
    import ctypes
    import ctypes.wintypes as wt
    import struct
    import types

    u32 = ctypes.windll.user32
    k32 = ctypes.windll.kernel32
    ns = types.SimpleNamespace(ctypes=ctypes, wt=wt, u32=u32, k32=k32, struct=struct)
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
    u32.GetWindowThreadProcessId.argtypes = [wt.HWND, ctypes.POINTER(wt.DWORD)]
    u32.GetWindowThreadProcessId.restype = wt.DWORD
    u32.AttachThreadInput.argtypes = [wt.DWORD, wt.DWORD, wt.BOOL]
    u32.GetClassNameW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
    u32.IsWindowVisible.argtypes = [wt.HWND]
    u32.DefWindowProcW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]
    u32.DefWindowProcW.restype = ctypes.c_ssize_t
    k32.GetModuleHandleW.restype = wt.HMODULE
    k32.GetModuleHandleW.argtypes = [wt.LPCWSTR]
    k32.OpenFileMappingW.restype = wt.HANDLE
    k32.OpenFileMappingW.argtypes = [wt.DWORD, wt.BOOL, wt.LPCWSTR]
    k32.MapViewOfFile.restype = ctypes.c_void_p
    k32.MapViewOfFile.argtypes = [wt.HANDLE, wt.DWORD, wt.DWORD, wt.DWORD, ctypes.c_size_t]

    ns.WNDPROC = ctypes.WINFUNCTYPE(ctypes.c_ssize_t, wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM)

    class WNDCLASSW(ctypes.Structure):
        _fields_ = [("style", wt.UINT), ("lpfnWndProc", ns.WNDPROC), ("cbClsExtra", ctypes.c_int),
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

    ns.WNDCLASSW, ns.KEYBDINPUT, ns.INPUT = WNDCLASSW, KEYBDINPUT, INPUT

    def send_vk(vk):
        for flags in (0, 2):
            i = INPUT(type=1)
            i.ki = KEYBDINPUT(vk, 0, flags, 0, 0)
            u32.SendInput(1, ctypes.byref(i), ctypes.sizeof(INPUT))
            time.sleep(0.03)

    def pump(seconds):
        end = time.time() + seconds
        msg = wt.MSG()
        while time.time() < end:
            while u32.PeekMessageW(ctypes.byref(msg), None, 0, 0, 1):
                u32.TranslateMessage(ctypes.byref(msg))
                u32.DispatchMessageW(ctypes.byref(msg))
            time.sleep(0.01)

    def tid_pid(hwnd):
        pid = wt.DWORD()
        tid = u32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        return tid, pid.value

    def cls_of(hwnd):
        buf = ctypes.create_unicode_buffer(256)
        u32.GetClassNameW(hwnd, buf, 256)
        return buf.value

    def force_foreground(hwnd):
        """hwnd を前面にする。前面になったか(GetForegroundWindow が hwnd か)を返す。

        前面でないプロセスからの SetForegroundWindow は、前面の制限で黙って失敗することがある。
        1) 今の前面のスレッドへ入力をアタッチする、2) それでも駄目なら Alt キーを一度押して(制限が外れる)もう一度。
        """
        def attempt():
            fg = u32.GetForegroundWindow()
            cur = k32.GetCurrentThreadId()
            fg_tid = tid_pid(fg)[0] if fg else 0
            if fg_tid and fg_tid != cur:
                u32.AttachThreadInput(cur, fg_tid, True)
            u32.ShowWindow(hwnd, 9)
            u32.SetForegroundWindow(hwnd)
            if fg_tid and fg_tid != cur:
                u32.AttachThreadInput(cur, fg_tid, False)
            time.sleep(0.03)
            return u32.GetForegroundWindow() == hwnd

        if attempt():
            return True
        u32.keybd_event(0x12, 0, 0, 0)   # VK_MENU down / up
        u32.keybd_event(0x12, 0, 2, 0)
        return attempt()

    views = {}

    def read_block(tid):
        """自分のプロセスの(通常の)共有メモリを読む: 48 バイトの tuple、無ければ None。"""
        v = views.get(tid)
        if v is None:
            h = k32.OpenFileMappingW(4, False, f"Local\\mozc-readable-ime-{tid}")
            if not h:
                return None
            v = k32.MapViewOfFile(h, 4, 0, 0, 48)
            if not v:
                return None
            views[tid] = v
        for _ in range(50):
            a = struct.unpack("<IIIIIIIIIIQ", ctypes.string_at(v, 48))
            if a[2] % 2 == 0 and a == struct.unpack("<IIIIIIIIIIQ", ctypes.string_at(v, 48)):
                return a
        return None

    ns.send_vk, ns.pump, ns.tid_pid, ns.cls_of = send_vk, pump, tid_pid, cls_of
    ns.force_foreground, ns.read_block = force_foreground, read_block
    return ns


def emit(obj):
    print(json.dumps(obj, ensure_ascii=True), flush=True)


class Window:
    """1 つのスレッドが持つ窓(EDIT 付き)。そのスレッドの上で作る・状態を設定する・メッセージを回す。"""

    def __init__(self, w, name, cls_suffix):
        self.w = w
        self.name = name
        self.cls = f"ShmCi{cls_suffix}"
        self.hwnd = None
        self.edit = None
        self.tid = None
        self._wndproc = None

    def create(self):
        w, u32, k32, ctypes = self.w, self.w.u32, self.w.k32, self.w.ctypes
        self._wndproc = w.WNDPROC(lambda h, m, wp, lp: u32.DefWindowProcW(h, m, wp, lp))
        wc = w.WNDCLASSW()
        wc.lpfnWndProc = self._wndproc
        wc.hInstance = k32.GetModuleHandleW(None)
        wc.lpszClassName = self.cls
        u32.RegisterClassW(ctypes.byref(wc))
        self.hwnd = u32.CreateWindowExW(0, self.cls, f"shmci-{self.name}", 0x10CF0000, 100, 100, 500, 200,
                                        None, None, wc.hInstance, None)
        self.edit = u32.CreateWindowExW(0x200, "EDIT", "", 0x50010000 | 0x0080, 10, 10, 460, 120, self.hwnd,
                                        None, wc.hInstance, None)
        u32.ShowWindow(self.hwnd, 5)
        self.tid = k32.GetCurrentThreadId()

    def focus(self):
        self.w.force_foreground(self.hwnd)
        self.w.u32.SetFocus(self.edit)

    def state_open(self):
        a = self.w.read_block(self.tid)
        return None if a is None else a[5]  # tsf_open

    def active(self):
        a = self.w.read_block(self.tid)
        return None if a is None else a[9]

    def set_open(self, want):
        """IME を want(0/1)にする。ON はひらがなキー(冪等)を、確認しながら最大 5 回。OFF は既定の状態なので何もしない。"""
        if not want:
            self.focus()
            deadline = time.time() + 6
            while time.time() < deadline:  # TIP が読み込まれて共有メモリができるのを待つ
                self.w.pump(0.1)
                if self.state_open() == 0:
                    return True
            return False
        for _ in range(5):
            self.focus()
            self.w.send_vk(0xF2)
            deadline = time.time() + 4
            while time.time() < deadline:
                self.w.pump(0.05)
                if self.state_open() == 1:
                    return True
        return False


SETUP_LOCK = None


def host_thread_main(w, name, want_open, ready_evt, results, idx, run_until, switch_after):
    win = Window(w, name, f"{name}")
    win.create()
    with SETUP_LOCK:
        ok = win.set_open(want_open)
    results[idx] = (win, ok)
    ready_evt.set()
    switched = False
    t0 = time.time()
    while time.time() < run_until:
        w.pump(0.1)
        if switch_after and not switched and time.time() - t0 >= switch_after:
            switched = True
            # TIP の無いレイアウトへ切り替える(セッション全体に効く)。active が 0 になるのを待つ。
            w.u32.LoadKeyboardLayoutW.restype = w.ctypes.c_void_p
            w.u32.ActivateKeyboardLayout.argtypes = [w.ctypes.c_void_p, w.wt.UINT]
            hkl = w.u32.LoadKeyboardLayoutW("00000409", 1)
            w.u32.ActivateKeyboardLayout(hkl, 0)
            deadline = time.time() + 8
            while time.time() < deadline and win.active() != 0:
                w.pump(0.05)
            emit({"ev": "inactive", "t": time.time(), "tid": win.tid, "name": name, "active": win.active()})


def cmd_host(args):
    """窓を作って IME を指定の状態にし、READY(真値)を出して、lifetime 秒生きる。--threads が複数なら同じプロセスの別スレッド。"""
    import threading
    global SETUP_LOCK
    SETUP_LOCK = threading.Lock()
    w = win32()
    opens = [int(x) for x in args.opens.split(",")]
    n = len(opens)
    run_until = time.time() + args.lifetime
    results = [None] * n
    events = [threading.Event() for _ in range(n)]
    threads = []
    for i, want in enumerate(opens):
        name = args.name if n == 1 else f"{args.name}{i}"
        th = threading.Thread(target=host_thread_main,
                              args=(w, name, want, events[i], results, i, run_until,
                                    args.switch_after if i == 0 else 0),
                              daemon=True)
        th.start()
        threads.append(th)
        events[i].wait(60)  # 順に設定する(前面を取り合わない)
    pid = os.getpid()
    for i, (win, ok) in enumerate(r for r in results if r):
        emit({"ev": "truth", "t": time.time(), "name": win.name, "tid": win.tid, "pid": pid,
              "hwnd": int(win.hwnd or 0), "open": int(opens[i]), "confirmed": bool(ok)})
    for th in threads:
        th.join(timeout=max(0.0, run_until - time.time()) + 5)
    return 0


# ---------------------------------------------------------------------------------------------
# ドライバー
# ---------------------------------------------------------------------------------------------

def spawn_host(here, name, opens, lifetime, switch_after=0, extra_env=None):
    cmd = [sys.executable, "-u", os.path.join(here, "shmci.py"), "host", "--name", name,
           "--opens", opens, "--lifetime", str(lifetime)]
    if switch_after:
        cmd += ["--switch-after", str(switch_after)]
    # コンソール窓を作らない(前面を奪ってキーを横取りされる。mozc-readable の run 39104699356)
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1,
                         creationflags=0x08000000, env=extra_env)
    return p


def read_events_until(p, want_truth, timeout):
    """host の出力から JSON 行を読み、truth が want_truth 個そろうまで返す。"""
    evs = []
    deadline = time.time() + timeout
    got = 0
    while got < want_truth and time.time() < deadline:
        line = p.stdout.readline()
        if not line:
            if p.poll() is not None:
                break
            continue
        line = line.strip()
        if not line.startswith("{"):
            print("host:", line, flush=True)
            continue
        try:
            e = json.loads(line)
        except json.JSONDecodeError:
            continue
        evs.append(e)
        if e.get("ev") == "truth":
            got += 1
    return evs


def drain_in_thread(p, sink):
    import threading

    def run():
        for line in p.stdout:
            line = line.strip()
            if line.startswith("{"):
                try:
                    sink.append(json.loads(line))
                except json.JSONDecodeError:
                    pass
            elif line:
                print("host:", line, flush=True)
    th = threading.Thread(target=run, daemon=True)
    th.start()
    return th


def focus_loop(w, targets, rounds, rng, events, dwell=(0.35, 1.6)):
    """targets: [(name, tid, hwnd)]。ランダムな順に前面にして、短い時間だけ待つ。

    前面になったことを確かめてから focus イベントを書く(前面にならなかった切り替えは、書かずに数える)。
    前面でないプロセスからの SetForegroundWindow は、黙って失敗することがある(失敗した切り替えを「フォーカスを移した」
    と書くと、awase が何も読まないのが、読まなかったのではなく移っていなかっただけ、という見分けがつかない)。
    戻り値は (成功した切り替え数, 失敗した切り替え数)。
    """
    last = None
    ok_n = 0
    ng_n = 0
    for _ in range(rounds):
        choices = [t for t in targets if t[1] != last]
        name, tid, hwnd = rng.choice(choices)
        ok = w.force_foreground(hwnd)
        if ok:
            events.append({"ev": "focus", "t": time.time(), "tid": tid, "name": name})
            last = tid
            ok_n += 1
        else:
            ng_n += 1
        time.sleep(rng.uniform(*dwell))
    return ok_n, ng_n


FLIP_DELAYS = (0.0, 0.02, 0.05, 0.1, 0.2)


def flip_loop(w, truths, rounds, rng, events):
    """真値が動く場面: 前面の窓の IME 状態を切り替え(ON=ひらがなキー、OFF=半角/全角キー)、すぐあとに別の窓へフォーカスを移す。

    切り替えてから移すまでの間隔は 0 / 20 / 50 / 100 / 200 ms を順に使う。真値は切り替えのキーを送る直前の時刻から新しい
    状態(状態の許容は --state-grace で短くする)。キーが効かなかったときは、共有メモリの実際の値を真値に直す(数える)。
    """
    cur = {t["tid"]: int(t["open"]) for t in truths}
    info = {t["tid"]: t for t in truths if t.get("hwnd")}
    tids = list(info)
    last = None
    ok_n = ng_n = lost = 0
    # 準備: 切り替えのキーが窓に届いて状態が変わるか、変わるまでの時間はどれくらいか(診断。測定の前提)
    for k in range(8):
        x = tids[k % len(tids)]
        fg = w.force_foreground(info[x]["hwnd"])
        time.sleep(0.5)
        want = 1 - cur[x]
        before = w.read_block(x)
        t0 = time.time()
        w.send_vk(0xF2 if want else 0x19)
        lat = None
        while time.time() - t0 < 3.0:
            a = w.read_block(x)
            if a is not None and int(a[5]) == want:
                lat = time.time() - t0
                break
            time.sleep(0.01)
        a = w.read_block(x)
        print(f"flip-probe {k}: tid={x} fg={fg} want={want} before={before and (before[5], before[9])} "
              f"after={a and (a[5], a[9])} latency_s={lat}", flush=True)
        if lat is not None:
            cur[x] = want
        elif a is not None:
            cur[x] = int(a[5])
        events.append({"ev": "truth", "t": t0, "tid": x, "name": info[x]["name"], "pid": info[x]["pid"],
                       "hwnd": info[x]["hwnd"], "open": cur[x], "confirmed": True})
    for i in range(rounds):
        x = rng.choice([t for t in tids if t != last])
        if not w.force_foreground(info[x]["hwnd"]):
            ng_n += 1
            continue
        events.append({"ev": "focus", "t": time.time(), "tid": x, "name": info[x]["name"]})
        time.sleep(0.6)                      # awase が x を読む時間
        want = 1 - cur[x]
        t_send = time.time()
        w.send_vk(0xF2 if want else 0x19)
        events.append({"ev": "truth", "t": t_send, "tid": x, "name": info[x]["name"], "pid": info[x]["pid"],
                       "hwnd": info[x]["hwnd"], "open": want, "confirmed": True, "flip": True})
        cur[x] = want
        time.sleep(FLIP_DELAYS[i % len(FLIP_DELAYS)])
        y = rng.choice([t for t in tids if t != x])
        if w.force_foreground(info[y]["hwnd"]):
            events.append({"ev": "focus", "t": time.time(), "tid": y, "name": info[y]["name"]})
            last = y
            ok_n += 1
        else:
            ng_n += 1
            last = x
        time.sleep(0.5)
        a = w.read_block(x)                  # キーが効いたか
        if a is not None and a[9] == 1 and int(a[5]) != want:
            lost += 1
            cur[x] = int(a[5])
            events.append({"ev": "truth", "t": time.time() - 0.5, "tid": x, "name": info[x]["name"],
                           "pid": info[x]["pid"], "hwnd": info[x]["hwnd"], "open": int(a[5]), "confirmed": True})
    return ok_n, ng_n, lost


def cmd_burn(_args):
    """CPU を使い切る(負荷時の read_us を測るための重り)。親が止める。"""
    while True:
        pass


def start_burners(n, procs):
    ps = []
    for _ in range(n):
        p = subprocess.Popen([sys.executable, "-u", os.path.abspath(__file__), "burn"], creationflags=0x08000000)
        ps.append(p)
        procs.append(p)
    if n:
        print(f"load: {n} CPU-burning processes started", flush=True)
    return ps


def cmd_tidreuse(args):
    """スレッド ID の再利用: スレッドを順に作っては捨て、同じ ID が再び使われたとき、前のスレッドの共有メモリが残っていないか。

    各スレッドは、自分の ID の共有メモリを窓を作る前に読み(前の持ち主の残り)、窓を作って IME を ON にして、終わる。
    再利用された ID で、窓を作る前に active=1 のブロックが見えれば、新しいスレッドが TIP を持つ前に、古い持ち主の状態を
    読む可能性がある(awase は process_id の一致も見るので、別のプロセスの残りは採らない。同じプロセスの再利用が残る)。
    """
    import threading
    w = win32()
    seen = {}
    rows = []
    lock = threading.Lock()

    def body(n):
        tid = w.k32.GetCurrentThreadId()
        pre = w.read_block(tid)
        win = Window(w, f"R{n}", f"R{n}")
        win.create()
        ok = win.set_open(1)
        post = w.read_block(tid)
        with lock:
            rows.append({"n": n, "tid": tid, "reused": tid in seen, "pre": pre, "set_ok": ok, "post": post})
            seen[tid] = seen.get(tid, 0) + 1
        w.u32.DestroyWindow(win.hwnd)

    for n in range(args.threads):
        th = threading.Thread(target=body, args=(n,))
        th.start()
        th.join(30)
        time.sleep(0.05)
    reused = [r for r in rows if r["reused"]]
    stale = [r for r in reused if r["pre"] is not None and r["pre"][9] == 1]
    seen_pre = [r for r in reused if r["pre"] is not None]
    print(f"threads: {len(rows)}, distinct thread ids: {len(seen)}, reused ids: {len(reused)}")
    print(f"reused and a shared block existed before the new thread created its window: {len(seen_pre)}")
    print(f"  of those, still active=1 (the previous owner's state is readable as if it were the new thread's): {len(stale)}")
    for r in stale[:10]:
        print(f"    tid={r['tid']} n={r['n']} pre={r['pre']}")
    print(f"IME could not be set to ON in the new thread: {sum(1 for r in rows if not r['set_ok'])}")
    return 0


def cmd_drive(args):
    w = win32()
    here = os.path.dirname(os.path.abspath(__file__))
    rng = random.Random(args.seed)
    events = []
    procs = []
    sinks = []
    scenario = args.scenario
    lifetime = args.rounds * max(1.6, args.dwell_max + 0.3) + 90
    exit_code = 0

    try:
        if scenario in ("focus", "flip"):
            specs = [("A", "1"), ("B", "0"), ("C", "1"), ("D", "0")]       # 別プロセス 4 つ。ON/OFF を交互に
            for name, opens in specs:
                p = spawn_host(here, name, opens, lifetime)
                procs.append(p)
                events += read_events_until(p, 1, 90)
                sinks.append([])
                drain_in_thread(p, sinks[-1])
        elif scenario == "threads":
            p = spawn_host(here, "T", "1,0,1,0", lifetime)                 # 同じプロセスの 4 スレッド
            procs.append(p)
            events += read_events_until(p, 4, 120)
            sinks.append([])
            drain_in_thread(p, sinks[-1])
            p2 = spawn_host(here, "X", "0", lifetime)                      # 別プロセスも 1 つ(比較用)
            procs.append(p2)
            events += read_events_until(p2, 1, 90)
            sinks.append([])
            drain_in_thread(p2, sinks[-1])
        elif scenario == "appcontainer":
            # AppContainer のプロセスは mozc_server を自分では起動できない(サンドボックス)。通常のプロセスが先に IME を
            # 有効にしてサーバーを起こしておく(mozc-readable の CI では、全セクションの前のウォームアップがこれ)。
            warm = spawn_host(here, "W", "1", 40)
            warm_events = read_events_until(warm, 1, 90)
            print(f"warm-up: {[e for e in warm_events if e.get('ev') == 'truth']}", flush=True)
            warm.terminate()
            time.sleep(1.0)
            events += run_appcontainer_setup(args, here, procs, sinks)
            p = spawn_host(here, "B", "0", lifetime)
            procs.append(p)
            events += read_events_until(p, 1, 90)
            sinks.append([])
            drain_in_thread(p, sinks[-1])
        elif scenario == "stale":
            p = spawn_host(here, "S", "1", lifetime, switch_after=args.switch_after)
            procs.append(p)
            events += read_events_until(p, 1, 90)
            sinks.append([])
            drain_in_thread(p, sinks[-1])
        else:
            raise SystemExit(f"unknown scenario {scenario}")

        truths = [e for e in events if e.get("ev") == "truth"]
        for t in truths:
            print(f"truth: {t}", flush=True)
        bad = [t for t in truths if not t.get("confirmed", True)]
        if bad:
            print(f"setup failed (the IME could not be set to the wanted state): {bad}", flush=True)

        targets = [(t["name"], t["tid"], t["hwnd"]) for t in truths if t.get("hwnd")]
        if scenario == "stale":
            # フォーカスは動かさない: S を前面に置いたまま、切り替え(--switch-after 秒後)と、そのあとの記録を待つ。
            s = truths[0]
            w.force_foreground(s["hwnd"])
            events.append({"ev": "focus", "t": time.time(), "tid": s["tid"], "name": s["name"]})
            time.sleep(args.switch_after + 12)
        elif scenario == "flip":
            ok_n, ng_n, lost = flip_loop(w, truths, args.rounds, rng, events)
            print(f"flip rounds: {ok_n} flipped and moved the focus, {ng_n} focus moves did not take effect, "
                  f"{lost} IME-state changes did not happen (truth was corrected from the shared memory)", flush=True)
            if ok_n < max(10, args.rounds // 2):
                print("too few flips took effect: the measurement is not meaningful", flush=True)
                exit_code = 3
        elif len(targets) < 2:
            print(f"need at least 2 windows to move the focus between, got {len(targets)}: setup failed", flush=True)
            exit_code = 2
        else:
            burners = start_burners(args.load, procs)
            ok_n, ng_n = focus_loop(w, targets, args.rounds, rng, events, dwell=(args.dwell_min, args.dwell_max))
            print(f"focus switches: {ok_n} took effect, {ng_n} did not (the window was not in the foreground afterwards)",
                  flush=True)
            if ok_n < max(10, args.rounds // 2):
                print("too few focus switches took effect: the measurement is not meaningful", flush=True)
                exit_code = 3
        time.sleep(2.0)  # 最後の tick が記録を出すのを待つ
    finally:
        for p in procs:
            try:
                p.terminate()
            except OSError:
                pass
        time.sleep(0.5)
        for s in sinks:
            events += s
        os.makedirs(os.path.dirname(os.path.abspath(args.events)), exist_ok=True)
        with open(args.events, "w", encoding="utf-8") as f:
            for e in sorted(events, key=lambda e: e.get("t", 0)):
                f.write(json.dumps(e, ensure_ascii=True) + "\n")
        print(f"wrote {len(events)} events to {args.events}", flush=True)
    return exit_code


def run_appcontainer_setup(args, here, procs, sinks):
    """mozc-readable の CI の部品(ac_launch.py / drive_foreground.py)で AppContainer の窓を ON にして、真値を返す。"""
    ci = args.mozc_ci_dir
    work = os.path.join(os.environ.get("RUNNER_TEMP", "."), "shmci_ac")
    os.makedirs(work, exist_ok=True)
    launch = subprocess.Popen([sys.executable, os.path.join(ci, "ac_launch.py"), work, "300", "300"],
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, creationflags=0x08000000)
    procs.append(launch)
    sinks.append([])
    time.sleep(12)
    out = subprocess.run([sys.executable, os.path.join(ci, "drive_foreground.py"), "ReadableCiHost", "F2=1:1", "40"],
                         capture_output=True, text=True, creationflags=0x08000000)
    print(out.stdout, flush=True)
    m = re.search(r"fg_tid=(\d+) fg_pid=(\d+)", out.stdout)
    if not m or out.returncode != 0:
        print(f"appcontainer setup failed (exit {out.returncode})", flush=True)
        return []
    w = win32()
    hwnd = None
    found = []

    def cb(h, _):
        if w.u32.IsWindowVisible(h) and w.cls_of(h) == "ReadableCiHost":
            found.append(h)
        return True
    w.u32.EnumWindows(w.ctypes.WINFUNCTYPE(w.wt.BOOL, w.wt.HWND, w.wt.LPARAM)(cb), 0)
    hwnd = int(found[0]) if found else 0
    return [{"ev": "truth", "t": time.time(), "name": "AC", "tid": int(m.group(1)), "pid": int(m.group(2)),
             "hwnd": hwnd, "open": 1, "confirmed": True}]


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    h = sub.add_parser("host")
    h.add_argument("--name", required=True)
    h.add_argument("--opens", default="0", help="スレッドごとの状態 0/1 のカンマ区切り(複数なら同じプロセスの別スレッド)")
    h.add_argument("--lifetime", type=float, default=120)
    h.add_argument("--switch-after", type=float, default=0, help="この秒数後に、先頭のスレッドを TIP の無いレイアウトへ切り替える")

    d = sub.add_parser("drive")
    d.add_argument("scenario", choices=["focus", "threads", "appcontainer", "stale", "flip"])
    d.add_argument("--events", required=True)
    d.add_argument("--rounds", type=int, default=100)
    d.add_argument("--dwell-min", type=float, default=0.35, help="次の切り替えまでの最短の滞在(秒)")
    d.add_argument("--dwell-max", type=float, default=1.6, help="次の切り替えまでの最長の滞在(秒)")
    d.add_argument("--seed", type=int, default=1)
    d.add_argument("--load", type=int, default=0, help="測定中、CPU を使い切るプロセスをこの数だけ動かす(負荷時の read_us)")
    d.add_argument("--switch-after", type=float, default=12)
    d.add_argument("--mozc-ci-dir", default="mozc-readable/readable/ci")

    a = sub.add_parser("analyze")
    a.add_argument("--awase-log", required=True)
    a.add_argument("--events", required=True)
    a.add_argument("--min-accepted", type=int, default=20)
    a.add_argument("--require-inactive", action="store_true")
    a.add_argument("--focus-grace", type=float, default=GRACE_S,
                   help="記録のスレッドとして許す、直近のフォーカス先の範囲(秒)。速い切り替えでは長くする")
    a.add_argument("--state-grace", type=float, default=GRACE_S,
                   help="状態の変化から、記録が新しい状態に追いつくまでの許容(秒)。flip では短くする")
    a.add_argument("--summary-json")

    sub.add_parser("burn")
    t = sub.add_parser("tidreuse")
    t.add_argument("--threads", type=int, default=60)
    sub.add_parser("selftest")

    args = ap.parse_args(argv)
    return {"host": cmd_host, "drive": cmd_drive, "analyze": cmd_analyze, "selftest": cmd_selftest,
            "burn": cmd_burn, "tidreuse": cmd_tidreuse}[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main())
