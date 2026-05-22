//! TSF ゲートステートマシン。
//!
//! フォーカス変更直後、TSF モードが確定するまでの間キーを一時保留する。
//! Win32 API に依存しない純粋な状態機械 + キー保留ラッパーを定義する。
//!
//! ## 2層構造
//!
//! - [`TsfGateMachine`]: timed-fsm ベースの純粋ステートマシン。副作用なし。テスト可能。
//! - [`TsfGate`]: `TsfGateMachine` を内包し、`RawKeyEvent` の保留を追加するラッパー。
//!
//! ## 状態遷移
//!
//! ```text
//! (起動時)  Bypass
//!     │
//!     ▼  FocusChange ── タイマー Set(WarmupTimeout, 500ms)
//! PendingWarmup ─── キーを held に蓄積 ─────────────────────┐
//!     │                                                      │
//!     │ TsfConfirmed ── TimerKill     │ BypassConfirmed ── Kill
//!     ▼                              ▼
//! Probing ─── (DrainHeld)        Bypass ─── (DrainHeld)
//!     │
//!     ▼  ProbeComplete
//! Ready
//!
//!     │  on_timeout(WarmupTimeout) ← 500ms タイムアウト
//! PendingWarmup → Bypass ─── (DrainHeld)
//! ```

use std::time::Duration;

use timed_fsm::{Response, TimedStateMachine};

use crate::types::RawKeyEvent;

/// PendingWarmup フォールバックタイムアウト（ms）。
pub const WARMUP_TIMEOUT_MS: u64 = 500;

const HELD_MAX: usize = 32;

// ── イベント / アクション / タイマー ──────────────────────────────────────────

/// TsfGate への外部イベント。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)]
pub enum GateEvent {
    /// フォーカス変更検知（win_event_proc T=0）
    FocusChange,
    /// フォーカスプローブ完了: TSF モード確定
    TsfConfirmed,
    /// フォーカスプローブ完了: 非 TSF モード確定
    BypassConfirmed,
    /// TSF プローブ（advance_tsf_probe）完了
    ProbeComplete,
}

/// TsfGate が発行するアクション。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)]
pub enum GateAction {
    /// 保留キーをドレインすること（呼び出し元が `INPUT_DEFER.replay_later` を呼ぶ）
    DrainHeld,
}

/// TsfGate のタイマー ID。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)]
pub enum GateTimer {
    /// PendingWarmup のフォールバックタイムアウト
    WarmupTimeout,
}

/// TsfGate のステート。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)]
pub enum TsfGateState {
    /// フォーカス変更直後。TSF モードが確定するまでキーを保留する。
    PendingWarmup,
    /// TSF モードと確定。F2 warmup 送信済み。pending_tsf が処理中。
    Probing,
    /// TSF プローブ完了。通常動作中。
    Ready,
    /// 非 TSF アプリ（VK/Unicode モード）。保留しない。
    Bypass,
}

// ── TsfGateMachine ──────────────────────────────────────────────────────────

/// timed-fsm ベースの純粋ステートマシン。
///
/// 状態遷移ロジックのみを保持し、`RawKeyEvent` の保留は行わない。
/// [`TimedStateMachine`] を実装しているため、プラットフォーム依存なしでテスト可能。
#[derive(Debug)]
#[allow(clippy::module_name_repetitions)]
pub struct TsfGateMachine {
    state: TsfGateState,
}

impl TsfGateMachine {
    /// 初期状態 `Bypass` でステートマシンを生成する。
    #[must_use]
    pub fn new() -> Self {
        Self { state: TsfGateState::Bypass }
    }

    /// 現在のステートを返す。
    #[must_use]
    pub fn state(&self) -> TsfGateState {
        self.state
    }
}

impl Default for TsfGateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl TimedStateMachine for TsfGateMachine {
    type Event = GateEvent;
    type Action = GateAction;
    type TimerId = GateTimer;

