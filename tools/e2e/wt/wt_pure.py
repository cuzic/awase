"""Windows Terminal 制御語彙のうち OS に依存しない部分(echo ログの解析と分類)。Linux の unittest で検査できる。"""
import re

# wt_echo.ps1 が 1 キーにつき 1 行書く: `経過ms<TAB>KeyChar の文字コード<TAB>ConsoleKey<TAB>修飾`
ROW_RE = re.compile(r"^(\d+)\t(-?\d+)\t(-?\d+)\t(.*)$")


def parse_echo(text: str):
    """echo ログ全体 → (ready, [(ms, charcode, consolekey, mods), ...])。壊れた行は捨てる。"""
    ready = False
    rows = []
    for ln in text.lstrip("﻿").splitlines():
        ln = ln.rstrip("\r")
        if ln.strip() == "READY":
            ready = True
            continue
        m = ROW_RE.match(ln)
        if m:
            rows.append((int(m.group(1)), int(m.group(2)), int(m.group(3)), m.group(4)))
    return ready, rows


def classify(rows):
    """受け取った文字を種別ごとに数える。

    kana=ひらがな/カタカナ(U+3040〜U+30FF)、cjk=その他の非 ASCII、at='@'(BUG-113 の余計な文字)、
    ascii_alpha=英字(ローマ字リテラル化の疑い)、enter=Enter(13)、other=その他。
    """
    c = {"kana": 0, "cjk": 0, "at": 0, "ascii_alpha": 0, "enter": 0, "other": 0, "total": 0}
    for _ms, code, _key, _mods in rows:
        c["total"] += 1
        if code == 64:
            c["at"] += 1
        elif code == 13:
            c["enter"] += 1
        elif 0x3040 <= code <= 0x30FF:
            c["kana"] += 1
        elif code > 127:
            c["cjk"] += 1
        elif chr(code).isalpha() if 0 < code < 128 else False:
            c["ascii_alpha"] += 1
        else:
            c["other"] += 1
    return c


def text_of(rows):
    """受け取った文字列(制御文字は <hex> に)。ログ・summary 用。"""
    out = []
    for _ms, code, _key, _mods in rows:
        out.append(chr(code) if code >= 32 and code != 127 else f"<{code:02X}>")
    return "".join(out)


def rows_between(rows, t0_ms, t1_ms):
    return [r for r in rows if t0_ms <= r[0] < t1_ms]
