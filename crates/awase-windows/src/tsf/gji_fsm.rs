//! GJI (Google Japanese Input) 内部状態推測 FSM。
//!
//! GJI の観測可能なイベント（IME ON/OFF、フォーカス変更、composition イベント、
//! warmup probe 結果）から GJI の内部状態を推測する。
//!
//! ## 状態空間（5状態）
//!
//! ```text
//!                ImeOn / FocusChange
//! OffCold ──────────────────────────────────────────────► OnCold(Short)
//!                                                              │
//!                                                   WarmupComplete / WarmupFailed
//!                                                              │
//! OnComposing ◄── StartComposition ── OnWarm ◄───────────────┘
//!     │                                  │
//!     │ EndComposition(epoch ✓)     LongIdle timeout
//!     │                                  │
//!     └──────────────── OnWarm          OnCold(Long, NotStarted)
//! ```
//!
//! ## 設計根拠
//!
//! - `pending` は `OnCold` バリアント内に持つ（型レベルで OffCold/OnWarm/OnComposing の
//!   pending を不正状態として排除）。
//! - `probe_id` により stale な WarmupComplete/WarmupFailed を安全に弾く。
//! - `epoch` により stale な EndComposition を安全に弾く。
//! - `OnCold(Long)` は `LongIdle` タイムアウト直後に入る状態で、最初の `KeyInput` が
//!   来るまで probe を開始しない（`ProbeStatus::NotStarted`）。
//! - LongIdle タイマーは timed-fsm の `on_timeout` で管理し、
//!   `KeyInput` ごとに `with_timer` でリセットする。

use std::time::Duration;

use timed_fsm::{Response, TimedStateMachine};

use crate::output::InjectionMode;
use crate::tsf::probe_fsm::DeferredVk;
use crate::tuning;

// ── プリミティブ型 ────────────────────────────────────────────────────────────

/// フォーカス epoch。stale な `EndComposition` を弾くための単調カウンタ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct FocusEpoch(u32);

/// probe ID。stale な `WarmupComplete` / `WarmupFailed` を弾くための識別子。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProbeId(u32);

// ── WarmupResult ─────────────────────────────────────────────────────────────

/// warmup probe の完了経路（4-bool の旧 WarmupResult を enum に昇格）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WarmupPath {
    /// OBJ_NAMECHANGE 発火を確認した通常経路
    NameChangeConfirmed,
    /// eager LiteralDetect 経由で composition を確認した経路
    EagerLiteralDetected,
    /// GJI long-idle 後に I/O 応答を確認した経路
    GjiResumed,
    /// budget 枯渇による保守的フォールバック
    TimedOutFallback,
}

/// warmup probe の結果。送信方法（`SendInput`）の決定に使う。
#[derive(Debug, Clone, Copy)]
pub(crate) struct WarmupResult {
    pub path: WarmupPath,
    pub prepend_f2_warmup: bool,
    pub nc_fired: bool,
    pub gji_resumed: bool,
}

impl WarmupResult {
    /// budget 枯渇時の保守的フォールバック値（`WarmupFailed` イベントで使用）。
    pub(crate) const fn conservative_fallback() -> Self {
        Self {
            path: WarmupPath::TimedOutFallback,
            prepend_f2_warmup: true,
            nc_fired: false,
            gji_resumed: false,
        }
    }
}

// ── PendingInput ─────────────────────────────────────────────────────────────

/// `OnCold` 中に蓄積する入力バッファ（旧 `TsfProbeMachine::SendState` に相当）。
///
/// warmup 完了後に `GjiAction::SendInput` に格納して dispatcher に渡す。
#[derive(Debug, Clone, Default)]
pub(crate) struct PendingInput {
    pub romaji: String,
    pub deferred_vks: Vec<DeferredVk>,
}

