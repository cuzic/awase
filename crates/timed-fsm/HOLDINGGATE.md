# HoldingGate 統合引継書

**対象**: `src/gate.rs` の `HoldingGate<M, T>` を `crates/timed-fsm/` に移植する  
**作成日**: 2026-06-25  
**ステータス**: 未着手（spike 未実施）

---

## 1. 概要

### HoldingGate が解く問題

「ある条件が成立している間だけ入力アイテムを溜め、drain（全解放）か drop（廃棄）をステートマシンが決める」バッファが欲しい場面がある。

典型的な要件：
- フォーカス切替直後の 50ms 間はキーイベントを保留し、IME 確定後にまとめて流す
- IME ウォームアップが完了しない場合はそのままバッファを捨てる
- 容量を超えたら保留をやめてスルーさせる（back-pressure）

シンプルな `Vec` では「いつ drain するか」の判断ロジックが呼び出し元に漏れる。
`HoldingGate` はその判断を `TimedStateMachine` に委譲し、呼び出し元は
`try_hold()` → `on_event()` → `on_timeout()` の 3 メソッドだけを叩けばよい。

### なぜ timed-fsm に統合するのか

現状、`HoldingGate` は `awase` クレート（`src/gate.rs`）に置かれているが、
以下の理由から `crates/timed-fsm` に移すべきである：

1. **依存の方向が逆**  
   `HoldingGate` は `timed_fsm::TimedStateMachine` と `timed_fsm::Response` に依存する。
   ゲートが `awase` に残ると `awase → timed-fsm` の依存だけでなく、
   「timed-fsm のユーザーが HoldingGate を使いたければ awase に依存しなければならない」
   という逆転が起きる。

2. **汎用性**  
   `HoldingGate` の本体に awase 固有のロジックは一切ない。
   `TimedStateMachine<Action = GateAction>` を実装しているマシンならどれでも差し込める。
   Go2 Runtime v1 など別プロジェクトでも再利用できる（§6 参照）。

3. **SyncKeyGateMachine は awase 固有なので残す**  
   sync key（IME ON/OFF キー）は awase ドメイン概念。`SyncKeyGateMachine` は
   awase 側に残し、timed-fsm には移さない。

---

## 2. 現在のコード所在

### ファイル

```
rust-nicola/
  src/
    gate.rs          ← HoldingGate / GateAction / SyncKeyGateMachine（現在地）
    tsf.rs           ← TsfGateMachine + TsfGate（HoldingGate の利用側）
    lib.rs           ← pub mod gate;
  crates/
    timed-fsm/
      src/
        lib.rs       ← 統合先
```

