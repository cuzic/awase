//! 汎用ホールディングゲート。
//!
//! [`HoldingGate<M, T>`] は [`timed_fsm::TimedStateMachine`] が [`GateAction`] を emit することで
//! アイテムの保留/解放を制御する汎用ラッパー。
//!
//! # 動作
//! - マシンが [`GateAction::InitiateHold`] を emit → 保留モード開始（`try_hold` でアイテムを蓄積）
//! - マシンが [`GateAction::DrainHeld`] を emit → 保留モード解除し全アイテムを返す
//! - 容量超過時は `try_hold` が `false` を返す（呼び出し元がゲートを強制解除すること）

use timed_fsm::{Response, TimedStateMachine};

/// ゲート用共有アクション。
///
/// [`crate::tsf::TsfGateMachine`]・[`SyncKeyGateMachine`] 等、[`HoldingGate`] と組み合わせる
/// [`TimedStateMachine`] 実装はこれを `Action` 型として使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateAction {
    /// 保留モード開始（アイテムを `held` バッファに蓄積し始める）。
    InitiateHold,
    /// 保留解除・全アイテムをドレインする。
    DrainHeld,
}

/// 汎用ホールディングゲート。
///
/// `M` が [`GateAction::InitiateHold`] を emit するとアイテムの蓄積を開始し、
/// `M` が [`GateAction::DrainHeld`] を emit すると蓄積アイテムを全て返す。
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
    /// 新しい `HoldingGate` を生成する。
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
    /// `true` = 保留（呼び出し元は `Consumed` を返すこと）。
    /// `false` = 通過（ゲートが非保留状態 or 容量超過）。
    pub fn try_hold(&mut self, item: T) -> bool {
        if !self.holding {
            return false;
        }
        if self.held.len() >= self.capacity {
            return false; // 呼び出し元が強制解除すること
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

    /// バッファをクリアし、保留モードも解除する（`panic_reset` 等の緊急用）。
    pub fn clear(&mut self) {
        self.held.clear();
        self.holding = false;
    }

    /// 現在保留モードかどうかを返す。
    #[must_use]
    pub const fn is_holding(&self) -> bool {
        self.holding
    }

    // ── 内部ヘルパー ──────────────────────────────────────────

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
                    drained = std::mem::take(&mut self.held);
                }
            }
        }
        drained
    }

    /// イベントをマシンに渡し、ドレインされたアイテムを返す。
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
}

// ── SyncKeyGateMachine ──────────────────────────────────────────────────────

/// sync キー（IME ON/OFF）押下時のキー保留ゲートマシン。
///
/// - `Activate` イベント → `InitiateHold`（保留モード開始）
/// - `Deactivate` イベント → `DrainHeld`（保留解除）
/// - タイマーなし（イベント駆動のみ）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncKeyGateState {
    /// 通常状態（保留なし）。
    Inactive,
    /// sync key 押下中（後続キーを保留）。
    Active,
}

/// `SyncKeyGateMachine` への外部イベント。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncKeyGateEvent {
    /// sync key KeyDown 検出 → 保留モード開始。
    Activate,
    /// sync key KeyUp / IME 再観測完了 → 保留解除。
    Deactivate,
}

/// sync key 押下時のキー保留ステートマシン。
///
/// イベント駆動のみ（タイマーなし）。
#[derive(Debug)]
pub struct SyncKeyGateMachine {
    state: SyncKeyGateState,
}

impl SyncKeyGateMachine {
    /// 初期状態 `Inactive` でステートマシンを生成する。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: SyncKeyGateState::Inactive,
        }
    }

    /// 現在のステートを返す。
    #[must_use]
    pub const fn state(&self) -> SyncKeyGateState {
        self.state
    }
}

impl Default for SyncKeyGateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl TimedStateMachine for SyncKeyGateMachine {
    type Event = SyncKeyGateEvent;
    type Action = GateAction;
    type TimerId = (); // タイマーなし

    fn on_event(&mut self, event: SyncKeyGateEvent) -> Response<GateAction, ()> {
        match (self.state, event) {
            (_, SyncKeyGateEvent::Activate) => {
                self.state = SyncKeyGateState::Active;
                Response::emit_one(GateAction::InitiateHold)
            }
            (SyncKeyGateState::Active, SyncKeyGateEvent::Deactivate) => {
                self.state = SyncKeyGateState::Inactive;
                Response::emit_one(GateAction::DrainHeld)
            }
            _ => Response::pass_through(),
        }
    }

