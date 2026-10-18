"""ADR-265 stage 2, physical-key measurement: how often was awase's belief already right when awase read a new shared state?

usage: analyze_physical.py <dir with physical.jsonl and awase_measure.log>
No pass/fail criterion: it reports counts and rates. Only the OPEN state is comparable: awase's belief_mode is the input method
(e.g. ObservedRomaji), not the IME conversion mode (hiragana/katakana).

For every change of the shared block's (open) state seen by the driver (ground truth: the same block awase reads):
  * the awase record that first reports the new state (accepted, same process, within [t-0.1 s, t+3 s]) carries the belief at the
    end of the tick in which awase read it. belief_open == new open  -> "belief already right";  != -> "belief wrong" (the
    shared state would have corrected it);  no such record -> "awase did not report it within 3 s".
  * a change is "caused by a key" when a PHYSICAL IME-mode key went down within 1.0 s before it (injected keys are awase's own
    re-injection of the same key, counted separately).
"""
import json
import os
import sys
from collections import Counter
from datetime import datetime, timedelta, timezone

WAIT_S = 3.0
CAUSE_S = 1.0


def ts(s):
    s = s.rstrip("Z")
    if "." in s:
        a, b = s.split(".")
        s = a + "." + b[:6]
    return datetime.fromisoformat(s).replace(tzinfo=timezone.utc)


def main(d):
    ev = [json.loads(l) for l in open(os.path.join(d, "physical.jsonl"), encoding="utf-8")]
    keys = [(ts(e["t_utc"]), e) for e in ev if e["event"] == "key"]
    downs = [(t, e) for t, e in keys if e["down"] and not e["injected"]]  # PHYSICAL presses only
    inj_downs = [(t, e) for t, e in keys if e["down"] and e["injected"]]  # awase's own re-injection of the same key
    shm = [(ts(e["t_utc"]), e) for e in ev if e["event"] == "shm" and e.get("in_target") and e.get("block") and "eff_open" in e["block"]]
    print(f"events: {len(ev)}  key records: {len(keys)}  PHYSICAL key downs: {len(downs)}  injected key downs (not counted as causes): "
          f"{len(inj_downs)}  shm observations: {len(shm)}")
    print("  physical key downs by vk:", dict(Counter(e["vk"] for _, e in downs)))
    print("  injected key downs by vk:", dict(Counter(e["vk"] for _, e in inj_downs)))
    if len(shm) < 2:
        print("RESULT: NO DATA (fewer than 2 shared-state observations)")
        return
    pid = shm[0][1]["focus_pid"]
    awase = []
    for line in open(os.path.join(d, "awase_measure.log"), encoding="utf-8", errors="replace"):
        if "[mozc-shm] {" in line:
            j = json.loads(line.split("[mozc-shm] ", 1)[1])
            if j["process_id"] == pid and j["verdict"] == "accepted":
                awase.append((ts(line.split()[0]), j))
    print(f"awase accepted records for pid {pid}: {len(awase)}")
    opens = modes_only = uncaused = 0
    right = wrong = missed = 0
    lat = []
    wrong_cases = []
    prev = None
    for t, e in shm:
        b = e["block"]
        cur = (b["eff_open"], b["eff_mode"])
        if prev is None:
            prev = cur
            continue
        if cur == prev:
            continue
        caused = [k for tk, k in downs if timedelta(0) <= t - tk <= timedelta(seconds=CAUSE_S)]
        if not caused:
            uncaused += 1
        if cur[0] != prev[0]:
            opens += 1
            want = bool(cur[0])
            hit = None
            for ta, j in awase:
                if t - timedelta(seconds=0.1) <= ta <= t + timedelta(seconds=WAIT_S) and bool(j["effective_open"]) == want:
                    hit = (ta, j)
                    break
            if hit is None:
                missed += 1
            else:
                ta, j = hit
                lat.append((ta - t).total_seconds())
                if j.get("belief_open") is None:
                    missed += 1
                elif bool(j["belief_open"]) == want:
                    right += 1
                else:
                    wrong += 1
                    wrong_cases.append((t, cur, j.get("belief_open"), caused[-1]["vk"] if caused else None))
        else:
            modes_only += 1
        prev = cur
    print(f"shared-state changes: open changes={opens}, mode-only changes={modes_only} (belief has no comparable field), "
          f"changes with no PHYSICAL IME-mode key in the previous {CAUSE_S}s={uncaused}")
    print(f"open changes: belief already RIGHT when awase read the new state: {right}   belief WRONG: {wrong}   "
          f"not reported by awase within {WAIT_S}s / no belief value: {missed}")
    if lat:
        lat.sort()
        print(f"shm change -> awase's record (s): n={len(lat)} min={lat[0]:.2f} median={lat[len(lat)//2]:.2f} max={lat[-1]:.2f}")
    for t, cur, b, vk in wrong_cases[:8]:
        print(f"   WRONG {t:%H:%M:%S.%f} shared open={bool(cur[0])} belief_open={b} (key before: {vk})")
    print("RESULT: counts only (no criterion). benefit of adopting the shared state = the WRONG count above.")


main(sys.argv[1])
