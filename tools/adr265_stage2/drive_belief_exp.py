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
WM_IME_CONTROL, IMC_SETOPENSTATUS, IMC_GETOPENSTATUS = 0x0283, 0x0006, 0x0005
WM_GETTEXT, WM_GETTEXTLENGTH = 0x000D, 0x000E
u32.SendMessageW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, ctypes.c_void_p]
u32.SendMessageW.restype = ctypes.c_ssize_t


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
    # the first key of a fresh process can take about 8 s to act (cold start), so wait up to 40 s
    end = time.time() + 40
    n = 0
    # 新しい窓では、TIP がテキスト入力を 1 回受けるまで IME キーが効かないことがある（rust-nicola-56 の実測）。
    # 自前の窓に「a」と Enter を 1 回打って起こす（入力欄は自前のもの）。
    dm.send_vk(0x41)
    time.sleep(0.4)
    dm.send_vk(0x0D)
    time.sleep(0.8)
    while time.time() < end:
        _, ftid, fpid, fcls, _ = dm.focus_info()
        blk = dm.read_block(ftid, fpid) if fpid in pids else None
        if blk and blk.get("eff_open") == 1:
            out.emit(event="ensure_on", ok=True, keys_sent=n)
            return True
        if n < 3 or time.time() > end - 20:  # a few keys, then wait; more keys late if still off
            dm.send_vk(0xF2)
            n += 1
        time.sleep(1.5)
    out.emit(event="ensure_on", ok=False, keys_sent=n, last_block=blk, focus_pid=fpid, focus_class=fcls)
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


def e1watch(args, out):
    """E1 (automatic): focus the host's text edit, turn the IME on, then only record every shared-block change of that
    thread for --rounds seconds (the actions are done by e1_uia.ps1, which logs its own UTC timestamps)."""
    pids = {int(x) for x in args.pids.split(",") if x}
    tt = text_thread(pids)
    if not tt:
        out.emit(event="abort", reason="no text focus")
        return 2
    h, tid, pid, cls = tt
    if not ensure_on(pids, out):
        out.emit(event="abort", reason="could not turn the IME on")
        return 4
    out.emit(event="ready", tid=tid, pid=pid, cls=cls)
    stop = threading.Event()
    t = threading.Thread(target=watcher, args=(out, tid, pid, stop), daemon=True)
    t.start()
    time.sleep(float(args.rounds))
    stop.set()
    out.emit(event="done", scenario="e1watch")
    return 0


TEST_INJECTION_MARKER = 0x5350494B  # awase.hook::TEST_INJECTION_MARKER (AWASE_TEST_INJECTION=1 の debug ビルドが物理キー扱いする)


def send_marked(vk):
    for flags in (0, 2):
        i = dm.INPUT(type=1)
        i.ki = dm.KEYBDINPUT(vk, 0, flags, 0, TEST_INJECTION_MARKER)
        u32.SendInput(1, ctypes.byref(i), ctypes.sizeof(dm.INPUT))
        time.sleep(0.03)


def e5(args, out):
    """ADR-268 D3 の検証: 明示意図(物理の IME キー)が残っている状態で、外から IME を閉じたとき、awase が追随するか。
    awase は AWASE_TEST_INJECTION=1 で起動してあること。各周: 漢字キー(物理扱い)で閉→開(明示意図を作る)→数秒待つ
    (tick のタイマーが止まるなら止まる時間)→ IMM で外から閉→ awase.log の `follow external change` を見る。"""
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
    out.emit(event="ready", tid=tid, pid=pid, ime_wnd=int(ime_wnd or 0))
    if not ime_wnd:
        out.emit(event="abort", reason="no default IME window")
        return 5

    def state():
        b = dm.read_block(tid, pid)
        return None if not b or "eff_open" not in b else b["eff_open"]

    def host_has_focus():
        return dm.focus_info()[2] in pids

    time.sleep(2.0)
    for r in range(args.rounds):
        # 各周の頭で、ホスト窓にフォーカスを戻す（実験中にほかの窓へフォーカスが移ることがある。フォーカスの変化は
        # 明示意図を消すので、フォーカスが保たれた周だけを集計する）。
        if not ds.focus_text(pids)[0]:
            out.emit(event="cycle_skipped", round=r, reason="could not focus the host")
            time.sleep(2.0)
            continue
        time.sleep(0.5)
        if not state() == 1:
            ensure_on(pids, out)
        focus_start = host_has_focus()
        s0 = state()
        send_marked(0x19)  # 漢字キー(物理扱い): トグル
        time.sleep(1.5)
        s1 = state()
        send_marked(0x19)  # もう一度: 元の状態(開)へ。明示意図が残る
        time.sleep(float(args.settle))  # 追随の tick が止まる時間を与える
        s2 = state()
        focus_before_ext = host_has_focus()
        t_ext = time.time_ns()
        ok, res, t0, t1 = imm_set_open(ime_wnd, 0)  # 外から閉じる(ツールバーの操作の代わり)
        deadline = time.time() + 5.0
        s3 = None
        while time.time() < deadline:
            s3 = state()
            if s3 == 0:
                break
            time.sleep(0.01)
        time.sleep(4.0)  # awase が追随する時間
        focus_after = host_has_focus()
        out.emit(event="cycle", round=r, t_ext_utc=dm.utc_iso(t_ext), send_ok=bool(ok), shm_before=s0, shm_after_key1=s1,
                 shm_before_ext=s2, shm_after_ext=s3, focus_start=focus_start, focus_before_ext=focus_before_ext,
                 focus_after=focus_after)
        imm_set_open(ime_wnd, 1)  # 次の周のために外から開ける(awase はこれにも追随する)
        time.sleep(2.0)
    out.emit(event="done", scenario="e5")
    return 0


