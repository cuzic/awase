"""ADR-265 stage 2, additional measurement: analysis of the dangerous situations (Linux ok).

usage: analyze_scenarios.py <dir with scen_focus.jsonl / scen_away.jsonl and awase_measure.log>

What counts as a wrong adoption is fixed here BEFORE the measurement:

focus scenario (two windows with different, constant IME states):
  * MISMATCH  : an accepted record whose (open, mode) differs from the expected state of the window named by the
                record's (process_id, thread_id). A window mix-up shows up here.
  * UNKNOWN-THREAD : an accepted record for one of our processes but a thread the driver never saw as focus thread.
  * STALE     : the record names a window that lost the focus more than STALE_S seconds earlier.
  Reported, not failures: the lag (time from a switch to the first accepted record of the newly focused window).
away scenario (input method switched away from the TIP and back):
  * STALE-ADOPTION : an accepted record with open=true in [t_away + LAG_S, t_back) -- the TIP was inactive
    (active=0), so any adopted open state is a leftover.
"""
import json
import os
import sys
from collections import Counter
from datetime import datetime, timedelta, timezone

STALE_S = 1.0
LAG_S = 1.0
MODE = {"direct": 0, "hiragana": 1, "full_katakana": 2, "half_alnum": 3, "half_ascii": 3, "full_alnum": 4,
        "full_ascii": 4, "half_katakana": 5}


def ts(s):
    s = s.rstrip("Z")
    if "." in s:
        a, b = s.split(".")
        s = a + "." + b[:6]
    return datetime.fromisoformat(s).replace(tzinfo=timezone.utc)


def mode_num(name):
    return MODE.get(str(name).lower().replace("-", "_"), -1)


def load_awase(path):
    out = []
    for line in open(path, encoding="utf-8", errors="replace"):
        if "[mozc-shm] {" in line:
            out.append((ts(line.split()[0]), json.loads(line.split("[mozc-shm] ", 1)[1])))
    return out


def load_jsonl(path):
    return [json.loads(line) for line in open(path, encoding="utf-8")] if os.path.exists(path) else []


def state_ok(rec, expected):
    eo, em = expected
    return (1 if rec["effective_open"] else 0) == eo and (em is None or mode_num(rec["effective_mode"]) == em)


def focus(d, awase):
    ev = load_jsonl(os.path.join(d, "scen_focus.jsonl"))
    sw = [e for e in ev if e["event"] == "switch"]
    if not sw:
        print("[focus] no switch events (the scenario aborted):", [e for e in ev if e["event"] in ("abort", "setup")][-3:])
        return 0, 0
    ident = {}  # target -> (pid, tid), expected
    for name in ("notepad", "chrome"):
        c = Counter((e["focus_pid"], e["focus_tid"]) for e in sw if e["target"] == name and e["focus_in_target"])
        ident[name] = c.most_common(1)[0][0] if c else None
    exp = {e["target"]: (e["expected_open"], e["expected_mode"]) for e in sw}
    by_key = {v: k for k, v in ident.items() if v}
    print(f"[focus] windows: {ident}  expected: {exp}  switches: {len(sw)} "
          f"(driver-vs-expected anomalies: {sum(1 for e in sw if not e['driver_matches_expected'])}, "
          f"focus not in target: {sum(1 for e in sw if not e['focus_in_target'])})")
    t0 = ts(sw[0]["t_switch_utc"])
    t1 = ts(sw[-1]["t_switch_utc"]) + timedelta(seconds=3)
    pids = {p for p, _ in by_key}
    recs = [(t, j) for t, j in awase if t0 <= t <= t1 and j["process_id"] in pids]
    times = [(ts(e["t_switch_utc"]), e["target"]) for e in sw]

    def focused_at(t):
        cur = None
        for tt, name in times:
            if tt <= t:
                cur = name
        return cur

    def lost_focus_at(name, t):
        """When did `name` last lose the focus before t (None if it still has it)."""
        cur, since = None, None
        for tt, n in times:
            if tt > t:
                break
            if n == name:
                cur, since = name, None
            elif cur == name:
                since = tt
                cur = None
        return since

    acc = [(t, j) for t, j in recs if j["verdict"] == "accepted"]
    mism, unknown, stale = [], [], []
    for t, j in acc:
        key = (j["process_id"], j["thread_id"])
        name = by_key.get(key)
        if name is None:
            unknown.append((t, j))
            continue
        if not state_ok(j, exp[name]):
            mism.append((t, j, name))
        if focused_at(t) != name:
            lost = lost_focus_at(name, t)
            if lost is not None and (t - lost).total_seconds() > STALE_S:
                stale.append((t, j, name, (t - lost).total_seconds()))
    # lag: from each switch to the first accepted record of the newly focused window
    lags = []
    for i, (tt, name) in enumerate(times):
        key = ident.get(name)
        nxt = times[i + 1][0] if i + 1 < len(times) else t1
        first = [t for t, j in acc if key == (j["process_id"], j["thread_id"]) and tt <= t < nxt]
        if first:
            lags.append((first[0] - tt).total_seconds())
    print(f"[focus] awase records for our processes: {len(recs)}  by verdict: {dict(Counter(j['verdict'] for _, j in recs))}")
    print(f"[focus] accepted: {len(acc)}  MISMATCH: {len(mism)}  UNKNOWN-THREAD: {len(unknown)}  STALE(>{STALE_S}s): {len(stale)}")
    for t, j, name in mism[:5]:
        print(f"   MISMATCH {t:%H:%M:%S.%f} names {name} but state=(open={j['effective_open']},mode={j['effective_mode']}) expected={exp[name]}")
    for t, j in unknown[:5]:
        print(f"   UNKNOWN-THREAD {t:%H:%M:%S.%f} pid={j['process_id']} tid={j['thread_id']}")
    for t, j, name, age in stale[:5]:
        print(f"   STALE {t:%H:%M:%S.%f} names {name} which lost focus {age:.2f}s earlier")
    if lags:
        lags.sort()
        print(f"[focus] switch->first accepted record of the new window (s): n={len(lags)} min={lags[0]:.2f} "
              f"median={lags[len(lags)//2]:.2f} p95={lags[int(len(lags)*0.95)]:.2f} max={lags[-1]:.2f}  "
              f"(switches with no accepted record before the next switch: {len(times) - len(lags)})")
    return len(acc), len(mism) + len(unknown) + len(stale)