    fn on_event(&mut self, event: GateEvent) -> Response<GateAction, GateTimer> {
        use GateEvent::{BypassConfirmed, FocusChange, ProbeComplete, TsfConfirmed};
        use TsfGateState::{Bypass, Probing, Ready};

        match (self.state, event) {
            // どの状態でもフォーカス変更 → PendingWarmup
            (_, FocusChange) => {
                self.state = TsfGateState::PendingWarmup;
                Response::consume()
                    .with_timer(GateTimer::WarmupTimeout, Duration::from_millis(WARMUP_TIMEOUT_MS))
            }
            // PendingWarmup + TSF 確定 → Probing + DrainHeld
            (TsfGateState::PendingWarmup, TsfConfirmed) => {
                self.state = Probing;
                Response::emit_one(GateAction::DrainHeld)
                    .with_kill_timer(GateTimer::WarmupTimeout)
            }
            // PendingWarmup/Probing + Bypass 確定 → Bypass + DrainHeld
            (TsfGateState::PendingWarmup | Probing, BypassConfirmed) => {
                self.state = Bypass;
                Response::emit_one(GateAction::DrainHeld)
                    .with_kill_timer(GateTimer::WarmupTimeout)
            }
            // Probing + プローブ完了 → Ready
            (Probing, ProbeComplete) => {
                self.state = Ready;
                Response::consume()
            }
            _ => Response::pass_through(),
        }
    }

    fn on_timeout(&mut self, _id: GateTimer) -> Response<GateAction, GateTimer> {
        if self.state == TsfGateState::PendingWarmup {
            log::warn!(
                "[tsf-gate] WarmupTimeout: PendingWarmup が {}ms 継続 → Bypass にフォールバック",
                WARMUP_TIMEOUT_MS
            );
            self.state = TsfGateState::Bypass;
            Response::emit_one(GateAction::DrainHeld)
        } else {
            Response::pass_through()
        }
    }
}

// ── TsfGate ─────────────────────────────────────────────────────────────────

/// `TsfGateMachine` にキー保留機能を追加したラッパー。
///
/// Win32 タイマー（`TIMER_TSF_GATE`）の Set/Kill は呼び出し元が担当する:
/// - `on_focus_change()` 後 → `TIMER_TSF_GATE` を 500ms でセット
/// - `on_tsf_confirmed()` / `on_bypass()` 後 → `TIMER_TSF_GATE` を kill
/// - `message_handlers.rs` の `TIMER_TSF_GATE` ハンドラ → `on_warmup_timeout()` を呼ぶ
#[derive(Debug)]
#[allow(clippy::module_name_repetitions)]
pub struct TsfGate {
    machine: TsfGateMachine,
    held: Vec<RawKeyEvent>,
}

impl TsfGate {
    /// 初期状態 `Bypass` でゲートを生成する。
    #[must_use]
    pub fn new() -> Self {
        Self {
            machine: TsfGateMachine::new(),
            held: Vec::new(),
        }
    }

    /// 現在のステートを返す。
    #[must_use]
    pub fn state(&self) -> TsfGateState {
        self.machine.state()
    }

    /// フォーカス変更時に呼ぶ。`PendingWarmup` に遷移し `held` をクリアする。
    ///
    /// 呼び出し後に `TIMER_TSF_GATE` を `WARMUP_TIMEOUT_MS` ms でセットすること。
    pub fn on_focus_change(&mut self) {
        let _ = self.machine.on_event(GateEvent::FocusChange);
        self.held.clear();
        log::debug!("[tsf-gate] focus change → PendingWarmup (held cleared)");
    }

    /// TSF モードと確定した場合に呼ぶ。
    ///
    /// `PendingWarmup` → `Probing` に遷移し、保留キーを返す。
    /// 呼び出し後に `TIMER_TSF_GATE` を kill すること。
    /// すでに `Probing`/`Ready`/`Bypass` なら空 `Vec` を返す（再呼び出しされても safe）。
    #[must_use]
    pub fn on_tsf_confirmed(&mut self) -> Vec<RawKeyEvent> {
        let resp = self.machine.on_event(GateEvent::TsfConfirmed);
        if resp.actions.contains(&GateAction::DrainHeld) {
            log::debug!(
                "[tsf-gate] TSF confirmed → Probing (releasing {} held keys)",
                self.held.len()
            );
            std::mem::take(&mut self.held)
        } else {
            Vec::new()
        }
    }