impl PendingInput {
    pub(crate) fn new(romaji: impl Into<String>) -> Self {
        Self {
            romaji: romaji.into(),
            deferred_vks: Vec::new(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.romaji.is_empty() && self.deferred_vks.is_empty()
    }
}

// ── 状態型 ───────────────────────────────────────────────────────────────────

/// `OnCold` の種別と warmup budget。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColdKind {
    /// フォーカス変更・IME-ON 直後（budget = 100 ms）
    Short,
    /// LongIdle タイムアウト後（budget = GJI_LONG_IDLE_PROBE_TOTAL_MS = 350 ms）
    Long,
}

impl ColdKind {
    pub(crate) const fn budget_ms(self) -> u64 {
        match self {
            Self::Short => 100,
            Self::Long => tuning::GJI_LONG_IDLE_PROBE_TOTAL_MS,
        }
    }
}

/// `OnCold` 内の probe 進行状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProbeStatus {
    /// probe 未開始（`Long` タイムアウト直後、最初の `KeyInput` を待つ）
    NotStarted,
    /// probe 実行中
    Running { probe_id: ProbeId },
}

/// GJI FSM の状態。
#[derive(Debug)]
pub(crate) enum GjiState {
    /// IME OFF（初期状態）
    OffCold,
    /// IME ON、TSF cold。`pending` は型安全のためここだけが持つ。
    OnCold {
        kind: ColdKind,
        probe: ProbeStatus,
        pending: Vec<PendingInput>,
    },
    /// IME ON、TSF warm
    OnWarm { long_idle_ms: u64 },
    /// IME ON、TSF warm、変換中
    OnComposing { epoch: FocusEpoch },
}

// ── イベント・アクション・タイマー ──────────────────────────────────────────

/// GJI FSM に入力するイベント。
#[derive(Debug)]
pub(crate) enum GjiEvent {
    /// IME ON（エンジン起動）
    ImeOn { injection_mode: InjectionMode },
    /// IME OFF（エンジン停止）
    ImeOff,
    /// フォーカス変更
    FocusChange { injection_mode: InjectionMode },
    /// キー入力（ローマ字 + deferred VK）
    KeyInput(PendingInput),
    /// warmup probe 完了
    WarmupComplete { probe_id: ProbeId, result: WarmupResult },
    /// warmup probe が budget 内に完了しなかった
    WarmupFailed { probe_id: ProbeId },
    /// `WM_IME_STARTCOMPOSITION`
    StartComposition,
    /// `WM_IME_ENDCOMPOSITION`（epoch チェック付き）
    EndComposition { epoch: FocusEpoch },
    /// IME ON/OFF やフォーカス変化なしに composition context が無効化された
    /// (PassthroughKey, NativeF2Consumed, RawTsfLiteralRecovery 等)
    CompositionReset,
}

/// GJI FSM が出力するアクション（ディスパッチャが副作用を実行する）。
#[derive(Debug)]
pub(crate) enum GjiAction {
    /// 新しい warmup probe を開始する
    StartProbe { probe_id: ProbeId, budget_ms: u64 },
    /// 実行中の probe をキャンセルする
    CancelProbe { probe_id: ProbeId },
    /// warmup 完了後に蓄積入力を送信する
    SendInput { result: WarmupResult, pending: Vec<PendingInput> },
    /// warm 状態で即送信する
    SendInputDirect(PendingInput),
}

/// GJI FSM のタイマー識別子（timed-fsm の `TimerId`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GjiTimer {
    /// `OnWarm` 中のアイドル監視タイマー（発火 → `OnCold(Long)`）
    LongIdle,
}

// ── FSM 本体 ─────────────────────────────────────────────────────────────────

/// GJI 内部状態推測 FSM。
///
/// 副作用はなし。遷移ごとに `Response<GjiAction, GjiTimer>` を返し、
/// ディスパッチャが `GjiAction` を実行し timed-fsm ランタイムがタイマーを管理する。
pub(crate) struct GjiFsm {
    state: GjiState,
    /// `EndComposition` の stale 判定用 epoch
    epoch: FocusEpoch,
    /// `ProbeId` の連番カウンタ
    next_probe_id: u32,
    /// 現在フォーカス中アプリの injection_mode（`long_idle_ms` 計算用）
    injection_mode: InjectionMode,
}

