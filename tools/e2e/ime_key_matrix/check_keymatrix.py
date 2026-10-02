#!/usr/bin/env python3
"""ADR-208 L3b: 「ずれの作り方 × 明示キー」行列の判定(`typing_stress --mode=keymatrix` / `chrome_probe --keymatrix=`)。

目的は ADR-208 決定7 (2) の受け入れ条件: 明示キーは、内部状態(belief・applied)が何であっても、絶対キーは1押下・トグルは2押下で
実 IME がキーの意味に一致する(固着ゼロ)。セル(= キー × 種別 × ずれの作り方)ごとに試行を数える。

セルの書式(ハーネスの `--km-cells=` / `--keymatrix=` と同じ): `<key>=<kind>:<gap>`。
  kind: on(ON 絶対) / off(OFF 絶対) / tog(トグル)
  gap : sync(ずれなし) / close(awase に ON を書かせた後、実 IME を外から閉じる=S-1) / open(OFF を書かせた後、外から開く) /
        fresh(awase を再起動し belief・applied が未知の状態=ADR-208 決定4 の E2 の対照)

1試行の分類(押下ごとの実 IME 状態を、押した順に見る。目標状態 target = 絶対キーはキーの意味、トグルは押す前の状態の反転):
  gap_not_made  ずれ(または前提の状態)を作れなかった(pre_ok でない、または押す前の実 IME が r0 でない)。測定にならない
  conv1         1押下目で target に一致
  conv2         2押下目で一致(1押下目は変わらなかった/逆だった)
  late          3押下目で初めて一致(保証の範囲外)
  stuck         max_press 回押しても一致しない(固着)
  invalid       フォーカス喪失・押下の記録なし・実 IME 状態を全く読めない
実 IME 状態の証拠: 自前窓(typing_stress、config.evidence=api)は +2000ms の `ImmGetOpenStatus`(無ければ +500ms、さらに無ければ打鍵)。
実 Chrome(chrome_probe、evidence=typed)は API が TsfNative で信頼できないので、押下ごとの打鍵(k,a)の結果(かな=開、英字=閉)を使う。

セルの verdict(有効かつ「ずれを作れた」試行について):
  CONVERGED_1     全試行が conv1
  CONVERGED_2     全試行が conv1 か conv2(トグルならこれが合格。絶対キーでは1押下の保証違反)
  CONVERGED_LATE  stuck は無いが late がある(3押下目で一致。保証違反)
  STUCK           stuck が1件以上(固着)
  ENV_EXCEPTION   上のうち不合格だが、ADR-208 決定4(a)の環境の例外(MS-IME × 実 Chrome の OFF 方向)で、E2 の対照が成立した
                  (同じ job の fresh セルが、同じ程度に失敗する=内部状態を新鮮にしても閉じない)。fresh セル自身が例外セルで失敗した場合も、
                  内部状態に由来しえないので ENV_EXCEPTION。例外セルの外では fresh が失敗しても STUCK のまま
  GAP_NOT_MADE    ずれを1件も作れなかった
  INVALID         有効試行が半数未満、または job が中断・未完走
不合格 = STUCK / CONVERGED_LATE / 絶対キーの CONVERGED_2。E2 の判定: 不合格セルの失敗率 fc と、同じキー・種別の fresh セルの失敗率 ff が
ff >= 0.5 * fc かつ ff > 0 なら環境の例外(ENV_EXCEPTION)。ff が小さければ STUCK のまま(内部状態による固着=バグ。`fresh_ok` と出す)。
fresh セルが無ければ STUCK のまま(`exception_candidate=true`、E2 未実施と出す)。

物理 Ctrl の確認: ctrl+ のセルがあるとき、awase.log に `[engine-input] vk=0x<key> KeyDown … mods(c=true …` の行が全て
`phys_ctrl=true` であること(1件も無い=注入が届かない、false が混じる=物理キー扱いでない)を要求し、満たさなければ ctrl+ のセルは INVALID。

使い方: check_keymatrix.py [--json out.json] [--min-n 10] <typing_stress.log|chrome_probe.log> [awase.log]
終了コード: 0=全セルが合格(または ENV_EXCEPTION / GAP_NOT_MADE) / 1=不合格のセルあり / 3=INVALID / 2=使い方の誤り
出力の末尾に `KEYMATRIX_CELL:` を1セル1行、最後に `KEYMATRIX:` の要約1行。
"""
import json
import re
import sys

