# 局所修正で I1 違反はどれだけ減るか: 不変条件のオラクルによる測定（2026-10-26）

ADR-282 の Opus レビュー「ゼロベースの必要性を、小さな局所修正と比べて示せ」への答えの材料。基準の測定
（[belief-engine-invariant-baseline-2026-10-26.md](belief-engine-invariant-baseline-2026-10-26.md)、PR #857）と**同じオラクル・同じ字母・同じ列挙の範囲**
（全字母 19 種・長さ 3 まで、I2 は核の字母 12 種・長さ 3 まで）で、局所修正を入れた変種を測った。本番にはマージしない。

- 差分: ブランチ `exp/belief-local-fixes`（draft PR #862）。局所修正はスレッドごとのスイッチ（`crates/awase-ime-core/src/state/local_fixes.rs`）で入り、
  1 つのテストが 6 変種 × 2 窓モデル × 3 格子を並列に測る（`crates/awase-windows/tests/belief_local_fixes.rs`、golden `tests/golden/belief_local_fixes.txt`。CI run 40292397192 の実測から作成）。
- 既存テストへの影響: 既定の変種を変えた 4 本のブランチ `exp/belief-local-fixes-d1`（L1）/`-d2`（L2）/`-d4`（L3）/`-d7`（L1+L2+L3）（draft PR #864〜#867）の CI（run 40293851041 / 40293857568 / 40293018098 / 40293859554）。
- 変種の定義（差分の行数）:
  - **L1**（`ime_model.rs`、+5 行）: 開の予測（`KeyEffectPredicted { open: Some(true) }`）で `desired_open` も true に書く（`last_intent` は作らない）。
  - **L2**（`ime_model.rs`、+13 行）: `reduce_focus_changed` で、移動先が読めない窓（`Imm32Unavailable`/`TsfNative`）で、移動前の `last_intent` が無ければ `desired_open` を true にする。読める窓は従来どおり。
  - **L3**（`platform_state.rs`、+33/−12 行）: `IntentStore` の鍵を窓（`HwndId`）からスレッド（擬似窓では窓の `thread`）に変え、開閉の予測（true/false とも）を `IntentStore` に記録する（本物は予測のたびに削除していた）。
    `intent_store` を引く 10 箇所を `intent_key()` に替えた。**未対応**: 授権（`warrant_context` → `issue_open_warrant`）の `IntentStore` 参照は窓の鍵のままなので、明示 OFF の書き込みの授権が変わる（下記の `user-off` の最小列）。
  - 窓の種類の判定は `ImePolicyProfile`（`InputRelay` は `ImmCross` に写るので L2 では「読める窓」扱い。擬似窓の relay は読めない窓なので、`focus:relay` で終わる違反が L2 に残る）。

## 結果（T=0 の違反する列の本数。括弧は読める窓の poll が 1 回届いた後も残る本数）

I1（必須。全字母・長さ 3 まで）:

| 変種 | atok shared | msime shared | msime-native shared | atok per-thread | msime per-thread | msime-native per-thread |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| base | 315 (290) | 405 (375) | 405 (375) | 197 (172) | 185 (155) | 185 (155) |
| L1 | 183 (164) | 153 (135) | 153 (135) | 191 (172) | 185 (167) | 185 (167) |
| L2 | 229 (208) | 254 (230) | 254 (230) | 201 (180) | 214 (190) | 214 (190) |
| L3 | 189 (173) | 165 (148) | 165 (153) | 195 (179) | 181 (164) | 181 (169) |
| L1+L2 | 191 (172) | 182 (162) | 182 (162) | 199 (180) | 214 (194) | 214 (194) |
| L1+L2+L3 | 197 (183) | 194 (177) | 194 (184) | 203 (189) | 222 (205) | 222 (212) |

最小列（事象を 1 つ削ると違反しない列）の数 / うち「awase の操作・窓切替・正しい観測だけ」の数:

| 変種 | atok shared | msime shared | msime-native shared | atok per-thread | msime per-thread | msime-native per-thread |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| base | 31 / 18 | 65 / 36 | 65 / 36 | 21 / 8 | 49 / 20 | 49 / 20 |
| L1 | 13 / 0 | 29 / 0 | 29 / 0 | 21 / 8 | 61 / 32 | 61 / 32 |
| L2 | 22 / 11 | 39 / 14 | 39 / 14 | 22 / 11 | 43 / 18 | 43 / 18 |
| L3 | 19 / 8 | 41 / 16 | 41 / 16 | 19 / 8 | 45 / 20 | 45 / 20 |
| L1+L2 | 18 / 7 | 31 / 6 | 31 / 6 | 22 / 11 | 47 / 22 | 47 / 22 |
| L1+L2+L3 | 24 / 15 | 43 / 22 | 43 / 22 | 26 / 17 | 55 / 34 | 55 / 34 |