impl GjiFsm {
    pub(crate) fn new() -> Self {
        Self {
            state: GjiState::OffCold,
            epoch: FocusEpoch(0),
            next_probe_id: 0,
            injection_mode: InjectionMode::Unicode,
        }
    }

    pub(crate) const fn state(&self) -> &GjiState {
        &self.state
    }

    fn alloc_probe_id(&mut self) -> ProbeId {
        let id = ProbeId(self.next_probe_id);
        self.next_probe_id = self.next_probe_id.wrapping_add(1);
        id
    }

    fn bump_epoch(&mut self) -> FocusEpoch {
        self.epoch = FocusEpoch(self.epoch.0.wrapping_add(1));
        self.epoch
    }

    fn long_idle_ms(&self) -> u64 {
        long_idle_ms_for(self.injection_mode)
    }

    /// 現在状態が `OnCold(Running)` なら probe_id を返す。
    fn running_probe_id(&self) -> Option<ProbeId> {
        match &self.state {
            GjiState::OnCold {
                probe: ProbeStatus::Running { probe_id },
                ..
            } => Some(*probe_id),
            _ => None,
        }
    }

    /// OnCold 入場（既存 probe のキャンセルと新 probe の開始を含む）。
    ///
    /// `Short` → 即 probe 開始、`Long` → `NotStarted`（最初の `KeyInput` まで待機）。
    fn transition_to_cold(
        &mut self,
        kind: ColdKind,
        initial_pending: Vec<PendingInput>,
        old_probe: Option<ProbeId>,
    ) -> Response<GjiAction, GjiTimer> {
        let (probe_status, start_action) = if kind == ColdKind::Short {
            let probe_id = self.alloc_probe_id();
            (
                ProbeStatus::Running { probe_id },
                Some(GjiAction::StartProbe {
                    probe_id,
                    budget_ms: ColdKind::Short.budget_ms(),
                }),
            )
        } else {
            (ProbeStatus::NotStarted, None)
        };

        self.state = GjiState::OnCold {
            kind,
            probe: probe_status,
            pending: initial_pending,
        };

        let mut actions = Vec::new();
        if let Some(id) = old_probe {
            actions.push(GjiAction::CancelProbe { probe_id: id });
        }
        if let Some(a) = start_action {
            actions.push(a);
        }
        Response::emit(actions).with_kill_timer(GjiTimer::LongIdle)
    }

    /// OnWarm 入場（LongIdle タイマーを開始する）。
    fn transition_to_warm(
        &mut self,
        extra_actions: Vec<GjiAction>,
    ) -> Response<GjiAction, GjiTimer> {
        let long_idle_ms = self.long_idle_ms();
        self.state = GjiState::OnWarm { long_idle_ms };
        Response::emit(extra_actions)
            .with_timer(GjiTimer::LongIdle, Duration::from_millis(long_idle_ms))
    }

    /// OnComposing 入場（LongIdle タイマーを kill する）。
    fn transition_to_composing(
        &mut self,
        extra_actions: Vec<GjiAction>,
    ) -> Response<GjiAction, GjiTimer> {
        let epoch = self.bump_epoch();
        self.state = GjiState::OnComposing { epoch };
        Response::emit(extra_actions).with_kill_timer(GjiTimer::LongIdle)
    }
}

impl Default for GjiFsm {
    fn default() -> Self {
        Self::new()
    }
}

impl TimedStateMachine for GjiFsm {
    type Event = GjiEvent;
    type Action = GjiAction;
    type TimerId = GjiTimer;

