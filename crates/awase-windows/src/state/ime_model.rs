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
use super::conv_mode::ConvModeAuthority;
use super::force_guard::{ForceGuardSet, ObserveMissMonitor};
use awase::engine::InputModeState;

use super::ime_event::{
    ChordKind, ImeEvent, ImeEventEnvelope, InputModeApplyResult, UserIntentSource,
    ObservationConfidence,
};
use super::input_barrier::InputBarrier;
use super::observation_store::{ImeObservation, ObservationStore};
use std::time::Instant;
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
    ///
    /// private フィールド。`reduce()` 以外からの書き込みを禁止するため、
    /// 外部からは読み取り専用アクセサ `desired_open()` を使うこと
    /// （`conv_mode_authority` と同じパターン）。
    desired_open: bool,

    /// 入力モード（ローマ字/かな/英数/不明）の belief。
    ///
    /// H-3-b で追加。H-3-c で `ImeBelief::input_mode` への直接代入が
    /// `InputModeObserved` / `InputModeApplied` / `UserChangedInputMode` イベント経由に
    /// 置換されるまでは shadow として記録するのみで本番判定には使わない。
    /// H-3-d で `ImeBelief::input_mode` が private 化されたのち、このフィールドが SSOT になる。
    ///
    /// private フィールド。外部からは読み取り専用アクセサ `input_mode()` を使うこと。
    input_mode: InputModeState,

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

    /// IME 変換モードに対する awase の所有権状態 (H-3-e)。
    ///
    /// `ImeEvent::ConvModeOwnershipChanged` によってのみ更新される。
    /// 外部からの読み取りは `ImeModel::conv_mode_authority()` を経由すること。
    /// 書き込みは `reduce()` 経由のみ。
    conv_mode_authority: ConvModeAuthority,
}

#[derive(Debug, Clone)]
pub struct RecordedIntent {
    pub target: bool,
    pub source: UserIntentSource,
    pub at_ms: u64,
}

impl ImeModel {
    /// 既存 `ImeBelief` の初期値 (`ime_on=true`) に合わせる。
    #[must_use]
    pub fn new() -> Self {
        Self {
            desired_open: true,
            input_mode: InputModeState::ObservedRomaji, // ImeBelief 初期値に合わせる
            last_intent: None,
            observations: ObservationStore::default(),
            app_policy: AppImePolicy::standard(),
            input_barrier: None,
            force_guards: ForceGuardSet::default(),
            observe_miss_monitor: ObserveMissMonitor::default(),
            pending: None,
            applied: AppliedImeState::Unknown,
            conv_mode_authority: ConvModeAuthority::Unknown,
        }
    }

    /// conv mode 所有権状態を返す（読み取り専用アクセサ）。
    ///
    /// H-3-e: `conv_mode_authority` フィールドは private。外部から書き込まず
    /// `ImeEvent::ConvModeOwnershipChanged` 経由で reducer を通すこと。
    #[must_use]
    pub fn conv_mode_authority(&self) -> ConvModeAuthority {
        self.conv_mode_authority
    }

    /// awase が IME をこうしたい状態（読み取り専用アクセサ）。
    ///
    /// `desired_open` フィールドは private。外部から書き込まず
    /// `ImeEvent::UserImeSetIntent` / `UserImeToggleIntent` 経由で reducer を通すこと。
    /// 実効値が欲しい場合は `effective_open()` を使うこと（こちらは生の意図のみ）。
    #[must_use]
    pub const fn desired_open(&self) -> bool {
        self.desired_open
    }

    /// 入力モードの belief を返す（読み取り専用アクセサ）。
    ///
    /// `input_mode` フィールドは private。外部から書き込まず
    /// `InputModeObserved` / `InputModeApplied` / `UserChangedInputMode` 経由で
    /// reducer を通すこと。
    #[must_use]
    pub const fn input_mode(&self) -> InputModeState {
        self.input_mode
    }

    /// テスト専用: `desired_open` を直接設定する。
    ///
    /// carry-over シナリオ（focus 変更前の stale な desired_open）をテストで
    /// 模擬するための脱出口。本番コードから呼んではならない。
    #[cfg(test)]
    pub(crate) fn set_desired_open_for_test(&mut self, value: bool) {
        self.desired_open = value;
    }

    /// 現在 CtrlImeChord transaction が active か。
    /// `stage_post_decision` が二次 SetOpen を filter するかどうかの判断材料。
    #[must_use]
    pub const fn is_ctrl_ime_chord_active(&self) -> bool {
        matches!(self.input_barrier, Some(InputBarrier::CtrlImeChord { .. }))
    }

