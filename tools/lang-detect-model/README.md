# lang-detect-model

入力言語の判別(日本語 / 非日本語)を、**識別モデル**(ロジスティック回帰、比較用に小さな RNN)で作り直すための道具。ADR-280 の後続。
`src/lang_detect.rs` の今の判定器は、仮名 n-gram と英語のラテン文字 2 連の対数尤度比で、日本語側が鋭く英語側が緩い非対称がある
(実機の誤発火の例: 2026-11-01 14:33 JST、`ccndie` = すすめてくた が llr=-14.6 で英数と判定された)。

この PR は**データ作りだけ**(モデルは次の PR)。学習データはリポジトリに置かない。

```sh
cd tools/lang-detect-model
uv sync
uv run python -m ldm.fetch                  # 標本のダウンロード(約 0.6GB。~/.cache/awase-lang-detect-model/raw)
uv run python -m ldm.build_dataset          # エピソードの生成(約 40 秒、38MB。同 data/episodes.tsv と summary.json)
uv run pytest                               # キー格子・NICOLA 未シフト面・読みの変換の検査
```
`AWASE_LDM_CACHE` で置き場所を変えられる。

## データ
| 元 | ラベル | 条件 |
|---|---|---|
| 日本語 Wikipedia(20231101 の 1 シャード) | ja | CC BY-SA 4.0。既存の `data/ngram_hiragana.csv.gz` も日本語 Wikipedia 由来 |
| Tatoeba(jpn) | ja | 多くは CC BY 2.0 FR(帰属表示)。**口語の短い文**。CC0 の部分集合は日本語が 2 文しかなく使えない |
| 英語 Wikipedia(同、1 シャード)・Tatoeba(eng) | en | 同上 |
| Tatoeba(fra, spa, deu, ita, por) | other | 同上 |
| このリポジトリの `*.rs` `*.py` `*.md` など | code | 著作権者が同じ。`tests/fixtures`・`data/`・`docs/adr` などは除く |

リポジトリにはデータも文も置かない。学習後の重みは統計量だけにする。

## エピソード
`src/lang_detect.rs` の判定器が見るのは、**親指なしで続く未シフト面の打鍵**(JIS の QWERTY 格子の物理位置)。
- 日本語: Sudachi(mode C)で読みを付け、NICOLA の未シフト面(`layout/nicola.yab`)で打てる仮名が続く区間。濁点・小書き・`ー` など親指が要る文字で止まる。
- ラテン文字: 空白(Space)は『除外キー』で列を切るので、**1 語 = 1 エピソード**(最大 12 打)。
- 開始位置 `start`: `sentence`(文の先頭の語)/ `word`(文中の語頭)/ `mid`(語の途中から)。語頭・行頭を考慮するモデルの入力に使う。
- 分割 `split`(train 80% / dev 10% / test 10%)は、文・記事・ファイルの ID のハッシュ。元 `source`(wiki / tatoeba / repo)は列に残すので、
  『Wikipedia で学習して Tatoeba の口語で試す』ができる。今のフィクスチャ(`tests/fixtures/lang_detect_eval.tsv`)の日本語は判定器自身の分布から作った
  循環だったが、こちらは独立した実文。

## 最初の観察(2026-11-01、全体の集計)
- **日本語のエピソードは短い**: NICOLA の未シフト面で続く区間は、1 打が約 55%(tatoeba 59%、wiki 54%)、2 打が 23%、**5 打以上は 5〜8%**。
  英語は 1 語 4.5 打(平均)、コードは 6.7 打。つまり日本語は決める前に区間が終わることが多く、長く続いたときだけ誤判定の機会がある。
- 行数: 1,358,159(ja 28 万、en 52 万、other 49 万、code 6 万)。

## 限界
- 変換の Space で区切る習慣・修飾キー・Shift つきの大文字は反映していない。
- ラテン文字の語は Wikipedia/Tatoeba の書き言葉で、実際のコマンド・識別子・チャットの語とは分布が違う。
- 日本語の読みは Sudachi の `reading_form`。数字・英字は列を切る扱い(英数字として打つ想定)。
- 利用者の実際の打鍵は含まない。
