"""エピソード(親指なしの打鍵列)の学習・評価データを作る。

    uv run python -m ldm.build_dataset [--ja-articles 20000] [--tatoeba-cap 100000]

出力: `$AWASE_LDM_CACHE/data/episodes.tsv` と `summary.json`(リポジトリには置かない)。
1 行 = 1 エピソード: `split<TAB>source<TAB>label<TAB>start<TAB>keys`
- `split`: train / dev / test。**文(または記事・ファイル)の ID のハッシュ**で決める(同じ文の別の開始位置が分かれて漏れない)。
- `source`: wiki / tatoeba / repo。元ごとに分けて評価できるようにする(Wikipedia で学習して Tatoeba の口語で試す、など)。
- `label`: ja / en / other(fra, spa, deu, ita, por) / code
- `start`: sentence(文の先頭の語) / word(文中の語頭) / mid(語の途中から)。
- `keys`: QWERTY の物理位置の文字列(最大 12 打)。空白(Space)は『除外キー』で列を切るので、ラテン文字は 1 語が 1 エピソードの上限。

日本語は Sudachi(mode C)で読みを付け、NICOLA の未シフト面で打てる仮名が続く区間を取る(親指が要る仮名に着いたら止まる)。
**限界**: 変換の Space で区切る習慣・修飾キーを使うコード・大文字の Shift は反映していない。ラテン文字の語は Wikipedia/Tatoeba の
書き言葉の語で、実際のコマンド・識別子とは分布が違う。
"""
from __future__ import annotations

import argparse
import bz2
import hashlib
import json
import random
import subprocess
from collections import Counter
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

import pyarrow.parquet as pq

from .common import CACHE, ROOT, RUN_CAP, japanese_run, katakana_to_hiragana, latin_key_string, latin_tokens

RAW = CACHE / "raw"
OUT = CACHE / "data"
OTHER_LANGS = ["fra", "spa", "deu", "ita", "por"]


def digest(*parts: str) -> int:
    return int.from_bytes(hashlib.sha1(":".join(parts).encode()).digest()[:8], "big")


def split_of(source: str, doc_id: str) -> str:
    h = digest(source, doc_id) % 10
    return "train" if h < 8 else "dev" if h == 8 else "test"


# ---- 文の供給 ----

def tatoeba(lang: str, cap: int):
    path = RAW / f"tatoeba-{lang}.tsv.bz2"
    with bz2.open(path, "rt", encoding="utf-8") as f:
        for i, line in enumerate(f):
            if i >= cap:
                break
            parts = line.rstrip("\n").split("\t")
            if len(parts) >= 3:
                yield parts[0], parts[-1]


def wiki(path: Path, articles: int, per_article: int, ja: bool):
    pf = pq.ParquetFile(path)
    seen = 0
    for g in range(pf.metadata.num_row_groups):
        tbl = pf.read_row_group(g, columns=["id", "text"])
        for aid, text in zip(tbl["id"].to_pylist(), tbl["text"].to_pylist()):
            if seen >= articles:
                return
            seen += 1
            sents = []
            for para in text.split("\n"):
                para = para.strip()
                if not para:
                    continue
                if ja:
                    sents += [s + "。" for s in para.split("。") if 4 <= len(s) <= 120]
                else:
                    sents += [s.strip() for s in para.replace("? ", ". ").replace("! ", ". ").split(". ") if 15 <= len(s) <= 200]
            rng = random.Random(digest("wiki", aid))
            rng.shuffle(sents)
            for k, s in enumerate(sents[:per_article]):
                yield f"{aid}:{k}", s, aid


# ---- エピソードの抽出 ----

def latin_episodes(text: str, rng: random.Random) -> list[tuple[str, str]]:
    toks = [latin_key_string(t) for t in latin_tokens(text)]
    toks = [t for t in toks if t]
    if not toks:
        return []
    out = [("sentence", toks[0][:RUN_CAP])]
    for t in rng.sample(toks[1:], min(2, len(toks) - 1)):
        out.append(("word", t[:RUN_CAP]))
    long = [t for t in toks if len(t) >= 4]
    if long and rng.random() < 0.3:
        t = rng.choice(long)
        out.append(("mid", t[rng.randrange(1, len(t) - 1):][:RUN_CAP]))
    return [(s, k) for s, k in out if k]


_TOKENIZER = None


def _tokenizer():
    global _TOKENIZER
    if _TOKENIZER is None:
        from sudachipy import dictionary, tokenizer

        _TOKENIZER = (dictionary.Dictionary().tokenizer(mode=tokenizer.Tokenizer.SplitMode.C), tokenizer.Tokenizer.SplitMode.C)
    return _TOKENIZER


