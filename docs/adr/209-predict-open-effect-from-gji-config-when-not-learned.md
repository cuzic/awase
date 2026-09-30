---
id: ADR-209
title: |-
  GJI の MS-IME プリセットでは、TSF ネイティブの窓で、直接入力の変換が IME を開く。打鍵時予測の表を窓の種類(IMM32/TSF)別にして、素通しされた変換に Engine を追随させる
summary: |-
  実機(dragonflyg4、JIS、GJI 3.34.6260.0、`session_keymap=2`)で、IME OFF から変換を単独タップすると、awase 停止でも WT・メモ帳・Edge の全てで IME が ON になる。awase は「予測しない」ため Engine が OFF のまま(`ka`→`か`)。
  GitHub Actions(windows-latest、GJI の MS-IME プリセット、awase なし、run 36690572075)で、**実 Chrome(TSF)は開き、素の EDIT(IMM32)は開かない**ことを再現した。古い custom 表の有無は無関係(表なしでも同じ)。
  同梱表(`key_effect_table.rs`)は EDIT(IMM32)で学習したので、TSF ネイティブの窓では変換について誤っている。
status: |-
  草稿 v2(2026-09-30)。v1(「古い custom 表を実効とみなす」)は Opus 敵対レビューで Blocker 2件、実機の弁別実験と CI で棄却された。書き直し版を再レビュー待ち。
related_adr:
  - "ADR-186"
  - "ADR-191"
  - "ADR-192"
  - "ADR-195"
  - "ADR-196"
  - "ADR-199"
  - "ADR-206"
---

# ADR-209: 窓の種類別に、MS-IME プリセットの変換の効果を予測する(草稿 v2)

## 経緯(v1 の棄却)
v1 は「MS-IME プリセット(2)で、古い `custom_keymap_table` を実効とみなす」という決定3を置いた。Opus 敵対レビュー(round1)が、次を指摘した: 根拠(実機1台の観測)は未同定で、既存の証拠(ADR-186 決定2(c)、Mozc `keymap.cc`、CI の格子)は逆を指す。
実機で仮説を弁別した結果(2026-09-30):
- **X1**: IME ON で無変換を押すと、`カ`→`ｶ`→`か` と巡回した。古い表(無変換の行なし)が実効なら毎回 `か` のはずで、**プリセットが実効**。
- **overlay**: `config1.db` の field 68 が無い(protobuf を全解析)。overlay 100 ではない。
- **X3**: 変換を IME OFF から押すと、メモ帳・Edge でも IME が ON になった(WT と同じ)。**アプリ依存ではなく、TSF の窓で共通**。
- **CI**(run 36690572075): GJI の MS-IME プリセット、awase なしで、**実 Chrome は「直接入力→変換」で開く(古い表の有無によらず)**。**素の EDIT(`ime_key_matrix_spike` の `--seq=1C`)は開かない(`open=0`)**。無変換は実 Chrome でも `か`→`カ` と巡回(実機と同じ)。

## 背景(実機・CI の事実)
- GJI の MS-IME プリセットでは、直接入力の変換の定義は `Reconvert`(Mozc の `ms-ime.tsv`。IME の開閉とは無関係)。**それでも TSF ネイティブの窓では、再変換の処理が IME を開く。IMM32 の素の EDIT では開かない。**(仕組みは未確認。事実は CI と実機の観測)
- 同梱表(`key_effect_table.rs` の MSIME 表、`grid-tables/msime.json`)は、学習プロセスの EDIT(IMM32)で測ったもの。**TSF ネイティブの窓の予測には、そのまま使えない**(変換だけでなく、他のキーも窓の種類で違う可能性。未測定)。
- awase の予測(`key_effect_predictor.rs::predict_with_override`)は、`custom_keymap_table` がそのキーの行を持つと予測を打ち切る(`custom_table_overrides`)。**GJI はプリセット(CUSTOM 以外)のとき `custom_keymap_table` を読まない**(ADR-186 決定2(c)の ATOK、今日の X1)ので、この打ち切りは ATOK/MS-IME プリセットでは不要で、この実機では予測を止めている(古い表が変換の行を持つため)。
- TsfNative の窓では開閉を観測できない(`read_ime_state_*` は `None`、`ConvOpenInference` は開閉を区別できない)。**追随の手段は打鍵時予測(`KeyEffectPredicted`)だけ**。
- 学習(ADR-195/196)は、この実機では完走しない(BUG-178)。学習プロセスの入力先は素の EDIT なので、学習しても TSF の窓の効果は得られない。

