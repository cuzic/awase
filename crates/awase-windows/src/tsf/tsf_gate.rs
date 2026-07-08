//! TSF ゲートステートマシン。
//!
//! フォーカス変更直後、TSF モードが確定するまでの間キーを一時保留する。
//! Win32 API に依存しない純粋な状態機械 + キー保留ラッパーを定義する。
//!
//! ## `SyncKeyGate` との混同に注意
//!
//! 名前は似ているが [`TsfGate`] と `crate::state::hook_state::SyncKeyGate` は
//! 別目的・別トリガー・別レイヤーで動作する独立した仕組み:
//!
//! | | `TsfGate`（本モジュール） | `SyncKeyGate` |
//! |--|--|--|
//! | トリガー | フォーカス変更 | sync key（IME ON/OFF キー）KeyDown |
//! | 解除条件 | TSF/Bypass モード確定 or 500ms タイムアウト | sync key KeyUp + IME 再観測完了 |
//! | レイヤー | Output 層（TSF 注入直前） | Platform 層（フックコールバック） |
//! | 保留対象 | フォーカス直後のキー | sync key 直後のキー |
//!
//! 両者は完全に独立で、同時に active になることもある。
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

use awase::types::RawKeyEvent;
use timed_fsm::{GateAction, HoldingGate};

/// PendingWarmup フォールバックタイムアウト（ms）。
pub const WARMUP_TIMEOUT_MS: u64 = 500;

const HELD_MAX: usize = 64;

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

/// TSF ウォームアップ中のキー保留ステートマシン。
///
/// フォーカス変更後、新しいウィンドウの TSF が準備完了するまでキーを保留する。
/// 状態遷移ロジックのみを保持し、`RawKeyEvent` の保留は行わない。
/// [`TimedStateMachine`] を実装しているため、プラットフォーム依存なしでテスト可能。
///
/// `crate::state::hook_state::SyncKeyGate` とは異なる目的の仕組み:
/// `TsfGate` は TSF probe のタイムアウト（500ms）をトリガーとするが、
/// `SyncKeyGate` は sync key 直後のキー保留を担当する（モジュール doc 参照）。
#[derive(Debug)]
#[allow(clippy::module_name_repetitions)]
pub struct TsfGateMachine {
    state: TsfGateState,
}

