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
use super::ime_event::{ChordKind, ImeEvent, ImeEventEnvelope, IntentSource};
use super::input_barrier::InputBarrier;
use super::observation_store::{ImeObservation, ObservationStore};

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
            reduce_count: 0,
        }
    }

    /// 現在 CtrlImeChord transaction が active か。
    /// `stage_post_decision` が二次 SetOpen を filter するかどうかの判断材料。
    #[must_use]
    pub const fn is_ctrl_ime_chord_active(&self) -> bool {
        matches!(self.input_barrier, Some(InputBarrier::CtrlImeChord { .. }))
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
            ImeEvent::FocusChanged { profile, .. } => {
                // Step 1.5: policy 確定 → observation 評価の順序ルール。
                // FocusChanged を受けた時点で policy を更新し、以降の observation は
                // 新しい policy で評価される。
                self.app_policy = AppImePolicy::from_profile(profile);
                // フォーカス変更で intent / observation は clear する
                // (旧アプリの観測値が新アプリで有効と勘違いされないため)
                self.last_intent = None;
                self.observations.clear_on_focus_change();
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
            // 以下は Step 6/7 で実装。
            ImeEvent::ImeApplyRequested { .. }
            | ImeEvent::ImeApplySucceeded { .. }
            | ImeEvent::ImeApplyFailed { .. }
            | ImeEvent::DriftDetected { .. } => {}
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