    #[expect(clippy::too_many_lines)]
    fn on_event(&mut self, event: GjiEvent) -> Response<GjiAction, GjiTimer> {
        match event {
            // ── ImeOn ──────────────────────────────────────────────────────
            GjiEvent::ImeOn { injection_mode } => {
                self.injection_mode = injection_mode;
                match &self.state {
                    GjiState::OffCold => self.transition_to_cold(ColdKind::Short, vec![], None),
                    _ => {
                        log::debug!(
                            "[gji-fsm] ImeOn: already on ({}), ignored",
                            state_label(&self.state)
                        );
                        Response::consume()
                    }
                }
            }

            // ── ImeOff ─────────────────────────────────────────────────────
            GjiEvent::ImeOff => {
                let old_probe = self.running_probe_id();
                let pending_count = match &self.state {
                    GjiState::OnCold { pending, .. } => pending.len(),
                    _ => 0,
                };
                match &self.state {
                    GjiState::OffCold => return Response::consume(),
                    _ => {}
                }
                if pending_count > 0 {
                    log::warn!(
                        "[gji-fsm] ImeOff with {pending_count} pending input(s) — discarding"
                    );
                }
                let mut actions = Vec::new();
                if let Some(id) = old_probe {
                    actions.push(GjiAction::CancelProbe { probe_id: id });
                }
                self.state = GjiState::OffCold;
                Response::emit(actions).with_kill_timer(GjiTimer::LongIdle)
            }

            // ── FocusChange ────────────────────────────────────────────────
            GjiEvent::FocusChange { injection_mode } => {
                self.injection_mode = injection_mode;
                let old_probe = self.running_probe_id();
                let pending_count = match &self.state {
                    GjiState::OnCold { pending, .. } => pending.len(),
                    _ => 0,
                };
                let engine_on = !matches!(self.state, GjiState::OffCold);

                if !engine_on {
                    // エンジン OFF のままフォーカスが動いても状態変化なし
                    return Response::consume();
                }
                if pending_count > 0 {
                    log::warn!(
                        "[gji-fsm] FocusChange with {pending_count} pending input(s) — discarding"
                    );
                }
                self.transition_to_cold(ColdKind::Short, vec![], old_probe)
            }

            // ── KeyInput ───────────────────────────────────────────────────
            GjiEvent::KeyInput(input) => {
                // NotStarted の場合のみ probe_id を事前確保する（&mut self.state との二重借用を回避）
                let maybe_new_probe_id = if matches!(
                    &self.state,
                    GjiState::OnCold { probe: ProbeStatus::NotStarted, .. }
                ) {
                    Some(self.alloc_probe_id())
                } else {
                    None
                };

                match &mut self.state {
                    GjiState::OffCold => Response::pass_through(),

                    GjiState::OnCold { probe, pending, kind } => {
                        let kind = *kind;
                        match probe {
                            ProbeStatus::NotStarted => {
                                // Long の最初の KeyInput で probe を開始する
                                let probe_id = maybe_new_probe_id.unwrap();
                                let budget_ms = kind.budget_ms();
                                *probe = ProbeStatus::Running { probe_id };
                                pending.push(input);
                                Response::emit(vec![GjiAction::StartProbe { probe_id, budget_ms }])
                            }
                            ProbeStatus::Running { .. } => {
                                pending.push(input);
                                Response::consume()
                            }
                        }
                    }

                    GjiState::OnWarm { long_idle_ms } => {
                        let ms = *long_idle_ms;
                        Response::emit_one(GjiAction::SendInputDirect(input))
                            .with_timer(GjiTimer::LongIdle, Duration::from_millis(ms))
                    }

                    GjiState::OnComposing { .. } => {
                        Response::emit_one(GjiAction::SendInputDirect(input))
                    }
                }
            },

            // ── WarmupComplete ─────────────────────────────────────────────
            GjiEvent::WarmupComplete { probe_id, result } => {
                // 現在 Running の probe_id と照合（stale 判定）
                let current_id = match &self.state {
                    GjiState::OnCold {
                        probe: ProbeStatus::Running { probe_id: id },
                        ..
                    } => Some(*id),
                    _ => None,
                };
                if current_id != Some(probe_id) {
                    log::debug!(
                        "[gji-fsm] WarmupComplete {probe_id:?}: stale (current={current_id:?}), ignored"
                    );
                    return Response::consume();
                }
                // pending を take する（immutable borrow を解放してから）
                let pending = match &mut self.state {
                    GjiState::OnCold { pending, .. } => std::mem::take(pending),
                    _ => vec![],
                };
                let mut extra_actions = Vec::new();
                if !pending.is_empty() {
                    extra_actions.push(GjiAction::SendInput { result, pending });
                }
                self.transition_to_warm(extra_actions)
            }

            // ── WarmupFailed ───────────────────────────────────────────────
            GjiEvent::WarmupFailed { probe_id } => {
                let current_id = match &self.state {
                    GjiState::OnCold {
                        probe: ProbeStatus::Running { probe_id: id },
                        ..
                    } => Some(*id),
                    _ => None,
                };
                if current_id != Some(probe_id) {
                    log::debug!(
                        "[gji-fsm] WarmupFailed {probe_id:?}: stale (current={current_id:?}), ignored"
                    );
                    return Response::consume();
                }
                log::warn!(
                    "[gji-fsm] WarmupFailed {probe_id:?}: budget exhausted, using conservative fallback"
                );
                let pending = match &mut self.state {
                    GjiState::OnCold { pending, .. } => std::mem::take(pending),
                    _ => vec![],
                };
                let result = WarmupResult::conservative_fallback();
                let mut extra_actions = Vec::new();
                if !pending.is_empty() {
                    extra_actions.push(GjiAction::SendInput { result, pending });
                }
                self.transition_to_warm(extra_actions)
            }

            // ── StartComposition ───────────────────────────────────────────
            GjiEvent::StartComposition => match &self.state {
                GjiState::OnWarm { .. } => self.transition_to_composing(vec![]),

                GjiState::OnComposing { .. } => {
                    log::debug!("[gji-fsm] StartComposition: already composing, ignored");
                    Response::consume()
                }

                GjiState::OnCold { .. } => {
                    // Cold 中の StartComposition は通常起きないが、
                    // GJI が先に composition を開始した場合に備えて probe をキャンセルして遷移する。
                    log::warn!("[gji-fsm] StartComposition while cold — probe cancelled");
                    let cancel = self
                        .running_probe_id()
                        .map(|id| GjiAction::CancelProbe { probe_id: id });
                    self.transition_to_composing(cancel.into_iter().collect())
                }

                GjiState::OffCold => {
                    log::warn!("[gji-fsm] StartComposition while engine off — ignored");
                    Response::consume()
                }
            },

            // ── EndComposition ─────────────────────────────────────────────
            GjiEvent::EndComposition { epoch } => match &self.state {
                GjiState::OnComposing {
                    epoch: current_epoch,
                } => {
                    if epoch == *current_epoch {
                        self.transition_to_warm(vec![])
                    } else {
                        log::debug!(
                            "[gji-fsm] EndComposition: stale epoch {epoch:?} ≠ {current_epoch:?}, ignored"
                        );
                        Response::consume()
                    }
                }
                _ => {
                    log::debug!(
                        "[gji-fsm] EndComposition: not composing ({}), ignored",
                        state_label(&self.state)
                    );
                    Response::consume()
                }
            },

            // ── CompositionReset ───────────────────────────────────────────
            GjiEvent::CompositionReset => match &self.state {
                GjiState::OffCold => Response::consume(),

                GjiState::OnCold { .. } => {
                    // 既存 probe をキャンセルして Short で再開（pending も破棄）
                    let old = self.running_probe_id();
                    self.state = GjiState::OnCold {
                        kind: ColdKind::Short,
                        probe: ProbeStatus::NotStarted,
                        pending: vec![],
                    };
                    let mut actions = Vec::new();
                    if let Some(id) = old {
                        actions.push(GjiAction::CancelProbe { probe_id: id });
                    }
                    Response::emit(actions).with_kill_timer(GjiTimer::LongIdle)
                }

                GjiState::OnWarm { .. } | GjiState::OnComposing { .. } => {
                    self.state = GjiState::OnCold {
                        kind: ColdKind::Short,
                        probe: ProbeStatus::NotStarted,
                        pending: vec![],
                    };
                    Response::consume().with_kill_timer(GjiTimer::LongIdle)
                }
            },
        }
    }

