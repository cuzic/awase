# 局所修正で I1 違反はどれだけ減るか: 不変条件のオラクルによる測定（2026-10-26）

ADR-282 の Opus レビュー「ゼロベースの必要性を、小さな局所修正と比べて示せ」への材料。基準の測定
（[belief-engine-invariant-baseline-2026-10-26.md](belief-engine-invariant-baseline-2026-10-26.md)、PR #857）と**同じオラクル・同じ字母・同じ列挙の範囲**
（全字母 19 種・長さ 3 まで、I2 は核の字母 12 種・長さ 3 まで）で、局所修正を入れた変種を測った。変種の定義は belief-design が
コードで確認したもの（`/tmp/.../adr282-local-fixes-definition.md`）に従う。本番にはマージしない。

- 差分: ブランチ `exp/belief-local-fixes`（draft PR #862）。局所修正はスレッドごとのスイッチ（`crates/awase-ime-core/src/state/local_fixes.rs`）で入り、
  1 つのテスト（`crates/awase-windows/tests/belief_local_fixes.rs`、golden `tests/golden/belief_local_fixes.txt`）が 11 変種 × 2 窓モデル × 3 格子を並列に測る。CI run 40296295547 の実測と一致を確認（決定的）。
- 既存テストへの影響: 既定の変種を変えた 5 本のブランチ `exp/belief-local-fixes-d1`（L1）/`-d2`（L2）/`-d3`（L1+L2）/`-d4`（L3）/`-d6`（L2+L3）（draft PR #864〜#866・#869・#870、CI run 40296301271 / 40296303648 / 40296329146 / 40296313228 / 40296333927）。
- 変種（差分の行数。`ime_model.rs`／`platform_state.rs`）:
  - **L1**（+7 行）: `KeyEffectPredicted { open: Some(v) }` の `reduce` で `desired_open = v` を書く（true/false とも。`last_intent` は None のまま＝ポーリングは止まらない）。
  - **L2**（+9 行）: `reduce_focus_changed` の最後で、移動先の profile が `TsfNative`/`Imm32Unavailable`（`InputRelay` は `ImmCross` に写るので含まない）なら `desired_open = true`。無条件（明示意図の行は hub の `IntentStore` が上書きする）。
    **L2b**（+9 行）: さらに `AppProfileChanged`（同じフォーカス中の学習による降格）でも同じ。降格した explorer を読めない窓に含める版。
  - **L3**（+33/−12 行）: `IntentStore` の鍵を窓からその窓のスレッドに変え、`apply_key_effect_prediction` の `intent_store.remove` を予測値の記録（ON 10 秒・OFF 30 秒の TTL のまま）に置き換える。`last_intent` には入れない。
    `intent_store` を引く 10 箇所を `intent_key()` に替えた。授権（`warrant_context` → Step 1）の参照は窓の鍵のまま（比較用に許容）。
  - **+long**: 字母に `advance:11000ms`・`advance:31000ms`（`IntentStore` の TTL 切れを見る）を足した版（21 種）。
  - 組み合わせ: L1+L2、L2+L3。

## 結果（T=0 の違反する列の本数。括弧は読める窓の poll が 1 回届いた後も残る本数）

I1（必須。全字母・長さ 3 まで。+long は 21 種）:

