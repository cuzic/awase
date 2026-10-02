---
id: ADR-219
title: |-
  エンジンテストの読みにくさは、シナリオ DSL ではなくヘルパー 2 つとキー分類表の集約で解く(DSL は見送り)
summary: |-
  当初案「時刻つきキー列のテキスト DSL」は Opus round1 で不採用推奨となった(パイロット 5 本で行数基準が必ず不合格、DSL で書けるのは全 383 本中 40〜50 本、
  ミューテーション基準が変異体 0 件で合格する、実行器の無言の無検査経路)。読みにくさの実体は (1) マイクロ秒算術 `t0 + 30_000` と (2) 出力検査の 3 行であり、
  新しいヘルパーは作らず、既存の `assert_single_char` に `#[track_caller]` を付けて 21 か所の使い忘れを置き換え、`ms()` だけを足す。あわせて、純粋な対応表(vk→pos/vk→scan/修飾キー判定)と proptest 側の重複ヘルパー(`lit`/`make_layout` 等)を撤去する。親指の分類(`classify_*`)は各ハーネスに残す(SPACE の分類と scan が 3 か所で違い、寄せると proptest の入力の意味が変わる)。
status: |-
  提案(2026-10-02)。Opus round1 で DSL 案から方針転換、round2(`assert_emits` はコンパイル不可・既存ヘルパーで足りる・3表は同一でない)を反映、round3 の再確認待ち。未実装。
related_adr:
  - "ADR-115"
  - "ADR-158"
---

# ADR-219: エンジンテストはヘルパーとキー表の集約で読みやすくする(DSL は見送り)

## 背景

`src/engine/tests.rs` は 10,129 行・`#[test]` 383 本(TestHarness 領域 206、fsm_adapter 18、`engine_integration_tests` 159 ほか)。`Ev::down`/`Ev::up` は 672 か所。
典型(`test_pattern2_char_first_then_thumb`、334〜352 行)は次の形で、読みにくさは 2 点に集約される。

1. 時刻が `t0 + 30_000` というマイクロ秒の算術(`Timestamp` はマイクロ秒。`nicola_fsm.rs:453` の `threshold_ms * 1000` で確認済み)。
2. 出力検査が `result.assert_consumed(); assert_eq!(result.actions.len(), 1); assert!(matches!(result.actions[0], KeyAction::Char('ゔ')));` の 3 行。

当初はテキスト DSL(`A↓@0 => pending` 形式)を提案したが、Opus round1(`opus-review-adr219-round1.md`)で次が分かり、取り下げた。

- 行数の中止基準は 5 本パイロットでは必ず不合格(削減 30〜40 行 vs 実行器 100 行 + キー表。損益分岐は約 17 本)。
- 語彙で書けるのは全体の 40〜50 本(1 割強)。`engine_integration_tests` の 159 本は `Engine`/`Decision` 型でほぼ 0 本。`test_pattern*` 7 本中 4 本は書けない。
- 「書いた行だけ検査する」+ 寛容なパースで、綴り誤りや未知キーが無言で通る。ミューテーション基準は `#[cfg(test)]` のみの diff では変異体 0 件で合格する。
- 目的の半分は `tests/scenarios.rs`(公開 API + 本番レイアウトで「キー列 → 出力文字列」)が既に満たしている。

## 決定

### D1: 新しい汎用ヘルパーは作らない。既存の `assert_single_char` を使い回す

round2 で次が分かった。

- `KeyAction` は `#[derive(Debug, Clone)]` のみで `PartialEq` を持たない(`src/types.rs:377`)。当初案の `assert_emits`(`assert_eq!(r.actions.as_slice(), expected)`)はコンパイルできない。`PartialEq` を足すと本番の公開型の変更になる。
- 置換対象(`assert_eq!(X.actions.len(), N)` の後に要素を `matches!` で見る形)は `tests.rs` 全体で 28 か所だけ。うち 21 か所は `len == 1` + `Char(c)` で、`tests.rs:1625` の既存 `assert_single_char(resp, ch)`(consumed + len==1 + `Char(ch)`)と完全に同じ検査。残りは `Key(x)` が 3、`len == 0` が 1、その他 3(2118、5375 ほか)。

したがって: (a) 既存の `assert_single_char` に `#[track_caller]` を付ける(今は付いておらず、失敗位置がヘルパー内になる小さな欠陥)。(b) 21 か所をそれに置き換える。(c) `Key(x)` の 3 か所、`len == 0`、独自の失敗メッセージを持つ 2 か所(4452・4724)、その他は元のまま。`KeyAction` は変更しない。

**置換の原則は 1 つ**: 置換後は元と同等以上に強く、かつ現状で通ること。通らなければ置換しない。`any`/`actions[0]` だけを見る既存テストは、機械置換の範囲を小さく保つため本 ADR では触らない。

### D2: `ms()` を足す。置換するのは 1000 の倍数のリテラルだけ

```rust
const fn ms(n: u64) -> Timestamp { n * 1000 }
```

- 置換対象: `t0 + 30_000` のような **1000 の倍数のマイクロ秒リテラル**だけ。`11_700` や `106_550` のような実測値は触らない。
- 統合領域(`engine_integration_tests`、6108 行以降)の `.at(100)`(84 か所)・`.at(200)` ほか計 116 か所は **置換しない**。これらはマイクロ秒としては 0.1ms で、時刻が意味を持たないテストが書かれた可能性が高い。`ms()` 導入後に「100ms」と誤読されるのを避けるため、別途、それらがタイミングに依存しないことを確認し、1 行コメントで「マイクロ秒、時刻は意味を持たない」と残すか `.at(0)` に揃えるかを決める(本 ADR の実装範囲に含める)。

