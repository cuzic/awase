"""ADR-280 D1: 入力言語の判定器(`src/lang_detect.rs`)の評価用フィクスチャを、固定 seed で決定的に生成する。

    uv run -I tools/lang-detect-eval-fixture/gen_fixture.py            # tests/fixtures/lang_detect_eval.tsv を書き、指標を表示
    uv run -I tools/lang-detect-eval-fixture/gen_fixture.py --check    # 書かずに、今のファイルと再生成が一致するか確かめる

標準ライブラリだけを使う。データの元は `data/ngram_hiragana.csv.gz`(日本語の仮名 2 連)と `layout/nicola.yab`(未シフト面)、
このスクリプト自身に書いた英単語・コードの断片だけ。利用者の本文や外部のコーパスは含めない。
"""
import gzip
import math
import random
import sys
from collections import defaultdict
from pathlib import Path

SEED = 280
ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "tests" / "fixtures" / "lang_detect_eval.tsv"

N_JA_SENTENCES = 800
STARTS_PER_SENTENCE = 4
N_EN, N_CODE, N_RAND = 1000, 500, 500
RUN_CAP = 12

# 行ごとの QWERTY の並び(yab の行・列。JIS)。tests/lang_detect_eval.rs の GRID と同じ。
GRID = ["1234567890-^\\", "qwertyuiop@[", "asdfghjkl;:]", "zxcvbnm,./"]
POS = {ch: (r, c) for r, row in enumerate(GRID) for c, ch in enumerate(row)}

# nicola.yab の未シフト面に現れるローマ字→仮名(`KanaTable` のうち未シフト面で使うぶん)
ROMAJI_KANA = {
    "ka": "か", "ta": "た", "ko": "こ", "sa": "さ", "ra": "ら", "ti": "ち", "ku": "く", "tu": "つ",
    "u": "う", "si": "し", "te": "て", "ke": "け", "se": "せ", "ha": "は", "to": "と", "ki": "き",
    "i": "い", "nn": "ん", "hi": "ひ", "su": "す", "fu": "ふ", "he": "へ", "me": "め", "so": "そ",
    "ne": "ね", "ho": "ほ",
}


def fw(s):  # 全角英数→半角
    return "".join(chr(ord(c) - 0xFEE0) if 0xFF01 <= ord(c) <= 0xFF5E else c for c in s)


def load_face():
    """未シフト面を (行, 列) -> 仮名 / 'SYM'(仮名以外で定義済み) / None(無) で返す"""
    text = (ROOT / "layout" / "nicola.yab").read_text(encoding="utf-8")
    body = text.split("[ローマ字シフト無し]")[1].split("[")[0]
    rows = [ln.strip() for ln in body.splitlines() if ln.strip() and not ln.strip().startswith(";")]
    face = {}
    for r, ln in enumerate(rows):
        for c, cell in enumerate(ln.split(",")):
            cell = cell.strip()
            if cell == "無":
                face[(r, c)] = None
            elif cell.startswith("'"):
                face[(r, c)] = "SYM"
            else:
                face[(r, c)] = ROMAJI_KANA.get(fw(cell), "SYM")  # 後・逃などはここで SYM
    return face


FACE = load_face()
KEY_COUNT = sum(1 for v in FACE.values() if v is not None)
KANA_OF = {POS[ch]: FACE[POS[ch]] for ch in POS if FACE.get(POS[ch]) not in (None, "SYM")}
KEY_OF_KANA = {KANA_OF[POS[ch]]: ch for ch in POS if POS[ch] in KANA_OF}

# ---- 仮名 n-gram(`KanaModel::new` と同じ作り) ----
FREQ = {}
for line in gzip.open(ROOT / "data" / "ngram_hiragana.csv.gz", "rt", encoding="utf-8"):
    line = line.strip()
    if not line or line.startswith("#"):
        continue
    kind, chars, score = line.split(",")
    if kind == "2" and len(chars) == 2:
        FREQ[(chars[0], chars[1])] = 10 ** float(score)
LEFT, LISTED, VOCAB = defaultdict(float), defaultdict(int), set()
for (a, b), f in FREQ.items():
    LEFT[a] += f
    LISTED[a] += 1
    VOCAB |= {a, b}
FLOOR = min(FREQ.values()) * 0.1
TOTAL = sum(LEFT.values())


def delta(prev, pos):
    """`KanaModel::delta` の再現(#819 後: 仮名に読めないキーは 0)"""
    tok = KANA_OF.get(pos)
    if tok is None or tok not in VOCAB:
        return 0.0
    v = len(VOCAB)
    if prev is not None and prev in VOCAB:
        den = LEFT.get(prev, 0.0) + (v - LISTED[prev]) * FLOOR
        ja = math.log(FREQ.get((prev, tok), FLOOR) / den)
    else:
        ja = math.log((LEFT.get(tok, 0.0) + FLOOR) / (v * FLOOR + TOTAL))
    return ja + math.log(KEY_COUNT)


def detect(keys, ja_th=4.0, en_th=8.0, max_keys=7):
    """`EpisodeDetector`(親指なし)の再現。'J' / 'N'(NotJapanese) / 'U'(Undecided) / 'P'(列が尽きて Pending)と打鍵数"""
    llr, prev, n = 0.0, None, 0
    for ch in keys:
        pos = POS[ch]
        n += 1
        llr += delta(prev, pos)
        prev = KANA_OF.get(pos)
        if llr >= ja_th:
            return "J", n
        if llr <= -en_th:
            return "N", n
        if n >= max_keys:
            return "U", n
    return "P", n


