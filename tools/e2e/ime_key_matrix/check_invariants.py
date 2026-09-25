#!/usr/bin/env python3
"""awase のデバッグログ(RUST_LOG=debug の awase.log)から「不変条件(異常検出器)」の件数を数え、
既知バグの許容上限(ratchet、invariant_limits.json)と比べる。標準ライブラリだけで動く。

期待表(check.py の EXPECT)は設計変更のたびに古くなり、BUG-162/BUG-163 はログに出ていたのに
数日見逃された。ここでは「どの設計でも起きてはいけないこと」だけを数える:

  I1  明示意図なしの drift 補正: `[drift] correction: … → set_ime_open(…)` のうち、同じ観測サイクル
      (直前の `explicit_intent=` 行、既定 100ms 以内)が `explicit_intent=None` のもの(BUG-163)。
      起動(ログ先頭行)から window_s 秒の窓の件数(i1_startup_drift_no_intent)と、ログ全体の件数
      (i1_drift_no_intent_total)の2つ。直前に `explicit_intent=` 行が見つからないものも「意図の証拠なし」
      として数え、内訳(intent_unknown)に出す(書式が変わって黙って0件になるのを防ぐ)。
  I2  `ime open applied … outcome="Unwarranted"`(journal の1行)の件数(BUG-162)。同じ span
      (`on_ime_apply_complete{… outcome=Unwarranted …}`)の中で出た別の行(Timer set 等)は数えない。
      同じ seq の行が重複しても1件。(check.py / check_consistency.py は "outcome=Unwarranted" を含む行を
      数えるので、span 由来の行と合わせて1件を2件と数える)
  I3  情報のみ(上限なし): 自己注入の IME モードキー(`[hook] IME-mode vk=… down self_injected=true`)の件数と
      vk 別内訳、`[warrant-shadow] … would_have_blocked=true` の件数と chain/strategy 別内訳。

上限の判定(ratchet): 値 > max → FAIL。値 < max は OK で「上限未満」と表示し、さらに値が observed_min
(実測した揺れ幅の下限)も下回ったら「上限を下げてよい」と表示する(上限は下げる方向にだけ動かす)。

使い方: check_invariants.py [--config 構成名] [--limits invariant_limits.json] [--window 秒]
                            [--json 出力.json] awase.log
終了コード: 0=全て上限以内 / 1=上限超過あり / 3=ログが無い・awase の起動行が無い(INVALID) / 2=使い方の誤り
最終行は1行サマリ `INVARIANTS: verdict=… i1_startup=…` (ワークフローの summary が拾う)。
"""
import argparse
import datetime
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_LIMITS = os.path.join(HERE, "invariant_limits.json")

TS_RE = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?)Z\s")
START_MARK = "Keyboard Layout Emulator starting"
INTENT_RE = re.compile(r"explicit_intent=(\S+)")
DRIFT_RE = re.compile(r"\[drift\] correction: .*?set_ime_open\((true|false)\)")
DRIFT_SRC_RE = re.compile(r"source=(\w+)")
APPLIED_RE = re.compile(r"\bime open applied seq=(\d+)\b.*\boutcome=\"Unwarranted\"")
HOOK_SELF_RE = re.compile(r"\[hook\] IME-mode vk=(0x[0-9A-Fa-f]+) down self_injected=true")
WARRANT_RE = re.compile(r"\[warrant-shadow\] chain=(\S+) open=(\S+) .*?strategy: \"([^\"]*)\".*?would_have_blocked=true")

# 同じ観測サイクルとみなす、drift 行と直前の explicit_intent= 行の時間差(ms)。
# 実測(CI 12本、計 109 件): 0.03〜20.3ms。
SAME_CYCLE_MS = 100.0

GATED = ("i1_startup_drift_no_intent", "i1_drift_no_intent_total", "i2_unwarranted")