    fn on_timeout(&mut self, timer_id: GjiTimer) -> Response<GjiAction, GjiTimer> {
        match timer_id {
            GjiTimer::LongIdle => match &self.state {
                GjiState::OnWarm { .. } => {
                    log::debug!("[gji-fsm] LongIdle timeout → OnCold(Long, NotStarted)");
                    self.state = GjiState::OnCold {
                        kind: ColdKind::Long,
                        probe: ProbeStatus::NotStarted,
                        pending: vec![],
                    };
                    Response::consume()
                }
                _ => {
                    log::warn!(
                        "[gji-fsm] LongIdle timeout in unexpected state ({})",
                        state_label(&self.state)
                    );
                    Response::consume()
                }
            },
        }
    }
}

// ── ヘルパー関数 ──────────────────────────────────────────────────────────────

/// `InjectionMode` から `LongIdle` タイムアウト時間 (ms) を計算する。
///
/// - `Tsf`（WezTerm 等）: `LONG_IDLE_MS`（10 s）
/// - `Vk`（Chrome/Edge 等）: `CHROME_LONG_IDLE_MS`（5 s）
/// - `Unicode`（Win32 等）: `LONG_IDLE_MS`（保守的に長めに設定）
pub(crate) fn long_idle_ms_for(mode: InjectionMode) -> u64 {
    match mode {
        InjectionMode::Tsf => tuning::LONG_IDLE_MS,
        InjectionMode::Vk => tuning::CHROME_LONG_IDLE_MS,
        InjectionMode::Unicode => tuning::LONG_IDLE_MS,
    }
}

