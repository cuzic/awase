"""標本のダウンロード。`AWASE_LDM_CACHE`(既定 `~/.cache/awase-lang-detect-model`)の `raw/` に置く。リポジトリには置かない。

    uv run python -m ldm.fetch            # 全部
    uv run python -m ldm.fetch tatoeba    # Tatoeba(CC0 の部分集合)だけ

元と使用条件:
- Tatoeba の言語別の文(`per_language/<言語>/<言語>_sentences.tsv.bz2`): 多くは CC BY 2.0 FR(帰属表示が要る)。
  口語の短い文が多い。CC0 の部分集合(`sentences_CC0`)は日本語が 2 文しかなく使えない。リポジトリには文を置かず、統計量だけにする。
- 日本語/英語 Wikipedia(Hugging Face `wikimedia/wikipedia`、20231101 のスナップショットの 1 シャード): CC BY-SA 4.0。
  既存の `data/ngram_hiragana.csv.gz` も日本語 Wikipedia 由来なので扱いは同じ。学習したモデルの重みは文を含まない統計量だけにする。
ディスクが小さい環境のため、Wikipedia は各言語 1 シャード(約 0.2〜0.4GB)だけ。
"""
from __future__ import annotations

import hashlib
import sys
import urllib.request
from pathlib import Path

from .common import CACHE

RAW = CACHE / "raw"
TATOEBA_LANGS = ["jpn", "eng", "fra", "spa", "deu", "ita", "por"]
HF = "https://huggingface.co/datasets/wikimedia/wikipedia/resolve/main"
SOURCES = {
    "tatoeba": [
        (f"tatoeba-{lang}.tsv.bz2", f"https://downloads.tatoeba.org/exports/per_language/{lang}/{lang}_sentences.tsv.bz2")
        for lang in TATOEBA_LANGS
    ],
    "wiki": [
        ("jawiki-20231101-00014.parquet", f"{HF}/20231101.ja/train-00014-of-00015.parquet"),
        ("enwiki-20231101-00040.parquet", f"{HF}/20231101.en/train-00040-of-00041.parquet"),
    ],
}


def download(name: str, url: str) -> Path:
    RAW.mkdir(parents=True, exist_ok=True)
    dest = RAW / name
    if dest.exists() and dest.stat().st_size > 0:
        print(f"skip (exists): {dest} {dest.stat().st_size} bytes")
        return dest
    tmp = dest.with_suffix(dest.suffix + ".part")
    h = hashlib.sha256()
    with urllib.request.urlopen(url, timeout=60) as r, tmp.open("wb") as f:
        while chunk := r.read(1 << 20):
            f.write(chunk)
            h.update(chunk)
    tmp.rename(dest)
    print(f"downloaded: {dest} {dest.stat().st_size} bytes sha256={h.hexdigest()}")
    return dest


def main(argv: list[str]) -> None:
    names = argv or list(SOURCES)
    for key in names:
        for name, url in SOURCES[key]:
            download(name, url)


if __name__ == "__main__":
    main(sys.argv[1:])