    /// 非 TSF モードと確定した場合に呼ぶ。
    ///
    /// `Bypass` に遷移し、保留キーを返す。
    /// 呼び出し後に `TIMER_TSF_GATE` を kill すること。
    #[must_use]
    pub fn on_bypass(&mut self) -> Vec<RawKeyEvent> {
        let resp = self.machine.on_event(GateEvent::BypassConfirmed);
        if resp.actions.contains(&GateAction::DrainHeld) {
            log::debug!(
                "[tsf-gate] → Bypass (releasing {} held keys)",
                self.held.len()
            );
            std::mem::take(&mut self.held)
        } else {
            Vec::new()
        }
    }

    /// TSF プローブ完了時に呼ぶ。`Probing` → `Ready` に遷移する。
    pub fn on_ready(&mut self) {
        let _ = self.machine.on_event(GateEvent::ProbeComplete);
        if self.machine.state() == TsfGateState::Ready {
            log::debug!("[tsf-gate] TSF probe complete → Ready");
        }
    }

    /// `TIMER_TSF_GATE` タイムアウト発火時に呼ぶ。
    ///
    /// `PendingWarmup` が長すぎる場合（プローブが来なかった）に `Bypass` へフォールバックし
    /// 保留キーを返す。
    #[must_use]
    pub fn on_warmup_timeout(&mut self) -> Vec<RawKeyEvent> {
        let resp = self.machine.on_timeout(GateTimer::WarmupTimeout);
        if resp.actions.contains(&GateAction::DrainHeld) {
            std::mem::take(&mut self.held)
        } else {
            Vec::new()
        }
    }

    /// キーイベントをゲートで処理する。
    ///
    /// `true` = 保留（呼び出し元は `CallbackResult::Consumed` を返すこと）
    /// `false` = 通過（通常の処理を続行）
    ///
    /// `PendingWarmup` 状態中のみ保留する。上限超過時はキーを破棄してログ警告を出す。
    pub fn try_hold(&mut self, event: RawKeyEvent) -> bool {
        if self.machine.state() != TsfGateState::PendingWarmup {
            return false;
        }
        if self.held.len() >= HELD_MAX {
            log::warn!(
                "[tsf-gate] held queue full (max={HELD_MAX}), dropping vk=0x{:02X} {:?}",
                event.vk_code.0,
                event.event_type,
            );
            return true;
        }
        self.held.push(event);
        log::trace!(
            "[tsf-gate] held vk=0x{:02X} {:?} (total={})",
            event.vk_code.0,
            event.event_type,
            self.held.len(),
        );
        true
    }
}

impl Default for TsfGate {
    fn default() -> Self {
        Self::new()
    }
}