def away(d, awase):
    ev = load_jsonl(os.path.join(d, "scen_away.jsonl"))
    aw = [e for e in ev if e["event"] == "away"]
    bk = [e for e in ev if e["event"] == "back"]
    if not aw:
        print("[away] no events (the scenario aborted):", [e for e in ev if e["event"] in ("abort", "setup")][-3:])
        return 0, 0
    pid = aw[0]["focus_pid"]
    print(f"[away] rounds: {len(aw)}  pid={pid}  time_to_inactive_ms={[e['time_to_inactive_ms'] for e in aw][:8]}  "
          f"time_to_active_ms={[e['time_to_active_ms'] for e in bk][:8]}")
    stale_total = acc_total = 0
    for r, e in enumerate(aw):
        t_away = ts(e["t_away_utc"])
        t_back = ts(bk[r]["t_back_utc"]) if r < len(bk) else t_away + timedelta(seconds=4)
        span = [(t, j) for t, j in awase if t_away + timedelta(seconds=LAG_S) <= t < t_back and j["process_id"] == pid]
        acc = [(t, j) for t, j in span if j["verdict"] == "accepted"]
        bad = [(t, j) for t, j in acc if j["effective_open"]]
        verdicts = Counter((j["verdict"], j.get("reason")) for _, j in span)
        acc_total += len(acc)
        stale_total += len(bad)
        print(f"  round {r}: records while the TIP was inactive (after {LAG_S}s grace): {dict(verdicts)}  "
              f"STALE-ADOPTION(open adopted while inactive): {len(bad)}")
        for t, j in bad[:2]:
            print(f"     STALE-ADOPTION {t:%H:%M:%S.%f} open={j['effective_open']} mode={j['effective_mode']} tid={j['thread_id']}")
    print(f"[away] STALE-ADOPTION total: {stale_total}  (accepted records in the inactive spans: {acc_total})")
    return len(aw), stale_total


def main(d):
    awase = load_awase(os.path.join(d, "awase_measure.log"))
    print(f"awase [mozc-shm] records in the log: {len(awase)}")
    n1, bad1 = focus(d, awase)
    n2, bad2 = away(d, awase)
    print(f"\nTOTAL focus: accepted={n1} wrong={bad1}   away: rounds={n2} stale adoptions={bad2}")
    print("RESULT: " + ("NO wrong adoption found" if (bad1 + bad2) == 0 and (n1 > 0 or n2 > 0) else "WRONG ADOPTION(S) or no data"))


main(sys.argv[1])
