# 入力言語判別の LR(ADR-280 の後続): shadow、そして `decider = "lr"` が既定

2026-11-01。追加式ロジスティック回帰(`tools/lang-detect-model/`、`src/lang_detect/lr.rs`)の増分を打鍵ごとに数え、journal の `LangDetect` に残す(#924、判定は変えない shadow)。
続けて(所有者の判断で shadow の検証を待たずに)、**『日本語でない』の確定に使う**設定 `auto_detect.decider`(既定 `lr`、`kana_ngram` で従来)を入れた。
`mode` の既定は `observe`(動作を変えない)で、`ime_on`・`ime_on_eisu` は利用者が選ぶ機能なので、影響するのはそれを選んだ人だけ。
日本語の確定(親指の重なり・仮名 n-gram の尤度比 `ja_threshold`)は従来のまま。

## decider = "lr"
- 閾値(累積 log-odds が `-値` 以下で英数): `lr_en_threshold = 1.5`(`ime_on`)・`lr_eisu_threshold = 4.0`(`ime_on_eisu` の半角英数の入り)。
- 測定(`tools/lang-detect-model/` の test 分割 135,302 エピソード): 日本語を英数と誤確定する率は 1.5 で 6.4%(従来の `en_threshold = 8` は 6.8%)、
  4.0 で 0.47%(従来の `eisu_threshold = 13` は 0.71%)。同じ FPR で、英語の検出は 63% → 86%、コードは 70% → 97%。
- 設定(`config.toml` の `[auto_detect]`。サンプルの `config.toml` には書かない=実機の作業ツリーで追跡ファイルの衝突を避けるため):
  `decider = "lr"`(既定)/ `"kana_ngram"`(従来に戻す。`en_threshold`・`eisu_threshold` を使う)、`lr_en_threshold = 1.5`、`lr_eisu_threshold = 4.0`。
- 記録: journal の `LangDetect` に `decider`(`lr`/`kana-ngram`)と `lr_llr`・`lr_trace`。起動ログ `[lang-detect] config:` に `decider=` と `lr_threshold=`。
  従来の判定器の累積 `llr`・`llr_trace` も残るので、`decider = "lr"` でも従来なら確定したかを後から計算できる(エピソードは確定した時点で閉じるので、
  確定より後の打鍵は両方とも数えない)。

## 動機
2026-11-01 14:33 JST、「すすめてくだ」(C,C,N,D,I,E = すすめてくた)が llr=-14.6 で英数と判定され、半角英数トグルに入って Engine が OFF になった。
独立した実文(Wikipedia・Tatoeba・英語・他言語・コード、test 135,302 エピソード)で測ると、日本語を英数と誤確定する率(FPR)を同じにしたとき、
LR は今の判定器より非日本語の検出率が 20〜35 ポイント高く、1〜2 打早く決まる(`tools/lang-detect-model/README.md`)。

## 入れたもの
- `data/lang_lr_v1.txt`: int8 に量子化した重み(約 11.5 万個、231KB)。`tools/lang-detect-model/ldm/export_lr.py` が生成する。
  量子化しても test の指標は変わらない(FPR≤0.5% で英 83.3%、float32 は 83.2%)。
- `src/lang_detect/lr.rs`: 表の読み込みと `step`(キー位置 → 増分)。Python との一致は `tests/fixtures/lang_lr_vectors.tsv`(86 見本)で固定。
- `KanaModel::with_lr_shadow` と `EpisodeDetector` の `lr_llr`・`lr_trace`: `verdict`・`llr`・`presses` は変わらない
  (`lr_shadow_does_not_change_the_verdict`)。
- `LangDetectRecord`(journal)に `lr_llr`・`lr_trace`・`lr_not_japanese_at`(累積が -4.0 以下になった最初の打鍵、1 始まり)。
  `None`・空は直列化しないので、既存の journal の形は変わらない。debug ログ(`[lang-detect]`)にも `lr_llr`・`lr_not_japanese_at` が出る。

## 実機で見るもの
```sh
uv run --project tools/lang-detect-model python -m ldm.shadow_report awase.log awase.log.old
```
- **現在が英数に入った(`action != none`)のに shadow は確定しなかった件**: 今回のような誤発火を LR が避けられた件。日本語の打鍵だったかは、直後に
  親指の重なりが出た(次の `japanese` のエピソード)か、利用者の報告で確かめる。
- **現在は確定せず shadow が英数と確定した件**: 英文なら取りこぼしの改善、日本語なら shadow の誤発火。日本語だった件が出たら閾値(`SHADOW_NOT_JAPANESE` = 4.0)を見直す。
- 十分な件数(目安: 英数側の確定が数十件)が集まるまで切り替えない。

## 限界
- 学習データの非日本語は書き言葉(Wikipedia・Tatoeba)で、`code` はこのリポジトリのファイル。コマンド・チャットの語は未測定。
- 閾値 4.0 はクラス質量(日本語 0.5)に依存する。実機の事前確率は未測定。`bias`(+0.76)は足していない(評価も足していない)。
- エピソードの先頭は常に `BOS`(文頭・語頭)として扱う。語の途中から打ち始めた場合は学習の `MID` と合わない。
- shadow は、今の判定器が確定したあとの打鍵を数えない(エピソードがそこで閉じる)。今の判定器が早く確定した件は、shadow の累積がまだ閾値に届いていないことがある。
