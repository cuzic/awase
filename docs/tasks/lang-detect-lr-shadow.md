# 入力言語判別の LR を shadow で入れる(ADR-280 の後続)

2026-11-01。**判定・動作は変えない**。追加式ロジスティック回帰(`tools/lang-detect-model/`、`src/lang_detect/lr.rs`)の増分を
打鍵ごとに数え、journal の `LangDetect` に残すだけ。実機のデータで今の判定器と並べてから、既定を切り替えるか決める。

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