KM_LINE = re.compile(r"\] KM (\{.*\})\s*$")
KM_CONFIG_LINE = re.compile(r"\] KM_CONFIG (\{.*\})\s*$")
ENGINE_DOWN = re.compile(r"\[engine-input\] vk=0x([0-9A-Fa-f]+) KeyDown")
PHYS_CTRL = re.compile(r"mods\(c=true .*phys_ctrl=true")
CTRL_MODS = re.compile(r"mods\(c=true ")

# 例外(ADR-208 決定4(a)): MS-IME × 実 Chrome の OFF 方向。閉じた列挙で、増やすには ADR の改訂が要る。
EXCEPTION_IME = "msime"
EXCEPTION_FORMS = ("chromepage", "chrome")
E2_RATIO = 0.5


def load_records(path: str) -> list:
    """typing_stress.log(`[TS-JSON]`)または chrome_probe.log(`KM {json}` / `KM_CONFIG {json}`)を km_* のレコード列にする。"""
    recs = []
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            i = line.find("[TS-JSON] ")
            if i >= 0:
                try:
                    recs.append(json.loads(line[i + len("[TS-JSON] "):]))
                except json.JSONDecodeError:
                    pass
                continue
            m = KM_CONFIG_LINE.search(line)
            if m:
                try:
                    d = json.loads(m.group(1))
                    d["type"] = "km_config"
                    recs.append(d)
                except json.JSONDecodeError:
                    pass
                continue
            m = KM_LINE.search(line)
            if m:
                try:
                    recs.append(json.loads(m.group(1)))
                except json.JSONDecodeError:
                    pass
                continue
            if "全ケース完了" in line or "=== 完了 ===" in line:
                recs.append({"type": "done"})
            elif "KM_ABORT" in line:
                recs.append({"type": "abort", "reason": line.split("KM_ABORT", 1)[1].strip()})
    return recs


def press_state(press: dict, evidence: str):
    """1押下ぶんの実 IME 状態(True=開 / False=閉 / None=読めない)。"""
    if evidence == "typed":
        order = ("typed_open",)
    else:
        order = ("api2000", "api500", "typed_open")
    for k in order:
        v = press.get(k)
        if v is not None:
            return bool(v)
    return None


def classify_trial(t: dict, evidence: str) -> dict:
    """1試行の分類(docstring 参照)。"""
    r0 = t.get("r0")
    target = t.get("target")
    if t.get("focus_lost"):
        return {"kind": "invalid", "reason": "フォーカスが外れた"}
    pre_api = t.get("pre_api")
    if not t.get("pre_ok") or (pre_api is not None and pre_api != r0):
        return {"kind": "gap_not_made", "reason": f"前提を作れていない(pre_ok={t.get('pre_ok')} pre_api={pre_api} r0={r0})"}
    presses = t.get("presses") or []
    if not presses:
        return {"kind": "invalid", "reason": "押下の記録が無い"}
    states = [press_state(p, evidence) for p in presses]
    if all(s is None for s in states):
        return {"kind": "invalid", "reason": "実 IME 状態を全く読めない"}
    for i, s in enumerate(states, start=1):
        if s == target:
            return {"kind": {1: "conv1", 2: "conv2"}.get(i, "late"), "presses": i, "states": states}
    return {"kind": "stuck", "presses": len(states), "states": states,
            "moved": any(s is not None and s != r0 for s in states)}


def phys_ctrl_check(awase_lines, vk_hex: str):
    """awase.log の `[engine-input] vk=0x<vk> KeyDown` のうち Ctrl 修飾付きの件数と、そのうち物理 Ctrl(phys_ctrl=true)の件数。"""
    n = ok = 0
    want = int(vk_hex, 16)
    for line in awase_lines:
        m = ENGINE_DOWN.search(line)
        if m and int(m.group(1), 16) == want and CTRL_MODS.search(line):
            n += 1
            ok += bool(PHYS_CTRL.search(line))
    return n, ok


def is_pass(kind: str, verdict: str) -> bool:
    return verdict == "CONVERGED_1" or (verdict == "CONVERGED_2" and kind == "tog")


def trial_failed(kind: str, c: dict) -> bool:
    """1試行が、そのキー種別の保証(絶対=1押下、トグル=2押下以内)を満たさなかったか。"""
    if c["kind"] in ("conv1",):
        return False
    if c["kind"] == "conv2":
        return kind != "tog"
    return c["kind"] in ("late", "stuck")