fn state_label(state: &GjiState) -> &'static str {
    match state {
        GjiState::OffCold => "OffCold",
        GjiState::OnCold {
            kind: ColdKind::Short,
            ..
        } => "OnCold(Short)",
        GjiState::OnCold {
            kind: ColdKind::Long,
            ..
        } => "OnCold(Long)",
        GjiState::OnWarm { .. } => "OnWarm",
        GjiState::OnComposing { .. } => "OnComposing",
    }
}

// ── ユニットテスト ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn ime_on() -> GjiEvent {
        GjiEvent::ImeOn {
            injection_mode: InjectionMode::Vk,
        }
    }

    fn focus_change() -> GjiEvent {
        GjiEvent::FocusChange {
            injection_mode: InjectionMode::Vk,
        }
    }

    fn complete(fsm: &GjiFsm) -> GjiEvent {
        let probe_id = match fsm.state() {
            GjiState::OnCold {
                probe: ProbeStatus::Running { probe_id },
                ..
            } => *probe_id,
            s => panic!("expected OnCold(Running), got {}", state_label(s)),
        };
        GjiEvent::WarmupComplete {
            probe_id,
            result: WarmupResult {
                path: WarmupPath::NameChangeConfirmed,
                prepend_f2_warmup: false,
                nc_fired: true,
                gji_resumed: false,
            },
        }
    }

    fn failed(fsm: &GjiFsm) -> GjiEvent {
        let probe_id = match fsm.state() {
            GjiState::OnCold {
                probe: ProbeStatus::Running { probe_id },
                ..
            } => *probe_id,
            s => panic!("expected OnCold(Running), got {}", state_label(s)),
        };
        GjiEvent::WarmupFailed { probe_id }
    }

    // ── ImeOn → OnCold(Short) ────────────────────────────────────────────

    #[test]
    fn ime_on_from_off_cold_starts_short_probe() {
        let mut fsm = GjiFsm::new();
        let r = fsm.on_event(ime_on());
        r.assert_consumed();
        r.assert_action_count(1);
        assert!(matches!(r.actions[0], GjiAction::StartProbe { .. }));
        assert!(matches!(fsm.state(), GjiState::OnCold { kind: ColdKind::Short, .. }));
    }

    #[test]
    fn ime_on_while_on_warm_is_ignored() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        // now OnWarm
        let r = fsm.on_event(ime_on());
        r.assert_consumed();
        r.assert_action_count(0); // ignored
        assert!(matches!(fsm.state(), GjiState::OnWarm { .. }));
    }

    // ── WarmupComplete → OnWarm ──────────────────────────────────────────

    #[test]
    fn warmup_complete_transitions_to_warm_with_timer() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        let r = fsm.on_event(ev);
        r.assert_consumed();
        r.assert_timer_set(GjiTimer::LongIdle);
        assert!(matches!(fsm.state(), GjiState::OnWarm { .. }));
    }

    #[test]
    fn warmup_complete_flushes_pending_input() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        fsm.on_event(GjiEvent::KeyInput(PendingInput::new("ka")));
        fsm.on_event(GjiEvent::KeyInput(PendingInput::new("na")));
        let ev = complete(&fsm);
        let r = fsm.on_event(ev);
        // SendInput action が存在するはず
        assert!(
            r.actions.iter().any(|a| matches!(a, GjiAction::SendInput { pending, .. } if pending.len() == 2)),
            "expected SendInput with 2 pending items, got {:?}", r.actions
        );
    }

    // ── WarmupFailed → OnWarm（conservative fallback）───────────────────

    #[test]
    fn warmup_failed_transitions_to_warm_with_fallback() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = failed(&fsm);
        let r = fsm.on_event(ev);
        r.assert_consumed();
        r.assert_timer_set(GjiTimer::LongIdle);
        assert!(matches!(fsm.state(), GjiState::OnWarm { .. }));
    }

    // ── probe_id stale 防止 ──────────────────────────────────────────────

    #[test]
    fn stale_warmup_complete_is_ignored() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        // 古い probe_id でイベントを作成
        let stale_probe_id = match fsm.state() {
            GjiState::OnCold { probe: ProbeStatus::Running { probe_id }, .. } => *probe_id,
            _ => panic!(),
        };
        // FocusChange で probe を再起動
        fsm.on_event(focus_change());
        // 古い probe_id の Complete → 無視されるはず
        let r = fsm.on_event(GjiEvent::WarmupComplete {
            probe_id: stale_probe_id,
            result: WarmupResult::conservative_fallback(),
        });
        r.assert_consumed();
        r.assert_action_count(0);
        // まだ OnCold のまま
        assert!(matches!(fsm.state(), GjiState::OnCold { .. }));
    }

    // ── FocusChange ─────────────────────────────────────────────────────

    #[test]
    fn focus_change_while_warm_enters_cold_short() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        let r = fsm.on_event(focus_change());
        r.assert_consumed();
        r.assert_timer_kill(GjiTimer::LongIdle);
        assert!(
            r.actions.iter().any(|a| matches!(a, GjiAction::StartProbe { .. })),
            "expected StartProbe action"
        );
        assert!(matches!(fsm.state(), GjiState::OnCold { kind: ColdKind::Short, .. }));
    }

    #[test]
    fn focus_change_while_cold_cancels_old_probe_and_starts_new() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let r = fsm.on_event(focus_change());
        // CancelProbe(old) + StartProbe(new) の2アクション
        assert!(
            r.actions.iter().any(|a| matches!(a, GjiAction::CancelProbe { .. })),
            "expected CancelProbe"
        );
        assert!(
            r.actions.iter().any(|a| matches!(a, GjiAction::StartProbe { .. })),
            "expected StartProbe"
        );
    }

    #[test]
    fn focus_change_while_off_cold_is_noop() {
        let mut fsm = GjiFsm::new();
        let r = fsm.on_event(focus_change());
        r.assert_consumed();
        r.assert_action_count(0);
        assert!(matches!(fsm.state(), GjiState::OffCold));
    }

    // ── LongIdle タイムアウト ────────────────────────────────────────────

    #[test]
    fn long_idle_timeout_from_warm_enters_cold_long_not_started() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        let r = fsm.on_timeout(GjiTimer::LongIdle);
        r.assert_consumed();
        assert!(matches!(
            fsm.state(),
            GjiState::OnCold { kind: ColdKind::Long, probe: ProbeStatus::NotStarted, .. }
        ));
    }

    #[test]
    fn key_input_in_cold_long_not_started_starts_probe() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        fsm.on_timeout(GjiTimer::LongIdle);
        let r = fsm.on_event(GjiEvent::KeyInput(PendingInput::new("a")));
        assert!(
            r.actions.iter().any(|a| matches!(a, GjiAction::StartProbe { .. })),
            "expected StartProbe on first KeyInput in Long cold"
        );
        assert!(matches!(
            fsm.state(),
            GjiState::OnCold { probe: ProbeStatus::Running { .. }, .. }
        ));
    }

    // ── StartComposition / EndComposition ────────────────────────────────

    #[test]
    fn start_then_end_composition_returns_to_warm() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        fsm.on_event(GjiEvent::StartComposition);
        assert!(matches!(fsm.state(), GjiState::OnComposing { .. }));

        let epoch = match fsm.state() {
            GjiState::OnComposing { epoch } => *epoch,
            _ => panic!(),
        };
        let r = fsm.on_event(GjiEvent::EndComposition { epoch });
        r.assert_consumed();
        r.assert_timer_set(GjiTimer::LongIdle);
        assert!(matches!(fsm.state(), GjiState::OnWarm { .. }));
    }

    #[test]
    fn stale_end_composition_is_ignored() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        fsm.on_event(GjiEvent::StartComposition);
        // FocusChange で epoch が進む
        fsm.on_event(focus_change());
        let ev2 = complete(&fsm);
        fsm.on_event(ev2);
        fsm.on_event(GjiEvent::StartComposition);

        // 古い epoch で EndComposition → 無視
        let r = fsm.on_event(GjiEvent::EndComposition {
            epoch: FocusEpoch(0),
        });
        r.assert_consumed();
        r.assert_action_count(0);
        assert!(matches!(fsm.state(), GjiState::OnComposing { .. }));
    }

    // ── ImeOff ──────────────────────────────────────────────────────────

    #[test]
    fn ime_off_from_warm_enters_off_cold() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        let r = fsm.on_event(GjiEvent::ImeOff);
        r.assert_consumed();
        r.assert_timer_kill(GjiTimer::LongIdle);
        assert!(matches!(fsm.state(), GjiState::OffCold));
    }

    #[test]
    fn ime_off_cancels_running_probe() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let r = fsm.on_event(GjiEvent::ImeOff);
        assert!(
            r.actions.iter().any(|a| matches!(a, GjiAction::CancelProbe { .. })),
            "expected CancelProbe on ImeOff while cold"
        );
        assert!(matches!(fsm.state(), GjiState::OffCold));
    }

    // ── KeyInput in OnWarm → timer reset ─────────────────────────────────

    #[test]
    fn key_input_in_warm_sends_direct_and_resets_timer() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        let r = fsm.on_event(GjiEvent::KeyInput(PendingInput::new("a")));
        assert!(matches!(r.actions[0], GjiAction::SendInputDirect(_)));
        r.assert_timer_set(GjiTimer::LongIdle);
    }
}