impl TsfGateMachine {
    /// 初期状態 `Bypass` でステートマシンを生成する。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: TsfGateState::Bypass,
        }
    }

    /// 現在のステートを返す。
    #[must_use]
    pub const fn state(&self) -> TsfGateState {
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
                Response::emit_one(GateAction::InitiateHold).with_timer(
                    GateTimer::WarmupTimeout,
                    Duration::from_millis(WARMUP_TIMEOUT_MS),
                )
            }
            // PendingWarmup/Bypass + TSF 確定 → Probing + DrainHeld
            // Bypass からの遷移は WarmupTimeout 後に async タスクが遅れて confirm した回復パス。
            (TsfGateState::PendingWarmup | Bypass, TsfConfirmed) => {
                self.state = Probing;
                Response::emit_one(GateAction::DrainHeld).with_kill_timer(GateTimer::WarmupTimeout)
            }
            // PendingWarmup/Probing + Bypass 確定 → Bypass + DrainHeld
            (TsfGateState::PendingWarmup | Probing, BypassConfirmed) => {
                self.state = Bypass;
                Response::emit_one(GateAction::DrainHeld).with_kill_timer(GateTimer::WarmupTimeout)
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
                "[tsf-gate] WarmupTimeout: PendingWarmup が {WARMUP_TIMEOUT_MS}ms 継続 → Bypass にフォールバック"
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
/// フォーカス変更後の TSF ウォームアップ中にキーを `held` バッファに蓄積し、
/// TSF/Bypass モード確定時にまとめて返す。`crate::state::hook_state::SyncKeyGate`
/// （sync key 押下時のキー保留）とは別目的なので混同しないこと（モジュール doc 参照）。
///
/// Win32 タイマー（`TIMER_TSF_GATE`）の Set/Kill は呼び出し元が担当する:
/// - `on_focus_change()` 後 → `TIMER_TSF_GATE` を 500ms でセット
/// - `on_tsf_confirmed()` / `on_bypass()` 後 → `TIMER_TSF_GATE` を kill
/// - `message_handlers.rs` の `TIMER_TSF_GATE` ハンドラ → `on_warmup_timeout()` を呼ぶ
#[derive(Debug)]
#[allow(clippy::module_name_repetitions)]
pub struct TsfGate {
    inner: HoldingGate<TsfGateMachine, RawKeyEvent>,
}

impl TsfGate {
    /// 初期状態 `Bypass` でゲートを生成する。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inner: HoldingGate::new(TsfGateMachine::new(), HELD_MAX),
        }
    }

    /// 現在のステートを返す。
    #[must_use]
    pub const fn state(&self) -> TsfGateState {
        self.inner.machine.state()
    }

    /// フォーカス変更時に呼ぶ。`PendingWarmup` に遷移し `held` をクリアする。
    ///
    /// 呼び出し後に `TIMER_TSF_GATE` を `WARMUP_TIMEOUT_MS` ms でセットすること。
    pub fn on_focus_change(&mut self) {
        // `InitiateHold` アクションを `HoldingGate` が受けて
        // 自動的に held クリア + 保留モード ON を行う。
        let _ = self.inner.on_event(GateEvent::FocusChange);
        log::debug!("[tsf-gate] focus change → PendingWarmup (held cleared)");
    }

    /// TSF モードと確定した場合に呼ぶ。
    ///
    /// `PendingWarmup` / `Bypass` → `Probing` に遷移し、保留キーを返す。
    /// `Bypass` からの遷移は WarmupTimeout 後に async タスクが遅れて完了した場合の回復パス。
    /// 呼び出し後に `TIMER_TSF_GATE` を kill すること。
    /// すでに `Probing`/`Ready` なら空 `Vec` を返す（再呼び出しされても safe）。
    #[must_use]
    pub fn on_tsf_confirmed(&mut self) -> Vec<RawKeyEvent> {
        let (_, drained) = self.inner.on_event(GateEvent::TsfConfirmed);
        if !drained.is_empty() {
            log::debug!(
                "[tsf-gate] TSF confirmed → Probing (releasing {} held keys)",
                drained.len()
            );
        }
        drained
    }

    /// 非 TSF モードと確定した場合に呼ぶ。
    ///
    /// `Bypass` に遷移し、保留キーを返す。
    /// 呼び出し後に `TIMER_TSF_GATE` を kill すること。
    #[must_use]
    pub fn on_bypass(&mut self) -> Vec<RawKeyEvent> {
        let (_, drained) = self.inner.on_event(GateEvent::BypassConfirmed);
        if !drained.is_empty() {
            log::debug!(
                "[tsf-gate] → Bypass (releasing {} held keys)",
                drained.len()
            );
        }
        drained
    }

    /// TSF プローブ完了時に呼ぶ。`Probing` → `Ready` に遷移する。
    pub fn on_ready(&mut self) {
        let _ = self.inner.on_event(GateEvent::ProbeComplete);
        if self.inner.machine.state() == TsfGateState::Ready {
            log::debug!("[tsf-gate] TSF probe complete → Ready");
        }
    }

    /// `TIMER_TSF_GATE` タイムアウト発火時に呼ぶ。
    ///
    /// `PendingWarmup` が長すぎる場合（プローブが来なかった）に `Bypass` へフォールバックし
    /// 保留キーを返す。
    #[must_use]
    pub fn on_warmup_timeout(&mut self) -> Vec<RawKeyEvent> {
        let (_, drained) = self.inner.on_timeout(GateTimer::WarmupTimeout);
        drained
    }

    /// キーイベントをゲートで処理する。
    ///
    /// `true` = 保留（呼び出し元は `CallbackResult::Consumed` を返すこと）
    /// `false` = 通過（通常の処理を続行）
    ///
    /// `PendingWarmup` 状態中のみ保留する。上限超過時はキーを通過させてログ警告を出す（入力ロスを防ぐ）。
    pub fn try_hold(&mut self, event: RawKeyEvent) -> bool {
        if !self.inner.is_holding() {
            return false;
        }
        let vk = event.vk_code.0;
        let etype = event.event_type;
        let ok = self.inner.try_hold(event);
        if ok {
            log::debug!(
                "[tsf-gate] held vk=0x{vk:02X} {etype:?} (total={})",
                self.inner.len(),
            );
        } else {
            log::warn!(
                "[tsf-gate] held queue full (max={HELD_MAX}), passing through vk=0x{vk:02X} {etype:?}",
            );
        }
        ok
    }
}