def ja_episodes(text: str, seed: int) -> list[tuple[str, str]]:
    tok, mode = _tokenizer()
    reading, bounds = "", []
    for m in tok.tokenize(text, mode):
        surface = m.surface()
        r = m.reading_form()
        if surface.isascii():
            piece = "\0"  # 英数字は日本語の打鍵ではない(列を切る)
        else:
            piece = katakana_to_hiragana(r) if r and r != "キゴウ" else surface
        bounds.append(len(reading))
        reading += piece
    if not reading:
        return []
    rng = random.Random(seed)
    out = []
    starts = [(0, "sentence")]
    for b in rng.sample(bounds[1:], min(2, max(0, len(bounds) - 1))):
        starts.append((b, "word"))
    if len(reading) > 4 and rng.random() < 0.3:
        mid = rng.randrange(1, len(reading) - 1)
        if mid not in bounds:
            starts.append((mid, "mid"))
    for s, kind in starts:
        run = japanese_run(reading, s)
        if run:
            out.append((kind, run))
    return out


def _ja_chunk(items):
    return [(sid, src, split, ja_episodes(text, digest("ja", sid))) for sid, src, split, text in items]


def chunks(seq, n):
    for i in range(0, len(seq), n):
        yield seq[i : i + n]


def repo_episodes(cap: int):
    files = subprocess.run(["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True).stdout.split("\n")
    skip = ("tests/fixtures", "data/", "tools/lang-detect-eval-fixture", "docs/tasks", "docs/adr", ".lock", "Cargo.lock")
    n = 0
    for fn in sorted(files):
        if not fn.endswith((".rs", ".py", ".toml", ".ts", ".yml", ".md")) or any(s in fn for s in skip):
            continue
        try:
            text = (ROOT / fn).read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        rng = random.Random(digest("repo", fn))
        toks = [latin_key_string(t) for t in latin_tokens(text) if t.isascii()]
        toks = [t for t in toks if len(t) >= 2]
        for t in rng.sample(toks, min(200, len(toks))):
            yield fn, t[:RUN_CAP]
            n += 1
            if n >= cap:
                return


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--ja-articles", type=int, default=20000)
    ap.add_argument("--en-articles", type=int, default=20000)
    ap.add_argument("--per-article", type=int, default=3)
    ap.add_argument("--tatoeba-cap", type=int, default=100000)
    ap.add_argument("--other-cap", type=int, default=30000)
    ap.add_argument("--workers", type=int, default=4)
    a = ap.parse_args()

    OUT.mkdir(parents=True, exist_ok=True)
    rows: list[tuple[str, str, str, str, str]] = []

    # 日本語(Sudachi は重いので並列)
    ja_items = []
    for sid, text in tatoeba("jpn", a.tatoeba_cap):
        ja_items.append((sid, "tatoeba", split_of("tatoeba", sid), text))
    for sid, text, aid in wiki(RAW / "jawiki-20231101-00014.parquet", a.ja_articles, a.per_article, ja=True):
        ja_items.append((sid, "wiki", split_of("wiki", aid), text))
    print(f"ja sentences: {len(ja_items)}", flush=True)
    with ProcessPoolExecutor(a.workers) as ex:
        for res in ex.map(_ja_chunk, list(chunks(ja_items, 2000))):
            for _sid, src, split, eps in res:
                rows += [(split, src, "ja", kind, keys) for kind, keys in eps]

    # ラテン文字(英語・他言語・コード)
    def add_latin(label, src, split, text, sid):
        rng = random.Random(digest(label, sid))
        rows.extend((split, src, label, kind, keys) for kind, keys in latin_episodes(text, rng))

    for sid, text in tatoeba("eng", a.tatoeba_cap):
        add_latin("en", "tatoeba", split_of("tatoeba", sid), text, sid)
    for sid, text, aid in wiki(RAW / "enwiki-20231101-00040.parquet", a.en_articles, a.per_article, ja=False):
        add_latin("en", "wiki", split_of("wiki", aid), text, sid)
    for lang in OTHER_LANGS:
        for sid, text in tatoeba(lang, a.other_cap):
            add_latin("other", "tatoeba", split_of("tatoeba", f"{lang}{sid}"), text, f"{lang}{sid}")
    for fn, keys in repo_episodes(60000):
        rows.append((split_of("repo", fn), "repo", "code", "word", keys))

    with (OUT / "episodes.tsv").open("w", encoding="utf-8", newline="\n") as f:
        for r in rows:
            f.write("\t".join(r) + "\n")
    summary = {
        "rows": len(rows),
        "by_split_source_label": dict(Counter(f"{s}/{src}/{l}" for s, src, l, _k, _x in rows)),
        "by_start": dict(Counter(f"{l}/{k}" for _s, _src, l, k, _x in rows)),
        "mean_len": {l: round(sum(len(x) for _s, _src, ll, _k, x in rows if ll == l) / max(1, sum(1 for r in rows if r[2] == l)), 2) for l in ("ja", "en", "other", "code")},
    }
    (OUT / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=1), encoding="utf-8")
    print(json.dumps(summary, ensure_ascii=False, indent=1))


if __name__ == "__main__":
    main()
