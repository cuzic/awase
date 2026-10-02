---
id: ADR-219
title: |-
  エンジンの同時打鍵テストを「時刻つきキー列 → 期待出力」のテキスト形式で書けるようにする
summary: |-
  `src/engine/tests.rs`(10,129 行)の多くは、`engine.on_event(Ev::down(VK_A).at(t).build())` と `assert_pending` / `assert!(matches!(result.actions[0], KeyAction::Char('う')))` を
  交互に並べる形で、時系列と期待出力が読み取りにくい。1 行 1 イベントのテキスト(`A↓@0  CONVERT↓@30  timeout  => ゔ`)で書ける小さなパーサ + 実行器を
  テスト専用に足し、既存テストは「そのまま書き直せるもの」だけを移す。proptest や journal replay とは別物として置く。
  シナリオ数 5 本の移行で読みやすさと行数が改善しなければ中止する。
status: |-
  提案(2026-10-02)。未レビュー。実装なし。
related_adr:
  - "ADR-115"
  - "ADR-158"
  - "ADR-163"
---

# ADR-219: 同時打鍵テストの時刻つきキー列 DSL

## 背景

`src/engine/tests.rs` は 10,129 行。`Ev::down` / `Ev::up` の builder 呼び出しは 672 か所ある。例(`test_pattern2_char_first_then_thumb`):

```rust
let mut engine = make_engine();
let result = engine.on_event(Ev::down(VK_A).at(0).build());
assert_pending(&result);
let result = engine.on_event(Ev::down(VK_CONVERT).at(30_000).build());
assert_pending(&result);
let result = engine.on_timeout(TIMER_PENDING);
result.assert_consumed();
assert_eq!(result.actions.len(), 1);
assert!(matches!(result.actions[0], KeyAction::Char('ゔ')));
```

NICOLA の同時打鍵は「どのキーがいつ押され、どの時点で何が確定するか」が仕様そのものなので、テストが読みたいのは時系列表である。
今の形では (1) 時刻が `t0 + 30_000` のようにマイクロ秒の算術に埋もれる、(2) 各ステップの期待(pending / 消費 / 出力)が `assert_*` の呼び出しに分散する、
(3) 閾値 100ms の前後・確認モードの違いのように「同じ列を条件違いで繰り返す」テストが、列をコピーして書かれる、という読みにくさがある。

すでにある資産との関係:

- `Ev` builder は存在するが、DSL ではなく構築の短縮にとどまる。
- `src/engine/proptest_tests.rs`: 性質テスト。具体的な時系列の仕様例には向かない。
- journal replay(`awase-windows`): 実機記録の再生で、プラットフォーム側の `classify_*` が対象。エンジン内部の仕様例を人が書く用途ではない。
- `tests/scenarios.rs`: `config.toml` と `layout/nicola.yab` を読む結合テスト。DSL の置き場所の候補だが、`tests.rs` 側の内部 API(`TestHarness`)は使えない。

## 決定

### D1: 1 行 1 イベントのテキスト形式

```
# 文字キー → 親指キー(30ms 後)→ タイムアウトで同時打鍵として確定
A↓@0        => pending
CONVERT↓@30 => pending
timeout     => out:ゔ
```

- 行: `<キー><↓|↑>@<ms>` または `timeout [<timer>]` または `end`、続けて `=> <期待>`(省略可)。
- 期待は `pending`(消費されて保留)、`pass`(OS へ通す)、`out:<文字列>`(`KeyAction::Char` の連なり)、`consumed`。**書いた行だけを検査する**(書かない行は何も検査しない)。
- 時刻の単位はミリ秒で書き、内部のマイクロ秒に変換する(`Ev::at` の単位を `tests.rs` で確認してから実装する。現在の `t0 + 30_000` からマイクロ秒と推測しているが未確認)。
- キー名は既存の `test_support` の定数名(`A`, `S`, `CONVERT`, `NONCONVERT`)に限る。新しいキーの追加は `test_support` に定数を足すのと同じ作業にする。

