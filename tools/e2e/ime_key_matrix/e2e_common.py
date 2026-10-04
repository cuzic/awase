"""check_*.py が共有するログ読みの共通部品(`[TS-JSON]` の取り出し・awase.log の時刻付き行・時刻変換・フォーカス復帰の数え方)。

各 check_*.py が同じ処理を別々に書いていた(時刻変換4通り・awase.log 行分解・`[AUTO]` 判定5回・`[TS-JSON]` パース3回)ので集約した。
判定ロジックは各 check_*.py に残し、ここには「ログからどう値を取り出すか」だけを置く。
"""
import datetime as dt
import json
import re

TS_JSON_MARK = "[TS-JSON] "
FOCUS_RESTORE_MARK = "[AUTO] フォーカス復帰"
# スパイクの「手順1の記録」行。これ以降にフォーカスが外れた回だけ無効(起動直後のフォーカス取得は正常)。
SCRIPT_START_MARK = "KEY [SCRIPT 1/10"

# awase.log / typing_stress.log の行頭付近の時刻(`...T12:34:56.789Z`)。
TIME_RE = re.compile(r"T(\d\d:\d\d:\d\d\.\d{3})")


def ts_json_in_line(line: str):
    """行に `[TS-JSON] {json}` があれば dict を返す。無い・壊れた JSON なら None。"""
    i = line.find(TS_JSON_MARK)
    if i < 0:
        return None
    try:
        return json.loads(line[i + len(TS_JSON_MARK):])
    except ValueError:  # JSONDecodeError は ValueError の子
        return None


def ts_json_records(path: str) -> list:
    """ログ全体の `[TS-JSON]` レコード列。"""
    with open(path, encoding="utf-8", errors="replace") as f:
        return [r for r in map(ts_json_in_line, f) if r is not None]


def load_awase_timed(path: str) -> list:
    """awase.log を (HH:MM:SS.mmm, 行) の列にする。読めなければ空。"""
    out = []
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                m = TIME_RE.search(line[:40])
                if m:
                    out.append((m.group(1), line))
    except OSError:
        pass
    return out


def hms_to_ms(hms: str) -> int:
    """`HH:MM:SS.mmm` → その日の 0 時からのミリ秒。"""
    h, m, rest = hms.split(":")
    s, ms = rest.split(".")
    return ((int(h) * 60 + int(m)) * 60 + int(s)) * 1000 + int(ms)


def hms_to_seconds(hms: str) -> float:
    t = dt.datetime.strptime(hms, "%H:%M:%S.%f")
    return t.hour * 3600 + t.minute * 60 + t.second + t.microsecond / 1e6


def count_focus_restores(lines, start_mark: str = SCRIPT_START_MARK) -> int:
    """`start_mark` を含む行以降の「フォーカス復帰」行の数。フォーカス移動は awase の FocusChange を誘発し結果を汚すので、
    1回でもあればその回は INVALID にする側で使う。start_mark=None なら先頭から数える。"""
    started = start_mark is None
    n = 0
    for line in lines:
        if not started and start_mark in line:
            started = True
        if started and FOCUS_RESTORE_MARK in line:
            n += 1
    return n