| 変種 | atok shared | msime shared | msime-native shared | atok per-thread | msime per-thread | msime-native per-thread |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| base | 315 (290) | 405 (375) | 405 (375) | 197 (172) | 185 (155) | 185 (155) |
| L1 | 183 (164) | 153 (135) | 153 (135) | 191 (172) | 185 (167) | 185 (167) |
| L2 | 229 (208) | 256 (232) | 256 (232) | 201 (180) | 216 (192) | 216 (192) |
| L2b | 221 (200) | 242 (218) | 242 (218) | 201 (180) | 218 (194) | 218 (194) |
| L3 | 189 (173) | 165 (148) | 165 (153) | 195 (179) | 181 (164) | 181 (169) |
| L1+L2 | 191 (172) | 184 (164) | 184 (164) | 199 (180) | 216 (196) | 216 (196) |
| L2+L3 | 197 (183) | 194 (177) | 194 (184) | 203 (189) | 220 (203) | 220 (210) |
| base+long | 401 (338) | 513 (447) | 513 (447) | 259 (196) | 245 (179) | 245 (179) |
| L1+long | 245 (188) | 213 (159) | 213 (159) | 253 (196) | 245 (191) | 245 (191) |
| L3+long | 283 (231) | 289 (238) | 289 (245) | 265 (213) | 257 (206) | 257 (213) |
| L2+L3+long | 275 (225) | 294 (243) | 294 (252) | 273 (223) | 304 (253) | 304 (262) |

最小列の数 / うち「awase の操作・窓切替・正しい観測だけ」（核の最小列）:

| 変種 | atok shared | msime shared | msime-native shared | atok per-thread | msime per-thread | msime-native per-thread |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| base | 31 / 18 | 65 / 36 | 65 / 36 | 21 / 8 | 49 / 20 | 49 / 20 |
| L1 | 13 / 0 | 29 / 0 | 29 / 0 | 21 / 8 | 61 / 32 | 61 / 32 |
| L2 | 22 / 11 | 39 / 14 | 39 / 14 | 22 / 11 | 43 / 18 | 43 / 18 |
| L2b | 24 / 13 | 43 / 18 | 43 / 18 | 24 / 13 | 47 / 22 | 47 / 22 |
| L3 | 19 / 8 | 41 / 16 | 41 / 16 | 19 / 8 | 45 / 20 | 45 / 20 |
| L1+L2 | 18 / 7 | 31 / 6 | 31 / 6 | 22 / 11 | 47 / 22 | 47 / 22 |
| L2+L3 | 24 / 15 | 43 / 22 | 43 / 22 | 24 / 15 | 47 / 26 | 47 / 26 |
| base+long | 37 / 18 | 77 / 36 | 77 / 36 | 27 / 8 | 61 / 20 | 61 / 20 |
| L1+long | 19 / 0 | 41 / 0 | 41 / 0 | 27 / 8 | 73 / 32 | 73 / 32 |
| L3+long | 57 / 40 | 117 / 80 | 117 / 80 | 33 / 16 | 73 / 36 | 73 / 36 |
| L2+L3+long | 46 / 31 | 87 / 54 | 87 / 54 | 38 / 23 | 75 / 42 | 75 / 42 |

I2（逆向き。核の字母・長さ 3 まで。違反する列の本数 / 最小列の数）:

| 変種 | atok shared | msime shared | msime-native shared | atok per-thread | msime per-thread | msime-native per-thread |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| base | 43 / 3 | 35 / 7 | 35 / 7 | 43 / 3 | 57 / 7 | 57 / 7 |
| L1 | 43 / 3 | 31 / 3 | 31 / 3 | 43 / 3 | 109 / 15 | 109 / 15 |
| L2 = L2b = L1+L2 | 155 / 3 | 99 / 3 | 103 / 7 | 155 / 3 | 165 / 7 | 169 / 7 |
| L3 | 41 / 3 | 29 / 3 | 29 / 3 | 43 / 3 | 57 / 7 | 57 / 7 |
| L2+L3 | 123 / 3 | 67 / 3 | 67 / 3 | 155 / 3 | 165 / 7 | 169 / 7 |

- **BUG-223 の 4 本 + 最短列 `key:hiragana > focus:wezterm`**: base の msime・msime-native shared で 5/5 が I1 違反。**全変種で 0/5**（+long も、L3+long は TTL 切れ後の列を別に数える）。
- **T=2000ms**: 全変種で「事象なしで 2000ms 後も残る」本数は T=0 と同数（時間では治らない）。poll の効果は括弧のとおり（読める窓の分だけ 15〜20 本、読めない窓の分は残る）。
- **`WriteCommand` の件数**（固定 seed・長さ 10・200 本のランダム列の総数）: base 131〜136、全変種 131〜136（差は 1〜5 件）。局所修正は書き込みの件数をほとんど動かさない（授権 Step 4c と Step 1 の変化は、この字母では件数に出ない）。