def parse_ts(s):
    # Python 3.10 の fromisoformat は小数部6桁までしか読まない。7桁以上は切り詰める。
    if "." in s:
        head, frac = s.split(".", 1)
        s = head + "." + frac[:6].ljust(6, "0")
    return datetime.datetime.fromisoformat(s)


def analyze(lines, window_s):
    """ログ行の列から各不変条件の件数と詳細を返す(純関数、テスト対象)。"""
    t0 = None
    started = False
    last_intent = None  # (ts, value)
    drifts = []  # dict(t, intent, target, source)
    unwarranted_seqs = []
    self_keys = {}
    warrant = {}
    for line in lines:
        m = TS_RE.match(line)
        if not m:
            continue
        ts = parse_ts(m.group(1))
        if t0 is None:
            t0 = ts
        if START_MARK in line:
            started = True
        mi = INTENT_RE.search(line)
        if mi:
            last_intent = (ts, mi.group(1))
        md = DRIFT_RE.search(line)
        if md:
            intent = "unknown"
            if last_intent is not None and (ts - last_intent[0]).total_seconds() * 1000 <= SAME_CYCLE_MS:
                intent = last_intent[1]
            ms = DRIFT_SRC_RE.search(line)
            drifts.append(dict(t=round((ts - t0).total_seconds(), 3), intent=intent, target=md.group(1),
                               source=ms.group(1) if ms else "?"))
            continue
        ma = APPLIED_RE.search(line)
        if ma:
            if ma.group(1) not in unwarranted_seqs:
                unwarranted_seqs.append(ma.group(1))
            continue
        mh = HOOK_SELF_RE.search(line)
        if mh:
            vk = mh.group(1).upper().replace("0X", "0x")
            self_keys[vk] = self_keys.get(vk, 0) + 1
            continue
        mw = WARRANT_RE.search(line)
        if mw:
            k = f"{mw.group(1)}/{mw.group(3)}/open={mw.group(2)}"
            warrant[k] = warrant.get(k, 0) + 1
    no_intent = [d for d in drifts if d["intent"] in ("None", "unknown")]
    in_window = [d for d in no_intent if d["t"] <= window_s]
    return dict(
        started=started,
        window_s=window_s,
        counts=dict(
            i1_startup_drift_no_intent=len(in_window),
            i1_drift_no_intent_total=len(no_intent),
            i2_unwarranted=len(unwarranted_seqs),
            i3_self_injected_ime_mode_keys=sum(self_keys.values()),
            i3_warrant_shadow_would_block=sum(warrant.values()),
        ),
        detail=dict(
            drifts=drifts,
            drift_intent_unknown=sum(1 for d in drifts if d["intent"] == "unknown"),
            drift_with_intent=sum(1 for d in drifts if d["intent"] not in ("None", "unknown")),
            unwarranted_seqs=unwarranted_seqs,
            self_injected_by_vk=dict(sorted(self_keys.items())),
            would_block_by_chain=dict(sorted(warrant.items())),
        ),
    )


def load_limits(path, config):
    """limits ファイルから、この構成に効く上限(既定 + 構成別上書き)を返す。"""
    with open(path, encoding="utf-8") as f:
        doc = json.load(f)
    limits = {k: dict(v) for k, v in doc.get("limits", {}).items()}
    for k, v in doc.get("config_overrides", {}).get(config or "", {}).items():
        limits[k] = dict(limits.get(k, {}), **v)
    measured = config in doc.get("measured_configs", [])
    return doc.get("window_s", 10), limits, measured


