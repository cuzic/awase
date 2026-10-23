#!/usr/bin/env python3
"""ADR-279 段階 0(S0-1・S0-2)の集計。1 ジョブの typing_stress.log と awase.log を読む。

使い方: adr279_s0.py <result-dir>...   (各 dir に dist/typing_stress.log と dist/awase.log)
出力: 構成ごとの 1 行サマリと試行ごとの詳細(JSON 行)。
"""
import json
import re
import sys
from pathlib import Path

TS = re.compile(r"\[TS-JSON\]\s*(\{.*\})")
TIME = re.compile(r"T(\d\d:\d\d:\d\d\.\d{3})")
DIAG = re.compile(r"\[adr279-diag\] (.*)")
HS = re.compile(r"derive_any=Some\(HighSingle \{ source: (\w+), open: (\w+) \}\)")
MC = re.compile(r"derive_any=Some\(MediumConsensus \{ first: (\w+), second: ([^,]+), open: (\w+) \}\)")
KV = {k: re.compile(k + r"=(\S+)") for k in ("engine_active", "ime_on", "explicit", "effective", "drift_eval", "profile")}
STORE = re.compile(r"store_intent=(None|Some\(\((\w+), (\w+)\)\))")
BASE = re.compile(r"base: (\w+)")
ROLE = re.compile(r"\[adr279-role\] vk=0x(\w\w) ime=(\S+) configured=(\S+) role=(\S+) injected=(\w+)")
GIVEUP = "drift give-up diagnostic"


def recs(p):
    out = []
    for line in open(p, encoding="utf-8", errors="replace"):
        m = TS.search(line)
        if m:
            try:
                out.append(json.loads(m.group(1)))
            except ValueError:
                pass
    return out


def diag(p):
    samples, roles, giveups, fires = [], [], [], []
    for line in open(p, encoding="utf-8", errors="replace"):
        t = TIME.search(line[:40])
        if not t:
            continue
        ts = t.group(1)
        if GIVEUP in line:
            giveups.append((ts, line.strip()[-200:]))
        m = ROLE.search(line)
        if m:
            roles.append((ts, m.group(1), m.group(3), m.group(4), m.group(5)))
            continue
        m = DIAG.search(line)
        if not m:
            continue
        body = m.group(1)
        s = {"t": ts}
        for k, r in KV.items():
            mm = r.search(body)
            s[k] = mm.group(1) if mm else None
        mm = STORE.search(body)
        s["store"] = mm.group(1) if mm else None
        mm = BASE.search(body)
        s["base"] = mm.group(1) if mm else None
        mm = HS.search(body)
        mc = MC.search(body)
        if mm:
            s["derive"] = ("High", mm.group(1), mm.group(2) == "true")
        elif mc:
            s["derive"] = ("Medium", mc.group(1) + "+" + mc.group(2), mc.group(3) == "true")
        else:
            s["derive"] = None
        if (s["drift_eval"] or "").startswith("Fire"):
            fires.append(ts)
        samples.append(s)
    return samples, roles, giveups, fires


def between(samples, t0, t1):
    return [s for s in samples if t0 <= s["t"] <= t1]


