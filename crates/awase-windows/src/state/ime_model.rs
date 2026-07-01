//! IME 状態モデル (Step 1: Shadow Reducer 段階)
//!
//! 既存の `ImeBelief` / `ImeObservations` と並走する shadow model。
//! 現状 (Step 1) は本番判定には使わず、diff log で検証するのみ。
//!
//! ## 設計原則
//!
//! 1. **UserIntent だけが `desired_open` を即時に変えられる**
//! 2. **Observer は `desired_open` を直接壊せない** (last_observed に記録するのみ)
//! 3. **AppImePolicy / InputBarrier / ForceGuardSet は後続 Step で追加** (Step 1 では placeholder)

use super::app_ime_policy::AppImePolicy;
use super::force_guard::{ForceGuardSet, ObserveMissMonitor};
use super::ime_event::{ChordKind, ImeEvent, ImeEventEnvelope, IntentSource};
use super::input_barrier::InputBarrier;
use super::observation_store::{ImeObservation, ObservationStore};
use super::transition::ImeTransition;

// ── AppliedImeState ──────────────────────────────────────────────────────────

/// IME apply 結果の確信度。
///
/// `Option<(bool, u64)>` + センチネル値 `ts=0` で表現していた3状態を型で明示する。
/// - `Unknown`   : フォーカス直後・起動時。実 IME 状態が不明。
/// - `Optimistic`: ImmCross async の楽観的事前更新。OS 未確認。
/// - `Confirmed` : 実 apply 完了・確認済み。旧 `applied_at_ms > 0` に相当。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppliedImeState {
    #[default]
    Unknown,
    Optimistic(bool),
    Confirmed {
        open: bool,
        at_ms: u64,
    },
}

impl AppliedImeState {
    /// `build_ime_control_view` 互換の `Option<(bool, u64)>` に変換する。
    #[must_use]
    pub const fn to_pair(self) -> Option<(bool, u64)> {
        match self {
            Self::Unknown => None,
            Self::Optimistic(open) => Some((open, 0)),
            Self::Confirmed { open, at_ms } => Some((open, at_ms)),
        }
    }

    /// apply 済みの open 値を返す（Optimistic も含む）。Unknown は None。
    #[must_use]
    pub const fn applied_open(self) -> Option<bool> {
        match self {
            Self::Unknown => None,
            Self::Optimistic(open) | Self::Confirmed { open, .. } => Some(open),
        }
    }

    /// 確認済み (`Confirmed`) かどうか。
    #[must_use]
    pub const fn is_confirmed(self) -> bool {
        matches!(self, Self::Confirmed { .. })
    }

    /// `Confirmed { open, at_ms }` の `at_ms` を返す。それ以外は 0。
    #[must_use]
    pub const fn confirmed_at_ms(self) -> u64 {
        match self {
            Self::Confirmed { at_ms, .. } => at_ms,
            _ => 0,
        }
    }
}

/// Shadow IME モデル。最終形 (Phase 3 完了時) ではこれが SSOT になる予定。
///
/// Step 3 時点: desired_open + last_intent + observations (per-source + drift) + policy。
/// pending transition / barrier / force guard は後続 Step で追加。
#[derive(Debug)]
pub struct ImeModel {
    /// awase が IME をこうしたい状態。UserIntent のみが書き換える。
    pub desired_open: bool,

    /// 直近のユーザー意図 (intent guard 等の判断材料)
    pub last_intent: Option<RecordedIntent>,

    /// 観測値ストア (Step 3) — per-source + suspicious + drift。
    /// reducer の judge 材料: 鮮度・合意・乖離継続時間。
    pub observations: ObservationStore,

    /// 現フォーカスアプリの IME 制御ポリシー (Step 1.5)。
    /// FocusChanged event で更新される。
    pub app_policy: AppImePolicy,

    /// 入力 chord 等の一時 transaction (Step 4)。
    /// 旧 `ctrl_bypass_hold: bool` の置換。
    pub input_barrier: Option<InputBarrier>,

    /// 発火後の force-on ガード集合 (Step 6)。
    /// 旧 `ImeRecoveryState::force_on_*` 2 つの bool を `ForceGuardSet` に統合。
    pub force_guards: ForceGuardSet,

    /// 発火前の観測失敗カウンタ (Step 6)。
    /// 旧 `ImeRecoveryState::ime_detect_miss_count` の責務分離。
    pub observe_miss_monitor: ObserveMissMonitor,

    /// OS への apply 進行中の transition (Step 7)。
    /// 旧 `ImeEffect::SetOpen` (Layer 3) + 楽観的 latch を統合。
    pub pending: Option<ImeTransition>,