impl Default for TsfGate {
    fn default() -> Self {
        Self::new()
    }
}

// ── TsfReadiness ─────────────────────────────────────────────────────────────

/// TSF 出力の準備状態を多次元で表す構造体。
///
/// [`TsfGateState`] 単体では表現できない「IME ON 状態」「注入モード」を
/// 一つの型にまとめ、各種条件判定を名前付きメソッドで提供する。
///
/// # 各次元の意味
///
/// | フィールド | 条件 | 情報源 |
/// |-----------|------|--------|
/// | `gate` | ゲートの状態 | `TsfGate::state()` |
/// | `ime_on` | IME ON/OFF シャドウ | `composition.last_applied_ime_on()` |
/// | `is_tsf_mode` | 注入モードが TSF か | `resolve_injection_mode()` |
///
/// # 構築
///
/// `awase-windows` の `Output::tsf_readiness()` メソッドで生成する。
/// このメソッドを通じてすべての条件が一箇所に集約される。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)]
pub struct TsfReadiness {
    /// ゲートの現在状態
    pub gate: TsfGateState,
    /// IME ON/OFF のシャドウ状態
    pub ime_on: bool,
    /// 注入モードが TSF かどうか（`resolve_injection_mode() == Tsf`）
    pub is_tsf_mode: bool,
}

impl TsfReadiness {
    /// eager warmup (F2 前送信) を実行できる状態か。
    ///
    /// `ime_on && is_tsf_mode` が満たされればゲート状態によらず送信する。
    /// `PendingWarmup` 中も warmup は送信可能（むしろ先行送信が目的）。
    #[must_use]
    pub const fn can_warmup(&self) -> bool {
        self.ime_on && self.is_tsf_mode
    }

    /// キーをゲートで保留すべき状態か。
    ///
    /// `PendingWarmup` 中はキーを `held` に蓄積し、TSF/Bypass 確定後に再投入する。
    #[must_use]
    pub const fn is_holding(&self) -> bool {
        matches!(self.gate, TsfGateState::PendingWarmup)
    }

    /// TSF モードで文字送信を実行できる状態か。
    ///
    /// `is_tsf_mode && gate != PendingWarmup` — `Probing` 中も送信可能。
    #[must_use]
    pub const fn can_send_tsf(&self) -> bool {
        self.is_tsf_mode && !matches!(self.gate, TsfGateState::PendingWarmup)
    }

    /// すべての条件が整った完全な Ready 状態か。
    ///
    /// `is_tsf_mode && ime_on && gate == Ready`
    #[must_use]
    pub const fn is_fully_ready(&self) -> bool {
        self.is_tsf_mode && self.ime_on && matches!(self.gate, TsfGateState::Ready)
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
        resp.timers.iter().any(|t| {
            matches!(
                t,
                TimerCommand::Set {
                    id: GateTimer::WarmupTimeout,
                    ..
                }
            )
        })
    }

