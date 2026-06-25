//! 汎用ホールディングゲート。
//!
//! [`HoldingGate<M, T>`] は [`TimedStateMachine`] が [`GateAction`] を emit することで
//! アイテムの保留/解放を制御する汎用ラッパー。
//!
//! # 動作
//!
//! - マシンが [`GateAction::InitiateHold`] を emit → 保留モード開始（`try_hold` でアイテムを蓄積）
//! - マシンが [`GateAction::DrainHeld`] を emit → 保留モード解除し全アイテムを返す
//! - 容量超過時は `try_hold` が `false` を返す（呼び出し元がゲートを強制解除すること）
//!
//! # 実装側の契約
//!
//! `HoldingGate` と組み合わせるステートマシンは `type Action = GateAction` を宣言する。
//! ゲートの動作語彙（`InitiateHold` / `DrainHeld` の 2 variant）はこのクレートに定義されており、
//! マシン実装はどのプロジェクトにあっても同じ型を参照できる。
//!
//! ```
//! use timed_fsm::{GateAction, HoldingGate, Response, TimedStateMachine};
//!
//! struct AlwaysHold;
//! impl TimedStateMachine for AlwaysHold {
//!     type Event  = ();
//!     type Action = GateAction;
//!     type TimerId = ();
//!     fn on_event(&mut self, _: ()) -> Response<GateAction, ()> {
//!         Response::emit_one(GateAction::InitiateHold)
//!     }
//!     fn on_timeout(&mut self, _: ()) -> Response<GateAction, ()> {
//!         Response::pass_through()
//!     }
//! }
//!
//! let mut gate: HoldingGate<AlwaysHold, u32> = HoldingGate::new(AlwaysHold, 8);
//! gate.on_event(());          // InitiateHold → 保留モード開始
//! gate.try_hold(42);          // バッファに蓄積
//! ```

use crate::{Response, TimedStateMachine};

/// ゲート用共有アクション。
///
/// [`HoldingGate`] と組み合わせる [`TimedStateMachine`] 実装は
/// `type Action = GateAction` と宣言してこれを使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateAction {
    /// 保留モード開始（アイテムを `held` バッファに蓄積し始める）。
    InitiateHold,
    /// 保留解除・全アイテムをドレインする。
    DrainHeld,
}

/// 汎用ホールディングゲート。
///
/// マシン `M` が [`GateAction::InitiateHold`] を emit するとアイテムの蓄積を開始し、
/// `M` が [`GateAction::DrainHeld`] を emit すると蓄積アイテムを全て返す。
///
/// # 型パラメータ
///
/// - `M`: ゲートを制御するステートマシン。`TimedStateMachine<Action = GateAction>` を実装すること。
/// - `T`: 保留するアイテムの型。任意。
///
/// # 例
///
/// ```
/// use timed_fsm::{GateAction, HoldingGate, Response, TimedStateMachine};
///
/// struct Toggle(bool);
/// impl TimedStateMachine for Toggle {
///     type Event  = bool; // true=hold, false=drain
///     type Action = GateAction;
///     type TimerId = ();
///     fn on_event(&mut self, hold: bool) -> Response<GateAction, ()> {
///         if hold {
///             Response::emit_one(GateAction::InitiateHold)
///         } else {
///             Response::emit_one(GateAction::DrainHeld)
///         }
///     }
///     fn on_timeout(&mut self, _: ()) -> Response<GateAction, ()> {
///         Response::pass_through()
///     }
/// }
///
/// let mut gate: HoldingGate<Toggle, u32> = HoldingGate::new(Toggle(false), 8);
/// gate.on_event(true);              // 保留開始
/// assert!(gate.try_hold(1));
/// assert!(gate.try_hold(2));
/// let (_, drained) = gate.on_event(false); // 解放
/// assert_eq!(drained, vec![1, 2]);
/// ```
#[derive(Debug)]
pub struct HoldingGate<M, T>
where
    M: TimedStateMachine<Action = GateAction>,
{
    /// ゲートを制御するステートマシン。
    ///
    /// 呼び出し元が `state()` 等を直接参照するケースがあるため `pub`。
    pub machine: M,
    held: Vec<T>,
    capacity: usize,
    holding: bool,
}

