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


# ---------------------------------------------------------------- BUG-114(相 D)の判定

TS_RE = re.compile(r"T(\d\d):(\d\d):(\d\d\.\d+)")
SCOPE_RE = re.compile(r"\[focus-scope\] bootstrap initial scope:.*profile=(\w+)")
DRIFT_RE = re.compile(r"\[drift\] correction")
READ_RE = re.compile(r"drift_correction_read")
GAVE_UP_RE = re.compile(r"\[drift\] actuation gave up")
REARM_RE = re.compile(r"\[drift\] fresh observation after give-up")
CONV_OBS_RE = re.compile(r"\[idle-conv-check\] TsfNative: conv observation open=")
BLIND_MAX_ATTEMPTS = 5  # state/app_ime_policy.rs::IME_ACTUATION_BLIND_MAX_ATTEMPTS
BURST_GAP_S = 2.0       # 補正の間隔(DRIFT_CORRECTION_THRESHOLD_MS=400ms)より長く、再武装の待ち(3秒)より短い
REARM_LIMIT = 3


def _secs(line):
    m = TS_RE.search(line[:40])
    return int(m.group(1)) * 3600 + int(m.group(2)) * 60 + float(m.group(3)) if m else None


def bursts(times):
    """間隔 BURST_GAP_S 以内で続いた補正をひとまとまりにし、各まとまりの回数を返す。"""
    out = []
    for t in times:
        if out and t - out[-1][-1] <= BURST_GAP_S:
            out[-1].append(t)
        else:
            out.append([t])
    return [len(b) for b in out]


def judge_bug114(lines, expect_profile="TsfNative"):
    """awase.log の行から BUG-114 の判定を返す(verdict=PASS/FAIL/INVALID、数えた値、理由)。

    合否基準(走らせる前に確定): (1) `[focus-scope] bootstrap initial scope:` が1行で profile=expect_profile、
    (2) `drift_correction_read` が0件、(3) drift 補正が1回以上(0回は INVALID=観測経路に乗っていない)、
    (4) 連続した補正(間隔 BURST_GAP_S 以内)がどれも BLIND_MAX_ATTEMPTS 回以内、(5) give-up 後の再武装が REARM_LIMIT 回未満。
    """
    profiles = [m.group(1) for ln in lines if (m := SCOPE_RE.search(ln))]
    drift_t = [t for ln in lines if DRIFT_RE.search(ln) and (t := _secs(ln)) is not None]
    c = {"profile": profiles[0] if len(profiles) == 1 else profiles,
         "read": sum(bool(READ_RE.search(ln)) for ln in lines),
         "drift": len(drift_t), "bursts": bursts(drift_t),
         "gave_up": sum(bool(GAVE_UP_RE.search(ln)) for ln in lines),
         "rearm": sum(bool(REARM_RE.search(ln)) for ln in lines),
         "conv_obs": sum(bool(CONV_OBS_RE.search(ln)) for ln in lines)}
    invalid, failures = [], []
    if len(profiles) != 1:
        invalid.append(f"[focus-scope] bootstrap の行が1件でない({len(profiles)})")
    elif profiles[0] != expect_profile:
        failures.append(f"起動時の profile={profiles[0]}(期待 {expect_profile})")
    if c["read"]:
        failures.append(f"drift_correction_read={c['read']}(Blind であるべき)")
    if not c["drift"]:
        invalid.append("drift 補正 0 件(観測経路に乗っていない)")
    if any(n > BLIND_MAX_ATTEMPTS for n in c["bursts"]):
        failures.append(f"連続した補正が {BLIND_MAX_ATTEMPTS} 回を超えた bursts={c['bursts']}")
    if c["rearm"] >= REARM_LIMIT:
        failures.append(f"give-up 後の再武装が {c['rearm']} 回(連発)")
    verdict = "FAIL" if failures else "INVALID" if invalid else "PASS"
    return {"verdict": verdict, "counts": c, "failures": failures, "invalid": invalid}