# ---- 生成 ----
EN_WORDS = """the be to of and a in that have it for not on with he as you do at this but his by from they we say her she or an will
my one all would there their what so up out if about who get which go me when make can like time no just him know take people into
year your good some could them see other than then now look only come its over think also back after use two how our work first well
way even new want because any these give day most us file open close save text name list read write home page search help""".split()
CODE_PARTS = """git status commit push pull branch merge cargo test build run check clippy pnpm dev src/main.rs docs/readme.md
import os sys json const let var return function class public static void self cd.. ls-la cat tmp/log.txt x[0] y[1] a.b.c foo;bar
std::io user@host ssh-key key-value open-file test-case 192.168.0.1 8080 v1.2.3 2026-10-24 http://localhost:3000 /usr/bin/env
""".split()
PUNCT = {"、": "@", "。": "q"}


def gen_ja(rng):
    """ngram の分布から仮名列を作り、文中のあちこちから打ち始めたときの『親指なしで続く未シフト面の区間』を返す"""
    lefts = sorted(LEFT.items())
    nexts = defaultdict(list)
    for (a, b), f in sorted(FREQ.items()):
        nexts[a].append((b, f))
    lines = []
    for _ in range(N_JA_SENTENCES):
        # 文節ごとに作る: 文節の頭は仮名の出現頻度から独立に引き、中は 2 連の条件付き分布で続ける。
        # 全体を 1 本の 2 連の連鎖にすると、モデル自身が最も好む列ばかりになり、誤判定率が楽観的になりすぎる。
        toks = []
        while len(toks) < rng.randint(8, 30):
            toks.append(rng.choices([k for k, _ in lefts], [w for _, w in lefts])[0])
            for _ in range(rng.randint(1, 4)):
                cand = nexts.get(toks[-1]) or lefts
                toks.append(rng.choices([k for k, _ in cand], [w for _, w in cand])[0])
        # 読点・句点を挟む(未シフト面の `、` `。`。仮名に読めない記号の中立性の検査になる)
        stream = []
        for t in toks:
            stream.append(t)
            r = rng.random()
            if r < 0.06:
                stream.append("、")
            elif r < 0.09:
                stream.append("。")
        keyseq = [PUNCT.get(t) or KEY_OF_KANA.get(t) for t in stream]  # None = 親指が要る仮名
        for _ in range(STARTS_PER_SENTENCE):
            s = rng.randrange(len(keyseq))
            run = []
            for k in keyseq[s:]:
                if k is None or len(run) >= RUN_CAP:
                    break
                run.append(k)
            lines.append(("ja", "".join(run)))
    return lines


def gen_en(rng):
    out = []
    for _ in range(N_EN):
        s = "".join(rng.choice(EN_WORDS) for _ in range(rng.randint(2, 4)))
        out.append(("en", s[:RUN_CAP]))
    for _ in range(N_CODE):
        s = "".join(rng.choice(CODE_PARTS) for _ in range(rng.randint(1, 3)))
        s = "".join(ch for ch in s if ch in POS)
        out.append(("code", s[:RUN_CAP]))
    allk = list(POS)
    for _ in range(N_RAND):
        out.append(("rand", "".join(rng.choice(allk) for _ in range(rng.randint(3, 10)))))
    return out


def build():
    rng = random.Random(SEED)
    rows = gen_ja(rng) + gen_en(rng)
    return "".join(f"{label}\t{keys}\n" for label, keys in rows)


def report(text):
    by = defaultdict(list)
    for ln in text.splitlines():
        label, _, keys = ln.partition("\t")
        by[label].append(keys)
    print(f"key_count={KEY_COUNT} vocab={len(VOCAB)}  rows: " + ", ".join(f"{k}={len(v)}" for k, v in by.items()))
    for th in (8, 14, 20):
        r = [detect(k, en_th=th)[0] if k else "-" for k in by["ja"]]
        print(f"ja->NotJapanese  en_threshold={th:>2}: {r.count('N') / len(r):.4f}  (empty runs={r.count('-')})")
    for label in ("en", "code", "rand"):
        for th in (8, 14):
            r = [detect(k, en_th=th) for k in by[label]]
            nn = [n for v, n in r if v == "N"]
            print(f"{label:>4} released as NotJapanese en_threshold={th:>2}: {len(nn) / len(r):.4f} (mean presses {sum(nn) / max(1, len(nn)):.2f})")
        r = [detect(k)[0] for k in by[label]]
        print(f"{label:>4} -> Japanese(Likelihood): {r.count('J') / len(r):.4f}")


if __name__ == "__main__":
    text = build()
    if "--check" in sys.argv:
        same = OUT.exists() and OUT.read_text(encoding="utf-8") == text
        print("fixture is up to date" if same else "fixture DIFFERS from regenerated output")
        sys.exit(0 if same else 1)
    OUT.write_text(text, encoding="utf-8", newline="\n")
    print(f"wrote {OUT} ({len(text)} bytes)")
    report(text)