    fn on_timeout(&mut self, (): ()) -> Response<GateAction, ()> {
        Response::pass_through()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── HoldingGate のテスト ─────────────────────────────────────

    #[test]
    fn try_hold_returns_false_when_not_holding() {
        let mut gate: HoldingGate<SyncKeyGateMachine, u32> =
            HoldingGate::new(SyncKeyGateMachine::new(), 8);
        assert!(!gate.is_holding());
        assert!(!gate.try_hold(1));
        assert!(gate.is_empty());
    }

    #[test]
    fn activate_then_hold_then_deactivate_drains() {
        let mut gate: HoldingGate<SyncKeyGateMachine, u32> =
            HoldingGate::new(SyncKeyGateMachine::new(), 8);

        let (_, drained) = gate.on_event(SyncKeyGateEvent::Activate);
        assert!(gate.is_holding());
        assert!(drained.is_empty());

        assert!(gate.try_hold(1));
        assert!(gate.try_hold(2));
        assert!(gate.try_hold(3));
        assert_eq!(gate.len(), 3);

        let (_, drained) = gate.on_event(SyncKeyGateEvent::Deactivate);
        assert!(!gate.is_holding());
        assert_eq!(drained, vec![1, 2, 3]);
        assert!(gate.is_empty());
    }

    #[test]
    fn capacity_overflow_returns_false() {
        let mut gate: HoldingGate<SyncKeyGateMachine, u32> =
            HoldingGate::new(SyncKeyGateMachine::new(), 2);
        gate.on_event(SyncKeyGateEvent::Activate);
        assert!(gate.try_hold(1));
        assert!(gate.try_hold(2));
        assert!(!gate.try_hold(3));
        assert_eq!(gate.len(), 2);
    }

    #[test]
    fn clear_resets_holding_and_buffer() {
        let mut gate: HoldingGate<SyncKeyGateMachine, u32> =
            HoldingGate::new(SyncKeyGateMachine::new(), 8);
        gate.on_event(SyncKeyGateEvent::Activate);
        gate.try_hold(1);
        gate.try_hold(2);

        gate.clear();
        assert!(!gate.is_holding());
        assert!(gate.is_empty());
    }

    #[test]
    fn reactivate_clears_previous_buffer() {
        let mut gate: HoldingGate<SyncKeyGateMachine, u32> =
            HoldingGate::new(SyncKeyGateMachine::new(), 8);
        gate.on_event(SyncKeyGateEvent::Activate);
        gate.try_hold(1);
        gate.try_hold(2);
        assert_eq!(gate.len(), 2);

        // 再度 Activate → 既存バッファをクリア
        gate.on_event(SyncKeyGateEvent::Activate);
        assert!(gate.is_holding());
        assert!(gate.is_empty());
    }

    // ── SyncKeyGateMachine のテスト ──────────────────────────────

    #[test]
    fn sync_machine_initial_state_is_inactive() {
        let m = SyncKeyGateMachine::new();
        assert_eq!(m.state(), SyncKeyGateState::Inactive);
    }

    #[test]
    fn sync_machine_activate_emits_initiate_hold() {
        let mut m = SyncKeyGateMachine::new();
        let r = m.on_event(SyncKeyGateEvent::Activate);
        assert_eq!(m.state(), SyncKeyGateState::Active);
        assert_eq!(r.actions, vec![GateAction::InitiateHold]);
    }

    #[test]
    fn sync_machine_deactivate_from_active_emits_drain() {
        let mut m = SyncKeyGateMachine::new();
        let _ = m.on_event(SyncKeyGateEvent::Activate);
        let r = m.on_event(SyncKeyGateEvent::Deactivate);
        assert_eq!(m.state(), SyncKeyGateState::Inactive);
        assert_eq!(r.actions, vec![GateAction::DrainHeld]);
    }

    #[test]
    fn sync_machine_deactivate_from_inactive_is_pass_through() {
        let mut m = SyncKeyGateMachine::new();
        let r = m.on_event(SyncKeyGateEvent::Deactivate);
        assert_eq!(m.state(), SyncKeyGateState::Inactive);
        r.assert_pass_through();
    }

    #[test]
    fn sync_machine_reactivate_still_emits_initiate_hold() {
        let mut m = SyncKeyGateMachine::new();
        let _ = m.on_event(SyncKeyGateEvent::Activate);
        let r = m.on_event(SyncKeyGateEvent::Activate);
        assert_eq!(m.state(), SyncKeyGateState::Active);
        assert_eq!(r.actions, vec![GateAction::InitiateHold]);
    }

    #[test]
    fn sync_machine_timeout_is_pass_through() {
        let mut m = SyncKeyGateMachine::new();
        let _ = m.on_event(SyncKeyGateEvent::Activate);
        let r = m.on_timeout(());
        r.assert_pass_through();
    }
}