### GateAction（`src/gate.rs` L17-23）

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateAction {
    /// 保留モード開始（アイテムを held バッファに蓄積し始める）
    InitiateHold,
    /// 保留解除・全アイテムをドレインする
    DrainHeld,
}
```

`GateAction` は `HoldingGate` の動作語彙そのもので、awase 固有ではない。
**timed-fsm 側に移動する**（後述 §3 参照）。

### HoldingGate 構造体（`src/gate.rs` L29-38）

```rust
pub struct HoldingGate<M, T>
where
    M: TimedStateMachine<Action = GateAction>,
{
    pub machine: M,   // pub: 呼び出し元が state() 等を直接参照するケースがある
    held: Vec<T>,     // 保留バッファ（private）
    capacity: usize,  // 最大保留数（private）
    holding: bool,    // 現在保留中かどうか（private）
}
```

### HoldingGate メソッド一覧

| メソッド | 公開 | 説明 |
|----------|------|------|
| `new(machine, capacity)` | `pub const fn` | 初期化。`const fn` なので定数として置ける |
| `try_hold(&mut self, item: T) -> bool` | `pub fn` | `true` = 保留した。`false` = 非保留か容量超過 |
| `len() -> usize` | `pub const fn` | バッファ長 |
| `is_empty() -> bool` | `pub const fn` | バッファ空判定 |
| `is_holding() -> bool` | `pub const fn` | 現在保留中か |
| `clear()` | `pub fn` | バッファ＋保留フラグを強制リセット（緊急用） |
| `on_event(event) -> (Response, Vec<T>)` | `pub fn` | マシンにイベントを渡し、drain があれば Vec で返す |
| `on_timeout(id) -> (Response, Vec<T>)` | `pub fn` | タイムアウトをマシンに渡し、drain があれば Vec で返す |
| `apply_response(&Response) -> Vec<T>` | `fn`（private） | GateAction を解釈して held を更新する内部ヘルパー |

### try_hold の設計意図

```rust
pub fn try_hold(&mut self, item: T) -> bool {
    if !self.holding {
        return false; // 非保留中 → 呼び出し元はスルーとして扱う
    }
    if self.held.len() >= self.capacity {
        return false; // 容量超過 → 呼び出し元が強制解除すること
    }
    self.held.push(item);
    true
}
```

容量超過で `false` を返すのは「バッファが溢れた事実を呼び出し元に知らせ、
呼び出し元が `gate.clear()` を呼ぶか、アイテムをスルーするかを選ばせる」設計。
`HoldingGate` 自身は back-pressure ポリシーを持たない（単一責任）。

---

## 3. timed-fsm への統合方針

### 追加ファイル

```
crates/timed-fsm/src/gate.rs   ← 新規作成（HoldingGate + GateAction）
```

### lib.rs への追加（`crates/timed-fsm/src/lib.rs`）

現在の末尾：

```rust
pub use clock::{Clock, ManualClock, MonotonicClock};
pub use dispatch::{ActionExecutor, TimerRuntime};
pub use machine::TimedStateMachine;
pub use parser::{ParseAction, ShiftReduceParser};
pub use response::{Response, TimerCommand};
```

追加行：

```rust
pub mod gate;
pub use gate::{GateAction, HoldingGate};
```

### GateAction を timed-fsm 側に定義する理由

`GateAction` は `HoldingGate` が解釈する Action の語彙であり、
「どのマシンが `Action = GateAction` を実装するか」は awase・robotics 側が決める。
型の定義が timed-fsm にあることで、依存の逆転が起きない：

```
awase（TsfGateMachine）  ─ implements ─→  TimedStateMachine<Action = GateAction>
                                                   ↑
robotics（PendingStateMachine）                  timed-fsm に定義
```

`GateAction` を awase に残すと robotics が awase に依存しなければならず不適切。

### Action を型パラメータで外出しするか？

「`HoldingGate<M, A, T>` として Action を外から渡す」案も考えられるが採用しない。

理由：`HoldingGate` の `apply_response` は `InitiateHold` / `DrainHeld` の
**2 variant** を具体的に match する。Action を抽象化すると match できず、
「drain のタイミングを機械が決める」というコアな設計意図が trait 越しの
コールバックに散らばる。2 variant の固定語彙を持つ `GateAction` enum が
最もシンプルで保守しやすい。

---

## 4. API 設計案

移植後の公開 API（`crates/timed-fsm/src/gate.rs`）：

```rust
use crate::{Response, TimedStateMachine};

/// ゲートマシンが emit するアクション語彙。
///
/// [`HoldingGate`] と組み合わせる [`TimedStateMachine`] 実装は
/// `type Action = GateAction` と宣言してこれを使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateAction {
    /// 保留モード開始（held バッファへの蓄積を開始する）
    InitiateHold,
    /// 保留解除・held バッファ全体を呼び出し元へ返す
    DrainHeld,
}

/// 汎用ホールディングゲート。
///
/// マシン `M` が [`GateAction::InitiateHold`] を emit したら保留モードに入り、
/// [`GateAction::DrainHeld`] を emit したら保留解除して溜めたアイテムを返す。
///
/// # 型パラメータ
///
/// - `M`: ゲートを制御するステートマシン。`TimedStateMachine<Action = GateAction>` を実装すること。
/// - `T`: 保留するアイテムの型。任意。
#[derive(Debug)]
pub struct HoldingGate<M, T>
where
    M: TimedStateMachine<Action = GateAction>,
{
    pub machine: M,
    held: Vec<T>,
    capacity: usize,
    holding: bool,
}