### D2: 実行器は `tests.rs` 内のテスト専用ヘルパー 1 つ

```rust
fn run_scenario(harness: &mut TestHarness, script: &str) -> Vec<Resp>
```

行ごとに `engine.on_event` / `on_timeout` を呼び、期待がある行だけ検査する。失敗時は **シナリオの行番号と、その行の原文** を表示する(今の `assert_eq!` は行番号がテスト関数内の位置になる)。
パーサは 100 行以内を目安にする。それを超えそうなら、機能を削るか中止する。

### D3: 移すのは「そのまま書き直せるもの」だけ

次は移さない。

- `engine.layout.normal.insert(...)` のようにレイアウトを書き換えるテスト(セットアップがシナリオに収まらない)。
- `InputContext` のフィールド(`ime_on`, `input_mode`, 修飾キー保持)を途中で変えるテスト。
- 内部状態(`fsm.state`)を直接検査するテスト。

必要になったら D1 に `ctx ime=off` のような行を足すが、**最初のパイロットでは足さない**。

### D4: パイロットと中止基準

最初に移すのは `test_pattern*` 系の 5 本。次のどれかに当たれば残りを移さず、本 ADR を「取り下げ」にして実行器を削除する。

- 5 本の移行後、実行器 + シナリオの合計行数が移行前より少なくならない。
- 5 本のうち 2 本以上で、D1 の語彙(pending/pass/out/consumed)で元の `assert` が表せない。
- 移行した 5 本に対して、**元のテストを壊すミューテーション(例: 閾値の比較演算子の反転)が、移行後のシナリオでも落ちる**ことを、`cargo mutants --in-diff` の diff 限定実行か手動のミューテーションで確認できない。DSL で書いたテストが元より弱くなっていないことの確認である。

### D5: `ConfirmMode` と閾値の組合せ展開は別 ADR

「同じ列を条件違いで繰り返す」展開(`for mode in [Wait, NgramPredictive] { ... }`)は魅力的だが、期待値が条件で変わるので表現が必要になり、DSL が膨らむ。本 ADR では扱わない。

## 期待する効果と、まだ測っていない点

- 時系列と期待出力が 1 画面で読める。シナリオの行番号つきで失敗する。
- 行数: 1 テストあたり 10〜20 行が 4〜8 行になる見込み。ただし `Resp` の `assert_*` の語彙が実際にどれだけ必要かを、パイロットで測る。
- 懸念: **DSL の実行器に誤りがあると、移した全テストが一斉に意味を失う**。D4 の 3 つ目の基準(ミューテーションで元と同じ強さか)はこのためにある。

## 検討した代替案

- **何もしない**: 動いていて、`Ev` builder で既に短い。読みにくさは主観的。→ D4 の基準を満たせなければ、これが結論になる。
- **`macro_rules!` で `scenario! { A down at 0 => pending; ... }`**: パーサを書かずに済むが、エラー位置が悪く、`=>` の期待の形を増やすたびにマクロが複雑になる。テキスト形式のほうが、journal やログから貼り付けやすい。
- **journal replay を使う**: プラットフォーム側の入力列を再生する基盤で、エンジンの仕様例を手で書く用途ではない。journal のフォーマットが `awase-windows` 側にあり、ルート crate の `cargo test --lib` から使えない。
- **既存の proptest を増やす**: 性質は書けるが、「この時系列でこの出力」という仕様例の可読性は上がらない。
- **外部ファイル(`.scn`)にして `include_str!`**: 可読性は上がるが、テスト名とシナリオが離れ、`cargo test <name>` で実行しにくい。まず文字列リテラルで始める。

## 影響

- テスト専用。本番コードに影響しない。ホスト(Linux)の `cargo test --lib` で実行できる。
- `src/engine/test_support.rs` に、キー名 → `VkCode` の対応を足す必要がある。