def imm_set_open(ime_wnd, open_):
    res = ctypes.c_size_t(0)
    t0 = time.perf_counter_ns()
    ok = u32.SendMessageTimeoutW(ime_wnd, WM_IME_CONTROL, IMC_SETOPENSTATUS, 1 if open_ else 0, 2, 1000, ctypes.byref(res))
    return ok, res.value, t0, time.perf_counter_ns()


def imm_get_open(ime_wnd):
    res = ctypes.c_size_t(0)
    ok = u32.SendMessageTimeoutW(ime_wnd, WM_IME_CONTROL, IMC_GETOPENSTATUS, 0, 2, 1000, ctypes.byref(res))
    return res.value if ok else None


def typed_char_kind(hwnd, before_len):
    """What did the last typed key produce? Reads the control text but NEVER stores it: only the length change and
    the class of the last character ('hiragana' / 'ascii' / 'other') are returned (the text may be a user's document)."""
    n = u32.SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, None)
    if n < 0 or n > 200000:
        return None
    buf = ctypes.create_unicode_buffer(n + 2)
    u32.SendMessageW(hwnd, WM_GETTEXT, n + 1, ctypes.cast(buf, ctypes.c_void_p))
    last = buf.value[-1:] if buf.value else ""
    del buf
    kind = "none" if not last else "hiragana" if "\u3040" <= last <= "\u309f" else "ascii" if last.isascii() else "other"
    return {"len_delta": n - before_len, "last_kind": kind}


def text_len(hwnd):
    return u32.SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, None)


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
            getopen = imm_get_open(ime_wnd)
            typed = None
            if r < 3:  # output check: type 'a'; open -> hiragana, closed -> latin 'a'. Only the length change and the kind are kept.
                fh, _, _, _, _ = dm.focus_info()
                n0 = text_len(fh)
                dm.send_vk(0x41)
                time.sleep(0.5)
                typed = typed_char_kind(fh, n0)
            out.emit(event="imm_write", round=r, want_open=want, send_ok=bool(ok), result=res,
                     before=before, after=blk, changed=blk is not None, imm_getopen_after=getopen, typed=typed,
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


def click_at(h, fx, fy):
    """Click at fractions (fx, fy) of the window rectangle (the host window: text edit ~0.28, password edit ~0.58)."""
    r = wt.RECT()
    u32.GetWindowRect(h, ctypes.byref(r))
    u32.SetCursorPos(r.left + int((r.right - r.left) * fx), r.top + int((r.bottom - r.top) * fy))
    u32.mouse_event(2, 0, 0, 0, 0)
    u32.mouse_event(4, 0, 0, 0, 0)


def e4(args, out):
    """Win32 edit controls: a normal EDIT and an ES_PASSWORD EDIT in the same window (NOT a web password field).
    For each field and for the IME on/off, record the block and what typing 'a' produces (kind only, never the text)."""
    pids = {int(x) for x in args.pids.split(",") if x}
    tt = text_thread(pids)
    if not tt:
        out.emit(event="abort", reason="no text focus")
        return 2
    h, tid, pid, cls = tt
    if not ensure_on(pids, out):
        out.emit(event="abort", reason="could not turn the IME on")
        return 4
    out.emit(event="ready", tid=tid, pid=pid, cls=cls)
    for rep in range(args.rounds):
        for field, fy in (("text", 0.28), ("password", 0.58)):
            click_at(h, 0.5, fy)
            time.sleep(0.8)
            fh, ftid, fpid, fcls, how = dm.focus_info()
            for vk, label in ((0xF2, "ime-on"), (0x19, "ime-off")):
                dm.send_vk(vk)
                time.sleep(1.2)
                blk = dm.read_block(ftid, fpid) if fpid in pids else None
                n0 = text_len(fh)
                dm.send_vk(0x41)
                time.sleep(0.5)
                typed = typed_char_kind(fh, n0)
                out.emit(event="field", rep=rep, field=field, ime=label, focus_class=fcls, focus_tid=ftid, block=blk, typed=typed)
    out.emit(event="done", scenario="e4")
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("exp", choices=["e1", "e1watch", "e2", "e3", "e4", "e5"])
    ap.add_argument("--pids", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--rounds", type=int, default=10)
    ap.add_argument("--load", type=int, default=0)
    ap.add_argument("--settle", type=float, default=6.0)
    args = ap.parse_args()
    out = Out(args.out)
    out.emit(event="start", exp=args.exp)
    sys.exit({"e1": e1, "e1watch": e1watch, "e2": e2, "e3": e3, "e4": e4, "e5": e5}[args.exp](args, out))


if __name__ == "__main__":
    main()
