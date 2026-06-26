# ADR-051: HoldingGate の timed-fsm クレートへの移植

## ステータス

採用済み（2026-06-25 実装）

## コンテキスト

### HoldingGate の役割

`HoldingGate<M, T>` は「FSM が DrainHeld を返すまでキーイベントを保留する」
汎用ゲート機構で、NICOLA エンジンの同時打鍵判定に使われている。

```rust
pub enum GateAction {
    Hold,       // キーを保留
    Passthrough, // キーをそのまま通す
    DrainHeld,  // 保留キーを全て吐き出して通す
    Drop,       // キーを捨てる
}

pub struct HoldingGate<M, T> {
    machine: M,           // GateAction を返す FSM
    held: Vec<T>,         // 保留中のキー
}
```

### 問題: `awase` バイナリクレートにあった

`HoldingGate` と `GateAction` は `src/gate.rs`（`awase` バイナリクレート）に
定義されていた。`timed-fsm` クレートに移植すれば:

1. `timed-fsm` が「時間付き FSM」だけでなく「ゲート付き FSM」も提供できる
2. crates.io に公開した際に `HoldingGate` も利用可能になる
3. `awase-windows` が `src/gate.rs` に依存する経路が整理される

## 決定

### 移植先: `crates/timed-fsm/src/gate.rs`（commit ebbc9e4）

```rust
// timed-fsm/src/gate.rs
pub enum GateAction {
    Hold,
    Passthrough,
    DrainHeld,
    Drop,
}

pub struct HoldingGate<M, T> {
    machine: M,
    held: Vec<T>,
}

impl<M, T> HoldingGate<M, T>
where
    M: FnMut(&T) -> GateAction,
    T: Clone,
{
    pub fn new(machine: M) -> Self { ... }
    
    pub fn push(&mut self, item: T) -> Vec<T> { ... }
    
    pub fn apply_response(&mut self, action: GateAction) -> Vec<T> {
        // DrainHeld が複数 action に含まれる場合も extend(drain(..)) で安全に累積
        ...
    }
}
```

**8テストを追加**（AlwaysHold / AlwaysDrain / Toggle / TimedDrain の4種ダミーマシン使用）。

### re-export で既存コードへの影響ゼロ（commit ebbc9e4）

```rust
// src/gate.rs（変更後）
// 定義を削除し、timed-fsm の型を再エクスポートする
pub use timed_fsm::{GateAction, HoldingGate};
```

既存の `awase::gate::GateAction` / `awase::gate::HoldingGate` の参照は
変更なしに動作し続ける。

### `src/tsf.rs` の直接参照更新（commit ebbc9e4）

`src/tsf.rs` では `use crate::gate` を使っていたが、
`timed_fsm` を直接参照するよう更新した。

```rust
// Before
use crate::gate::{GateAction, HoldingGate};

// After
use timed_fsm::{GateAction, HoldingGate};
```

## なぜこのタイミングか

v1.4.0 リリース準備として `timed-fsm` の crates.io 公開（ADR-042 参照）を
進める中で、`HoldingGate` も公開 API に含めることが価値があると判断した。

NICOLA 同時打鍵判定のコアロジックである「ゲート」は、
他の日本語入力エミュレータや一般的なキーバッファリング用途でも使える。

## 結果

- `timed-fsm` が `Clock` トレイトに加え `HoldingGate` / `GateAction` を公開
- `awase` バイナリクレートの `gate.rs` が薄い re-export ファイルになった
- crates.io 公開時に HoldingGate が利用可能になる

## 関連 ADR

- ADR-042: Clock トレイト抽象化と timed-fsm のテスト可能性（crates.io 公開方針）
- ADR-008: 物理サム状態の分離（HoldingGate を使う NICOLA 同時打鍵の設計）
- ADR-015: シフト-リデュースパーサー（HoldingGate と組み合わさる FSM 設計）