## 読み方

1. **shared（IME の状態が窓をまたいで共有される。BUG-223 の事故に合うモデル）では L1 が最も効く**: msime/msime-native で 405 → 153（−62%）、atok で 315 → 183（−42%）。
   核の最小列は 36 → **0**（atok 18 → 0）。BUG-223 の核（予測が `desired_open` を書かない非対称）は、このオラクルの範囲では L1 だけで全部消える。
   L2・L3 は単独では効きが弱く（msime shared 256・165）、**L1 に足すと悪化**する（L1+L2 184、L2+L3 194）。L2 は ON 寄りの既定が閉の IME を ON と見なす（下の I2）。
2. **L3 は TTL が切れると効かなくなり、かえって悪化する**: +long の字母で base 513 → L3 289 は減って見えるが、核の最小列は base 36 → **L3 80**（msime shared）。予測が `IntentStore` に入って観測を覆い、TTL（ON 10 秒）が切れた後に別の違反が出る
   （L1+long は核の最小列が 0 のまま）。予測の寿命を `IntentStore` に預ける案は、TTL を越える静止（10 秒超）に弱い。
3. **残る違反（L1 の msime shared、153 本）はすべて「嘘の観測を含む」（32）か「外部変更を含む」（121）**。分類は `desired-fallback` 45・`derive-high` 30・`explicit-intent` 23・`most-recent-trusted` 21・`key-effect` 18・`ObservedEisu` のまま 10・`derive-medium` 6。代表例:
   `external-on`、`key:eisu > external-off > key:eisu`、`key:eisu > advance:3500ms > obs:poll-LIE`、`focus:wezterm > obs:conv-infer-open > key:eisu`。観測が無い・嘘のときに belief が何を既定にするかの問題で、予測を書く局所修正の守備範囲外。
4. **per-thread（窓ごとに別スレッド、初めての窓は閉）では局所修正は I1 を減らさず、I2 を悪化させる**: msime per-thread で base 185 → L1 185、L2 216、L3 181、L2+L3 220。L1 は I2 を 57 → 109（約 2 倍）にする。窓を移ると真の IME が入れ替わるので、予測の持ち越しは新しい窓（閉）と食い違う。
5. **L2 の I2 の悪化（窓の種類別）**: msime shared の I2 は base 35 → L2 99。増えた分は**読めない窓（chrome・wezterm 各 46、xaml 7）で終わる `desired-fallback`**。最小列は `focus:wezterm` 1 手（真が閉のまま読めない窓へ移ると Engine が ON になる）。
   per-thread でも 57 → 165（chrome・wezterm 各 68、xaml 27）。読める窓（explorer）の I2 は増えない。降格した explorer を含める L2b は、I1 を少し減らす（msime shared 256 → 242）が I2 は同じ。
6. **「ゼロベースの必要性」への含意**: BUG-223 の列は L1（7 行）で消える。ただし (a) 残る 153 本は観測側の問題で局所修正では減らない、(b) shared と per-thread の両方で I1 と I2 を同時に減らす局所修正は見つからなかった
   （L1 は shared で最良だが per-thread の I2 を倍にし、L2 は I2 を 3 倍近くにし、L3 は TTL に弱い）、(c) L1 は `architecture_guard` と `ime_model` の単体テストが固定している設計（予測は `desired_open` を書かない）を破る。
   ゼロベースが要るかは「実機の窓モデルが shared か per-thread か」と「観測が無い・嘘のときの既定を設計に含めるか」で決まる。このオラクルは I1/I2 の本数だけを測り、保守性・複雑性などの目的は扱わない。

