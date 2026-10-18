"""ADR-265 stage 2, additional measurement: the dangerous situations (Windows only, standard library only).

Reuses drive_measure.py (same directory). Targets are selected by PID (never by window class).

  --scenario focus : two windows (Notepad and a plain Win32 test window, ui_host.py from mozc-readable) hold DIFFERENT IME states. Focus is switched
                     between them (slowly, then quickly). awase must report the state of the window that really has the
                     focus; right after a switch a short lag is tolerated and measured.
  --scenario away  : in one window the IME is on; the input method is switched to a layout without the TIP (Win+Space,
                     the ja list holds only Mozc Readable, so it toggles with en-US) and back. While the TIP is not active the
                     shared block says active=0 and awase must NOT adopt the leftover open state.

Output: JSON Lines (every event has t_utc). awase's own readings come from awase.log ([mozc-shm] lines) and are compared
by analyze_scenarios.py. Not verified on a Windows machine yet (syntax-checked on Linux only).
"""
import argparse
import json
import sys
import time

import drive_measure as dm

u32 = dm.u32
SETTLE_S = 1.5
VK_LWIN, VK_SPACE = 0x5B, 0x20


def key_event(vk, flags):
    i = dm.INPUT(type=1)
    i.ki = dm.KEYBDINPUT(vk, 0, flags, 0, 0)
    u32.SendInput(1, dm.ctypes.byref(i), dm.ctypes.sizeof(dm.INPUT))


def win_space():
    """Win+Space: switch to the next input method (toggles Mozc Readable <-> en-US when only those two exist)."""
    key_event(VK_LWIN, 0)
    time.sleep(0.05)
    key_event(VK_SPACE, 0)
    time.sleep(0.05)
    key_event(VK_SPACE, 2)
    time.sleep(0.05)
    key_event(VK_LWIN, 2)


def goto(pids, attempts=5):
    """Bring the biggest window of `pids` to the foreground and VERIFY it. Returns hwnd or None.

    Windows refuses to hand the foreground to a process that does not hold it (SetForegroundWindow fails
    silently), so retry: plain, plain, tap Alt (unlocks it), plain, minimise+restore."""
    ws = dm.windows_of(pids)
    if not ws:
        return None
    h = ws[0]
    for i in range(attempts):
        dm.force_foreground(h)
        time.sleep(0.25)
        if dm.fg_info()[2] in pids:
            return h
        if i == 1:
            key_event(0x12, 0)  # VK_MENU down/up: makes the next SetForegroundWindow succeed
            key_event(0x12, 2)
        elif i == 3:
            u32.ShowWindow(h, 6)  # SW_MINIMIZE
            time.sleep(0.2)
            u32.ShowWindow(h, 9)  # SW_RESTORE
    return None


def focus_text(pids, tries=4):
    """Like a person after switching windows: bring the window forward, then click into the text area until the FOCUS
    thread has a TIP block (a thread without a block, e.g. modern Notepad's frame / XAML input site, cannot take IME
    keys). A click can also move the focus away from the text area (it toggles between the two threads), so check
    after every click. Returns (hwnd, clicks) or (None, clicks)."""
    h = goto(pids)
    if not h:
        return None, 0
    clicks = 0
    for _ in range(tries + 1):
        _, ftid, fpid, _, _ = dm.focus_info()
        if fpid in pids and dm.read_block(ftid, fpid) is not None:
            return h, clicks
        if clicks >= tries:
            break
        dm.click_inside(h)
        clicks += 1
        time.sleep(0.5)
    return None, clicks


def drive_to(pids, vk, eo, em, emit, what, tries=4):
    for attempt in range(1, tries + 1):
        dm.send_vk(vk)
        deadline = time.time() + 8
        while time.time() < deadline:
            _, ftid, fpid, _, _ = dm.focus_info()
            if fpid not in pids:
                emit(event="setup", step=what, ok=False, reason="focus left the target", focus_pid=fpid)
                return False
            if dm.matches(dm.read_block(ftid, fpid), eo, em):
                emit(event="setup", step=what, attempt=attempt, ok=True)
                return True
            time.sleep(0.05)
    emit(event="setup", step=what, ok=False, reason="state did not reach the expectation")
    return False


def snapshot():
    _, ftid, fpid, fcls, how = dm.focus_info()
    return ftid, fpid, fcls, how, (dm.read_block(ftid, fpid) if fpid else None)