    /// 最後に actuator が成功させた IME 開閉状態の確信度 (Step 7)。
    /// 旧 `applied_open: Option<bool>` + `applied_at_ms: u64` の置換。
    pub applied: AppliedImeState,
}

#[derive(Debug, Clone)]
pub struct RecordedIntent {
    pub target: bool,
    pub source: IntentSource,
    pub at_ms: u64,
}

impl ImeModel {
    /// 既存 `ImeBelief` の初期値 (`ime_on=true`) に合わせる。
    #[must_use]
    pub fn new() -> Self {
        Self {
            desired_open: true,
            last_intent: None,
            observations: ObservationStore::default(),
            app_policy: AppImePolicy::standard(),
            input_barrier: None,
            force_guards: ForceGuardSet::default(),
            observe_miss_monitor: ObserveMissMonitor::default(),
            pending: None,
            applied: AppliedImeState::Unknown,
        }
    }

    /// 現在 CtrlImeChord transaction が active か。
    /// `stage_post_decision` が二次 SetOpen を filter するかどうかの判断材料。
    #[must_use]
    pub const fn is_ctrl_ime_chord_active(&self) -> bool {
        matches!(self.input_barrier, Some(InputBarrier::CtrlImeChord { .. }))
    }

    /// `desired_open` を `force_guards` で override した最終値 (Step 6)。
    ///
    /// guard が active なら `true` を強制、そうでなければ `desired_open` をそのまま。
    /// Phase 3d 以降は `PlatformState::ime_on()` の SSOT。
    #[must_use]
    pub const fn effective_open(&self) -> bool {
        self.force_guards.effective_open(self.desired_open)
    }

    /// `AppliedImeState` を返す。executor の applied_snapshot 同期用。
    #[must_use]
    pub const fn applied_state(&self) -> AppliedImeState {
        self.applied
    }

    /// `build_ime_control_view` 互換の `Option<(bool, u64)>` を返す。
    #[must_use]
    pub const fn applied_pair(&self) -> Option<(bool, u64)> {
        self.applied.to_pair()
    }

    /// `pending` transition の generation を返す。apply 完了 event の照合用。
    #[must_use]
    pub fn pending_generation(&self) -> Option<u64> {
        self.pending.as_ref().map(|p| p.generation)
    }

    /// 現在の `input_barrier` が持つ chord kind を返す。
    #[must_use]
    pub fn active_chord_kind(&self) -> Option<ChordKind> {
        self.input_barrier.and_then(|b| b.chord_kind())
    }

    /// フォーカス切替直後の one-shot barrier が pending かどうか。
    #[must_use]
    pub fn is_focus_transition_pending(&self) -> bool {
        self.input_barrier
            .as_ref()
            .is_some_and(InputBarrier::is_focus_transition)
    }
}

impl Default for ImeModel {
    fn default() -> Self {
        Self::new()
    }
}

