#!/usr/bin/env python3
"""高速打鍵ストレス(ts-* 構成)の判定。`typing_stress` example のログ(`[TS-JSON]` 行)を読み、
確定文字列が期待文字列と一致するかを見て、崩れ方を分類する。awase 本体の挙動は読まない(入力先のテキストだけで判定)。

崩れ方の分類(1試行につき複数あり得る):
  loss       期待にあって実際に無い文字(消失)
  extra      期待に無いのに実際にある文字(余計な文字・重複)
  reorder    文字の集合は同じだが順序が違う(入れ替わり)
  literal    英字が残った(ローマ字のリテラル化。例 `ka` が `か` にならず `ka` のまま)
  substitute 別のかなに置き換わった(例: 親指シフトが効かず `が` が `か` になる)

使い方: check_typing_stress.py [--json out.json] <typing_stress.log>
終了コード: 0=全試行が一致 / 1=不一致あり(FAIL) / 3=INVALID(実行できなかった: 中断・試行0件・フォーカス喪失・注入の落ち) / 2=使い方の誤り
最終行が1行サマリ `TYPING_STRESS: verdict=... form=... ime=... mode=... interval_ms=... ...`。
"""
import difflib
import json
import re
import sys


def normalize(s: str) -> str:
    """改行(EDIT/RichEdit の \\r \\n)と前後の空白を除く。"""
    return s.replace("\r", "").replace("\n", "").strip()


def classify(expect: str, actual: str) -> dict:
    """期待と実際の差を分類する。{loss, extra, reorder, literal, substitute}(件数)と ok を返す。"""
    e, a = normalize(expect), normalize(actual)
    r = {"ok": e == a, "loss": 0, "extra": 0, "reorder": 0, "literal": 0, "substitute": 0}
    if r["ok"]:
        return r
    if sorted(e) == sorted(a):
        r["reorder"] = sum(1 for x, y in zip(e, a) if x != y)
        return r
    sm = difflib.SequenceMatcher(a=e, b=a, autojunk=False)
    for tag, i1, i2, j1, j2 in sm.get_opcodes():
        es, as_ = e[i1:i2], a[j1:j2]
        if tag == "delete":
            r["loss"] += len(es)
        elif tag == "insert":
            lit = sum(1 for c in as_ if c.isascii() and c.isalpha())
            r["literal"] += lit
            r["extra"] += len(as_) - lit
        elif tag == "replace":
            if sorted(es) == sorted(as_):
                r["reorder"] += len(es)
                continue
            lit = sum(1 for c in as_ if c.isascii() and c.isalpha())
            r["literal"] += lit
            rest = len(as_) - lit
            # 置換(同数)は substitute、長さが違う分は loss/extra。
            sub = min(len(es) - lit if len(es) > lit else 0, rest)
            r["substitute"] += sub
            r["loss"] += max(0, len(es) - lit - sub)
            r["extra"] += max(0, rest - sub)
    return r


def parse(path: str) -> list:
    recs = []
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            i = line.find("[TS-JSON] ")
            if i < 0:
                continue
            try:
                recs.append(json.loads(line[i + len("[TS-JSON] "):]))
            except json.JSONDecodeError:
                pass
    return recs


def analyze(recs: list) -> dict:
    cfg = next((r for r in recs if r.get("type") == "config"), {})
    aborts = [r["reason"] for r in recs if r.get("type") == "abort"]
    done = any(r.get("type") == "done" for r in recs)
    trials = [r for r in recs if r.get("type") == "trial"]
    injects = {(r["kind"], r["n"]): r for r in recs if r.get("type") == "inject"}
    ready = [r for r in recs if r.get("type") == "ready"]
    rows, invalid = [], []
    tot = {"loss": 0, "extra": 0, "reorder": 0, "literal": 0, "substitute": 0}
    by_kind = {}
    for t in trials:
        c = classify(t["expect"], t["actual"])
        inj = injects.get((t["kind"], t["n"]), {})
        if not t.get("focus_ok", True):
            invalid.append(f"{t['kind']}#{t['n']}: 試行後にフォーカスが外れていた")
        if inj and inj.get("sent_ok", 0) != inj.get("planned", 0):
            invalid.append(f"{t['kind']}#{t['n']}: SendInput が {inj.get('sent_ok')}/{inj.get('planned')} しか成功しなかった")
        rows.append((t, c, inj))
        for k in tot:
            tot[k] += c[k]
        bk = by_kind.setdefault(t["kind"], [0, 0])
        bk[0] += 1
        bk[1] += 0 if c["ok"] else 1
    n_fail = sum(1 for _, c, _ in rows if not c["ok"])
    if aborts:
        invalid.insert(0, "中断: " + "; ".join(aborts))
    if not done and not aborts:
        invalid.append("完走マーカー(done)が無い")
    if not trials:
        invalid.append("試行が0件")
    if invalid:
        verdict = "INVALID"
    elif n_fail:
        verdict = "FAIL"
    else:
        verdict = "PASS"
    late = [i.get("late_max_us", 0) for _, _, i in rows if i]
    seen = [(i.get("hook_seen", 0), i.get("planned", 0)) for _, _, i in rows if i]
    return {
        "verdict": verdict, "cfg": cfg, "rows": rows, "invalid": invalid, "n_fail": n_fail,
        "n_trials": len(trials), "totals": tot, "by_kind": by_kind, "ready": ready,
        "self": {
            "late_max_us": max(late) if late else None,
            "p95_late_max_us": max((i.get("late_p95_us", 0) for _, _, i in rows if i), default=None),
            "hook_seen_lt_planned": sum(1 for s, p in seen if s < p),
            "hook_seen_min_ratio": min((s / p for s, p in seen if p), default=None),
            "deliver_max_us": max((i.get("deliver_max_us", 0) for _, _, i in rows if i), default=None),
            "span_over_plan_ms": max((i.get("span_ms", 0) - i.get("planned_span_ms", 0) for _, _, i in rows if i), default=None),
        },
    }