    /// ユーザー/awase の明示的な意図が present かどうか。
    ///
    /// true の場合は `desired_open` を観測より優先する。
    /// false の場合は observation pool の `derive_open()` 結果を採用し、
    /// 観測が空なら `desired_open` にフォールバックする。
    ///
    /// `last_intent` は `UserImeSetIntent` / `UserImeToggleIntent` のみが設定する。
    /// `PanicReset` / `HwndCacheRestored` は設定しないため、ここで除外不要。
    fn has_user_explicit_intent(&self) -> bool {
        self.last_intent.is_some()
    }

    /// 観測プールと `desired_open` を統合した最終 belief (Step 6)。
    ///
    /// - ユーザーの明示意図がある場合: `desired_open` を優先（観測で上書きしない）
    /// - 明示意図なし（フォーカス変化直後等）:
    ///   1. `derive_open()`（Medium+ の合意 / High 即採用）の結果を採用
    ///   2. それが `None` なら `most_recent_trusted()`（confidence 不問、最新優先）
    ///      にフォールバック。cache-miss 等の安全デフォルト推測（Low confidence の
    ///      `HeuristicDefault`）はここでのみ効き、後から届いた実観測（Lowでも）が
    ///      新しければそちらが優先される。
    ///   3. 観測が一切なければ `desired_open` にフォールバック
    /// - 最後に `force_guards` を適用（guard が active なら強制 ON）
    #[must_use]
    pub fn effective_open(&self) -> bool {
        let now = Instant::now();
        let base = if self.has_user_explicit_intent() {
            self.desired_open
        } else {
            self.observations
                .derive_open(now)
                .or_else(|| self.observations.most_recent_trusted(now).map(|o| o.open))
                .unwrap_or(self.desired_open)
        };
        self.force_guards.effective_open(base)
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
            ImeEvent::PanicReset { target } => {
                // 復旧操作: desired_open を安全デフォルト値に戻す。
                // UserImeSetIntent と異なり last_intent を設定しない。
                // ForceGuard::PanicReset が IME ON を保証するため、
                // has_user_explicit_intent() を汚染しない。
                self.desired_open = target;
            }
            ImeEvent::HwndCacheRestored { target } => {
                // HWND キャッシュ復元: 前回フォーカス時の desired_open を回復する。
                // ユーザーの能動的操作ではないため last_intent を設定しない。
                // has_user_explicit_intent() が false のまま維持され、
                // 後続の実観測が effective_open() を上書きできる。
                self.desired_open = target;
            }
            ImeEvent::ObserverReported {
                open,
                source,
                confidence,
                hwnd,
                focus_epoch,
            } => {
                // 絶対ルール: Observer は desired_open を直接書き換えない
                self.observations.record(ImeObservation {
                    open,
                    source,
                    at: envelope.time.monotonic,
                    hwnd,
                    confidence,
                    expires_at: None,
                    focus_epoch,
                });
                // drift 追跡 (desired と observed の乖離)
                self.observations
                    .update_drift(self.desired_open, open, envelope.time.monotonic);
            }
            ImeEvent::FocusChanged { profile, to, focus_epoch, .. } => {
                // Step 1.5/5: policy 確定 → observation 評価の順序ルール。
                // FocusChanged を受けた時点で policy を更新し、以降の observation は
                // 新しい policy で評価される。
                self.app_policy = AppImePolicy::from_profile(profile);
                // フォーカス変更で intent / observation / applied / force_guard / drift は clear する
                // (旧アプリの観測値が新アプリで有効と勘違いされないため)
                self.last_intent = None;
                // 新しい epoch を store に伝える。derive_open() はこれ以降、
                // 古い epoch の ImmCrossProbe / FocusProbe を無視する。
                self.observations.clear_on_focus_change(focus_epoch);
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
            ImeEvent::InputModeObserved { mode, confidence, .. } => {
                // ON/OFF の derive_open() と同じ考え方: Low confidence 単独では
                // belief を動かさない（記録のみ）。Medium+ のみ input_mode を上書きする。
                if confidence >= ObservationConfidence::Medium {
                    self.input_mode = mode;
                } else {
                    log::debug!(
                        "[input-mode] Low confidence observation 無視: {mode:?} (confidence={confidence:?})"
                    );
                }
            }
            ImeEvent::InputModeApplied { mode, result, .. } => {
                // Skipped の場合はモード変更が起きていないため更新しない。
                if result == InputModeApplyResult::Applied {
                    self.input_mode = mode;
                }
            }
            ImeEvent::UserChangedInputMode { mode, .. } => {
                // ユーザーの明示操作 → 観測と同等の信頼度で即時反映する。
                self.input_mode = mode;
            }
            ImeEvent::ConvModeOwnershipChanged { authority } => {
                // H-3-e: エンジン ON/OFF・warmup 開始/終了で conv mode 所有権を更新する。
                // `conv_mode_authority` は private フィールドのため、このイベント経由のみで更新される。
                log::debug!(
                    "[conv-authority] {:?} → {authority:?}",
                    self.conv_mode_authority
                );
                self.conv_mode_authority = authority;
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
                source: UserIntentSource::PhysicalImeKey,
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
                source: UserIntentSource::PhysicalImeKey,
            },
        ));
        assert!(!model.desired_open);
        model.reduce(&envelope(
            2,
            ImeEvent::UserImeToggleIntent {
                source: UserIntentSource::PhysicalImeKey,
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
                focus_epoch: 0,
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
    fn effective_open_falls_back_to_most_recent_trusted_when_derive_open_is_none() {
        let mut model = ImeModel::new(); // desired_open = true, 明示 intent なし
        // Low confidence 単独 → derive_open() は None（Medium+ 専用のため）。
        model.reduce(&envelope(
            1,
            ImeEvent::ObserverReported {
                open: false,
                source: ObservationSource::HeuristicDefault,
                hwnd: HwndId::NULL,
                confidence: ObservationConfidence::Low,
                focus_epoch: 0,
            },
        ));
        assert!(
            !model.effective_open(),
            "derive_open()=None でも most_recent_trusted() の Low observation が \
             desired_open より優先される"
        );
    }

    #[test]
    fn effective_open_medium_observation_overrides_low_fallback() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::ObserverReported {
                open: false,
                source: ObservationSource::HeuristicDefault,
                hwnd: HwndId::NULL,
                confidence: ObservationConfidence::Low,
                focus_epoch: 0,
            },
        ));
        model.reduce(&envelope(
            2,
            ImeEvent::ObserverReported {
                open: true,
                source: ObservationSource::ObserverPoll,
                hwnd: HwndId::NULL,
                confidence: ObservationConfidence::Medium,
                focus_epoch: 0,
            },
        ));
        assert!(
            model.effective_open(),
            "Medium confidence の derive_open() 結果が Low fallback より常に優先される"
        );
    }