impl<M, T> HoldingGate<M, T>
where
    M: TimedStateMachine<Action = GateAction>,
{
    /// 新しい `HoldingGate` を生成する。`capacity` を超えた場合 `try_hold` は `false` を返す。
    pub const fn new(machine: M, capacity: usize) -> Self;

    /// アイテムの保留を試みる。
    ///
    /// - `true`: 保留成功。呼び出し元はこのアイテムを「消費済み」として扱う。
    /// - `false`: 非保留状態 or 容量超過。呼び出し元がアイテムをスルーするか強制解除するかを決める。
    pub fn try_hold(&mut self, item: T) -> bool;

    pub const fn len(&self) -> usize;
    pub const fn is_empty(&self) -> bool;
    pub const fn is_holding(&self) -> bool;

    /// バッファと保留フラグを強制リセットする（パニックリセット・安全フィルタ用）。
    pub fn clear(&mut self);

    /// イベントをマシンに渡し、drain されたアイテムを返す。
    ///
    /// 戻り値: `(Response<GateAction, M::TimerId>, Vec<T>)`
    /// - `Response` はタイマーコマンドを含む。呼び出し元が `dispatch` する。
    /// - `Vec<T>` は `DrainHeld` アクションが emit されたときのみ非空。
    pub fn on_event(&mut self, event: M::Event) -> (Response<GateAction, M::TimerId>, Vec<T>);

    /// タイムアウトをマシンに渡し、drain されたアイテムを返す。
    pub fn on_timeout(&mut self, id: M::TimerId) -> (Response<GateAction, M::TimerId>, Vec<T>);
}
```

awase 側（`src/gate.rs`）に残すもの：

```rust
// awase 固有のゲートマシン
pub struct SyncKeyGateMachine { ... }
pub enum SyncKeyGateState { Inactive, Active }
pub enum SyncKeyGateEvent { Activate, Deactivate }
impl TimedStateMachine for SyncKeyGateMachine {
    type Action = GateAction; // timed-fsm から re-export された GateAction を使う
    ...
}
```

---

## 5. テスト方針

### 既存テスト（`src/gate.rs` L205-328）

現在 `src/gate.rs` の `#[cfg(test)]` ブロックに 10 ケースある：

| テスト名 | 検証内容 |
|----------|----------|
| `try_hold_returns_false_when_not_holding` | 非保留中は try_hold が false |
| `activate_then_hold_then_deactivate_drains` | Activate → hold × 3 → Deactivate でドレイン |
| `capacity_overflow_returns_false` | capacity=2 で 3 個目は false |
| `clear_resets_holding_and_buffer` | clear() で holding=false・バッファ空 |
| `reactivate_clears_previous_buffer` | 再 Activate でバッファがクリアされる |
| `sync_machine_initial_state_is_inactive` | SyncKeyGateMachine 初期状態 |
| `sync_machine_activate_emits_initiate_hold` | Activate → InitiateHold |
| `sync_machine_deactivate_from_active_emits_drain` | Deactivate（Active中）→ DrainHeld |
| `sync_machine_deactivate_from_inactive_is_pass_through` | Deactivate（Inactive中）→ pass_through |
| `sync_machine_reactivate_still_emits_initiate_hold` | 連続 Activate でも InitiateHold |

### timed-fsm 側に移すテスト

`SyncKeyGateMachine` は awase に残るので、timed-fsm 側のテストは
ダミーマシンで `HoldingGate` 本体の動作を検証する：

```rust
// crates/timed-fsm/src/gate.rs の #[cfg(test)] に追加
struct AlwaysHoldMachine;      // on_event で常に InitiateHold
struct AlwaysDrainMachine;     // on_event で常に DrainHeld
struct TimedDrainMachine { .. } // on_timeout で DrainHeld（タイマー付き）
```

追加すべきテストケース：

| テストケース | 理由 |
|-------------|------|
| `hold_then_drain_via_event` | 基本フロー（InitiateHold → try_hold × N → DrainHeld） |
| `hold_then_drop_via_clear` | drop パス（clear による強制解除） |
| `capacity_overflow_false_and_caller_decides` | 容量超過 → false を返す（呼び出し元責任の明示） |
| `reactivate_clears_buffer` | 再 InitiateHold で既存バッファがクリアされることを確認 |
| `drain_via_timeout` | タイマー駆動の drain（タイマーマシン経由） |
| `on_event_returns_empty_vec_when_no_drain` | drain なしのとき Vec が空 |
| `multiple_drain_actions_in_one_response` | Response に DrainHeld が複数 emit されても安全 |