def analyze(recs: list, awase_lines=None, min_n: int = 10) -> dict:
    cfg = next((r for r in recs if r.get("type") in ("km_config", "config")), {})
    form = cfg.get("form") or ""
    ime = cfg.get("ime") or ""
    evidence = cfg.get("evidence") or ("typed" if form in EXCEPTION_FORMS else "api")
    aborts = [r.get("reason", "") for r in recs if r.get("type") == "abort"]
    done = any(r.get("type") == "done" for r in recs)
    by_cell = {}
    order = []
    for r in recs:
        if r.get("type") == "km_trial":
            if r["cell"] not in by_cell:
                order.append(r["cell"])
            by_cell.setdefault(r["cell"], []).append(r)
    job_invalid = []
    if aborts:
        job_invalid.append("中断: " + "; ".join(aborts))
    if not done and not aborts:
        job_invalid.append("完走マーカーが無い")
    if not by_cell:
        job_invalid.append("km_trial が0件")
    cells = []
    for name in order:
        trials = by_cell[name]
        first = trials[0]
        kind, gap, key = first["kind"], first["gap"], first["key"]
        classes = [classify_trial(t, evidence) for t in trials]
        counts = {k: sum(1 for c in classes if c["kind"] == k)
                  for k in ("conv1", "conv2", "late", "stuck", "gap_not_made", "invalid")}
        n = len(trials)
        valid = n - counts["invalid"]
        made = counts["conv1"] + counts["conv2"] + counts["late"] + counts["stuck"]
        failed = sum(1 for c in classes if trial_failed(kind, c))
        cell = {"cell": name, "key": key, "kind": kind, "gap": gap, "n": n, "valid": valid, "made": made,
                "target": first["target"], "counts": counts, "failed": failed,
                "fail_rate": (failed / made) if made else 0.0, "reason": None}
        ctrl_invalid = None
        if key.startswith("ctrl+") and awase_lines is not None:
            nc, ok = phys_ctrl_check(awase_lines, key[len("ctrl+"):])
            if nc == 0:
                ctrl_invalid = "awase.log に Ctrl 付きの engine-input が無い(注入が awase に届かない)"
            elif ok != nc:
                ctrl_invalid = f"物理 Ctrl 扱いでない(Ctrl 付き {nc} 件中 phys_ctrl=true は {ok} 件)"
        if job_invalid:
            verdict = "INVALID"
        elif ctrl_invalid:
            verdict, cell["reason"] = "INVALID", ctrl_invalid
        elif n == 0 or valid * 2 < n:
            verdict, cell["reason"] = "INVALID", f"有効試行が半数未満({valid}/{n})"
        elif made == 0:
            verdict = "GAP_NOT_MADE"
        elif counts["stuck"] > 0:
            verdict = "STUCK"
        elif counts["late"] > 0:
            verdict = "CONVERGED_LATE"
        elif counts["conv2"] > 0:
            verdict = "CONVERGED_2"
        else:
            verdict = "CONVERGED_1"
        cell["verdict"] = verdict
        cell["pass"] = is_pass(kind, verdict)
        cell["meets_n"] = made >= min_n
        cell["moved"] = sum(1 for c in classes if c["kind"] == "stuck" and c.get("moved"))
        cell["exception_cell"] = (ime == EXCEPTION_IME and form in EXCEPTION_FORMS and first["target"] is False)
        cell["fresh_fail_rate"] = None
        cell["e2"] = None
        cells.append(cell)
    apply_e2(cells)
    bad = [c for c in cells if c["verdict"] in ("STUCK", "CONVERGED_LATE")
           or (c["verdict"] == "CONVERGED_2" and c["kind"] != "tog")]
    invalid = bool(job_invalid) or any(c["verdict"] == "INVALID" for c in cells)
    overall = "INVALID" if invalid else ("FAIL" if bad else "PASS")
    return {"verdict": overall, "cfg": cfg, "cells": cells, "invalid": job_invalid, "evidence": evidence}