I2（参考・逆向き。核の字母・長さ 3 まで。違反する列の本数 / 最小列の数）:

| 変種 | atok shared | msime shared | msime-native shared | atok per-thread | msime per-thread | msime-native per-thread |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| base | 43 / 3 | 35 / 7 | 35 / 7 | 43 / 3 | 57 / 7 | 57 / 7 |
| L1 | 43 / 3 | 35 / 7 | 39 / 11 | 43 / 3 | 113 / 15 | 117 / 15 |
| L2 | 129 / 3 | 77 / 3 | 81 / 7 | 129 / 3 | 143 / 7 | 147 / 7 |
| L3 | 41 / 3 | 29 / 3 | 29 / 3 | 43 / 3 | 57 / 7 | 57 / 7 |
| L1+L2 | 129 / 3 | 77 / 3 | 81 / 7 | 129 / 3 | 143 / 7 | 147 / 7 |
| L1+L2+L3 | 123 / 3 | 67 / 3 | 67 / 3 | 129 / 3 | 143 / 7 | 147 / 7 |

- **T=2000ms の許容**: 全変種・全設定で「事象なしで 2000ms 後も残る」本数は T=0 と同じ（時間では 1 本も治らない）。poll が 1 回届く場合の本数は上表の括弧（L1 は読める窓の分が 18〜19 本治るが、読めない窓の分は残る）。
- **BUG-223 の再現（5 本: 基準の 4 本 + `key:hiragana > focus:wezterm`）**: base は msime・msime-native の shared で 5/5 が I1 違反。**L1・L2・L3・L1+L2・L1+L2+L3 のどれでも 0/5**（全設定）。最短列 `key:hiragana > focus:wezterm` も全変種で消える。

## 読み方

1. **shared（IME の状態が窓をまたいで共有される。BUG-223 の事故に合うモデル）では L1 一つが最も効く**: msime/msime-native で 405 → 153（−62%）、atok で 315 → 183（−42%）。
   「awase の操作・窓切替・正しい観測だけ」で起きる最小列は 36 → **0**（atok も 18 → 0）。つまり、BUG-223 の核（F2 が `desired_open` を書かない非対称）は、`KeyEffectPredicted` の腕に 5 行足すだけで、このオラクルの範囲では全部消える。
   L2・L3 は L1 より効かず、L1 に足すと**悪化**する（153 → 182 → 194）。L2 は ON 寄りの既定が閉の IME を ON と見なす I2 の悪化を生み（msime shared の I2 が 35 → 77、atok は 43 → 129）、L3 は IntentStore の予測が観測を覆うため別の違反を作る。
2. **残る違反（L1 の msime shared、153 本）はすべて「嘘の観測を含む」（32）か「外部変更を含む」（121）**。分類は `desired-fallback` 45・`derive-high` 30・`explicit-intent` 23・`most-recent-trusted` 21・`key-effect` 18・入力モードが `ObservedEisu` のまま 10・`derive-medium` 6。代表例:
   `external-on`（言語バー等で IME が ON になり、観測が来ない）、`key:eisu > external-off > key:eisu`、`key:eisu > advance:3500ms > obs:poll-LIE`、`focus:wezterm > obs:conv-infer-open > key:eisu`。
   これらは「観測が無いとき、または観測が嘘のときに belief が何を既定にするか」の問題で、局所修正（予測を `desired_open` に書く）の守備範囲外。ゼロベース設計の対象はここ（観測の信頼度と無観測時の既定）。
3. **per-thread（窓ごとに別スレッド、初めての窓は閉）では局所修正は I1 を減らさない**: base 185 → L1 185、L2 214、L3 181、L1+L2+L3 222。L1 は I2 を 57 → 113（msime）に倍にする。窓を移ると真の IME が入れ替わるので、予測を `desired_open` に持ち越すと新しい窓（閉）と食い違う。
   この窓モデルで I1 を減らすには、窓（スレッド）ごとに belief を分ける（L3 の方向）か、窓切替で「その窓の IME 状態は未知」とする設計が要る。**shared と per-thread のどちらが実機かが分かれば、局所修正で足りるかが決まる**（BUG-223 の記述は shared に合う。per-thread の実機での確認は未測定）。