def analyze(d: Path):
    tl, al = d / "dist" / "typing_stress.log", d / "dist" / "awase.log"
    if not tl.exists() or not al.exists():
        return None
    rs = recs(tl)
    samples, roles, giveups, fires = diag(al)
    by = {}
    for r in rs:
        if "n" in r and r.get("type", "").startswith("drift_on"):
            by.setdefault(r["n"], {}).setdefault(r["type"], []).append(r)
    trials = []
    for n, g in sorted(by.items()):
        reopen = (g.get("drift_on_reopen") or [{}])[0]
        close = (g.get("drift_on_close") or [{}])[0]
        checks = sorted(g.get("drift_on_check", []), key=lambda x: x["checkpoint_ms"])
        thumb = (g.get("drift_on_thumb") or [None])[0]
        typed = (g.get("drift_on_typed") or [{}])[0]
        t0 = reopen.get("utc") or close.get("utc")
        t_end = (thumb or {}).get("utc") or typed.get("press_utc")
        if not t0 or not t_end:
            continue
        gap_made = close.get("real_ime_open") is True
        win = between(samples, t0, t_end)
        # 3 秒以上続いたか: +3500 以降のチェックで実 IME が開いたまま、かつ同じ時刻の belief(effective)が閉
        late = [c for c in checks if c["checkpoint_ms"] >= 3500]
        persisted = None
        if gap_made and late:
            ok = True
            for c in late:
                ss = [s for s in win if s["t"] <= c.get("utc", "99")]
                eff = ss[-1]["effective"] if ss else None
                ok = ok and c.get("real_ime_open") is True and eff == "false"
            persisted = ok
        hs = [s for s in win if s["derive"] and s["derive"][0] == "High"]
        # 一致率: HighSingle の値と、その時刻以前で最も近いハーネスの読み(reopen 後の close 記録→チェック)
        reads = [(close.get("utc") or t0, close.get("real_ime_open"))] + [(c.get("utc"), c.get("real_ime_open")) for c in checks if c.get("utc")]
        agree = disagree = 0
        for s in hs:
            prev = [v for (tt, v) in reads if tt and tt <= s["t"] and v is not None]
            if prev:
                if prev[-1] == s["derive"][2]:
                    agree += 1
                else:
                    disagree += 1
        src = {}
        for s in hs:
            src[s["derive"][1]] = src.get(s["derive"][1], 0) + 1
        tr = {
            "n": n, "gap_made": gap_made, "reopen_open": reopen.get("real_ime_open"),
            "checks": {c["checkpoint_ms"]: c.get("real_ime_open") for c in checks},
            "persisted_3s": persisted, "samples": len(win),
            "high_frac": round(len(hs) / len(win), 3) if win else None, "high_src": src,
            "agree": agree, "disagree": disagree,
            "engine_active_frac": round(sum(s["engine_active"] == "true" for s in win) / len(win), 3) if win else None,
            "effective_true_frac": round(sum(s["effective"] == "true" for s in win) / len(win), 3) if win else None,
            "explicit_last": win[-1]["explicit"] if win else None, "store_last": win[-1]["store"] if win else None,
            "base_last": win[-1]["base"] if win else None, "profile": win[-1]["profile"] if win else None,
            "fires": sum(1 for f in fires if t0 <= f <= t_end),
            "typed_ok": typed.get("ok"), "typed_text": typed.get("text"), "typed_open": typed.get("real_ime_open"),
        }
        if thumb:
            after = between(samples, thumb["utc"], thumb.get("end_utc", "99"))
            last = after[-1] if after else {}
            tr["thumb"] = {
                "kind": thumb["thumb"], "before": thumb.get("before_open"), "o300": thumb.get("open_300"),
                "o1500": thumb.get("open_1500"), "text": thumb.get("text"),
                "explicit_after": last.get("explicit"), "store_after": last.get("store"),
                "engine_active_after": last.get("engine_active"), "effective_after": last.get("effective"),
                "derive_after": last.get("derive"),
                "roles": [r for r in roles if thumb["utc"] <= r[0] <= thumb.get("end_utc", "99")],
            }
        trials.append(tr)
    return {"trials": trials, "giveups": len(giveups), "fires_total": len(fires), "diag_samples": len(samples)}


def main(argv):
    for a in argv:
        d = Path(a)
        r = analyze(d)
        name = d.name
        if r is None:
            print(f"{name}: no logs")
            continue
        ts = r["trials"]
        g = [t for t in ts if t["gap_made"]]
        pers = [t for t in g if t["persisted_3s"]]
        agree = sum(t["agree"] for t in g)
        dis = sum(t["disagree"] for t in g)
        hf = [t["high_frac"] for t in g if t["high_frac"] is not None]
        src = {}
        for t in g:
            for k, v in t["high_src"].items():
                src[k] = src.get(k, 0) + v
        eng = [t["engine_active_frac"] for t in g if t["engine_active_frac"] is not None]
        line = (f"{name}: trials={len(ts)} gap_made={len(g)} persisted_3s={len(pers)} "
                f"high_frac_mean={round(sum(hf)/len(hf),3) if hf else None} high_src={src} agree={agree} disagree={dis} "
                f"engine_active_frac_mean={round(sum(eng)/len(eng),3) if eng else None} fires={sum(t['fires'] for t in g)} "
                f"giveup_diag={r['giveups']} diag_samples={r['diag_samples']} "
                f"typed_ok={sum(1 for t in g if t['typed_ok'])}/{len(g)}")
        th = [t["thumb"] for t in g if "thumb" in t]
        if th:
            line += (f" thumb={th[0]['kind']} o1500_open={sum(1 for x in th if x['o1500'] is True)}/{len(th)} "
                     f"explicit_after={sorted(set(str(x['explicit_after']) for x in th))} "
                     f"engine_after_true={sum(1 for x in th if x['engine_active_after']=='true')}/{len(th)} "
                     f"roles={sorted(set((r[1], r[3]) for x in th for r in x['roles']))}")
        print(line)
        for t in ts:
            print("  " + json.dumps(t, ensure_ascii=False))


if __name__ == "__main__":
    main(sys.argv[1:])
