# ADR-047: TickableFsm / ImeWarmupStrategy — 出力層 FSM の抽象化

## ステータス

採用済み（2026-06-22〜2026-06-23 実装）

## コンテキスト

### 問題: Output 構造体が具体的な FSM 型に直接依存していた

ADR-046（GjiFsm）、ADR-002（TSF coldstart warmup）が進むにつれ、
`output/mod.rs` の `Output` 構造体が複数の具体的 FSM 型を直接フィールドとして持ち、
新しいアプリ対応ごとに `Output` の型シグネチャが変化するという問題が生じた。

| フィールド | 型（変更前） | 問題 |
|---|---|---|
| `gji_fsm` | `GjiFsm` | Windows Terminal / WezTerm / Chrome で FSM が異なるのに同一型 |
| `pending_tsf` | `Option<TsfProbeMachine>` | ChromeProbe が必要になったが型を変えると呼び出し側全体に影響 |

また、`ComposingWarmup` が追加されたとき（commit 11404a5）、
`StartComposition` イベントが `pending_gji_key_response` と `current_gji_probe_id` を
同時に消失させるバグが発生した。これは FSM の責務境界が明確でなかったことが原因。

### 必要性

- アプリ種別（GJI native / MS-IME / Chrome / WezTerm）に応じて
  warmup 戦略を差し替えられる設計
- `pending_tsf` を任意の FSM に差し替えられる設計
- 呼び出し側（`Output`）が具体的な FSM を知らない設計

## 決定

2つのトレイトを新設し、`Output` が `Box<dyn Trait>` 経由で FSM を操作する。

### ImeWarmupStrategy トレイト（commit eb8b9d4）

```rust
/// GJI の warm/cold 状態管理と probe 発行を抽象化する。
/// `Output.gji_fsm` が `Box<dyn ImeWarmupStrategy>` を保持する。
pub(crate) trait ImeWarmupStrategy {
    fn is_composition_warm(&self) -> bool;
    fn dispatch_event(&mut self, event: GjiEvent, env: &TsfEnvSnapshot);
    fn on_focus_change(&mut self, env: &TsfEnvSnapshot);
}
```

実装:
- `GjiFsm`: GJI native アプリ（Chrome, WezTerm, Windows Terminal）
- `MsImeStrategy`: MS-IME / Google IME（VK モード、probe 不要）

### TickableFsm トレイト（commit d22e987）

```rust
/// 時間駆動 tick と event 受信を持つ FSM を抽象化する。
/// `Output.pending_tsf` が `Box<dyn TickableFsm>` を保持する。
pub(crate) trait TickableFsm {
    /// 時刻を受け取って内部状態を進める。完了時 None を返す。
    fn tick(&mut self, env: &TsfEnvSnapshot) -> Option<TickResult>;
    
    fn on_event(&mut self, event: TsfEvent) -> Option<TickResult>;
    
    /// この FSM がすでに終了済みか（外部からのクリーンアップ用）。
    fn is_done(&self) -> bool;
}
```

実装:
- `TsfProbeMachine`: GJI I/O 観測による probe（commit d22e987）
- `GjiWarmupFsm`: GJI cold-start専用 warmup FSM（commit fcd1b82）
- `ChromeProbe`: Chrome 用 SacrificialWarmup FSM（commit 51901c0）
- `LiteralDetectFsm`: warm パス・GJI post-transmit 共用（commit 8608062）

### Output 構造体の変化

```rust
// Before
pub(crate) struct Output {
    gji_fsm: GjiFsm,
    pending_tsf: Option<TsfProbeMachine>,
}

// After
pub(crate) struct Output {
    gji_fsm: Box<dyn ImeWarmupStrategy>,
    pending_tsf: Option<Box<dyn TickableFsm>>,
}
```

`pending_tsf` が `Box<dyn TickableFsm>` になったことで、
フォーカス変更時に `ChromeProbe` / `LiteralDetectFsm` を動的に差し込める。

### DispatchResult 型（commit 660ee19）

FSM 間の結果伝播を型安全にするため `DispatchResult` を導入した。

```rust
pub(crate) enum DispatchResult {
    /// FSM 継続中
    Pending,
    /// warmup 完了（is_composition_warm → true）
    WarmupComplete,
    /// リテラル化を検出（BS + 再送が必要）
    LiteralDetected { char_count: usize },
}
```

`StartLiteralDetect` イベントで `LiteralDetectFsm` を `pending_tsf` に差し込む
切り替えも `DispatchResult` の値で駆動する（commit 660ee19）。

### TickableFsm のセクション分け（commit a0311bf）

`TickableFsm` トレイトのメソッドをケイパビリティ別にドキュメントセクション分けし、
`ChromeProbe` が持っていた「死滅委譲」パターン（ChromeProbe が完了時に
次の FSM を生成して返す）を廃止した。

**理由**: FSM の「次の FSM を生成する」責務は呼び出し側（Output）が持つべき。
FSM が次の FSM を知ると依存方向が逆転し、テストが難しくなる。

## 結果

- `Output` が具体的な warmup アルゴリズムを知らない設計になった
- 新しいアプリ（Electron 系など）への対応が `ImeWarmupStrategy` impl の追加で完結
- `ChromeProbe` / `LiteralDetectFsm` が独立してユニットテスト可能
- `DispatchResult` により FSM 間の状態伝播が型安全になった

## 関連 ADR

- ADR-046: GjiFsm warm/cold FSM 一元管理（ImeWarmupStrategy の主要 impl）
- ADR-048: SacrificialWarmup（ChromeProbe = TickableFsm の主要 impl）
- ADR-034: GJI Direct Strategy（GjiFsm の設計背景）
- ADR-042: Clock トレイト（テスト可能性の設計思想）
