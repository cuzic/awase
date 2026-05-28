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
use super::ime_event::{
    ImeEvent, ImeEventEnvelope, IntentSource, ObservationConfidence, ObservationSource,
};

/// Shadow IME モデル。最終形 (Phase 3 完了時) ではこれが SSOT になる予定。
///
/// Step 1.5 時点: desired_open + 最低限の観測記録 + 現フォーカスアプリの policy。
/// pending transition / barrier / force guard は後続 Step で追加。
#[derive(Debug)]
pub struct ImeModel {
    /// awase が IME をこうしたい状態。UserIntent のみが書き換える。
    pub desired_open: bool,

    /// 直近のユーザー意図 (intent guard 等の判断材料)
    pub last_intent: Option<RecordedIntent>,

    /// 直近の外部観測 (Step 3 で ObservationStore に拡張予定)
    pub last_observation: Option<RecordedObservation>,

    /// 現フォーカスアプリの IME 制御ポリシー (Step 1.5)。
    /// FocusChanged event で更新される。
    pub app_policy: AppImePolicy,

    /// reduce 呼び出し回数。診断用。
    pub reduce_count: u64,
}

#[derive(Debug, Clone)]
pub struct RecordedIntent {
    pub target: bool,
    pub source: IntentSource,
    pub at_seq: u64,
}

#[derive(Debug, Clone)]
pub struct RecordedObservation {
    pub open: bool,
    pub source: ObservationSource,
    pub confidence: ObservationConfidence,
    pub at_seq: u64,
}

impl ImeModel {
    /// 既存 `ImeBelief` の初期値 (`ime_on=true`) に合わせる。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            desired_open: true,
            last_intent: None,
            last_observation: None,
            app_policy: AppImePolicy::standard(),
            reduce_count: 0,
        }
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
    /// Observer は `last_observation` に記録するだけで desired を壊さない。
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
                ..
            } => {
                // 絶対ルール: Observer は desired_open を直接書き換えない
                self.last_observation = Some(RecordedObservation {
                    open,
                    source,
                    confidence,
                    at_seq: envelope.time.seq,
                });
            }
            ImeEvent::FocusChanged { profile, .. } => {
                // Step 1.5: policy 確定 → observation 評価の順序ルール。
                // FocusChanged を受けた時点で policy を更新し、以降の observation は
                // 新しい policy で評価される。
                self.app_policy = AppImePolicy::from_profile(profile);
                // フォーカス変更で intent / observation は clear する
                // (旧アプリの観測値が新アプリで有効と勘違いされないため)
                self.last_intent = None;
                self.last_observation = None;
            }
            // 以下は Step 4 以降で実装。Step 1.5 では無視。
            ImeEvent::ImeApplyRequested { .. }
            | ImeEvent::ImeApplySucceeded { .. }
            | ImeEvent::ImeApplyFailed { .. }
            | ImeEvent::ChordStarted { .. }
            | ImeEvent::ChordEnded { .. }
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
    use super::super::ime_event::{EventTime, HwndId};
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
        assert_eq!(model.last_observation.as_ref().unwrap().open, false);
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
