# ADR-065: conv 分類の純粋関数化と awase-windows の段階的プラットフォーム非依存化

## ステータス

採用済み（2026-06-30 実装、commits 5ee724b〜a30dcf4）

## コンテキスト

awase-windows クレートには Win32 API を呼ばないにもかかわらず
`#[cfg(windows)]` ゲートに閉じ込められたコードが多数存在していた：

1. **conv 分類ロジック**：`ImmGetConversionStatus` の u32 値から `InputModeState` への変換、
   前回値→今回値の差分判定、idle-conv-check 実行判断のガード条件。
   これらはすべて整数演算のみで Win32 API を呼ばないが、`key_pipeline.rs` に
   インライン実装されており Linux でテストできなかった。

2. **blanket `#![cfg(windows)]`**：lib.rs 先頭の1行で全モジュールを Windows 専用にしており、
   純粋なデータ型・定数・ユーティリティもコンパイル対象外になっていた。

3. **ConvMode / Charset の配置**：`awase-windows/state/conv_mode.rs` に定義されており、
   platform 非依存クレートである `nicola` から参照できなかった。

この状況により：
- conv 分類のバグを Linux CI で検出できない
- `cargo test -p awase` で実行できるテストが極めて少ない
- macOS/Linux ポートで同じロジックを再実装するリスクがある

## 決定

### Step 1: conv 分類ロジックを nicola の純粋関数として抽出

`nicola::engine` に以下を追加し、`Win32 API` を一切呼ばない pure fn として実装する：

| 関数 | 役割 |
|------|------|
| `classify_idle_conv(conv: u32, belief: ...) → Option<InputModeState>` | ImmGetConversionStatus 絶対値 → 入力モード判定 |
| `classify_conv_transition(prev: u32, curr: u32, belief: ...) → Option<InputModeState>` | 前回→今回 差分 → 入力モード判定 |
| `should_run_idle_conv_check(is_key_down, is_tsf_native, in_flight_ms, explicit_age_ms) → bool` | idle-conv-check 実行ガード 4 条件 |

### Step 2: ConvMode + Charset を nicola クレートに移動

```
nicola/src/engine/conv.rs  （新設）
```

- `Charset` enum・`ConvMode` struct を platform 非依存クレートに移動
- `ConvMode::from_u32()` / `.classify_idle()` / `.classify_transition()` / `.imm_conv_target()` をメソッド化
- `awase-windows/state/conv_mode.rs` はスタブ化して nicola から re-export + `ConvModeMgr` のみ残存
- `awase-windows/state/conv_classifier.rs` を削除（不要）
- `ConvMode` / `Charset` に `Display` impl 追加（ログ文字列から `{:?}` フォーマットを除去）

### Step 3: awase-windows の `#![cfg(windows)]` blanket を個別ゲーティングに移行

lib.rs 先頭から `#![cfg(windows)]` を除去し、各モジュールに個別 `#[cfg(windows)]` を付与：

**Windows 専用（ゲート維持）**：
`autostart, hook, ime, imm, observer, output, runtime, state（一部）, tsf` 等

**Linux でも純粋（ungated 化）**：
`focus/{cache,class_names}, scanmap, single_thread_cell, tuning`

### Step 4: state/ の純粋サブモジュールを #[cfg(windows)] から解放

`state/mod.rs` を再構成：

| 純粋（ungated） | Windows 専用 |
|----------------|-------------|
| `belief, hook_state, conv_mode, app_ime_policy` | `platform_state` |
| `force_guard, ime_event, ime_model` | `ime_decision_view` |
| `input_barrier, observation_store, transition` | `ime_event_log` |

Linux でのみ発生する `dead_code` 警告は `#[cfg_attr(not(windows), allow(dead_code))]` で局所抑制。

## テストカバレッジ

| 追加テスト | 件数 | 実行環境 |
|-----------|------|---------|
| `classify_idle_conv`（conv → InputModeState 全 conv 値） | 17 | Linux CI |
| `classify_conv_transition`（差分判定全ケース） | 13 | Linux CI |
| `should_run_idle_conv_check`（ガード条件境界値） | 11 | Linux CI |
| `ConvMode` メソッド群（from_u32 / imm_conv_target 等） | 34 | Linux CI |

**合計 75 件**のテストが `cargo test -p awase` で Linux 上で実行可能になった。

## 削減されたコード

| ファイル | 削減量 |
|---------|-------|
| `key_pipeline.rs`（idle-conv-check インライン実装） | 約 85 行 → 35 行（-50 行） |
| `key_pipeline.rs`（idle-conv-check ガード条件） | 30 行 → 12 行（-18 行） |
| `observer/ime_observer.rs`（input_mode_from_conversion） | 40 行圧縮 |
| `engine/decision.rs` | 400+ 行削除（移行後の残骸撤去） |
| `state/conv_classifier.rs` | 全削除 |

## 検討した代替案

**awase-windows 内に test module を置いて #[cfg(test)] だけ Windows 依存なしで動かす**
→ 採用しなかった。テストが Windows 専用モジュールと同じ cfg ゲートに入るため
  Linux CI から実行できない。pure fn として nicola に出すことで CI で常時検証できる。

**全 conv 分類を trait 化して DI する**
→ 採用しなかった。Conv 分類は副作用のない数値変換であり、trait を挟む必要はない。
  trait は I/O 境界か OS API が必要な場合に限定する（ADR-014 方針）。

## 変更ファイル概要

| ファイル | 変更内容 |
|---------|---------|
| `nicola/src/engine/conv.rs` | 新設（Charset, ConvMode, 純粋関数群, テスト75件） |
| `nicola/src/engine/decision.rs` | classify_idle_conv / classify_conv_transition / should_run_idle_conv_check を追加後、conv.rs 移行完了後に削除 |
| `nicola/src/engine/mod.rs` | pub use conv::* を追加 |
| `awase-windows/src/lib.rs` | blanket cfg 除去・個別ゲーティング・定数の ungated 化 |
| `awase-windows/src/state/mod.rs` | 純粋 vs Windows 専用の分離 |
| `awase-windows/src/state/conv_mode.rs` | ConvMode/Charset 撤去 → nicola re-export + ConvModeMgr のみ残存 |
| `awase-windows/src/state/conv_classifier.rs` | 削除 |
| `awase-windows/src/runtime/key_pipeline.rs` | classify_idle_conv / should_run_idle_conv_check 委譲 |
| `awase-windows/src/observer/ime_observer.rs` | classify_conv_transition 委譲 |
| `awase-windows/src/focus/mod.rs` | Windows 専用サブモジュールの個別ゲーティング |

## 関連 ADR

- ADR-019: lib クレートのプラットフォーム非依存化（本 ADR の延長線）
- ADR-022: クロスプラットフォームクレート構造設計
- ADR-064: ConvModePolicy（同日実装、conv 制御の別側面）
