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


# ---------------------------------------------------------------- BUG-114(相 E)の判定

TS_RE = re.compile(r"T(\d\d):(\d\d):(\d\d\.\d+)")
SCOPE_RE = re.compile(r"\[focus-scope\] bootstrap initial scope:.*profile=(\w+)")
DRIFT_RE = re.compile(r"\[drift\] correction")
READ_RE = re.compile(r"drift_correction_read")
GAVE_UP_RE = re.compile(r"\[drift\] actuation gave up")
REARM_RE = re.compile(r"\[drift\] fresh observation after give-up")
PROCESS_CHANGE_RE = re.compile(r"focus transition .*changed_process=true")
BLIND_MAX_ATTEMPTS = 5  # state/app_ime_policy.rs::IME_ACTUATION_BLIND_MAX_ATTEMPTS
BURST_GAP_S = 2.0       # 実測の補正の間隔(20〜50ms)より十分長く、再武装の待ち(3 秒)より短い
REARM_COOLDOWN_S = 3.0  # tuning.rs::DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS
REARM_TAIL_S = 2.0      # 2 回目のきっかけの後、窓を閉じるまでに最低限ほしい時間


def secs_of_day(line):
    """awase.log の行頭の UTC 時刻(HH:MM:SS.fff)を 0 時からの秒にする。無ければ None。"""
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


def judge_bug114(lines, t_close, t_second, expect_profile="TsfNative"):
    """awase.log の行から BUG-114 の判定を返す。時刻はすべて UTC の 0 時からの秒。

    drift 補正を数えるのは窓を閉じる(`t_close`)より前の行だけ(閉じる操作そのものが GJI I/O を起こし、
    補正のきっかけになるため)。`t_second` は 2 回目のきっかけ(同じ WT プロセスの補助窓へ前面を移して戻す)を始めた時刻。
    合否基準(走らせる前に確定):
      1. `[focus-scope] bootstrap initial scope:` が 1 行で profile=expect_profile
      2. `drift_correction_read` が 0 件
      3. drift 補正が 1 回以上(0 回は INVALID=観測経路に乗っていない)
      4. 連続した補正がどれも BLIND_MAX_ATTEMPTS 回以内で、`gave up` が 1 回以上(止まったこと)
      5. give-up 後の再武装(`fresh observation after give-up`)が 0 件。ただし 2 回目のきっかけが最後の gave up から
         REARM_COOLDOWN_S 以上後で、その後 REARM_TAIL_S 以上窓が開いていた回だけ確かめられる(そうでなければ INVALID)
    前提: プロセスの切り替え(`focus transition … changed_process=true`)は起動時の 1 回だけ(2 回以上なら FocusChanged で
    app_policy が作り直され、起動時経路を見ていないので INVALID)。
    FAIL(1・2・4・5 の違反)> INVALID > PASS。`reproduced` は BUG-114 の症状が出たか(2・4・5 のどれかに違反、
    1 の profile 違いは含めない)。修正を外した対照で「再現した」を機械的に判定するのに使う。
    """
    profiles = [m.group(1) for ln in lines if (m := SCOPE_RE.search(ln))]
    timed = [(t, ln) for ln in lines if (t := secs_of_day(ln)) is not None and t < t_close]
    drift_t = [t for t, ln in timed if DRIFT_RE.search(ln)]
    gave_up_t = [t for t, ln in timed if GAVE_UP_RE.search(ln)]
    c = {"profile": profiles[0] if len(profiles) == 1 else profiles,
         "read": sum(bool(READ_RE.search(ln)) for _, ln in timed),
         "drift": len(drift_t), "bursts": bursts(drift_t), "gave_up": len(gave_up_t),
         "rearm": sum(bool(REARM_RE.search(ln)) for _, ln in timed),
         "process_changes": sum(bool(PROCESS_CHANGE_RE.search(ln)) for _, ln in timed)}
    invalid, failures, symptoms = [], [], []
    if c["process_changes"] != 1:
        invalid.append(f"プロセスの切り替えが起動時の 1 回でない({c['process_changes']})")
    if len(profiles) != 1:
        invalid.append(f"[focus-scope] bootstrap の行が1件でない({len(profiles)})")
    elif profiles[0] != expect_profile:
        failures.append(f"起動時の profile={profiles[0]}(期待 {expect_profile})")
    if c["read"]:
        symptoms.append(f"drift_correction_read={c['read']}(Blind であるべき)")
    if not c["drift"]:
        invalid.append("窓を閉じる前の drift 補正 0 件(観測経路に乗っていない)")
    else:
        if any(n > BLIND_MAX_ATTEMPTS for n in c["bursts"]):
            symptoms.append(f"連続した補正が {BLIND_MAX_ATTEMPTS} 回を超えた bursts={c['bursts']}")
        if not gave_up_t:
            symptoms.append("drift 補正の後に gave up が無い(止まっていない)")
    if c["rearm"]:
        symptoms.append(f"give-up 後の再武装が {c['rearm']} 回")
    elif gave_up_t and not (t_second >= gave_up_t[0] + REARM_COOLDOWN_S and t_close >= t_second + REARM_TAIL_S):
        invalid.append("再武装を確かめる窓が無い(2 回目のきっかけがクールダウン明けでない/その後すぐ閉じた)")
    failures += symptoms
    verdict = "FAIL" if failures else "INVALID" if invalid else "PASS"
    return {"verdict": verdict, "reproduced": bool(symptoms), "counts": c, "failures": failures, "invalid": invalid}