### awase 側に残すテスト

`SyncKeyGateMachine` 関連の 5 ケース（`sync_machine_*`）は `src/gate.rs` に残す。

---

## 6. Go2 Runtime v1 での想定用途

### 位置づけ

Go2 Runtime v1 の **C 層（Collector / ingest）** では、センサーから届いた生イベントを
一定条件が揃うまで保留し、条件成立後にまとめて後段に流す処理が必要になる。
`HoldingGate` はそのバッファとして使う想定。

### 型の例

```rust
// C 層のインジェスト処理（概念コード）
use timed_fsm::{GateAction, HoldingGate};

/// 通信確立待ちの間、受信イベントを保留するステートマシン。
/// 接続確立 → DrainHeld、タイムアウト or 切断 → clear()。
struct PendingStateMachine { ... }

impl TimedStateMachine for PendingStateMachine {
    type Event  = ConnEvent;
    type Action = GateAction;      // timed-fsm の GateAction を使う
    type TimerId = PendingTimer;
    ...
}

// 使い方
let mut gate: HoldingGate<PendingStateMachine, RawSensorEvent> =
    HoldingGate::new(PendingStateMachine::new(), 64);

// 受信ループ内
if gate.try_hold(raw_event) {
    // 保留成功 → 次の tick まで待つ
} else {
    // 保留失敗（非保留 or 容量超過）→ スルーか強制 clear
}

// 接続イベント到着時
let (_resp, drained) = gate.on_event(ConnEvent::Established);
for event in drained {
    pipeline.ingest(event); // まとめて後段へ
}
```

### SCOPED C 層（`scoped_client_*.py` 相当の Rust 側）

`sim/config/scoped_client_*.py` で現在 Python で書かれている ingest 処理を
将来 Rust に移植する際、`HoldingGate<PendingStateMachine, RawEvent>` で
「DDS セッション確立待ちの間はイベントをバッファし、確立後に drain」
というパターンを型安全に表現できる。

---

## 7. 作業チェックリスト

移植時の作業順（参考）：

- [ ] `crates/timed-fsm/src/gate.rs` を新規作成（`GateAction` + `HoldingGate` のみ）
- [ ] `crates/timed-fsm/src/lib.rs` に `pub mod gate; pub use gate::{GateAction, HoldingGate};` を追加
- [ ] `crates/timed-fsm/src/gate.rs` に `#[cfg(test)]` でダミーマシンを使ったテストを追加
- [ ] `src/gate.rs` の `use timed_fsm::{Response, TimedStateMachine};` を `use timed_fsm::{GateAction, HoldingGate, Response, TimedStateMachine};` に変更
- [ ] `src/gate.rs` から `GateAction` と `HoldingGate` の定義を削除し、re-export のみにするか完全に除去
- [ ] `SyncKeyGateMachine` は `src/gate.rs` に残したまま `GateAction` を timed-fsm からインポート
- [ ] `src/tsf.rs` の `use crate::gate::{GateAction, HoldingGate};` を `use timed_fsm::{GateAction, HoldingGate};` に変更
- [ ] `cargo test -p timed-fsm` で新テスト全通過を確認
- [ ] `cargo test` でワークスペース全体を確認
- [ ] `cargo clippy --workspace -- -D warnings` でリント通過を確認

---

## 8. 注意事項

### `pub machine: M` フィールドの公開範囲

現在 `machine` フィールドが `pub` になっている。これは `TsfGate` 等が
`gate.inner.machine.state()` を直接参照するためだが、timed-fsm の汎用型として
公開する場合は `pub(crate)` か getter メソッドに格下げを検討すること。
（完全強制は crate 分割後。現状は規律ベース）

### `const fn new()` の維持

`new()` が `const fn` であることを維持すること。定数として初期化できる
ことが組み込みユースケースで有用。`Vec::new()` は `const` なので問題ない。

### timed-fsm の依存ゼロ方針

`crates/timed-fsm/Cargo.toml` は `[dependencies]` が空（依存ゼロ）。
`HoldingGate` を追加しても `std` のみで実装できるため、この方針を維持できる。