impl ImeModel {
    /// Event を反映する。
    ///
    /// **UserIntent だけが `desired_open` を即時に変えられる**。
    /// Observer は `observations` に記録するだけで desired を壊さない。
    pub fn reduce(&mut self, envelope: &ImeEventEnvelope) {
        match envelope.event {
            ImeEvent::UserImeToggleIntent { source } => {
                let target = !self.desired_open;
                self.desired_open = target;
                self.last_intent = Some(RecordedIntent {
                    target,
                    source,
                    at_ms: envelope.time.tick_ms,
                });
            }
            ImeEvent::UserImeSetIntent { target, source } => {
                self.desired_open = target;
                self.last_intent = Some(RecordedIntent {
                    target,
                    source,
                    at_ms: envelope.time.tick_ms,
                });
            }
            ImeEvent::ObserverReported {
                open,
                source,
                confidence,
                hwnd,
            } => {
                // 絶対ルール: Observer は desired_open を直接書き換えない
                self.observations.record(ImeObservation {
                    open,
                    source,
                    at: envelope.time.monotonic,
                    hwnd,
                    confidence,
                    expires_at: None,
                });
                // drift 追跡 (desired と observed の乖離)
                self.observations
                    .update_drift(self.desired_open, open, envelope.time.monotonic);
            }
            ImeEvent::FocusChanged { profile, to, .. } => {
                // Step 1.5/5: policy 確定 → observation 評価の順序ルール。
                // FocusChanged を受けた時点で policy を更新し、以降の observation は
                // 新しい policy で評価される。
                self.app_policy = AppImePolicy::from_profile(profile);
                // フォーカス変更で intent / observation / applied / force_guard / drift は clear する
                // (旧アプリの観測値が新アプリで有効と勘違いされないため)
                self.last_intent = None;
                self.observations.clear_on_focus_change();
                log::debug!("[explicit-intent] cleared (focus change)");
                self.applied = AppliedImeState::Unknown;
                // force_guard: 旧アプリ文脈の guard を新アプリに引き継がない
                self.force_guards.clear_for_focus_change();
                // observe_miss_monitor: 旧アプリの miss_count が新アプリで閾値を誤超えしないようリセット
                self.observe_miss_monitor.record_success();
                // Step 5: FocusTransition barrier を立てる (旧 focus_transition_pending 相当)。
                // settle_until は AppImePolicy.focus_settle_ms 由来。
                let settle_until = envelope.time.monotonic
                    + std::time::Duration::from_millis(self.app_policy.focus_settle_ms);
                self.input_barrier = Some(InputBarrier::FocusTransition {
                    to_hwnd: to,
                    started_seq: envelope.time.seq,
                    started_at: envelope.time.monotonic,
                    settle_until,
                });
            }
            ImeEvent::ChordStarted { kind } => {
                // Step 4: chord transaction を開始。barrier を立てる。
                // CtrlMuhenkanImeOff/CtrlHenkanImeOn から target を導出。
                let target = matches!(kind, ChordKind::CtrlHenkanImeOn);
                self.input_barrier = Some(InputBarrier::CtrlImeChord {
                    target,
                    kind,
                    started_seq: envelope.time.seq,
                    started_at: envelope.time.monotonic,
                });
            }
            ImeEvent::ChordEnded { .. } => {
                // Step 4: chord transaction を終了。barrier を解除。
                self.input_barrier = None;
            }
            ImeEvent::ImeApplyRequested {
                target,
                generation,
                ctrl_held,
            } => {
                // Step 7: pending transition を立てる。
                // 実際の timeout / actuator 詳細は呼び出し元 (Phase 3 cleanup) が
                // 個別 dispatch で渡す想定。Step 7 では最低限の placeholder。
                self.pending = Some(ImeTransition {
                    target,
                    generation,
                    timeout_at: envelope.time.monotonic + std::time::Duration::from_secs(1),
                });
                // Chord 開始判断: IME OFF 要求 + Ctrl 押下中 → CtrlImeChord barrier を立てる。
                // KANJI（Ctrl なし）では立てない: ChordEnded のトリガが Ctrl KeyUp なので
                // ペアにならず永続する事故を防ぐ。
                if !target && ctrl_held {
                    self.input_barrier = Some(InputBarrier::CtrlImeChord {
                        target: false,
                        kind: ChordKind::CtrlMuhenkanImeOff,
                        started_seq: envelope.time.seq,
                        started_at: envelope.time.monotonic,
                    });
                }
                // Chord 中に IME ON 要求が来た場合 → chord を即時終了する。
                if target && self.is_ctrl_ime_chord_active() {
                    self.input_barrier = None;
                }
            }
            ImeEvent::ImeApplySucceeded { target, generation } => {
                // Step 7: **必須** generation 照合で stale apply を排除。
                // pending の generation と一致しなければ無視する。
                if self.pending.as_ref().map(|p| p.generation) == Some(generation) {
                    self.applied = AppliedImeState::Confirmed {
                        open: target,
                        at_ms: envelope.time.tick_ms,
                    };
                    self.pending = None;
                }
                // 一致しない場合は何もしない (stale → 無視)
            }
            ImeEvent::ImeApplyFailed { generation, .. } => {
                // 同じく generation 照合
                if self.pending.as_ref().map(|p| p.generation) == Some(generation) {
                    self.pending = None;
                }
            }
            ImeEvent::DriftDetected { desired, .. } => {
                // skip_override を無効化する: Optimistic にリセットすることで
                // 次の SetOpen(desired) が「確認済み apply がない」扱いになり skip されなくなる。
                // applied は desired に合わせて楽観的にセット（ImmCross async 送信と同じ扱い）。
                self.applied = AppliedImeState::Optimistic(desired);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ime_event::{
        EventTime, HwndId, ImePolicyProfile, ObservationConfidence, ObservationSource,
    };
    use super::*;
    use crate::state::force_guard::{ForceGuard, ForceOnReason};
    use std::time::Instant;

    fn envelope(seq: u64, event: ImeEvent) -> ImeEventEnvelope {
        ImeEventEnvelope {
            time: EventTime {
                seq,
                monotonic: Instant::now(),
                tick_ms: 0,
            },
            event,
        }
    }

    #[test]
    fn user_intent_sets_desired() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::UserImeSetIntent {
                target: false,
                source: IntentSource::PhysicalImeKey,
            },
        ));
        assert!(!model.desired_open);
        assert_eq!(model.last_intent.as_ref().unwrap().target, false);
    }