    #[test]
    fn input_mode_observed_low_confidence_is_ignored() {
        let mut model = ImeModel::new(); // input_mode = ObservedRomaji (初期値)
        model.reduce(&envelope(
            1,
            ImeEvent::InputModeObserved {
                mode: InputModeState::ObservedEisu,
                source: ObservationSource::FocusProbe,
                confidence: ObservationConfidence::Low,
                at: crate::state::TickMs(0),
            },
        ));
        assert_eq!(
            model.input_mode(),
            InputModeState::ObservedRomaji,
            "Low confidence の観測は input_mode を上書きしない"
        );
    }

    #[test]
    fn input_mode_observed_medium_confidence_updates() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::InputModeObserved {
                mode: InputModeState::ObservedEisu,
                source: ObservationSource::ObserverPoll,
                confidence: ObservationConfidence::Medium,
                at: crate::state::TickMs(0),
            },
        ));
        assert_eq!(
            model.input_mode(),
            InputModeState::ObservedEisu,
            "Medium+ confidence の観測は input_mode を更新する"
        );
    }

    fn focus_changed_event(seq: u64) -> ImeEventEnvelope {
        envelope(
            seq,
            ImeEvent::FocusChanged {
                from: None,
                to: HwndId::NULL,
                profile: ImePolicyProfile::ImmCross,
                focus_epoch: seq,
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
                source: UserIntentSource::PhysicalImeKey,
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

    // ── ConvModeOwnershipChanged (H-3-e) ─────────────────────────────────────

    // 初期状態は Unknown。
    #[test]
    fn conv_mode_authority_starts_unknown() {
        let model = ImeModel::new();
        assert_eq!(
            model.conv_mode_authority(),
            ConvModeAuthority::Unknown,
            "初期状態は Unknown"
        );
    }

    // EngineStateChanged ON → AwaseOwned に遷移する。
    #[test]
    fn conv_mode_authority_engine_on_sets_awase_owned() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::ConvModeOwnershipChanged {
                authority: ConvModeAuthority::AwaseOwned,
            },
        ));
        assert_eq!(model.conv_mode_authority(), ConvModeAuthority::AwaseOwned);
        assert!(
            model.conv_mode_authority().allows_conv_mutation(),
            "AwaseOwned は conv mutation を許可する"
        );
    }

    // EngineStateChanged OFF → UserOwned に遷移し conv mutation が禁止される。
    #[test]
    fn conv_mode_authority_engine_off_sets_user_owned() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::ConvModeOwnershipChanged {
                authority: ConvModeAuthority::AwaseOwned,
            },
        ));
        model.reduce(&envelope(
            2,
            ImeEvent::ConvModeOwnershipChanged {
                authority: ConvModeAuthority::UserOwned,
            },
        ));
        assert_eq!(model.conv_mode_authority(), ConvModeAuthority::UserOwned);
        assert!(
            !model.conv_mode_authority().allows_conv_mutation(),
            "UserOwned は conv mutation を禁止する"
        );
    }

    // TemporarilyUnowned も conv mutation を禁止する。
    #[test]
    fn conv_mode_authority_temporarily_unowned_forbids_mutation() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::ConvModeOwnershipChanged {
                authority: ConvModeAuthority::TemporarilyUnowned,
            },
        ));
        assert_eq!(
            model.conv_mode_authority(),
            ConvModeAuthority::TemporarilyUnowned
        );
        assert!(
            !model.conv_mode_authority().allows_conv_mutation(),
            "TemporarilyUnowned は conv mutation を禁止する"
        );
    }

    // ── PanicReset ────────────────────────────────────────────────────────────

    #[test]
    fn panic_reset_sets_desired_open() {
        let mut model = ImeModel::new(); // desired_open = true
        model.reduce(&envelope(1, ImeEvent::PanicReset { target: true }));
        assert!(model.desired_open, "PanicReset は desired_open を target に設定する");
    }

    // 最重要: PanicReset は last_intent を設定しない。
    // これが UserImeSetIntent との本質的な差異。
    // last_intent が None のままなので has_user_explicit_intent() = false となり、
    // 後続の実観測が effective_open() を上書きできる。
    #[test]
    fn panic_reset_does_not_set_last_intent() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(1, ImeEvent::PanicReset { target: true }));
        assert!(
            model.last_intent.is_none(),
            "PanicReset は last_intent を設定しない（ForceGuard に委ねる）"
        );
    }

    // PanicReset 後は has_user_explicit_intent() が false のため、
    // Medium+ の実観測が effective_open() を上書きできることを確認。
    #[test]
    fn panic_reset_allows_observation_to_override_effective_open() {
        let mut model = ImeModel::new();
        // PanicReset で desired_open=true に戻す
        model.reduce(&envelope(1, ImeEvent::PanicReset { target: true }));
        assert!(model.desired_open);
        // Medium 観測が false を報告
        model.reduce(&envelope(
            2,
            ImeEvent::ObserverReported {
                open: false,
                source: ObservationSource::ObserverPoll,
                hwnd: HwndId::NULL,
                confidence: ObservationConfidence::Medium,
                focus_epoch: 0,
            },
        ));
        assert!(
            !model.effective_open(),
            "PanicReset 後は explicit intent がないため、Medium 観測が effective_open を上書きする"
        );
        assert!(
            model.desired_open,
            "desired_open は PanicReset の値 (true) のまま変わらない"
        );
    }

    // PanicReset ≠ UserImeSetIntent の対比：UserImeSetIntent は観測で上書きされない。
    #[test]
    fn user_intent_blocks_observation_unlike_panic_reset() {
        let mut model = ImeModel::new();
        // ユーザーが明示的に IME ON に設定した
        model.reduce(&envelope(
            1,
            ImeEvent::UserImeSetIntent {
                target: true,
                source: UserIntentSource::PhysicalImeKey,
            },
        ));
        // Medium 観測が false を報告（PanicReset とは違い上書きされない）
        model.reduce(&envelope(
            2,
            ImeEvent::ObserverReported {
                open: false,
                source: ObservationSource::ObserverPoll,
                hwnd: HwndId::NULL,
                confidence: ObservationConfidence::Medium,
                focus_epoch: 0,
            },
        ));
        assert!(
            model.effective_open(),
            "UserImeSetIntent 後は explicit intent があるため、観測は effective_open を上書きしない"
        );
    }

    // ── HwndCacheRestored ─────────────────────────────────────────────────────

    #[test]
    fn hwnd_cache_restored_sets_desired_open() {
        let mut model = ImeModel::new(); // desired_open = true
        model.reduce(&envelope(1, ImeEvent::HwndCacheRestored { target: false }));
        assert!(!model.desired_open, "HwndCacheRestored は desired_open を target に設定する");
    }

    // 最重要: HwndCacheRestored は last_intent を設定しない。
    // キャッシュ復元はユーザーの能動的操作ではないため、
    // has_user_explicit_intent() を true にしてはならない。
    #[test]
    fn hwnd_cache_restored_does_not_set_last_intent() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(1, ImeEvent::HwndCacheRestored { target: false }));
        assert!(
            model.last_intent.is_none(),
            "HwndCacheRestored は last_intent を設定しない（後続の実観測で上書き可能）"
        );
    }

    // HwndCacheRestored 後は has_user_explicit_intent() が false のため、
    // Medium+ の実観測が effective_open() を上書きできることを確認。
    // これが PanicReset と同じ「非意図 desired 書き換え」の設計。
    #[test]
    fn hwnd_cache_restored_allows_observation_to_override_effective_open() {
        let mut model = ImeModel::new();
        // キャッシュから desired_open=false を復元
        model.reduce(&envelope(1, ImeEvent::HwndCacheRestored { target: false }));
        assert!(!model.desired_open);
        // 実際の API 観測が true を返す（実 IME 状態は ON）
        model.reduce(&envelope(
            2,
            ImeEvent::ObserverReported {
                open: true,
                source: ObservationSource::ImmGetOpenStatus,
                hwnd: HwndId::NULL,
                confidence: ObservationConfidence::High,
                focus_epoch: 0,
            },
        ));
        assert!(
            model.effective_open(),
            "HwndCacheRestored 後は explicit intent がないため、High 観測が effective_open を上書きする"
        );
        assert!(
            !model.desired_open,
            "desired_open はキャッシュの復元値 (false) のまま変わらない"
        );
    }

    // HwndCacheRestored ≠ UserImeSetIntent の対比：
    // UserImeSetIntent は観測で effective_open が変わらないが、
    // HwndCacheRestored はキャッシュ起源なので観測で上書きされる。
    #[test]
    fn user_intent_blocks_observation_but_hwnd_cache_does_not() {
        // UserImeSetIntent の場合
        let mut model_intent = ImeModel::new();
        model_intent.reduce(&envelope(
            1,
            ImeEvent::UserImeSetIntent {
                target: false,
                source: UserIntentSource::SyncKey,
            },
        ));
        model_intent.reduce(&envelope(
            2,
            ImeEvent::ObserverReported {
                open: true,
                source: ObservationSource::ImmGetOpenStatus,
                hwnd: HwndId::NULL,
                confidence: ObservationConfidence::High,
                focus_epoch: 0,
            },
        ));
        assert!(
            !model_intent.effective_open(),
            "UserImeSetIntent 後は explicit intent が High 観測を遮断する"
        );

        // HwndCacheRestored の場合（同じ操作）
        let mut model_cache = ImeModel::new();
        model_cache.reduce(&envelope(1, ImeEvent::HwndCacheRestored { target: false }));
        model_cache.reduce(&envelope(
            2,
            ImeEvent::ObserverReported {
                open: true,
                source: ObservationSource::ImmGetOpenStatus,
                hwnd: HwndId::NULL,
                confidence: ObservationConfidence::High,
                focus_epoch: 0,
            },
        ));
        assert!(
            model_cache.effective_open(),
            "HwndCacheRestored 後は explicit intent がなく、High 観測が通過する"
        );
    }

    // InputModeApplied のテスト

    #[test]
    fn input_mode_applied_updates_input_mode() {
        let mut model = ImeModel::new();
        // 初期状態は ObservedRomaji
        assert_eq!(model.input_mode(), InputModeState::ObservedRomaji);

        model.reduce(&envelope(
            1,
            ImeEvent::InputModeApplied {
                mode: InputModeState::ObservedEisu,
                strategy: crate::state::ime_event::InputModeApplyStrategy::ImmBrokenCorrection,
                result: InputModeApplyResult::Applied,
                at: crate::state::TickMs(0),
            },
        ));
        assert_eq!(
            model.input_mode(),
            InputModeState::ObservedEisu,
            "InputModeApplied(Applied) は input_mode を更新する"
        );
    }

    #[test]
    fn input_mode_applied_skipped_does_not_update_input_mode() {
        let mut model = ImeModel::new();
        // 初期状態は ObservedRomaji
        assert_eq!(model.input_mode(), InputModeState::ObservedRomaji);

        model.reduce(&envelope(
            1,
            ImeEvent::InputModeApplied {
                mode: InputModeState::ObservedEisu,
                strategy: crate::state::ime_event::InputModeApplyStrategy::ImmBrokenCorrection,
                result: InputModeApplyResult::Skipped,
                at: crate::state::TickMs(0),
            },
        ));
        assert_eq!(
            model.input_mode(),
            InputModeState::ObservedRomaji,
            "InputModeApplied(Skipped) は input_mode を変更しない"
        );
    }
}