    fn timer_killed(resp: &Response<GateAction, GateTimer>) -> bool {
        resp.timers.iter().any(|t| {
            matches!(
                t,
                TimerCommand::Kill {
                    id: GateTimer::WarmupTimeout
                }
            )
        })
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

    // ── シナリオ E: タイムアウト後回復（WezTerm race condition 修正） ──────

    /// E: WarmupTimeout → Bypass 後に async タスクが TsfConfirmed → Probing に回復する
    ///
    /// 再現シナリオ:
    ///   T=0   FocusChange → PendingWarmup + TIMER_TSF_GATE(500ms)
    ///   T=500 WarmupTimeout → Bypass（async タスクがまだ完了していない）
    ///   T=600 async タスク完了 → TsfConfirmed → Probing（回復）
    #[test]
    fn scenario_e_bypass_tsf_confirmed_recovers_to_probing() {
        let mut m = TsfGateMachine::new();
        let _ = m.on_event(GateEvent::FocusChange);
        assert_state(&m, TsfGateState::PendingWarmup);

        // T=500ms: タイムアウト → Bypass
        let _ = m.on_timeout(GateTimer::WarmupTimeout);
        assert_state(&m, TsfGateState::Bypass);

        // T=600ms: async タスク完了 → Probing に回復
        let r = m.on_event(GateEvent::TsfConfirmed);
        assert_state(&m, TsfGateState::Probing);
        r.assert_consumed();
        assert!(has_action(&r, GateAction::DrainHeld));
        // 死んだタイマーへの Kill は no-op だが、コマンドとして発行されることを確認
        assert!(timer_killed(&r));
    }

    /// E2: Bypass→Probing→Ready の完全な回復フロー
    #[test]
    fn scenario_e2_bypass_recovery_full_flow() {
        let mut m = TsfGateMachine::new();
        let _ = m.on_event(GateEvent::FocusChange);
        let _ = m.on_timeout(GateTimer::WarmupTimeout); // → Bypass
        let _ = m.on_event(GateEvent::TsfConfirmed); // → Probing（回復）

        let r = m.on_event(GateEvent::ProbeComplete);
        assert_state(&m, TsfGateState::Ready);
        r.assert_consumed();
    }

    // ── タイマーコマンドの詳細検証 ────────────────────────────────────────

    #[test]
    fn focus_change_sets_timer_with_correct_duration() {
        let mut m = TsfGateMachine::new();
        let r = m.on_event(GateEvent::FocusChange);
        let set_cmd = r.timers.iter().find(|t| {
            matches!(
                t,
                TimerCommand::Set {
                    id: GateTimer::WarmupTimeout,
                    ..
                }
            )
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
        use awase::types::{
            ImeRelevance, KeyClassification, KeyEventType, ModifierState, RawKeyEvent, ScanCode,
            VkCode,
        };

        let mut gate = TsfGate::new();
        gate.on_focus_change(); // PendingWarmup へ（HoldingGate 内部で holding=true）

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
            injected: false,
        };
        assert!(gate.try_hold(dummy));

        let drained = gate.on_warmup_timeout();
        assert_eq!(drained.len(), 1);
        assert_eq!(gate.state(), TsfGateState::Bypass);
    }

    // ── TsfReadiness のテスト ─────────────────────────────────────────────

    fn readiness(gate: TsfGateState, ime_on: bool, is_tsf_mode: bool) -> TsfReadiness {
        TsfReadiness {
            gate,
            ime_on,
            is_tsf_mode,
        }
    }

    /// can_warmup: ime_on && is_tsf_mode が必要十分条件
    #[test]
    fn readiness_can_warmup() {
        // 両方 true → warmup 可
        assert!(readiness(TsfGateState::PendingWarmup, true, true).can_warmup());
        assert!(readiness(TsfGateState::Ready, true, true).can_warmup());
        // ime_on=false → 不可
        assert!(!readiness(TsfGateState::Ready, false, true).can_warmup());
        // is_tsf_mode=false → 不可
        assert!(!readiness(TsfGateState::Ready, true, false).can_warmup());
    }

    /// is_holding: PendingWarmup 中のみ true
    #[test]
    fn readiness_is_holding() {
        assert!(readiness(TsfGateState::PendingWarmup, true, true).is_holding());
        assert!(!readiness(TsfGateState::Probing, true, true).is_holding());
        assert!(!readiness(TsfGateState::Ready, true, true).is_holding());
        assert!(!readiness(TsfGateState::Bypass, true, true).is_holding());
    }

    /// can_send_tsf: TSF モード && PendingWarmup でない
    #[test]
    fn readiness_can_send_tsf() {
        assert!(!readiness(TsfGateState::PendingWarmup, true, true).can_send_tsf());
        assert!(readiness(TsfGateState::Probing, true, true).can_send_tsf());
        assert!(readiness(TsfGateState::Ready, true, true).can_send_tsf());
        // Bypass 中でも TSF モードなら送信可（TSF モードが Bypass になることは稀だが）
        assert!(readiness(TsfGateState::Bypass, true, true).can_send_tsf());
        // TSF モードでなければ不可
        assert!(!readiness(TsfGateState::Ready, true, false).can_send_tsf());
    }

    /// is_fully_ready: 3次元すべてが揃った完全 Ready
    #[test]
    fn readiness_is_fully_ready() {
        assert!(readiness(TsfGateState::Ready, true, true).is_fully_ready());
        // gate が Ready でない → false
        assert!(!readiness(TsfGateState::Probing, true, true).is_fully_ready());
        assert!(!readiness(TsfGateState::PendingWarmup, true, true).is_fully_ready());
        // ime_on=false → false
        assert!(!readiness(TsfGateState::Ready, false, true).is_fully_ready());
        // is_tsf_mode=false → false
        assert!(!readiness(TsfGateState::Ready, true, false).is_fully_ready());
    }
}