### D3: 重複の撤去(これが本題)。共有するのは純粋な対応表だけ

3 つの表は同じものの重複ではない(round2 の実測)。

| 項目 | `tests.rs`(224-286) | `proptest_tests.rs`(180-212) | `tests/scenarios.rs`(60-120) |
| --- | --- | --- | --- |
| SPACE の分類 | LeftThumb | Passthrough | Passthrough |
| scan code | 主要キーは実値 | **常に `ScanCode(0)`** | A〜Z・親指は実値 |
| 位置(vk→pos) | A,S,D,F,C,V | 同左(tests.rs と同一) | 同左 + L(2,8) |
| 修飾キー | Shift/Ctrl/Alt 系 | 同じ集合 | 常に `None` |

proptest の scan 0 は `output_history.rs:136` の `find_action_by_scan`(KeyUp の対応)に効くので、実値に寄せると通る経路が変わり、新たに落ちる可能性がある挙動変更になる。SPACE の分類を `tests.rs` に寄せると proptest の `VK_POOL` で左親指の出現率が変わり、proptest に寄せると `make_engine_with_space_thumb`(`tests.rs:991`)系の約 20 本が壊れる。
親指の割り当ては「表」ではなくハーネスの設定(`NicolaFsm::new(.., VK_NONCONVERT, VK_CONVERT, ..)` と対)。

よって:

- **共有する**: 純粋な対応表 3 つ(`vk_to_pos`・`vk_to_scan`・`classify_modifier`)を `test_support` に置き、`tests.rs` と `proptest_tests.rs` の重複(vk→pos は同一)を除く。`classify_*`(親指の決定)は各ハーネスに残す。
- **proptest の scan 0 は別の判断**: 実値に変えるかは別 PR とし、変えるなら proptest が新たに落ちた場合の扱いを先に書く。本 ADR では変えない。
- **proptest のより単純な重複を除く**: `lit`(`proptest_tests.rs:66`、`test_support::lit` と同一)、`make_layout`(`test_support::make_layout` に `left_thumb_shift`/`right_thumb_shift` の 2 行を足すだけ)、`empty_special_keys`/`ime_on_ctx`/`make_test_engine`(`tests.rs:6119/6168/6129` と同じ。差は proptest 版が `set_thumb_shift_faces_enabled(true)` を呼ぶ点だけ)。`TestHarness` は統合しない(proptest 版はフィールド名 `fsm`・`Deref` なし・faces 有効。統合すると `tests.rs` の 206 本の既定が変わる)。
- **`tests/scenarios.rs`**: `test_support` は `#[cfg(test)] pub(crate)` なので統合テストから使えない(feature を足すのは採らない)。代わりに、ルートの `[dev-dependencies]` に `awase-vkmap` を足し、手書きの `vk_to_pos`(13 行)を `awase_vkmap::vk_to_pos` に置き換える。**注意**: `awase-vkmap` は `awase` に依存するので、**単体テスト(`src/engine/tests.rs`・`proptest_tests.rs`)から使うと `awase` が 2 回ビルドされて `VkCode` の型が合わない**。使えるのは `tests/*.rs` だけ。

### D4: 検証と中止

- D1/D2 の置換は、置換前後で該当テストの合否が変わらないこと(`cargo test --lib`)。通らないものは置換しない。
- D3 の挙動を変えない統合は、`cargo test --lib`・`cargo test --test scenarios`・`cargo nextest run --workspace --lib` が全て通ること。ただし挙動を変えない統合は通って当たり前なので、**proptest の分布・scan を変える変更はこの ADR に含めない**ことが前提。
- ミューテーション確認は不要(`#[cfg(test)]` のみの diff は `--in-diff` で変異体 0 件。検査の強さは D1 の原則「同等以上かつ通る」で担保する)。
- 中止基準: D3 の対応表の共有で、`tests.rs` か proptest のどちらかの挙動が変わる(テストの合否・分布が変わる)ことが分かったら、その表は共有しない。

## 期待する効果(概算、実測は実装時)

D1: 21 か所 × 2 行 = 約 40 行。D3: 純粋な対応表の重複で約 30 行、proptest の `lit`/`make_layout` 等で約 60 行、`scenarios.rs` の `vk_to_pos` で 13 行。合計 100〜150 行の削減、ヘルパーの追加は `ms()` の 1 関数のみ。

## 将来テキスト形式を再検討する条件

D1〜D3 の実施後もなお「時系列が読めない」具体的なテストを 17 本以上挙げられること。かつ `tests/scenarios.rs` 側に置く案(本番レイアウト・公開 API。D3 の `awase-vkmap` 化で敷居が下がる)を先に検討すること。

## 検討した代替案

- **テキスト DSL**: round1 の理由で見送り(書けるのは全 383 本中 40〜50 本、行数基準が必ず不合格、無言の無検査の経路)。
- **`assert_emits`(新しい汎用ヘルパー)**: `KeyAction` に `PartialEq` が無くコンパイルできず、`PartialEq` の追加は本番型の変更。既存の `assert_single_char` で足りる。
- **3 表の全面統合**: 上の表の理由で、proptest の入力の意味が変わるため採らない。
- **何もしない**: `ms()` と使い忘れ 21 か所の置換は低コストで読みやすさが上がるので、これよりは良い。

## 影響

テスト専用(`KeyAction` を含む本番コードは変更しない)。ホスト(Linux)で実行できる。