4. **「ゼロベースの必要性」への含意**: BUG-223 の列（msime shared の最短列を含む 5 本）だけなら、L1（5 行）で足りる。ただし (a) 残る 153 本は観測側の問題で局所修正では減らない、(b) shared/per-thread の両方で I1 と I2 を同時に減らす局所修正は見つからなかった
   （L1 は shared で最良だが per-thread の I2 を倍にし、L2/L3 は I2 を悪化させる）、(c) L1 は `architecture_guard` が固定している設計の非対称（`KeyEffectPredicted` は `desired_open` を書かない）を破る。ゼロベースが必要かは、「窓モデルがどちらか」と「観測が無い・嘘のときの既定を設計に含めるか」で決まる。
   このオラクルが測るのは I1/I2 の本数であり、ゼロベースが他の目的（保守性・複雑性の削減）で必要かは扱わない。

## 壊れた既存テスト（変種ごと。CI の `test` ジョブ、`--no-fail-fast`）

| 変種 | 壊れたテスト | 何を固定しているか |
| --- | --- | --- |
| L1 | `awase-ime-core state::ime_model::tests::open_prediction_supersedes_an_older_explicit_intent` | 「開の予測は `desired_open` を書かない（意図が捨てられるだけ）」を直接固定 |
| L1 | `architecture_guard key_effect_predicted_event_is_constructed_only_in_apply_key_effect_prediction` | `KeyEffectPredicted` の腕が `desired_open` を書かないこと（belief 書き込み口の固定、ADR-170） |
| L2 | `awase-ime-core state::ime_model::tests::initial_focus_scope_matches_focus_changed_except_input_barrier` | 起動時の `InitialFocusScopeEstablished` と `FocusChanged` が同一性を揃えること。**L2 の実装がプレースホルダ旗（`desired_is_placeholder`）を倒した副作用**で、旗を保てば避けられる（実装の都合） |
| L3 | `architecture_guard intent_store_record_call_sites_are_limited_to_explicit_user_actions` | `intent_store.record(` の本番での使用箇所が 1（`record_explicit_intent` 内のみ） |
| L3 | `closed_loop_scenarios random_key_walks_keep_predictions_consistent_with_truth` | ランダムなキー列で予測が真値と食い違わない（L3 は予測が `IntentStore` に残り、予測と真値の食い違いが 10/360 本に増える） |
| L3 | `pseudo_ime_exhaustive prediction_exhaustive_golden`（golden `pseudo_ime_prediction_mismatches.txt`） | 予測の全列挙の食い違い一覧（L3 で増減） |
| L1+L2+L3 | 上の全部 | |
| 全変種 | `belief_engine_invariant`（基準の golden と `bug223_sequences_are_i1_violations_when_ime_state_is_shared`） | 期待どおり: 違反が減る/消えるので落ちる（測定が効いている証拠） |
| 全変種 | `layer_boundary_guard core_modules_classify_every_ungated_state_module` | 実験用の `local_fixes.rs` が未分類なだけ（測定の足場の副作用） |

- `architecture_guard` 以外の `golden_scenarios`・`journal_replay`・`ime_key_sequence_golden` などは、どの変種でも壊れなかった（Linux で走るもの）。`#[cfg(windows)]` の殻のテストは対象外。
- L2 は `closed_loop_scenarios`・`pseudo_ime_exhaustive` を壊さなかった（読めない窓への窓切替の既定だけを変え、予測器には触れないため）。

## 限界

- 窓モデルの限界は基準の測定と同じ（TSF 窓の ON キャッシュ復元・hwnd キャッシュ復元を写していない。読めない窓の観測が字母に無い）。**L2 は本物ではキャッシュ復元・`HwndCacheRestored`（`desired_open` を書く）と重なるので、ここでの効果は過大・過小の両方がありうる**。
- L3 の実装は半端（授権の `IntentStore` 参照・`consume_mode_key_pass_mark` の削除などは窓の鍵のまま）。完全な (pid, tid) の鍵にはハブが tid を持つ必要があり、本測定ではスレッドの鍵を擬似窓から渡した。pid は使っていない。
  結果の L3 は「IntentStore を IME の持ち主（スレッド）ごとにする」効果の下限の見積もり。
- 変種どうしを同じ列挙の範囲で比べているだけで、**実機での効果・副作用（特に L2 の ON 寄りの既定の体感）は未測定**。
- I2 の違反本数は核の字母（12 種）で、I1 の全字母（19 種）より狭い。
