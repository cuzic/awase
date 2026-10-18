"""ADR-265 stage 2: compare awase's [mozc-shm] records with the driver's operation-derived expectations.

Per accepted awase record (verdict=accepted) inside a drive span:
  allowed states = the expectation of the step whose window contains the record's time; during the first 0.8 s
  after a key the previous step's expectation is also allowed (the shared block changes ~61 ms after the key but
  awase only reads about every 500 ms and records changes).
A record is a MISMATCH if its (open, mode) is not in the allowed set. It is a WRONG-THREAD record if its thread_id is
not the focus thread the driver observed for that step (GetGUIThreadInfo, same rule as awase).
belief_* columns are ignored (injected keys do not move awase's belief).
usage: analyze.py <dir with drive_*.jsonl and awase_measure.log>
"""
import json, sys, os, collections
from datetime import datetime, timedelta, timezone

MODE = {"direct": 0, "hiragana": 1, "full_katakana": 2, "half_alnum": 3, "half_ascii": 3, "full_alnum": 4,
        "full_ascii": 4, "half_katakana": 5}
GRACE = 0.8


def ts(s):
    s = s.rstrip("Z")
    if "." in s:
        a, b = s.split(".")
        s = a + "." + b[:6]
    return datetime.fromisoformat(s).replace(tzinfo=timezone.utc)


def load_awase(path):
    out = []
    for line in open(path, encoding="utf-8", errors="replace"):
        if "[mozc-shm] {" not in line:
            continue
        t = ts(line.split()[0])
        out.append((t, json.loads(line.split("[mozc-shm] ", 1)[1])))
    return out


def load_drive(path):
    steps, start, done = [], None, None
    for line in open(path, encoding="utf-8"):
        j = json.loads(line)
        if j["event"] == "step":
            steps.append(j)
        elif j["event"] == "start":
            start = j
        elif j["event"] == "done":
            done = j
    return start, steps, done


def mode_num(name):
    if isinstance(name, int):
        return name
    return MODE.get(str(name).lower().replace("-", "_"), -1)


def main(d):
    awase = load_awase(os.path.join(d, "awase_measure.log"))
    print(f"awase [mozc-shm] records in the whole log: {len(awase)}")
    total_acc = total_mis = total_wrong_thread = 0
    names = collections.Counter()
    unknown_names = collections.Counter()
    for label in ("notepad", "chrome"):
        p = os.path.join(d, f"drive_{label}.jsonl")
        if not os.path.exists(p):
            print(f"[{label}] no drive file")
            continue
        start, steps, done = load_drive(p)
        if not steps:
            print(f"[{label}] no steps (drive aborted)")
            continue
        pid = steps[0]["focus_pid"]
        t0 = ts(steps[0]["t_send_utc"])
        t_end = ts(steps[-1]["t_send_utc"]) + timedelta(seconds=2.0)
        win = [(ts(s["t_send_utc"]), s) for s in steps]
        anomalies = [s for s in steps if not s["driver_matches_expected"]]
        recs = [(t, j) for t, j in awase if t0 <= t <= t_end and j["process_id"] == pid]
        by_verdict = collections.Counter((j["verdict"], j.get("reason")) for _, j in recs)
        threads = collections.Counter(j["thread_id"] for _, j in recs)
        acc = [(t, j) for t, j in recs if j["verdict"] == "accepted"]
        mismatches, wrong_thread = [], []
        for t, j in acc:
            for k in range(len(win) - 1, -1, -1):
                if win[k][0] <= t:
                    break
            tk, sk = win[k]
            prev = win[k - 1][1] if k > 0 else None
            allowed = [(sk["expected_open"], sk["expected_mode"])]
            if (t - tk).total_seconds() < GRACE:
                allowed.append((prev["expected_open"], prev["expected_mode"]) if prev else (0, None))
            open_v = 1 if j["effective_open"] else 0
            mode_v = mode_num(j["effective_mode"])
            if mode_v == -1:
                unknown_names[str(j["effective_mode"])] += 1
            names[str(j["effective_mode"])] += 1
            ok = any(eo == open_v and (em is None or em == mode_v) for eo, em in allowed)
            if not ok:
                mismatches.append((t, j, sk, allowed))
            if j["thread_id"] != sk["focus_tid"]:
                wrong_thread.append((t, j, sk))
        total_acc += len(acc)
        total_mis += len(mismatches)
        total_wrong_thread += len(wrong_thread)
        print(f"\n[{label}] pid={pid} focus_tid(driver)={sorted({s['focus_tid'] for s in steps})} steps={len(steps)} "
              f"driver_vs_expected anomalies={len(anomalies)}")
        print(f"  awase records for this pid in the span: {len(recs)}  by (verdict, reason): {dict(by_verdict)}")
        print(f"  awase read threads (thread_id: records): {dict(threads)}")
        print(f"  accepted records: {len(acc)}   MISMATCH vs expectation: {len(mismatches)}   WRONG-THREAD: {len(wrong_thread)}")
        for t, j, sk, allowed in mismatches[:6]:
            print(f"   MISMATCH {t.strftime('%H:%M:%S.%f')[:-3]} awase=(open={j['effective_open']},mode={j['effective_mode']}) "
                  f"allowed={allowed} step=c{sk['cycle']}s{sk['step']} tid={j['thread_id']}")
        for t, j, sk in wrong_thread[:6]:
            print(f"   WRONG-THREAD {t.strftime('%H:%M:%S.%f')[:-3]} awase tid={j['thread_id']} driver focus_tid={sk['focus_tid']}")
        rd = sorted(j["read_us"] for _, j in acc)
        if rd:
            print(f"  read_us (accepted): min={rd[0]} median={rd[len(rd)//2]} p95={rd[int(len(rd)*0.95)]} max={rd[-1]}")
        # steps where the last awase record at the end of the step window is not accepted (awase had no adopted state)
        gaps = 0
        for k, (tk, sk) in enumerate(win):
            te = win[k + 1][0] if k + 1 < len(win) else tk + timedelta(seconds=2)
            last = [j for t, j in recs if t <= te]
            if not last or last[-1]["verdict"] != "accepted":
                gaps += 1
        print(f"  steps whose end-of-step awase state was not 'accepted' (not adopted): {gaps}/{len(steps)}")
    print(f"\nTOTAL accepted records: {total_acc}   MISMATCH: {total_mis}   WRONG-THREAD: {total_wrong_thread}")
    print(f"mode names seen: {dict(names)}   UNKNOWN names (would be counted as mismatches): {dict(unknown_names)}")
    print("CRITERION (N>=200 accepted, mismatches == 0): " + ("MET" if total_acc >= 200 and total_mis == 0 else "NOT MET"))


main(sys.argv[1])