## 決定
1. **予測表を窓の種類別にする**: GJI の MS-IME プリセットで、TSF ネイティブの窓(`TsfNative`/`Imm32Unavailable` のプロファイル)では、**閉状態(DirectInput)の変換(0x1C)は「開く(ひらがな)」**と予測する。IMM32 の窓(`ImmCross`)は従来どおり(開かない)。根拠は CI と実機の観測(上)。他のキーの窓別の違いは未測定で、この ADR の対象外。
2. **予測の打ち切りを見直す**: `session_keymap` が ATOK/MS-IME/KOTOERI/MOBILE のとき、`custom_keymap_table` の行を理由に予測を打ち切らない(GJI はその表を読まない)。CUSTOM のときの扱いは従来どおり。
3. **予測は開閉軸と、絶対設定に限った入力モードだけ**: 変換で開くとき、入力モードは「ひらがな(ネイティブ)」とする(Reconvert 経由で IME が開いたときの実測: 実 Chrome で `か` = かな・ひらがな)。
4. **止める設定**: 新しい bool 設定(例 `predict_open_from_gji_config` の類。名前は実装時に決める)を置き、既定は入れる。偽 ON が実機で出たとき、ビルドし直さずに止められる。
5. 新しいイベント・I/O・actuation の合流点・tuning 定数は作らない。fence は既存の `KEY_EFFECT_SETTLE_MS`。

## 非目的
素通し後に awase が IME へ書くこと(ADR-191 決定1)。ADR-206 決定1(α)(Suppress × エンジン非活性では生キーが IME に届く)の変更。窓別の表の**全キーの網羅**(別 ADR: TSF 形式の入力先での学習、または受動学習)。学習プロセスの修正(BUG-178)。PR #360(別件、保留)。

## 代替案
- **v1(古い表を実効とみなす)**: 棄却(上)。
- **利用者への案内(ADR-192 の `UserOverride`)**: 設定の食い違いではなく、プリセットの通常の挙動なので該当しない。
- **受動学習(窓の種類別に、実際の効果を観測して覚える)**: 原理的に最も一般的だが、新しい保存・証拠・採否の設計が要る(v2 の後)。この ADR の窓別表は、その学習の初期値になる。
- **候補窓の検出による自己修復(ADR-203 案C)**: 追随が遅れる(最初の数文字が `か`)。安全網として別 ADR。

## リスク(再レビューで詰める)
1. **偽 ON**: 窓の種類の判定を誤ると(例: ImmCross の窓を TSF と判定)、開かないのに「開く」と予測する。緩和: プロファイル判定は既存の `AppKind`/`profile` を使う。実機・CI で TSF/IMM32 それぞれを検証。
2. **他の GJI バージョン**: 実機は 3.34.6260.0。CI の版は未取得。バージョンによって Reconvert の副作用が違う可能性。
3. **予測は `desired_open` を書かない**: 直前の Ctrl+無変換(明示 OFF、TTL 30秒)の後、belief だけが ON になる。drift 補正・`ActivationSync`・hwnd キャッシュ(偽 ON の1時間保持)との相互作用を、テストで固定する(v1 レビュー M1・M2。予測が正しいときは問題にならないが、窓判定を誤ったときの被害)。
4. **予測の後に GjiFsm の開き直し(`kp_reopen_gji_fsm(Predict)`)が走る**: 変換で IME が実際に開いているので、正しい動作のはず(要確認)。

## 検証方針
- **CI(windows-latest)で閉ループ検証できる**: `chrome_probe`(実 Chrome)を **awase あり**で、GJI の MS-IME プリセットで実行し、「直接入力→変換」の後に Engine が追随して NICOLA の文字になることを確認する(今の awase では `か`)。素の EDIT(ImmCross)では従来どおり(開かない)ことも確認する。
- 単体テスト(予測器: MS-IME × TsfNative × 閉 × 変換 → 開く。ImmCross → 開かない。`custom_keymap_table` に行があっても、プリセットでは予測する)。既存テスト `realdev_msime_preset_with_stale_custom_table` の期待値を更新する。
- `closed_loop_scenarios`: 明示 OFF の後に変換を素通しして、drift・`VK_IME_ON` の送信が出ないこと。窓判定を誤った負の場合。
- `architecture_guard`: `KeyEffectPredicted` の dispatch 元が1箇所のまま。
- 実機 A/B(dragonflyg4、WT・メモ帳・Edge): 「IME OFF → 変換 → `ka`」が `きう`(NICOLA)になること。

## 未検証事項
- Reconvert が TSF の窓で IME を開く仕組み。他の TSF の窓(VS Code、Electron、UWP)でも開くか。
- 変換以外のキーの、窓の種類別の差。
- 実機以外の GJI のバージョンでの挙動。