// ── シナリオテスト ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use timed_fsm::TimerCommand;

    // ── ヘルパー ────────────────────────────────────────────────────────────

    fn assert_state(m: &TsfGateMachine, expected: TsfGateState) {
        assert_eq!(
            m.state(),
            expected,
            "state mismatch: expected {expected:?}, got {:?}",
            m.state()
        );
    }

    fn has_action(resp: &Response<GateAction, GateTimer>, action: GateAction) -> bool {
        resp.actions.contains(&action)
    }

    fn timer_set(resp: &Response<GateAction, GateTimer>) -> bool {
        resp.timers
            .iter()
            .any(|t| matches!(t, TimerCommand::Set { id: GateTimer::WarmupTimeout, .. }))
    }

    fn timer_killed(resp: &Response<GateAction, GateTimer>) -> bool {
        resp.timers
            .iter()
            .any(|t| matches!(t, TimerCommand::Kill { id: GateTimer::WarmupTimeout }))
    }

    // ── シナリオ A: 正常フロー (TSF アプリ) ────────────────────────────────

    /// A1: FocusChange → PendingWarmup + タイマーセット
    #[test]
    fn scenario_a1_focus_change_to_pending_warmup() {
        let mut m = TsfGateMachine::new();
        assert_state(&m, TsfGateState::Bypass);

        let r = m.on_event(GateEvent::FocusChange);
        assert_state(&m, TsfGateState::PendingWarmup);
        r.assert_consumed();
        assert!(timer_set(&r), "WarmupTimeout タイマーがセットされるべき");
        assert!(!has_action(&r, GateAction::DrainHeld));
    }

    /// A2: TsfConfirmed → Probing + DrainHeld + タイマーkill
    #[test]
    fn scenario_a2_tsf_confirmed_to_probing() {
        let mut m = TsfGateMachine::new();
        let _ = m.on_event(GateEvent::FocusChange);

        let r = m.on_event(GateEvent::TsfConfirmed);
        assert_state(&m, TsfGateState::Probing);
        r.assert_consumed();
        assert!(has_action(&r, GateAction::DrainHeld));
        assert!(timer_killed(&r), "WarmupTimeout タイマーが kill されるべき");
    }

    /// A3: ProbeComplete → Ready
    #[test]
    fn scenario_a3_probe_complete_to_ready() {
        let mut m = TsfGateMachine::new();
        let _ = m.on_event(GateEvent::FocusChange);
        let _ = m.on_event(GateEvent::TsfConfirmed);

        let r = m.on_event(GateEvent::ProbeComplete);
        assert_state(&m, TsfGateState::Ready);
        r.assert_consumed();
        assert!(!has_action(&r, GateAction::DrainHeld));
    }

    /// A_full: 完全な正常フロー A1→A2→A3
    #[test]
    fn scenario_a_full_normal_tsf_flow() {
        let mut m = TsfGateMachine::new();

        // T=0: focus
        m.on_event(GateEvent::FocusChange);
        assert_state(&m, TsfGateState::PendingWarmup);

        // T≈100ms: TSF 確定
        m.on_event(GateEvent::TsfConfirmed);
        assert_state(&m, TsfGateState::Probing);

        // T≈300ms: プローブ完了
        m.on_event(GateEvent::ProbeComplete);
        assert_state(&m, TsfGateState::Ready);
    }

    // ── シナリオ B: 再フォーカス ────────────────────────────────────────────

    /// B: Probing 中に再フォーカス → PendingWarmup に戻る
    #[test]
    fn scenario_b_refocus_during_probing() {
        let mut m = TsfGateMachine::new();
        let _ = m.on_event(GateEvent::FocusChange);
        let _ = m.on_event(GateEvent::TsfConfirmed);
        assert_state(&m, TsfGateState::Probing);

        let r = m.on_event(GateEvent::FocusChange);
        assert_state(&m, TsfGateState::PendingWarmup);
        r.assert_consumed();
        assert!(timer_set(&r), "再フォーカスでタイマーがリセットされるべき");
    }

    /// B2: Ready 状態でのフォーカス変更 → PendingWarmup
    #[test]
    fn scenario_b2_refocus_from_ready() {
        let mut m = TsfGateMachine::new();
        let _ = m.on_event(GateEvent::FocusChange);
        let _ = m.on_event(GateEvent::TsfConfirmed);
        let _ = m.on_event(GateEvent::ProbeComplete);
        assert_state(&m, TsfGateState::Ready);

        let r = m.on_event(GateEvent::FocusChange);
        assert_state(&m, TsfGateState::PendingWarmup);
        r.assert_consumed();
        assert!(timer_set(&r));
    }

    // ── シナリオ C: タイムアウトフォールバック ──────────────────────────────

    /// C: PendingWarmup でタイムアウト → Bypass + DrainHeld
    #[test]
    fn scenario_c_warmup_timeout_fallback() {
        let mut m = TsfGateMachine::new();
        let r_focus = m.on_event(GateEvent::FocusChange);
        assert_state(&m, TsfGateState::PendingWarmup);
        assert!(timer_set(&r_focus));

        let r = m.on_timeout(GateTimer::WarmupTimeout);
        assert_state(&m, TsfGateState::Bypass);
        assert!(has_action(&r, GateAction::DrainHeld));
    }

    /// C2: Probing でのタイムアウトは無視
    #[test]
    fn scenario_c2_timeout_in_probing_is_ignored() {
        let mut m = TsfGateMachine::new();
        let _ = m.on_event(GateEvent::FocusChange);
        let _ = m.on_event(GateEvent::TsfConfirmed);
        assert_state(&m, TsfGateState::Probing);

        let r = m.on_timeout(GateTimer::WarmupTimeout);
        assert_state(&m, TsfGateState::Probing);
        r.assert_pass_through();
    }

    // ── シナリオ D: 非 TSF アプリ (Bypass) ─────────────────────────────────

    /// D: BypassConfirmed → Bypass + DrainHeld + タイマーkill
    #[test]
    fn scenario_d_bypass_app() {
        let mut m = TsfGateMachine::new();
        let _ = m.on_event(GateEvent::FocusChange);
        assert_state(&m, TsfGateState::PendingWarmup);

        let r = m.on_event(GateEvent::BypassConfirmed);
        assert_state(&m, TsfGateState::Bypass);
        assert!(has_action(&r, GateAction::DrainHeld));
        assert!(timer_killed(&r), "Bypass 確定でタイマーが kill されるべき");
    }

    /// D2: Bypass 確定済みに再度 BypassConfirmed → pass-through（べき等）
    #[test]
    fn scenario_d2_bypass_confirmed_idempotent() {
        let mut m = TsfGateMachine::new();
        let _ = m.on_event(GateEvent::FocusChange);
        let _ = m.on_event(GateEvent::BypassConfirmed);
        assert_state(&m, TsfGateState::Bypass);

        let r = m.on_event(GateEvent::BypassConfirmed);
        r.assert_pass_through();
        assert_state(&m, TsfGateState::Bypass);
    }

    // ── タイマーコマンドの詳細検証 ────────────────────────────────────────

    #[test]
    fn focus_change_sets_timer_with_correct_duration() {
        let mut m = TsfGateMachine::new();
        let r = m.on_event(GateEvent::FocusChange);
        let set_cmd = r.timers.iter().find(|t| {
            matches!(t, TimerCommand::Set { id: GateTimer::WarmupTimeout, .. })
        });
        let set_cmd = set_cmd.expect("Set コマンドが存在するべき");
        if let TimerCommand::Set { duration, .. } = *set_cmd {
            assert_eq!(duration, Duration::from_millis(WARMUP_TIMEOUT_MS));
        }
    }

    #[test]
    fn tsf_confirmed_kills_timer() {
        let mut m = TsfGateMachine::new();
        let _ = m.on_event(GateEvent::FocusChange);
        let r = m.on_event(GateEvent::TsfConfirmed);
        r.assert_timer_kill(GateTimer::WarmupTimeout);
    }

    #[test]
    fn bypass_confirmed_kills_timer() {
        let mut m = TsfGateMachine::new();
        let _ = m.on_event(GateEvent::FocusChange);
        let r = m.on_event(GateEvent::BypassConfirmed);
        r.assert_timer_kill(GateTimer::WarmupTimeout);
    }

    // ── TsfGate ラッパーのテスト ──────────────────────────────────────────

    #[test]
    fn gate_wrapper_on_warmup_timeout_drains_held() {
        use crate::engine::ModifierState;
        use crate::types::{ImeRelevance, KeyClassification, KeyEventType, RawKeyEvent, ScanCode, VkCode};

        let mut gate = TsfGate::new();
        gate.machine.on_event(GateEvent::FocusChange); // PendingWarmup へ

        let dummy = RawKeyEvent {
            vk_code: VkCode(0x41), // 'A'
            scan_code: ScanCode(0x1E),
            event_type: KeyEventType::KeyDown,
            extra_info: 0,
            timestamp: 0,
            key_classification: KeyClassification::Passthrough,
            physical_pos: None,
            ime_relevance: ImeRelevance::default(),
            modifier_key: None,
            modifier_snapshot: ModifierState::default(),
        };
        gate.held.push(dummy);

        let drained = gate.on_warmup_timeout();
        assert_eq!(drained.len(), 1);
        assert!(gate.held.is_empty());
        assert_eq!(gate.state(), TsfGateState::Bypass);
    }
}
