# lang-detect-model

入力言語の判別(日本語 / 非日本語)を、**識別モデル**(ロジスティック回帰、比較用に小さな RNN)で作り直すための道具。ADR-280 の後続。
`src/lang_detect.rs` の今の判定器は、仮名 n-gram と英語のラテン文字 2 連の対数尤度比で、日本語側が鋭く英語側が緩い非対称がある
(実機の誤発火の例: 2026-11-01 14:33 JST、`ccndie` = すすめてくた が llr=-14.6 で英数と判定された)。

データ作り(`ldm.fetch`・`ldm.build_dataset`)と、**追加式のロジスティック回帰**(`ldm.lr`)の学習・評価。学習データ・重みはリポジトリに置かない。

```sh
cd tools/lang-detect-model
uv sync
uv run python -m ldm.fetch                  # 標本のダウンロード(約 0.6GB。~/.cache/awase-lang-detect-model/raw)
uv run python -m ldm.build_dataset          # エピソードの生成(約 40 秒、38MB。同 data/episodes.tsv と summary.json)
uv run pytest                               # キー格子・NICOLA 未シフト面・今の判定器の移植(Rust と一致)・特徴の検査
uv sync --group model                       # torch(CPU)。モデルの学習・評価に必要(約 0.4GB)
uv run --group model python -u run_ablation.py   # 基準 + LR の特徴の足し引き(約 25 分。混んだ環境で)
uv run --group model python -u run_final.py      # 最終候補の学習・保存・領域をまたぐ評価(約 5 分)
uv run --group model python -m ldm.export_lr     # 保存した重みを int8 にして data/lang_lr_v1.txt と tests/fixtures/lang_lr_vectors.tsv に書く(Rust の shadow が読む)
uv run python -m ldm.shadow_report awase.log     # 実機のログから、今の判定器と shadow(LR)の食い違いを数える
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

## モデルと評価

**基準**: 今の判定器(`TrigramInterp` + `LatinBackground::embedded()`)の Python 移植 `ldm/baseline.py`。
実機の誤発火 `ccndie` の各打鍵の増分が Rust(CI のプローブ #922)と 0.02 以内で一致することを `tests/test_baseline.py` で固定している。

**LR**(`ldm/lr.py`): 打鍵ごとの特徴(キーの 1 連・2 連・3 連、直前の文脈に開始記号 BOS=文頭/語頭・MID=語の途中)の重みの**和**が累積の log-odds(日本語)。
検出器の `llr += 増分` にそのまま差し替えられる形。学習は全プレフィックスの損失(1 エピソード = 重み 1 を各打鍵に等分)、
クラス質量は ja 0.5・en 0.2・other 0.15・code 0.15。重みは約 11.5 万個(float32 で約 450KB、int8 で約 110KB)。

**評価**: 尺度の違うモデルを同じ軸に載せるため、**日本語を NotJapanese に確定してしまう率(FPR)を固定して、非日本語の検出率(TPR)を比べる**。
閾値は FPR が目標以下になる最小の値。数値は test 分割(135,302 行。学習・dev に使っていない文・記事・ファイル)。

| FPR | モデル | en | other | code | en の平均打鍵 |
|---|---|---|---|---|---|
| ≤0.5% | 基準(今の判定器) | 56.5% | 54.5% | 67.5% | 3.5 |
| ≤0.5% | LR 1 連+2 連 | 74.1% | 68.9% | 94.4% | 2.7 |
| ≤0.5% | LR 1〜3 連 | 79.1% | 77.3% | 96.2% | 2.5 |
| ≤0.5% | **LR 1〜3 連 + BOS** | **81.8%** | **77.4%** | **96.6%** | 2.5 |
| ≤0.5% | 同(`run_final.py`、6 エポック) | 83.2% | 78.0% | 96.2% | 2.5 |
| ≤1% | 基準 | 62.8% | 59.5% | 69.5% | 3.3 |
| ≤1% | **LR 1〜3 連 + BOS** | **86.2%** | **80.7%** | **97.4%** | 2.4 |
| ≤1% | 同(`run_final.py`) | 86.7% | 81.0% | 97.0% | 2.4 |
| ≤0.5% | GRU hidden 32、12 エポック | 79.5% | 76.2% | 96.9% | 2.5 |
| ≤0.5% | GRU hidden 64、12 エポック | 79.8% | 77.9% | 97.3% | 2.5 |
| ≤1% | GRU hidden 32、12 エポック | 87.6% | 82.1% | 97.7% | 2.3 |
| ≤1% | GRU hidden 64、12 エポック | 87.7% | 83.7% | 98.0% | 2.3 |

- 3 連が 2 連に足す効果が大きく(en +5pt)、BOS(語頭・文頭の文脈)は +2〜3pt。1 連だけでは足りない(en 33〜41%)。
- **領域をまたぐ**(Wikipedia + repo + 他言語で学習 → Tatoeba の ja/en で評価。基準は同じ test): FPR≤0.5% で en 47.7% → **68.9%**、≤1% で 59.6% → **73.9%**。
  同じ領域より下がるが、基準よりは大きく上。
- 実機の誤発火 `ccndie`(すすめてくた)の累積 log-odds(日本語): 基準は **−14.6**(英数と確定)、LR+BOS は `[-0.35, 1.38, 2.33, 1.74, 1.49, -2.14]`
  (閾値 3.5〜4.5 に届かない)。`hello` は −4.7、`windows` は −7.8、`git` は −5.6 で確定する。

**GRU との比較**(`ldm/gru.py`、`run_gru.py`): 小さな GRU(embedding 16 + GRU 32 または 64 + 線形、開始記号を先頭に入力)でも、LR とほぼ同じ。
dev の損失は LR 0.2512 に対し GRU(32)0.2516・GRU(64)0.2498。FPR≤1% で en +1〜1.5pt、other +1〜3pt、code +0.6〜1pt、≤0.5% では en が LR より低い。
差は数 pt 以内で、追加式でない(`llr += 増分` に差し替えられない)・Rust への移植が重いことを考えると、**RNN を採る理由は無い**。LR を採る。
GRU は開発機の仮想 CPU(AVX2・FMA 無し)で SIGILL になるため、AVX2 のある実機(dragonflyg4、BelowNormal・4 スレッド、32 は 25 秒/エポック)で学習した。

### 限界(重要)
- **ラテン文字の語は Wikipedia/Tatoeba の書き言葉**。コマンド・チャット・URL の語の分布は未測定で、`code` は**このリポジトリのファイル**(別のファイルだが似た識別子)で評価している。
  実際の非日本語の検出率は、この表より低い可能性が高い。
- 日本語の評価は読み付き実文(Wikipedia・Tatoeba)の**未シフト面の区間**。長さの分布(1 打が約 55%)が実際の打鍵と合うかは、実機の journal で未確認。
- 閾値(log-odds で約 3.5〜4.5)は、クラス質量(ja 0.5)に依存する。実機の本当の事前確率は分かっていない。
- Rust への組み込み(重みの出力、`KanaModel` の置き換え、固定小数点化、実機の journal との突き合わせ)は未実施。
