#!/usr/bin/env python3
"""suspend.rs(--suspend-proc)で相手プロセスを一時停止した run が、「本当に効いたか」を typing_stress.log と awase.log から数える。

「遅延の下でも壊れなかった」と読む前に、停止が効いた証拠を確かめるための部品(標準ライブラリだけ):
  suspended    type:"suspend" のうち、少なくとも 1 プロセスが status=0 で停止できた試行数 / 全試行数
  no_match     matched が空(プロセス名違い・権限不足)の試行数。0 でなければ、その run の結果は使えない
  window       停止〜再開+1s の間に awase.log の `[engine-input] … delay=Nms` の最大値(停止の間に awase 側の処理が遅れたか)
  stale        その窓の中の `per-VK[i/n] stale confirm 検出 … escape=…` の件数、うち escape=true・idx>=1
使い方: suspend_report.py <typing_stress.log> <awase.log>
出力: `SUSPEND_EFFECT: suspended=a/b no_match=c delay_max_in_window_ms=… stale_in_window=… stale_escape_idx1=… verdict=EFFECTIVE|NOT_EFFECTIVE|NO_SUSPEND`
終了コード: 0=EFFECTIVE / 1=NOT_EFFECTIVE(停止が効いていない) / 3=NO_SUSPEND(suspend レコードが無い)
"""
import re
import sys

import e2e_common as ec

DELAY_RE = re.compile(r"\[engine-input\] vk=0x\w+ \w+ .*?\bdelay=(\d+)ms")
STALE_RE = re.compile(r"per-VK\[(\d+)/\d+\] stale confirm 検出.*escape=(true|false)")


def suspends(ts_records):
    return [r for r in ts_records if r.get("type") == "suspend"]


def effective(rec):
    return any(m.get("suspend_status") == 0 for m in rec.get("matched", []))


def window_ms(rec):
    """(開始ms, 終了ms+1000)。utc が欠けていれば None。"""
    try:
        return ec.hms_to_ms(rec["suspended_utc"]), ec.hms_to_ms(rec["resumed_utc"]) + 1000
    except (KeyError, ValueError):
        return None


def analyze(ts_records, awase_timed):
    sus = suspends(ts_records)
    if not sus:
        return {"verdict": "NO_SUSPEND", "suspended": 0, "total": 0, "no_match": 0,
                "delay_max_ms": None, "stale": 0, "stale_escape_idx1": 0}
    ok = [r for r in sus if effective(r)]
    no_match = [r for r in sus if not r.get("matched")]
    wins = [w for w in (window_ms(r) for r in ok) if w]
    delay_max = None
    stale = esc1 = 0
    for hms, line in awase_timed:
        t = ec.hms_to_ms(hms)
        if not any(a <= t <= b for a, b in wins):
            continue
        m = DELAY_RE.search(line)
        if m:
            delay_max = max(delay_max or 0, int(m.group(1)))
        m = STALE_RE.search(line)
        if m:
            stale += 1
            if m.group(2) == "true" and int(m.group(1)) >= 1:
                esc1 += 1
    return {"verdict": "EFFECTIVE" if ok and not no_match else "NOT_EFFECTIVE",
            "suspended": len(ok), "total": len(sus), "no_match": len(no_match),
            "delay_max_ms": delay_max, "stale": stale, "stale_escape_idx1": esc1}


def main(argv):
    if len(argv) != 2:
        print(__doc__)
        return 2
    r = analyze(ec.ts_json_records(argv[0]), ec.load_awase_timed(argv[1]))
    print("SUSPEND_EFFECT: suspended={suspended}/{total} no_match={no_match} delay_max_in_window_ms={d} "
          "stale_in_window={stale} stale_escape_idx1={stale_escape_idx1} verdict={verdict}".format(
              d="-" if r["delay_max_ms"] is None else r["delay_max_ms"], **r))
    return {"EFFECTIVE": 0, "NOT_EFFECTIVE": 1, "NO_SUSPEND": 3}[r["verdict"]]


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
