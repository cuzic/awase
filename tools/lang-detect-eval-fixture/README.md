# lang-detect-eval-fixture

ADR-280 D1。入力言語の判定器(`src/lang_detect.rs`)の誤り率を測るフィクスチャ `tests/fixtures/lang_detect_eval.tsv` の生成手順。
率を固定するテストは `tests/lang_detect_eval.rs`。

```sh
uv run --no-project python -I tools/lang-detect-eval-fixture/gen_fixture.py          # 再生成して指標を表示
uv run --no-project python -I tools/lang-detect-eval-fixture/gen_fixture.py --check  # 今のファイルと一致するか
```

- seed は `280`(`gen_fixture.py` の `SEED`)。標準ライブラリだけ(依存なし)。
- 元データは `data/ngram_hiragana.csv.gz`(リポジトリ既存の仮名 2 連)、`layout/nicola.yab`(未シフト面)、スクリプトに書いた英単語・コード断片だけ。
  **外部のコーパス・利用者の本文は含まない**(サイズ約 44KB、3200 + 1000 + 500 + 500 行)。
- 行は `ラベル<TAB>QWERTY の文字列`(親指なしの打鍵列。Down→Up の順で 1 打ずつ)。
  - `ja`: 文節ごとに作った仮名列(文節の頭は出現頻度から、中は 2 連の条件付き分布)に読点・句点を挟み、文中のあちこちから打ち始めて、
    親指が要る仮名に着くまでの未シフト面の区間を取る(空の行は、最初から親指が要る開始位置)。
  - `en`(英単語の連結)・`code`(コード断片)・`rand`(一様なキー列)。
- スクリプトは `KanaModel::delta`(#819 後。仮名に読めないキーは 0)と `EpisodeDetector` の Python 再現を持ち、同じ指標を表示する。
  Rust のテストの合格値は、この表示値に余裕を持たせたもの。判定器・仮名表・配列を変えたら、再生成して値を見直す。

## 限界

- `ja` は判定器と同じ仮名 2 連の分布から作るので、実際の文より誤判定が少なく**楽観的**(`en_threshold` 8 で 0.7%。#819 の評価は実文で 7.0%)。
  実文の評価(リポジトリ外の私的資料。LLM 生成の文で、出典・ライセンスはリポジトリに持ち込めない)の代わりではなく、退行の検出が目的。
- 英文・コードは短い断片の連結で、実際の文体の分布ではない。スペース・大文字・Shift 記号は含めない。
