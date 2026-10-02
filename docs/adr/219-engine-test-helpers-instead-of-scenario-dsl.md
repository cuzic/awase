---
id: ADR-219
title: |-
  エンジンテストの読みにくさは、シナリオ DSL ではなくヘルパー 2 つとキー分類表の集約で解く(DSL は見送り)
summary: |-
  当初案「時刻つきキー列のテキスト DSL」は Opus round1 で不採用推奨となった(パイロット 5 本で行数基準が必ず不合格、DSL で書けるのは全 383 本中 40〜50 本、
  ミューテーション基準が変異体 0 件で合格する、実行器の無言の無検査経路)。読みにくさの実体は (1) マイクロ秒算術 `t0 + 30_000` と (2) 出力検査の 3 行であり、
  パーサなしの `ms()` と `assert_emits` で解ける。あわせて、VK→scan/位置/分類の表が tests.rs・proptest_tests.rs・tests/scenarios.rs の 3 か所に重複しているので 1 つに寄せる(撤去が主)。
status: |-
  提案(2026-10-02)。Opus round1 の指摘で当初の DSL 案から方針転換。round2 の再確認待ち。未実装。
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

### D1: ヘルパー 2 つを足す(パーサなし)

```rust
const fn ms(n: u64) -> Timestamp { n * 1000 }

#[track_caller]
fn assert_emits(r: &Resp, expected: &[KeyAction]) {
    r.assert_consumed();
    assert_eq!(r.actions.as_slice(), expected);
}
```

`assert_single_char`(`tests.rs:1625`、10 か所で使用)の一般化である。`#[track_caller]` により失敗位置はテスト関数内の正確な行になる。
**移行は機械的な置換に限る**: `exact` で書かれた検査(`assert_eq!(actions.len(), N)` + 各要素 `matches!`)だけを `assert_emits` に直す。`any` や `actions[0]` だけを見る検査は元の強さを保つため触らない(強くすると別テストの仕様変更になる)。

### D2: キー分類表の 3 重複を 1 つにする(これが本題)

VK→scan・位置・分類の表が `tests.rs:224-286`、`proptest_tests.rs:178-200` 付近、`tests/scenarios.rs:60-120` の 3 か所にある(round1 の指摘)。
`test_support.rs` には `VK_A` など 5 定数しかなく、`VK_D`/`SPACE`/`SHIFT`/`LALT` 等と `vk_to_scan`・`classify_test_key`・`test_vk_to_pos` は `tests.rs` ローカル。
これを `test_support` に寄せ、3 か所から使う。`tests/scenarios.rs` は統合テスト(別クレート扱い)なので `test_support`(`#[cfg(test)] pub(crate)`)を使えない。**共有できない場合は `tests.rs` と `proptest_tests.rs` の 2 つの統合に留める**(実装時に確認。確認できなければ scenarios.rs は触らない)。
同名で挙動が違う `TestHarness`(`proptest_tests.rs:129` は `set_thumb_shift_faces_enabled(true)` 付き)は、統合の際に差分を明記する。

### D3: 撤去が主、足すのはヘルパー 2 つだけ

本 ADR で足すのは D1 の 2 関数のみ。D2 は重複の撤去。新しい言語・パーサ・語彙は足さない。
将来テキスト形式を再検討する場合の再挑戦条件: D1/D2 実施後もなお「時系列が読めない」具体的なテスト 17 本以上を挙げられること、かつ `tests/scenarios.rs` 側に置く案(本番レイアウト・公開 API)を先に検討すること。

### D4: 中止・検証

- D1 の置換は、置換前後で該当テストの合否が変わらないこと(`cargo test --lib`)。`assert_emits` の `exact` 化で落ちるテストが出たら、そのテストは置換しない。
- D2 は、`cargo test --lib` と `cargo test --test scenarios`、`cargo nextest run --workspace --lib` が全て通ること。
- ミューテーション確認は不要(テスト専用の変更で、`#[cfg(test)]` のみの diff は変異体 0 件で意味がない。round1 Must-3)。

## 検討した代替案

- **テキスト DSL**: 上の理由で見送り。round1 Must 4 件と、書ける範囲の狭さ。
- **何もしない**: `ms()` と `assert_emits` は低コストで可読性が上がるので、これよりは良い。
- **`tests/scenarios.rs` に時系列テストを足す**: 公開 API と本番レイアウトを通る点で仕様例として価値が高い。D3 の再挑戦条件で先に検討する。
- **`macro_rules!`**: 期待の形が増えるたびにマクロが複雑になる。不要。

## 影響

テスト専用。本番コードに影響しない。ホスト(Linux)で実行できる。