def summary_line(r: dict) -> str:
    c, t, s = r["cfg"], r["totals"], r["self"]
    kinds = ",".join(f"{k}:{v[1]}/{v[0]}" for k, v in r["by_kind"].items())
    hs = s["hook_seen_min_ratio"]
    return (
        f"TYPING_STRESS: verdict={r['verdict']} form={c.get('form', '?')} ime={c.get('ime', '?')} "
        f"mode={c.get('mode', '?')} interval_ms={c.get('interval_ms', '?')} trials={r['n_trials']} fail={r['n_fail']} "
        f"loss={t['loss']} extra={t['extra']} reorder={t['reorder']} literal={t['literal']} substitute={t['substitute']} "
        f"fail_by_kind={kinds or '-'} inject_late_max_us={s['late_max_us']} hook_seen_min_ratio="
        f"{'-' if hs is None else f'{hs:.3f}'} hook_seen_lt_planned={s['hook_seen_lt_planned']}"
    )


def main(argv) -> int:
    args = list(argv)
    json_out = None
    if "--json" in args:
        i = args.index("--json")
        if i + 1 >= len(args):
            print(__doc__)
            return 2
        json_out = args[i + 1]
        del args[i:i + 2]
    if len(args) != 1:
        print(__doc__)
        return 2
    try:
        recs = parse(args[0])
    except OSError as e:
        print(f"ログを読めない: {e}")
        print("TYPING_STRESS: verdict=INVALID reason=no-log")
        return 3
    r = analyze(recs)
    cfg = r["cfg"]
    print(f"入力先={cfg.get('form')} IME={cfg.get('ime')} mode={cfg.get('mode')} 間隔={cfg.get('interval_ms')}ms "
          f"文字数/試行={cfg.get('len')} 候補セル(単打,左,右)={cfg.get('cells')} 入力欄クラス={cfg.get('child_class')}")
    for x in r["ready"]:
        print(f"  ready attempt={x.get('attempt')} ok={x.get('ok')} text={x.get('text')!r}")
    for t, c, inj in r["rows"]:
        kinds = [k for k in ("loss", "extra", "reorder", "literal", "substitute") if c[k]]
        tag = "PASS" if c["ok"] else "FAIL(" + ",".join(f"{k}={c[k]}" for k in kinds) + ")"
        print(f"  [{t['kind']:6} #{t['n']}] {tag}  注入 送信={inj.get('sent_ok')}/{inj.get('planned')} "
              f"遅れmax={inj.get('late_max_us')}us フック到着={inj.get('hook_seen')}")
        if not c["ok"]:
            print(f"      期待: {normalize(t['expect'])}")
            print(f"      実際: {normalize(t['actual'])}")
            print(f"      打鍵: {t.get('keys')}")
    for x in r["invalid"]:
        print(f"  INVALID: {x}")
    line = summary_line(r)
    print(line)
    if json_out:
        with open(json_out, "w", encoding="utf-8") as f:
            json.dump({"verdict": r["verdict"], "cfg": cfg, "totals": r["totals"], "by_kind": r["by_kind"],
                       "self": r["self"], "line": line, "invalid": r["invalid"]}, f, ensure_ascii=False)
    return {"PASS": 0, "FAIL": 1, "INVALID": 3}[r["verdict"]]


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
