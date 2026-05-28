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
use super::force_guard::{DriftMonitor, ForceGuardSet};
use super::ime_event::{ChordKind, ImeEvent, ImeEventEnvelope, IntentSource};
use super::input_barrier::InputBarrier;
use super::observation_store::{ImeObservation, ObservationStore};
use super::transition::ImeTransition;

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
    pub drift_monitor: DriftMonitor,

    /// OS への apply 進行中の transition (Step 7)。
    /// 旧 `ImeEffect::SetOpen` (Layer 3) + 楽観的 latch を統合。
    pub pending: Option<ImeTransition>,

    /// 最後に actuator が成功させた IME 開閉状態 (Step 7)。
    /// 旧 `last_applied_ime_on` (Layer 4) の置換。`None` は未適用。
    pub applied_open: Option<bool>,

    /// reduce 呼び出し回数。診断用。
    pub reduce_count: u64,
}

#[derive(Debug, Clone)]
pub struct RecordedIntent {
    pub target: bool,
    pub source: IntentSource,
    pub at_seq: u64,
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
            drift_monitor: DriftMonitor::default(),
            pending: None,
            applied_open: None,
            reduce_count: 0,
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
}

impl Default for ImeModel {
    fn default() -> Self {
        Self::new()
    }
}

/// Reducer が比較・公開する "最終的な判断結果"。
///
/// `old_belief.ime_on` と `new_model.desired_open` を直接比較するのは意味論が違うため、
/// reducer / engine が最終的に判断する値同士を比較する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImeEffectiveState {
    /// Engine を活性化すべきか (desired_open + policy + user_enabled から導出)
    pub engine_should_be_active: bool,
    /// OS IME へ適用すべき開閉状態
    pub ime_target_open: bool,
    /// OS への apply が必要か (Step 7 で意味を持つ、Step 1 では未使用)
    pub apply_needed: bool,
}

/// Diff log の重大度。Step 1 中の 1 週間モニタで分類する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffSeverity {
    /// 既知の許容範囲 (focus 直後等)。カウントのみ。
    Expected,
    /// 説明が必要。調査して Expected または Regression に分類する。
    Suspicious,
    /// 修正必須。次 Step に進む前に解消する。
    Regression,
}

impl ImeEffectiveState {
    /// 旧 belief との diff を分類する。
    ///
    /// Step 1 では完全な policy が無いため、observer による上書きが起きうる
    /// シナリオは `Suspicious` 扱いとして 1 週間モニタで実態を見る。
    #[must_use]
    pub fn classify_diff(old_ime_on: bool, new_target: bool) -> Option<DiffSeverity> {
        if old_ime_on == new_target {
            return None;
        }
        // Step 1: 全 diff を Suspicious として記録。
        // Step 1.5 (AppImePolicy) 導入後に Expected/Regression の細分化を行う。
        Some(DiffSeverity::Suspicious)
    }
}

impl ImeModel {
    /// Event を反映する。
    ///
    /// **UserIntent だけが `desired_open` を即時に変えられる**。
    /// Observer は `observations` に記録するだけで desired を壊さない。
    pub fn reduce(&mut self, envelope: &ImeEventEnvelope) {
        self.reduce_count = self.reduce_count.wrapping_add(1);
        match envelope.event {
            ImeEvent::UserImeToggleIntent { source } => {
                let target = !self.desired_open;
                self.desired_open = target;
                self.last_intent = Some(RecordedIntent {
                    target,
                    source,
                    at_seq: envelope.time.seq,
                });
            }
            ImeEvent::UserImeSetIntent { target, source } => {
                self.desired_open = target;
                self.last_intent = Some(RecordedIntent {
                    target,
                    source,
                    at_seq: envelope.time.seq,
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
                    recorded_seq: envelope.time.seq,
                    hwnd,
                    confidence,
                    expires_at: None,
                });
                // drift 追跡 (desired と observed の乖離)
                self.observations.update_drift(
                    self.desired_open,
                    open,
                    envelope.time.monotonic,
                    envelope.time.seq,
                );
            }
            ImeEvent::FocusChanged { profile, to, .. } => {
                // Step 1.5/5: policy 確定 → observation 評価の順序ルール。
                // FocusChanged を受けた時点で policy を更新し、以降の observation は
                // 新しい policy で評価される。
                self.app_policy = AppImePolicy::from_profile(profile);
                // フォーカス変更で intent / observation は clear する
                // (旧アプリの観測値が新アプリで有効と勘違いされないため)
                self.last_intent = None;
                self.observations.clear_on_focus_change();
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
            ImeEvent::ImeApplyRequested { target, generation } => {
                // Step 7: pending transition を立てる。
                // 実際の timeout / actuator 詳細は呼び出し元 (Phase 3 cleanup) が
                // 個別 dispatch で渡す想定。Step 7 では最低限の placeholder。
                self.pending = Some(ImeTransition {
                    target,
                    generation,
                    requested_at: envelope.time.monotonic,
                    actuator: self.app_policy.actuator_kind,
                    optimistic_applied: false,
                    timeout_at: envelope.time.monotonic + std::time::Duration::from_millis(1000),
                });
            }
            ImeEvent::ImeApplySucceeded { target, generation } => {
                // Step 7: **必須** generation 照合で stale apply を排除。
                // pending の generation と一致しなければ無視する。
                if self.pending.as_ref().map(|p| p.generation) == Some(generation) {
                    self.applied_open = Some(target);
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
            ImeEvent::DriftDetected { .. } => {
                // Step 7 では reducer 側で扱わない (Phase 3 cleanup で
                // observation_store.update_drift と統合予定)
            }
        }
    }

    /// 最終的な判断結果を返す (Step 1 では desired_open を直訳)。
    ///
    /// AppImePolicy / engine_active / pending が揃う Step 7 でこのロジックは充実する。
    #[must_use]
    pub const fn effective_state(&self) -> ImeEffectiveState {
        ImeEffectiveState {
            engine_should_be_active: self.desired_open,
            ime_target_open: self.desired_open,
            apply_needed: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ime_event::{
        EventTime, HwndId, ObservationConfidence, ObservationSource,
    };
    use super::*;
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

    #[test]
    fn diff_severity_match_returns_none() {
        assert_eq!(ImeEffectiveState::classify_diff(true, true), None);
        assert_eq!(ImeEffectiveState::classify_diff(false, false), None);
    }

    #[test]
    fn diff_severity_mismatch_returns_suspicious() {
        assert_eq!(
            ImeEffectiveState::classify_diff(true, false),
            Some(DiffSeverity::Suspicious)
        );
    }
}
