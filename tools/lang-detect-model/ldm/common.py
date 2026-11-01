"""物理キーの格子と、文字列 → キー列の変換。

`src/lang_detect.rs` の判定器が見るのは、親指なしで続く**未シフト面の打鍵**(JIS の QWERTY 格子の物理位置)。
ここでは同じ位置を QWERTY の文字で表す(`tests/lang_detect_eval.rs` の `GRID`、`gen_fixture.py` の `GRID` と同じ)。
"""
from __future__ import annotations

import os
import re
import unicodedata
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CACHE = Path(os.environ.get("AWASE_LDM_CACHE", Path.home() / ".cache" / "awase-lang-detect-model"))

GRID = ["1234567890-^\\", "qwertyuiop@[", "asdfghjkl;:]", "zxcvbnm,./"]
POS = {ch: (r, c) for r, row in enumerate(GRID) for c, ch in enumerate(row)}
RUN_CAP = 12

# nicola.yab の未シフト面に現れるローマ字→仮名(`gen_fixture.py` の ROMAJI_KANA と同じ)
_ROMAJI_KANA = {
    "ka": "か", "ta": "た", "ko": "こ", "sa": "さ", "ra": "ら", "ti": "ち", "ku": "く", "tu": "つ",
    "u": "う", "si": "し", "te": "て", "ke": "け", "se": "せ", "ha": "は", "to": "と", "ki": "き",
    "i": "い", "nn": "ん", "hi": "ひ", "su": "す", "fu": "ふ", "he": "へ", "me": "め", "so": "そ",
    "ne": "ね", "ho": "ほ",
}


def _fw(s: str) -> str:
    return "".join(chr(ord(c) - 0xFEE0) if 0xFF01 <= ord(c) <= 0xFF5E else c for c in s)


def load_unshifted_face(yab: Path | None = None) -> dict[str, str]:
    """未シフト面で打てる文字(仮名・`、` `。` `，` `．` `／` `－` と数字)→ QWERTY の文字。

    打てない文字(濁点・小書き・`ー` など親指が要るもの)は含めない。
    """
    text = (yab or ROOT / "layout" / "nicola.yab").read_text(encoding="utf-8")
    body = text.split("[ローマ字シフト無し]")[1].split("[")[0]
    rows = [ln.strip() for ln in body.splitlines() if ln.strip() and not ln.strip().startswith(";")]
    out: dict[str, str] = {}
    for r, ln in enumerate(rows):
        for c, cell in enumerate(ln.split(",")):
            cell = cell.strip()
            key = GRID[r][c] if r < len(GRID) and c < len(GRID[r]) else None
            if key is None or cell == "無":
                continue
            if cell.startswith("'"):
                out[_fw(cell.strip("'"))] = key
            else:
                kana = _ROMAJI_KANA.get(_fw(cell))
                if kana:
                    out[kana] = key
    return out


UNSHIFTED = load_unshifted_face()


def katakana_to_hiragana(s: str) -> str:
    return "".join(chr(ord(c) - 0x60) if "ァ" <= c <= "ヶ" else c for c in s)


def japanese_run(reading: str, start: int, cap: int = RUN_CAP) -> str:
    """読み(ひらがな)の `start` から、未シフト面で続く打鍵列。親指が要る文字に着いたら止める。"""
    out = []
    for ch in reading[start:]:
        key = UNSHIFTED.get(ch)
        if key is None or len(out) >= cap:
            break
        out.append(key)
    return "".join(out)


_LATIN_OK = set(POS)


def latin_key_string(token: str) -> str:
    """ラテン文字の語(アクセント付きも)→ 格子の文字列。格子に無い文字は落とす(空白など列を切る文字は呼び出し側)。"""
    s = unicodedata.normalize("NFKD", token).lower()
    s = "".join(c for c in s if not unicodedata.combining(c))
    return "".join(c for c in s if c in _LATIN_OK)


_SPLIT = re.compile(r"\s+")


def latin_tokens(text: str) -> list[str]:
    """空白で区切った語。Space は『除外キー』で列を切るので、語 = 1 エピソードの上限。"""
    return [t for t in _SPLIT.split(text) if t]