def scenario_focus(args, emit):
    np_pids = {int(x) for x in args.notepad_pids.split(",") if x}
    second_pids = {int(x) for x in args.second_pids.split(",") if x}
    wins = {"notepad": np_pids, "second": second_pids}
    # setup: Notepad = full katakana (1,2); the second window = IME off (0,*)  -> the two windows hold different states
    for name, setup in (("notepad", [(0xF2, 1, 1), (0xF1, 1, 2)]), ("second", [(0xF2, 1, 1), (0x19, 0, None)])):
        h, clicks = focus_text(wins[name])
        if not h:
            emit(event="abort", reason=f"no text focus for {name}", clicks=clicks)
            return 2
        time.sleep(1.0)
        for vk, eo, em in setup:
            if not drive_to(wins[name], vk, eo, em, emit, f"{name}:{vk:#04x}", tries=8):
                return 4
    expected = {"notepad": (1, 2), "second": (0, None)}
    emit(event="ready", expected={k: list(v) for k, v in expected.items()})
    n = 0
    failed = 0
    # phase 1: slow alternation (settle 1.5 s each) ; phase 2: quick alternation (0.15 s apart, then settle)
    plan = [("slow", args.rounds, SETTLE_S)] + [("quick", args.rounds, 0.15)]
    for phase, rounds, gap in plan:
        for r in range(rounds):
            for name in ("notepad", "second"):
                t_send = time.time_ns()
                h, clicks = focus_text(wins[name])
                if not h:
                    emit(event="switch_failed", phase=phase, round=r, target=name,
                         t_switch_utc=dm.utc_iso(t_send), fg_pid=dm.fg_info()[2], clicks=clicks)
                    failed += 1
                    if failed > 10:
                        emit(event="abort", reason="too many failed switches")
                        return 5
                    continue
                n += 1
                time.sleep(gap)
                ftid, fpid, fcls, how, blk = snapshot()
                emit(event="switch", phase=phase, seq_no=n, round=r, target=name, t_switch_utc=dm.utc_iso(t_send),
                     clicks=clicks,
                     expected_open=expected[name][0], expected_mode=expected[name][1], focus_tid=ftid, focus_pid=fpid,
                     focus_class=fcls, focus_from=how, focus_in_target=fpid in wins[name], driver_block=blk,
                     driver_matches_expected=dm.matches(blk, *expected[name]))
        time.sleep(SETTLE_S)  # let awase's last tick land before the next phase / the end
    emit(event="done", scenario="focus", switches=n, failed_switches=failed)
    return 0


def scenario_away(args, emit):
    pids = {int(x) for x in args.notepad_pids.split(",") if x}
    h, clicks = focus_text(pids)
    if not h:
        emit(event="abort", reason="no text focus", clicks=clicks)
        return 2
    time.sleep(1.0)
    if not drive_to(pids, 0xF2, 1, 1, emit, "on"):
        return 4
    time.sleep(1.5)
    for r in range(args.rounds):
        # --- switch away from the TIP
        t_away = time.time_ns()
        win_space()
        t_inactive = None
        deadline = time.time() + 8
        blk = None
        while time.time() < deadline:
            _, ftid, fpid, _, _ = dm.focus_info()
            blk = dm.read_block(ftid, fpid) if fpid in pids else None
            if blk is None or blk.get("rejected"):  # no block for the (new) layout, or active=0
                t_inactive = time.time_ns()
                break
            time.sleep(0.03)
        ftid, fpid, fcls, how, blk2 = snapshot()
        emit(event="away", round=r, t_away_utc=dm.utc_iso(t_away),
             time_to_inactive_ms=None if t_inactive is None else (t_inactive - t_away) // 1_000_000,
             focus_tid=ftid, focus_pid=fpid, driver_block=blk2)
        time.sleep(3.0)  # observe: awase must not adopt the leftover open state while the TIP is inactive
        # --- and back
        t_back = time.time_ns()
        win_space()
        t_active = None
        deadline = time.time() + 8
        while time.time() < deadline:
            _, ftid, fpid, _, _ = dm.focus_info()
            blk = dm.read_block(ftid, fpid) if fpid in pids else None
            if blk and "eff_open" in blk:
                t_active = time.time_ns()
                break
            time.sleep(0.03)
        ftid, fpid, fcls, how, blk3 = snapshot()
        emit(event="back", round=r, t_back_utc=dm.utc_iso(t_back),
             time_to_active_ms=None if t_active is None else (t_active - t_back) // 1_000_000,
             focus_tid=ftid, focus_pid=fpid, driver_block=blk3)
        time.sleep(2.0)
        if t_inactive is None or t_active is None:
            emit(event="abort", reason="the input method switch did not take effect", round=r)
            return 6
    emit(event="done", scenario="away", rounds=args.rounds)
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--scenario", choices=["focus", "away"], required=True)
    ap.add_argument("--notepad-pids", required=True)
    ap.add_argument("--second-pids", default="")
    ap.add_argument("--rounds", type=int, default=25)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    out = open(args.out, "w", encoding="utf-8")

    def emit(**kw):
        kw["t_utc"] = dm.utc_iso(time.time_ns())
        out.write(json.dumps(kw, ensure_ascii=False) + "\n")
        out.flush()
        print(json.dumps(kw, ensure_ascii=False), flush=True)

    emit(event="start", scenario=args.scenario)
    sys.exit(scenario_focus(args, emit) if args.scenario == "focus" else scenario_away(args, emit))


if __name__ == "__main__":
    main()