impl<M, T> HoldingGate<M, T>
where
    M: TimedStateMachine<Action = GateAction>,
{
    /// 新しい `HoldingGate` を生成する。
    ///
    /// `capacity` を超えた場合 [`try_hold`](Self::try_hold) は `false` を返す。
    /// `const fn` なので静的初期化・定数として使用できる。
    pub const fn new(machine: M, capacity: usize) -> Self {
        Self {
            machine,
            held: Vec::new(),
            capacity,
            holding: false,
        }
    }

    /// アイテムをバッファに追加する。
    ///
    /// - `true` : 保留成功。呼び出し元はこのアイテムを「消費済み」として扱う。
    /// - `false`: 非保留状態 or 容量超過。呼び出し元がアイテムをスルーするか
    ///   [`clear`](Self::clear) で強制解除するかを決める。
    pub fn try_hold(&mut self, item: T) -> bool {
        if !self.holding {
            return false;
        }
        if self.held.len() >= self.capacity {
            return false;
        }
        self.held.push(item);
        true
    }

    /// バッファの長さを返す。
    #[must_use]
    pub const fn len(&self) -> usize {
        self.held.len()
    }

    /// バッファが空かどうかを返す。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// 現在保留モードかどうかを返す。
    #[must_use]
    pub const fn is_holding(&self) -> bool {
        self.holding
    }

    /// バッファをクリアし、保留モードも解除する。
    ///
    /// パニックリセット・安全フィルタ等の緊急用。
    /// 通常は `on_event` / `on_timeout` 経由でマシンに DrainHeld を emit させること。
    pub fn clear(&mut self) {
        self.held.clear();
        self.holding = false;
    }

    /// イベントをマシンに渡し、ドレインされたアイテムを返す。
    ///
    /// 戻り値:
    /// - `Response`: タイマーコマンドを含む。呼び出し元が `dispatch` する。
    /// - `Vec<T>`: `DrainHeld` が emit されたときのみ非空。
    pub fn on_event(&mut self, event: M::Event) -> (Response<GateAction, M::TimerId>, Vec<T>) {
        let resp = self.machine.on_event(event);
        let drained = self.apply_response(&resp);
        (resp, drained)
    }

    /// タイムアウトをマシンに渡し、ドレインされたアイテムを返す。
    pub fn on_timeout(&mut self, id: M::TimerId) -> (Response<GateAction, M::TimerId>, Vec<T>) {
        let resp = self.machine.on_timeout(id);
        let drained = self.apply_response(&resp);
        (resp, drained)
    }

    fn apply_response(&mut self, resp: &Response<GateAction, M::TimerId>) -> Vec<T> {
        let mut drained = Vec::new();
        for action in &resp.actions {
            match action {
                GateAction::InitiateHold => {
                    self.held.clear();
                    self.holding = true;
                }
                GateAction::DrainHeld => {
                    self.holding = false;
                    drained.extend(self.held.drain(..));
                }
            }
        }
        drained
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Response;

    // ── テスト用ダミーマシン ──────────────────────────────────────────

    /// on_event で常に InitiateHold を emit するマシン。
    struct AlwaysHoldMachine;

    impl TimedStateMachine for AlwaysHoldMachine {
        type Event = ();
        type Action = GateAction;
        type TimerId = ();

        fn on_event(&mut self, _: ()) -> Response<GateAction, ()> {
            Response::emit_one(GateAction::InitiateHold)
        }

        fn on_timeout(&mut self, _: ()) -> Response<GateAction, ()> {
            Response::pass_through()
        }
    }

    /// on_event で常に DrainHeld を emit するマシン。
    struct AlwaysDrainMachine;

    impl TimedStateMachine for AlwaysDrainMachine {
        type Event = ();
        type Action = GateAction;
        type TimerId = ();

        fn on_event(&mut self, _: ()) -> Response<GateAction, ()> {
            Response::emit_one(GateAction::DrainHeld)
        }

        fn on_timeout(&mut self, _: ()) -> Response<GateAction, ()> {
            Response::pass_through()
        }
    }

    /// bool イベントで Hold/Drain を切り替えるマシン。
    struct ToggleMachine;

    impl TimedStateMachine for ToggleMachine {
        type Event = bool; // true=InitiateHold, false=DrainHeld
        type Action = GateAction;
        type TimerId = ();

        fn on_event(&mut self, hold: bool) -> Response<GateAction, ()> {
            if hold {
                Response::emit_one(GateAction::InitiateHold)
            } else {
                Response::emit_one(GateAction::DrainHeld)
            }
        }

        fn on_timeout(&mut self, _: ()) -> Response<GateAction, ()> {
            Response::pass_through()
        }
    }

    /// on_timeout で DrainHeld を emit するマシン（タイマー駆動の drain テスト用）。
    struct TimedDrainMachine;

    impl TimedStateMachine for TimedDrainMachine {
        type Event = ();
        type Action = GateAction;
        type TimerId = ();

        fn on_event(&mut self, _: ()) -> Response<GateAction, ()> {
            Response::emit_one(GateAction::InitiateHold)
        }

        fn on_timeout(&mut self, _: ()) -> Response<GateAction, ()> {
            Response::emit_one(GateAction::DrainHeld)
        }
    }

    // ── HoldingGate 本体のテスト ────────────────────────────────────

    #[test]
    fn try_hold_returns_false_when_not_holding() {
        let mut gate: HoldingGate<AlwaysDrainMachine, u32> =
            HoldingGate::new(AlwaysDrainMachine, 8);
        assert!(!gate.is_holding());
        assert!(!gate.try_hold(1));
        assert!(gate.is_empty());
    }

    #[test]
    fn hold_then_drain_via_event() {
        let mut gate: HoldingGate<ToggleMachine, u32> = HoldingGate::new(ToggleMachine, 8);

        let (_, drained) = gate.on_event(true); // InitiateHold
        assert!(gate.is_holding());
        assert!(drained.is_empty());

        assert!(gate.try_hold(10));
        assert!(gate.try_hold(20));
        assert!(gate.try_hold(30));
        assert_eq!(gate.len(), 3);

        let (_, drained) = gate.on_event(false); // DrainHeld
        assert!(!gate.is_holding());
        assert_eq!(drained, vec![10, 20, 30]);
        assert!(gate.is_empty());
    }

    #[test]
    fn hold_then_drop_via_clear() {
        let mut gate: HoldingGate<AlwaysHoldMachine, u32> =
            HoldingGate::new(AlwaysHoldMachine, 8);
        gate.on_event(());
        gate.try_hold(1);
        gate.try_hold(2);
        assert!(gate.is_holding());
        assert_eq!(gate.len(), 2);

        gate.clear();
        assert!(!gate.is_holding());
        assert!(gate.is_empty());
    }

    #[test]
    fn capacity_overflow_false_and_caller_decides() {
        let mut gate: HoldingGate<AlwaysHoldMachine, u32> =
            HoldingGate::new(AlwaysHoldMachine, 2);
        gate.on_event(());
        assert!(gate.try_hold(1));
        assert!(gate.try_hold(2));
        assert!(!gate.try_hold(3)); // 容量超過 → false
        assert_eq!(gate.len(), 2); // バッファはそのまま
    }

    #[test]
    fn reactivate_clears_buffer() {
        let mut gate: HoldingGate<AlwaysHoldMachine, u32> =
            HoldingGate::new(AlwaysHoldMachine, 8);
        gate.on_event(()); // InitiateHold
        gate.try_hold(1);
        gate.try_hold(2);
        assert_eq!(gate.len(), 2);

        gate.on_event(()); // 再 InitiateHold → 既存バッファをクリア
        assert!(gate.is_holding());
        assert!(gate.is_empty());
    }

    #[test]
    fn drain_via_timeout() {
        let mut gate: HoldingGate<TimedDrainMachine, u32> =
            HoldingGate::new(TimedDrainMachine, 8);
        gate.on_event(()); // InitiateHold
        gate.try_hold(7);
        gate.try_hold(8);

        let (_, drained) = gate.on_timeout(()); // DrainHeld via timeout
        assert!(!gate.is_holding());
        assert_eq!(drained, vec![7, 8]);
    }

    #[test]
    fn on_event_returns_empty_vec_when_no_drain() {
        let mut gate: HoldingGate<AlwaysHoldMachine, u32> =
            HoldingGate::new(AlwaysHoldMachine, 8);
        let (_, drained) = gate.on_event(());
        assert!(drained.is_empty());
    }

    #[test]
    fn multiple_drain_actions_in_one_response_is_safe() {
        // Response に DrainHeld が複数入っていても二重解放にならないことを確認。
        // apply_response は actions を順に処理し、2 回目の DrainHeld では
        // already-empty な held を take するだけで panic しない。
        struct DoubleDrainMachine;
        impl TimedStateMachine for DoubleDrainMachine {
            type Event = ();
            type Action = GateAction;
            type TimerId = ();
            fn on_event(&mut self, _: ()) -> Response<GateAction, ()> {
                let mut r = Response::emit_one(GateAction::DrainHeld);
                r.actions.push(GateAction::DrainHeld);
                r
            }
            fn on_timeout(&mut self, _: ()) -> Response<GateAction, ()> {
                Response::pass_through()
            }
        }

        let mut gate: HoldingGate<DoubleDrainMachine, u32> =
            HoldingGate::new(DoubleDrainMachine, 8);
        // 事前に保留モードに入れてアイテムを蓄積
        gate.holding = true;
        gate.held.push(1);
        gate.held.push(2);

        let (_, drained) = gate.on_event(());
        // 1 回目の DrainHeld でアイテムが返り、2 回目は空 Vec を take するだけ
        assert_eq!(drained, vec![1, 2]);
        assert!(!gate.is_holding());
        assert!(gate.is_empty());
    }
}