## 壊れた既存テスト（変種ごと。各ブランチの CI の `test` ジョブ、`--no-fail-fast`）

| 変種 | 壊れたテスト | 何を固定しているか |
| --- | --- | --- |
| L1 | `awase-ime-core ime_model::tests::open_prediction_supersedes_an_older_explicit_intent` | 開の予測は `desired_open` を書かない（意図が捨てられるだけ） |
| L1 | `awase-ime-core ime_model::tests::key_effect_prediction_moves_open_and_mode_without_touching_desired_open` | 予測は `desired_open` を書かない（ドリフト補正が IME へ書き戻さないため） |
| L1 | `architecture_guard key_effect_predicted_event_is_constructed_only_in_apply_key_effect_prediction` | `KeyEffectPredicted` の腕が `desired_open` を書かないこと |
| L2 | `warmup_gate_focus_scope desired_open_survives_focus_change` | `desired_open` は `FocusChanged` でクリアされず次の明示書き込みまで残る（ADR-132 の前提） |
| L2 | `warmup_gate_focus_scope drift_re_establishes_in_new_window_when_stale_desired_conflicts_with_fresh_observation` | 窓 B の stale な `desired_open=false` と新しい観測の食い違いで drift が再確立される |
| L2 | `awase-windows platform_state::tests::check_drift_correction_ignores_heuristic_default_alone_without_explicit_intent` | 生の `desired_open()` フォールバックが false のまま |
| L3 | `architecture_guard intent_store_record_call_sites_are_limited_to_explicit_user_actions` | `intent_store.record(` の本番での使用箇所が 1 |
| L3 | `closed_loop_scenarios random_key_walks_keep_predictions_consistent_with_truth` | ランダムなキー列で予測が真値と食い違わない（L3 は予測が `IntentStore` に残り食い違いが増える） |
| L3 | `pseudo_ime_exhaustive prediction_exhaustive_golden` | 予測の全列挙の食い違い一覧 |
| L1+L2 | L1 と L2 の上の全部 | |
| L2+L3 | L2 と L3 の上の全部 | |
| 全変種 | `belief_engine_invariant`（基準の golden と `bug223_sequences_are_i1_violations_when_ime_state_is_shared`） | 期待どおり: 違反が減るので落ちる |
| 全変種（実験の足場） | `architecture_guard intent_store_record_call_sites…`・`key_effect_predicted_event…`（実験ブランチ本体でも）、`layer_boundary_guard core_modules_classify_every_ungated_state_module` | 実験用コード（`local_fixes.rs`、L1/L3 の差分）が入っているだけで落ちる文字列走査 |

- L2 が壊すのは `desired_open` が窓切替で持ち越される（stale のまま残る）ことを仕様として固定したテスト群。L2 は予測器には触れないので `closed_loop_scenarios`・`pseudo_ime_exhaustive` は壊れない。
- `golden_scenarios`・`journal_replay`・`ime_key_sequence_golden` などは、どの変種でも壊れなかった（Linux で走るもの。`#[cfg(windows)]` の殻のテストは対象外）。

## 限界

- 窓モデルの限界は基準の測定と同じ（TSF 窓の ON キャッシュ復元・hwnd キャッシュ復元を写していない。読めない窓の観測が字母に無い）。**L2 は本物では `HwndCacheRestored`（`desired_open` を書く）と重なるので、ここでの効果は過大・過小の両方がありうる**。
- L3 は授権の `IntentStore` 参照が窓の鍵のまま。tid はハブが持たないので擬似窓の `thread` を渡した（pid は使っていない）。
- `WriteCommand` の件数は固定 seed のランダム列の総数で、列挙の範囲での比較ではない。実機での効果・副作用（特に L2 の ON 寄りの既定の体感）は未測定。
- I2 は核の字母（12 種）で、I1 の全字母（19〜21 種）より狭い。
