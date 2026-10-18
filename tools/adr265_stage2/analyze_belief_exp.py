"""ADR-265 belief experiments: analysis (Linux ok). usage: analyze_belief_exp.py e1|e2|e3|e4 <jsonl>

Criteria are fixed in docs/tasks/adr265-belief-experiments.md BEFORE the measurement."""
import json
import sys


def load(p):
    return [json.loads(line) for line in open(p, encoding="utf-8")]


def pct(v, q):
    v = sorted(v)
    return v[min(len(v) - 1, int(len(v) * q))] if v else None


def e1(ev):
    cues = [e for e in ev if e["event"] == "cue"]
    shm = [e for e in ev if e["event"] == "shm" and e["block"] and "eff_open" in e["block"]]
    if not cues:
        print("no cues:", [e for e in ev if e["event"] in ("abort", "ready")]); return
    ok = 0
    for i, c in enumerate(cues):
        t0 = c["t_ns"]
        t1 = cues[i + 1]["t_ns"] if i + 1 < len(cues) else t0 + 15_000_000_000
        win = [s for s in shm if t0 <= s["t_ns"] < t1]
        prev = [s for s in shm if s["t_ns"] < t0]
        base = prev[-1]["block"]["eff_open"] if prev else None
        changes = []
        last = base
        for s in win:
            o = s["block"]["eff_open"]
            if o != last:
                changes.append((o, (s["t_ns"] - t0) / 1e9)); last = o
        good = len(changes) == 1 and changes[0][0] == c["want_open"]
        ok += good
        print(f"step {c['step']:2d} want_open={c['want_open']} base={base} changes(open,s after cue)={changes} {'OK' if good else 'NG'}")
    print(f"E1: {ok}/{len(cues)} windows had exactly one change in the expected direction (criterion: all)")


def e2(ev):
    w = [e for e in ev if e["event"] == "imm_write"]
    if not w:
        print("no writes:", [e for e in ev if e["event"] in ("abort", "ready")]); return
    ch = [e for e in w if e["changed"] and e["after"].get("eff_open") == e["want_open"]]
    wrong = [e for e in w if e["changed"] and e["after"].get("eff_open") != e["want_open"]]
    same = [e for e in w if e["changed"] and "tsf_open" in e["after"] and e["after"]["tsf_open"] == e["after"]["eff_open"]]
    d = [e["delay_ms"] for e in w if e["changed"]]
    print(f"E2: writes={len(w)} reflected in the expected direction={len(ch)} wrong direction={len(wrong)} "
          f"not reflected={sum(1 for e in w if not e['changed'])} tsf_open==effective_open after change={len(same)}")
    print(f"    delay ms (send done -> seq change): p50={pct(d,.5)} p95={pct(d,.95)} max={max(d) if d else None}")
    print(f"    send_ok false={sum(1 for e in w if not e['send_ok'])}  (criterion: reflected = all)")
    for e in w:
        if not e["changed"] or e["after"].get("eff_open") != e["want_open"]:
            print("    NG round", e["round"], "want", e["want_open"], "before", e["before"], "after", e["after"])


def e3(ev):
    k = [e for e in ev if e["event"] == "key"]
    if not k:
        print("no keys:", [e for e in ev if e["event"] in ("abort", "ready")]); return
    load = k[0]["load"]
    for name, sel in (("from key down", "from_key_down_ms"), ("from send done", "from_send_done_ms")):
        v = [e[sel] for e in k if e["changed"]]
        print(f"E3 load={load} {name}: n={len(v)}/{len(k)} p50={pct(v,.5):.0f} p95={pct(v,.95):.0f} p99={pct(v,.99):.0f} max={max(v):.0f} ms"
              if v else f"E3 load={load} {name}: no change observed")
    wrong = [e for e in k if e["changed"] and e["eff_open"] != e["want_open"]]
    print(f"    wrong direction={len(wrong)}  not changed={sum(1 for e in k if not e['changed'])}  (170 ms = KEY_EFFECT_SETTLE_MS)")
    over = [e for e in k if e["changed"] and e["from_key_down_ms"] > 170]
    print(f"    changes later than 170 ms from key down: {len(over)}")


def e4(ev):
    f = [e for e in ev if e["event"] == "field"]
    if not f:
        print("no fields:", [e for e in ev if e["event"] in ("abort", "ready")]); return
    for e in f:
        b = e["block"] or {}
        print(f"field={e['field']:8s} ime={e['ime']:8s} eff=({b.get('eff_open')},{b.get('eff_mode')}) tsf=({b.get('tsf_open')},{b.get('tsf_mode')}) "
              f"title: {e['title_before']!r} -> {e['title_after']!r}")
    split = [e for e in f if e["block"] and "eff_open" in e["block"] and e["block"]["eff_open"] != e["block"]["tsf_open"]]
    print(f"E4: records where tsf_open != effective_open: {len(split)}/{len(f)}")


{"e1": e1, "e2": e2, "e3": e3, "e4": e4}[sys.argv[1]](load(sys.argv[2]))