def apply_e2(cells: list) -> None:
    """E2(ADR-208 決定4): 例外セルの不合格を、fresh セルとの比較で ENV_EXCEPTION / STUCK(内部状態による固着)に確定する。"""
    for c in cells:
        failing = c["verdict"] in ("STUCK", "CONVERGED_LATE") or (c["verdict"] == "CONVERGED_2" and c["kind"] != "tog")
        if not failing or not c["exception_cell"]:
            continue
        if c["gap"] == "fresh":
            # 内部状態が新鮮でも失敗する=内部状態に由来しえない環境の失敗。
            c["verdict"], c["e2"] = "ENV_EXCEPTION", "fresh_fails"
            continue
        sibs = [x for x in cells if x["gap"] == "fresh" and x["key"] == c["key"] and x["kind"] == c["kind"]
                and x["verdict"] not in ("INVALID", "GAP_NOT_MADE")]
        if not sibs:
            c["e2"] = "no_fresh_cell"
            continue
        ff = sibs[0]["fail_rate"]
        c["fresh_fail_rate"] = ff
        if ff > 0 and ff >= E2_RATIO * c["fail_rate"]:
            c["verdict"], c["e2"] = "ENV_EXCEPTION", "fresh_similar"
        else:
            c["e2"] = "fresh_ok"
    for c in cells:
        c["pass"] = is_pass(c["kind"], c["verdict"])


def cell_line(c: dict, cfg: dict) -> str:
    k = c["counts"]
    ff = "-" if c["fresh_fail_rate"] is None else f"{c['fresh_fail_rate']:.2f}"
    return (f"KEYMATRIX_CELL: cell={c['cell']} verdict={c['verdict']} pass={'true' if c['pass'] else 'false'} "
            f"form={cfg.get('form', '?')} ime={cfg.get('ime', '?')} key={c['key']} kind={c['kind']} gap={c['gap']} "
            f"target={'ON' if c['target'] else 'OFF'} n={c['n']} made={c['made']} c1={k['conv1']} c2={k['conv2']} "
            f"late={k['late']} stuck={k['stuck']} gap_not_made={k['gap_not_made']} invalid={k['invalid']} "
            f"fail_rate={c['fail_rate']:.2f} exception_cell={'true' if c['exception_cell'] else 'false'} "
            f"e2={c['e2'] or '-'} fresh_fail_rate={ff} meets_n={'true' if c['meets_n'] else 'false'}")


def summary_line(r: dict) -> str:
    cells = r["cells"]
    cnt = {}
    for c in cells:
        cnt[c["verdict"]] = cnt.get(c["verdict"], 0) + 1
    parts = " ".join(f"{k}={cnt[k]}" for k in sorted(cnt))
    return (f"KEYMATRIX: verdict={r['verdict']} form={r['cfg'].get('form', '?')} ime={r['cfg'].get('ime', '?')} "
            f"cells={len(cells)} pass={sum(1 for c in cells if c['pass'])} {parts}".rstrip())


def main(argv) -> int:
    args = list(argv)
    json_out = None
    min_n = 10
    if "--json" in args:
        i = args.index("--json")
        if i + 1 >= len(args):
            print(__doc__)
            return 2
        json_out = args[i + 1]
        del args[i:i + 2]
    if "--min-n" in args:
        i = args.index("--min-n")
        if i + 1 >= len(args):
            print(__doc__)
            return 2
        min_n = int(args[i + 1])
        del args[i:i + 2]
    if len(args) not in (1, 2):
        print(__doc__)
        return 2
    try:
        recs = load_records(args[0])
    except OSError as e:
        print(f"ログを読めない: {e}")
        print("KEYMATRIX: verdict=INVALID form=? ime=? cells=0 pass=0")
        return 3
    awase_lines = None
    if len(args) == 2:
        try:
            with open(args[1], encoding="utf-8", errors="replace") as f:
                awase_lines = f.read().splitlines()
        except OSError:
            awase_lines = None
    r = analyze(recs, awase_lines, min_n)
    cfg = r["cfg"]
    print(f"入力先={cfg.get('form')} IME={cfg.get('ime')} 証拠={r['evidence']}")
    for x in r["invalid"]:
        print(f"  INVALID: {x}")
    for c in r["cells"]:
        if c["reason"]:
            print(f"  {c['cell']}: INVALID({c['reason']})")
        print(cell_line(c, cfg))
    line = summary_line(r)
    print(line)
    if json_out:
        with open(json_out, "w", encoding="utf-8") as f:
            json.dump({"verdict": r["verdict"], "cfg": cfg, "cells": r["cells"], "line": line,
                       "invalid": r["invalid"]}, f, ensure_ascii=False)
    return {"PASS": 0, "FAIL": 1, "INVALID": 3}[r["verdict"]]


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