def judge(counts, limits):
    """(verdict, 行の列)。verdict は OK / FAIL。上限の無い不変条件は判定しない。"""
    rows, fail = [], False
    for key in GATED:
        v = counts[key]
        lim = limits.get(key)
        if lim is None or "max" not in lim:
            rows.append((key, v, None, "上限なし(情報のみ)"))
            continue
        mx, lo, bug = lim["max"], lim.get("observed_min", lim["max"]), lim.get("bug", "?")
        if v > mx:
            fail = True
            msg = f"FAIL: 上限 {mx}({bug})を超えた"
        elif v < lo:
            msg = f"OK: 上限 {mx}({bug})未満。実測の揺れ幅の下限 {lo} も下回った → 上限を {v} へ下げてよい"
        elif v < mx:
            msg = f"OK: 上限 {mx}({bug})未満(実測の揺れ幅 {lo}〜{mx} の内)"
        else:
            msg = f"OK: 上限 {mx}({bug})ちょうど"
        rows.append((key, v, mx, msg))
    return ("FAIL" if fail else "OK"), rows


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("log")
    ap.add_argument("--config", default="", help="構成名(invariant_limits.json の config_overrides を引く)")
    ap.add_argument("--limits", default=DEFAULT_LIMITS)
    ap.add_argument("--window", type=float, default=None, help="I1 の起動後の窓(秒)。既定は limits の window_s")
    ap.add_argument("--json", default=None, help="結果を JSON で書く先")
    a = ap.parse_args(argv)

    window_s, limits, measured = load_limits(a.limits, a.config)
    if a.window is not None:
        window_s = a.window

    if not os.path.exists(a.log) or os.path.getsize(a.log) == 0:
        res = dict(verdict="INVALID", rc=3, reason="awase.log が無いか空", config=a.config)
    else:
        with open(a.log, encoding="utf-8", errors="replace") as f:
            r = analyze(f, window_s)
        if not r["started"]:
            res = dict(verdict="INVALID", rc=3, reason=f"awase の起動行({START_MARK})が無い", config=a.config, **r)
        else:
            verdict, rows = judge(r["counts"], limits)
            res = dict(verdict=verdict, rc=0 if verdict == "OK" else 1, config=a.config, measured_config=measured,
                       rows=[dict(key=k, value=v, max=m, message=msg) for k, v, m, msg in rows], **r)

    if res["verdict"] == "INVALID":
        print(f"不変条件: INVALID ({res['reason']})")
    else:
        c, d = res["counts"], res["detail"]
        print(f"不変条件(構成={a.config or '-'}, 上限={'実測済みの構成' if measured else '既定値(この構成は未実測)'}, "
              f"I1 窓={window_s:g}s)")
        for row in res["rows"]:
            print(f"  {row['key']:<30} {row['value']:>4}  {row['message']}")
        print(f"  {'i3_self_injected_ime_mode_keys':<30} {c['i3_self_injected_ime_mode_keys']:>4}  情報: {d['self_injected_by_vk']}")
        print(f"  {'i3_warrant_shadow_would_block':<30} {c['i3_warrant_shadow_would_block']:>4}  情報: {d['would_block_by_chain']}")
        for dr in d["drifts"]:
            print(f"    drift +{dr['t']:.2f}s intent={dr['intent']} → set_ime_open({dr['target']}) source={dr['source']}")
        if d["drift_intent_unknown"]:
            print(f"  注意: 直前 {SAME_CYCLE_MS:g}ms 以内に explicit_intent= 行が無い drift が {d['drift_intent_unknown']} 件"
                  "(意図なしとして数えた。ログ書式の変更を疑う)")
    c = res.get("counts", {})
    print("INVARIANTS: verdict={} rc={} i1_startup={} i1_total={} i2_unwarranted={} i3_self_keys={} i3_would_block={}".format(
        res["verdict"], res["rc"], c.get("i1_startup_drift_no_intent", "-"), c.get("i1_drift_no_intent_total", "-"),
        c.get("i2_unwarranted", "-"), c.get("i3_self_injected_ime_mode_keys", "-"),
        c.get("i3_warrant_shadow_would_block", "-")))
    if a.json:
        with open(a.json, "w", encoding="utf-8") as f:
            json.dump(res, f, ensure_ascii=False, indent=1)
    return res["rc"]


if __name__ == "__main__":
    sys.exit(main())