    #[test]
    fn toggle_intent_flips_desired() {
        let mut model = ImeModel::new(); // desired_open = true (default)
        model.reduce(&envelope(
            1,
            ImeEvent::UserImeToggleIntent {
                source: IntentSource::PhysicalImeKey,
            },
        ));
        assert!(!model.desired_open);
        model.reduce(&envelope(
            2,
            ImeEvent::UserImeToggleIntent {
                source: IntentSource::PhysicalImeKey,
            },
        ));
        assert!(model.desired_open);
    }

    #[test]
    fn observer_does_not_change_desired() {
        let mut model = ImeModel::new(); // desired_open = true
        model.reduce(&envelope(
            1,
            ImeEvent::ObserverReported {
                open: false,
                source: ObservationSource::ObserverPoll,
                hwnd: HwndId::NULL,
                confidence: ObservationConfidence::Medium,
            },
        ));
        assert!(model.desired_open, "observer は desired を壊さない");
        assert_eq!(
            model
                .observations
                .per_source
                .observer_poll
                .as_ref()
                .unwrap()
                .open,
            false
        );
    }

    fn focus_changed_event(seq: u64) -> ImeEventEnvelope {
        envelope(
            seq,
            ImeEvent::FocusChanged {
                from: None,
                to: HwndId::NULL,
                profile: ImePolicyProfile::ImmCross,
            },
        )
    }

    #[test]
    fn focus_change_clears_force_guards() {
        let mut model = ImeModel::new();
        model.force_guards.add(ForceGuard {
            reason: ForceOnReason::BrokenAppBootstrap,
            expires_at: None,
            generation: 1,
        });
        assert!(model.force_guards.requires_on());

        model.reduce(&focus_changed_event(2));

        assert!(
            !model.force_guards.requires_on(),
            "focus change で force guard が解除される"
        );
    }

    #[test]
    fn focus_change_resets_observe_miss_monitor() {
        let mut model = ImeModel::new();
        let t = Instant::now();
        model.observe_miss_monitor.record_miss(t);
        model.observe_miss_monitor.record_miss(t);
        model.observe_miss_monitor.record_miss(t);
        assert!(model.observe_miss_monitor.exceeds(3));

        model.reduce(&focus_changed_event(2));

        assert!(
            !model.observe_miss_monitor.exceeds(1),
            "focus change で observe_miss_monitor がリセットされる"
        );
    }

    #[test]
    fn focus_change_does_not_clear_desired_open() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::UserImeSetIntent {
                target: true,
                source: IntentSource::PhysicalImeKey,
            },
        ));
        assert!(model.desired_open);

        model.reduce(&focus_changed_event(2));

        assert!(
            model.desired_open,
            "focus change は desired_open を変えない"
        );
    }

    // ── ImeApplyRequested による chord barrier 制御 (Phase 2) ──

    #[test]
    fn ime_off_with_ctrl_held_starts_chord() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::ImeApplyRequested {
                target: false,
                generation: 1,
                ctrl_held: true,
            },
        ));
        assert!(
            model.is_ctrl_ime_chord_active(),
            "IME OFF 要求 + Ctrl 押下中 → chord 開始"
        );
        assert_eq!(model.active_chord_kind(), Some(ChordKind::CtrlMuhenkanImeOff));
    }

    #[test]
    fn ime_off_without_ctrl_does_not_start_chord() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::ImeApplyRequested {
                target: false,
                generation: 1,
                ctrl_held: false,
            },
        ));
        assert!(
            !model.is_ctrl_ime_chord_active(),
            "KANJI（Ctrl なし）IME OFF では chord を開始しない"
        );
    }

    #[test]
    fn ime_on_during_chord_ends_it() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::ImeApplyRequested {
                target: false,
                generation: 1,
                ctrl_held: true,
            },
        ));
        assert!(model.is_ctrl_ime_chord_active());

        model.reduce(&envelope(
            2,
            ImeEvent::ImeApplyRequested {
                target: true,
                generation: 2,
                ctrl_held: true,
            },
        ));
        assert!(
            !model.is_ctrl_ime_chord_active(),
            "chord 中の IME ON 要求は chord を即時終了する"
        );
    }
}
