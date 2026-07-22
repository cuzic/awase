use crate::focus::{AppKind, FocusKind};
use awase::engine::InputModeState;

use super::belief::ImeBelief;
use super::force_guard::{ForceGuard, ForceOnReason};
use super::hook_state::SyncKeyGate;
use super::ime_event::{
    ChordKind, HwndId, ImeEvent, ImeEventEnvelope, InputModeApplyResult, InputModeApplyStrategy,
    ObservationConfidence, ObservationSource, UserIntentSource,
};
use super::ime_event_log::ImeEventLog;
use super::ime_model::ImeModel;
use super::input_barrier::InputBarrier;
use super::TickMs;
use crate::journal::{JournalEntry, UnifiedJournal};

// ────────────────────────────────────────────────────────────────────────────
// ImeStateHub
// ────────────────────────────────────────────────────────────────────────────

/// IME 観測・判断を担う凝集ユニット。
///
/// `PlatformState` から IME 関連フィールドを切り出すことで、
/// 「観測」「フォーカス状態」「フック設定」の混在を解消する。
///
/// - `belief`        : input_mode / is_japanese_ime / prev_conversion_mode（IME ON/OFF 自体は shadow_model が SSOT）
/// - `shadow_model`  : IME ON/OFF と force_guards / observe_miss_monitor を持つ SSOT
#[derive(Debug)]
pub(crate) struct ImeStateHub {
    /// input_mode・is_japanese_ime・prev_conversion_mode を保持する。
    pub(crate) belief: ImeBelief,
    /// IME 状態変更 event のリングバッファ (Step 0)。
    pub(crate) event_log: ImeEventLog,
    /// 統合ジャーナル: エンジン + IME 両イベントを記録する。
    pub(crate) journal: UnifiedJournal,

    /// Shadow IME モデル (Step 1)。Phase 3a で recovery 統合済。
    /// IME ON/OFF (desired_open / applied_open) と force_guards / observe_miss_monitor を持つ SSOT。
    shadow_model: ImeModel,

    /// ユーザーが明示的に IME OFF にした最終時刻 (tick_ms)。
    ///
    /// `FocusChanged` でクリアされない永続フィールド。複数の rapid focus 変化が連続する
    /// 場合（仮想デスクトップ切替等）でも、最初のフォーカス変化後に `last_intent` が
    /// クリアされても guard が機能し続けるようにする。
    ///
    /// - SyncKey / PhysicalImeKey による `target=false` で更新。
    /// - SyncKey / PhysicalImeKey による `target=true` でリセット。
    /// - FocusChanged / Recovery / HwndCache ではリセットしない。
    last_user_explicit_off_ms: u64,

    /// エンジンが明示的 IME ON/OFF を適用した最終時刻 (tick_ms)。0 = 未操作。
    ///
    /// `handle_engine_set_open` が実際に apply を実行したときに更新される。
    /// idle-conv-check が明示的 IME 操作直後に belief を上書きしないよう
    /// `EXPLICIT_IME_SUPPRESS_MS` の間スキップするために参照する。
    last_explicit_ime_action_ms: u64,
}

/// [`ImeStateHub::capture_poll_state`] で取得する IME ポーリング入力スナップショット。
///
/// `poll_and_classify_ime` / `classify_fetched_snapshot` の 4 引数をひとつにまとめることで
/// `ir_poll_and_learn` 内の同一フィールド二重読み取りを解消する。
#[derive(Clone, Copy)]
pub(crate) struct ImePollState {
    pub(crate) ime_on: bool,
    pub(crate) force_guard: bool,
    pub(crate) input_mode: InputModeState,
    pub(crate) prev_conv: Option<u32>,
}

impl ImeStateHub {
    /// デフォルト値で初期化する。
    pub(crate) fn new() -> Self {
        Self {
            belief: ImeBelief::default(),
            event_log: ImeEventLog::default(),
            journal: UnifiedJournal::default(),
            shadow_model: ImeModel::default(),
            last_user_explicit_off_ms: 0,
            last_explicit_ime_action_ms: 0,
        }
    }
}

impl ImeStateHub {
    /// Event を log に記録し、shadow_model にも reduce する (Step 1)。
    ///
    /// `event_log.record()` だけを呼ぶより、こちらを使うと record + reduce が
    /// 同一 envelope で進む。write_* メソッドはこちらを使う。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    /// state/ 層が `hook::current_tick_ms()` を直接呼ばないよう注入する。
    pub(crate) fn dispatch_event(&mut self, event: ImeEvent, tick_ms: TickMs) {
        // ユーザー明示の IME OFF/ON を永続タイムスタンプに反映する。
        // FocusChanged で last_intent がクリアされても guard が機能し続けるよう、
        // ImeStateHub 側で独自に保持する。
        if let ImeEvent::UserImeSetIntent { target, source } = &event {
            if matches!(
                source,
                UserIntentSource::SyncKey | UserIntentSource::PhysicalImeKey
            ) {
                if *target {
                    self.last_user_explicit_off_ms = 0;
                } else {
                    self.last_user_explicit_off_ms = tick_ms.0;
                }
            }
        }

        let description = format!("{event:?}");
        let event_for_reduce = event.clone();
        let time = self.event_log.record(event, tick_ms);
        let envelope = ImeEventEnvelope {
            time,
            event: event_for_reduce,
        };
        self.shadow_model.reduce(&envelope);
        self.journal.record(JournalEntry::ImeEvent { description });
    }

    /// shadow_model から派生した最新の explicit intent。
    ///
    /// (Step 2B 以降の SSOT。Priority 4-5 observer による上書きを block する根拠。)
    pub(crate) fn explicit_intent(&self) -> Option<bool> {
        self.shadow_model.last_intent.as_ref().map(|i| i.target)
    }

    /// applied_open / applied_at_ms を更新する（apply 完了時の SSOT 更新）。
    ///
    /// ImeModel アクセス可能なサイトで `set_ime_apply_latch` の代わりに呼ぶ。
    /// executor 内部 (PlatformState 非アクセス) は ImeApplySucceeded event 経由で更新される。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    pub(crate) fn mirror_applied_open(&mut self, value: bool, tick_ms: TickMs) {
        self.mirror_applied_open_with_ts(value, tick_ms.0);
    }

    /// `applied` を指定タイムスタンプで更新する。
    ///
    /// `ts = 0` → `Optimistic`（ImmCross async 送信直後など、楽観的未確認）
    /// `ts > 0` → `Confirmed`（実 apply 完了後）
    pub(crate) fn mirror_applied_open_with_ts(&mut self, value: bool, ts: u64) {
        use crate::state::ime_model::AppliedImeState;
        self.shadow_model.applied = if ts == 0 {
            AppliedImeState::Optimistic(value)
        } else {
            AppliedImeState::Confirmed {
                open: value,
                at_ms: ts,
            }
        };
        // 同じ apply が完了した扱いなので pending も clear
        if let Some(p) = &self.shadow_model.pending {
            if p.target == value {
                self.shadow_model.pending = None;
            }
        }
    }

    // ── Chord barrier ──

    pub(crate) const fn is_ctrl_ime_chord_active(&self) -> bool {
        self.shadow_model.is_ctrl_ime_chord_active()
    }

    pub(crate) fn active_chord_kind(&self) -> Option<ChordKind> {
        self.shadow_model.active_chord_kind()
    }

    /// Engine が SetOpen を要求したときの chord-aware 処理を一元化するメソッド。
    ///
    /// chord active + IME OFF の組み合わせは「chord transaction 中の二次要求」として
    /// フィルタする（write_set_open_request と ImeApplyRequested の両方をスキップ）。
    /// パイプラインがコード状態を直接参照しなくて済むよう、判断をここに集約する。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    ///
    /// 戻り値: apply 要求が実行されたか（ログ用）
    ///
    /// `focus_transition_was_pending`: この event の処理開始時点（`kp_stage_focus_probe`
    /// が barrier を consume する前）で FocusTransition barrier が settle 期間内だったか。
    /// 呼び出し元はこの値を event 処理の先頭でスナップショットして渡すこと
    /// （本関数の呼び出し時点で `is_focus_transition_settling` を評価しても、既に
    /// consume 済みで false になっているため無意味）。
    pub(crate) fn handle_engine_set_open(
        &mut self,
        target: bool,
        ctrl_held: bool,
        focus_transition_was_pending: bool,
        generation: u64,
        tick_ms: TickMs,
    ) -> bool {
        if self.is_ctrl_ime_chord_active() && !target {
            // chord transaction 中の二次 IME OFF 要求: フィルタ。
            // ChordEnded（Ctrl KeyUp）が barrier を解除するため、ここでは何もしない。
            return false;
        }
        if focus_transition_was_pending {
            // belief 保護の最終防衛線（P3-1: 3→2 集約）。
            //
            // 一次フィルタは decision からの SetOpen effect 除去
            // （`runtime::executor::strip_ime_set_open_if_settling`。キーボード経路 =
            // key_pipeline::kp_run_inner と非キーボード経路 = execute_from_loop の両方から呼ぶ）。
            // ここは意図が異なり（decision 除去 ≠ belief 汚染防止）、万一その一次フィルタを
            // すり抜けた SetOpen 要求が belief（desired_open 等）を書き換えるのを防ぐ二重化。
            //
            // フォーカス遷移直後（settle_until 未経過）は、Alt+Tab 等の高速な多重フォーカス遷移で
            // 中間ウィンドウ（Alt+Tab スイッチャー等）の未確定 belief に基づき Engine が SetOpen を
            // 発行し得る（2026-07-05 実機ログで確認）。barrier consume 時に kick される非同期
            // focus probe が観測を更新すれば、次の入力イベントで正しい SetOpen が再発行され自己修復する。
            log::debug!(
                "[focus-settle] SetOpen({target}) request filtered at belief last line of defense \
                 (focus transition barrier still settling at event start)"
            );
            return false;
        }
        self.write_set_open_request(target, tick_ms);
        self.on_set_open_requested();
        self.dispatch_event(
            ImeEvent::ImeApplyRequested {
                target,
                generation,
                ctrl_held,
            },
            tick_ms,
        );
        self.last_explicit_ime_action_ms = tick_ms.0;
        true
    }

    /// Ctrl 系 KeyUp で chord barrier を解除する。
    ///
    /// パイプラインが chord 状態を直接参照しなくて済むよう、
    /// is_ctrl_ime_chord_active / active_chord_kind の参照をここに集約する。
    /// 呼び出し元は `crate::vk::is_ctrl_variant` チェック後に呼ぶこと。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    pub(crate) fn on_ctrl_key_up(&mut self, vk: awase::types::VkCode, tick_ms: TickMs) {
        if !self.is_ctrl_ime_chord_active() {
            return;
        }
        let kind = self
            .active_chord_kind()
            .unwrap_or(ChordKind::CtrlMuhenkanImeOff);
        self.dispatch_event(ImeEvent::ChordEnded { kind }, tick_ms);
        log::debug!("[ctrl-bypass] chord barrier cleared (Ctrl KeyUp vk=0x{vk:02X})");
    }

    // ── Input barrier ──

    /// フォーカス遷移 barrier が pending なら消費して true を返す。
    pub(crate) fn consume_focus_barrier(&mut self) -> bool {
        if self.shadow_model.is_focus_transition_pending() {
            self.shadow_model.input_barrier = None;
            true
        } else {
            false
        }
    }

    /// input_barrier を無条件クリアする（panic reset・フォーカス変更確定等）。
    pub(crate) const fn clear_input_barrier(&mut self) {
        self.shadow_model.input_barrier = None;
    }

    /// FocusTransition barrier が未設定なら設定する。
    pub(crate) fn try_set_focus_transition_barrier(
        &mut self,
        to_hwnd: HwndId,
        started_at: std::time::Instant,
    ) {
        if self.shadow_model.input_barrier.is_none() {
            let settle = self.shadow_model.app_policy.focus_settle_ms;
            self.shadow_model.input_barrier = Some(InputBarrier::FocusTransition {
                to_hwnd,
                started_seq: self.event_log.next_seq(),
                started_at,
                settle_until: started_at + std::time::Duration::from_millis(settle),
            });
        }
    }

    // ── Explicit intent timing ──

    /// 直近の明示的 IME 操作からの経過 ms。
    ///
    /// 未操作の場合は `u64::MAX` を返す。
    /// `EXPLICIT_IME_SUPPRESS_MS` との比較で idle-conv-check を抑制するために使う。
    ///
    /// `now_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    /// idle-conv-check 抑止用に「明示的 IME 操作」時刻を記録する。
    ///
    /// `handle_engine_set_open` 以外の能動的 IME 書き込み（Shift 解放時の conv 復元等）
    /// から呼ぶ。`EXPLICIT_IME_SUPPRESS_MS` の間 idle-conv-check がスキップされる。
    pub(crate) fn note_explicit_ime_action(&mut self, tick_ms: TickMs) {
        self.last_explicit_ime_action_ms = tick_ms.0;
    }

    pub(crate) fn explicit_ime_action_age_ms(&self, now_ms: TickMs) -> u64 {
        if self.last_explicit_ime_action_ms == 0 {
            return u64::MAX;
        }
        now_ms.saturating_sub(self.last_explicit_ime_action_ms)
    }

    /// フォーカス変化をまたいで持続するユーザー明示 IME OFF タイムスタンプ。
    ///
    /// `last_explicit_off_ms()` は `FocusChanged` で `last_intent` がクリアされると 0 に
    /// 戻るため、複数の rapid focus 変化（仮想デスクトップ切替等）では 2 回目以降の
    /// guard が機能しない。このメソッドは SyncKey / PhysicalImeKey による明示 OFF のみを
    /// 追跡し、FocusChanged でリセットしない。
    pub(crate) fn persistent_explicit_off_ms(&self) -> u64 {
        self.last_user_explicit_off_ms
    }

    pub(crate) fn effective_open(&self) -> bool {
        self.shadow_model.effective_open()
    }

    /// フォーカス切替直後の settle 期間内（`settle_until` 未経過）かどうか。
    pub(crate) fn is_focus_transition_settling(&self, now: std::time::Instant) -> bool {
        self.shadow_model.is_focus_transition_settling(now)
    }

    pub(crate) fn detect_miss_count(&self) -> u32 {
        self.shadow_model
            .observe_miss_monitor
            .consecutive_miss_count
    }

    pub(crate) fn is_force_on_guard_active(&self) -> bool {
        self.shadow_model.force_guards.requires_on()
    }

    /// 現在の入力モードを返す（SSOT = `shadow_model.input_mode`）。
    ///
    /// H-3-d 以降、`belief.input_mode` は private 化されたため、
    /// 呼び出し元はすべてこのメソッドを使うこと。
    pub(crate) fn input_mode(&self) -> InputModeState {
        self.shadow_model.input_mode()
    }

    /// `poll_and_classify_ime` / `classify_fetched_snapshot` に渡す 4 フィールドを一括取得する。
    ///
    /// `ir_poll_and_learn` で同じ 4 フィールドを 2 回読んでいた重複を解消する。
    pub(crate) fn capture_poll_state(&self) -> ImePollState {
        ImePollState {
            ime_on: self.effective_open(),
            force_guard: self.is_force_on_guard_active(),
            input_mode: self.input_mode(),
            prev_conv: self.belief.prev_conversion_mode(),
        }
    }

    /// `belief.is_japanese_ime() && effective_open()` の複合述語。
    ///
    /// `apply_force_on_for_imm_broken` / `try_force_on_bootstrap` で重複していたガード条件。
    /// `engine.is_user_enabled()` と組み合わせて IME force-ON の前提条件として使う。
    pub(crate) fn is_eligible_for_ime_force_on(&self) -> bool {
        self.belief.is_japanese_ime() && self.effective_open()
    }

    /// 現在のアプリの focus settle 期間（ms、`AppImePolicy` 由来）。
    ///
    /// settle 中にスキップした force-ON の再試行スケジュールに使う。
    pub(crate) fn focus_settle_ms(&self) -> u64 {
        self.shadow_model.app_policy.focus_settle_ms
    }

    /// 次のイベント generation 番号を払い出す。
    ///
    /// 呼び出し元で `self.platform_state.ime.event_log.next_seq()` を直接書かずに
    /// このメソッドを使うこと（3 段チェーンの解消）。
    pub(crate) fn allocate_event_generation(&self) -> u64 {
        self.event_log.next_seq()
    }

    /// IMM-broken アプリで IME-ON が確認されたとき、`input_mode` を補正すべき値を返す。
    ///
    /// `ImeBelief::correction_for_imm_broken` と同じロジックを `shadow_model.input_mode`
    /// に対して適用する（H-3-d で `belief.input_mode` が private 化されたため移譲）。
    pub(crate) fn correction_for_imm_broken(&self) -> Option<InputModeState> {
        use awase::engine::AssumedReason;
        let mode = self.shadow_model.input_mode();
        if mode.is_romaji_capable() || matches!(mode, InputModeState::ObservedEisu) {
            return None;
        }
        Some(InputModeState::AssumedRomaji {
            reason: AssumedReason::ImmBridgeBroken,
        })
    }

    /// `ImeModel` への読み取り専用アクセス。
    ///
    /// 書き込みはすべて `dispatch_event()` 経由とすること。
    pub(crate) fn model(&self) -> &ImeModel {
        &self.shadow_model
    }

    // ── Desired state / drift correction ──

    /// desired ≠ observed ドリフトが補正閾値を超えているか判定し、超えていれば補正情報を返す。
    ///
    /// 戻り値: `Some((desired, observed, duration_ms))` — 補正が必要な場合
    /// `explicit_intent`: `PlatformState::explicit_intent()` の値をそのまま渡す。
    pub(crate) fn check_drift_correction(
        &self,
        now: std::time::Instant,
        explicit_intent: Option<bool>,
    ) -> Option<(bool, bool, u64)> {
        let desired = self.shadow_model.desired_open();

        let dur = self.shadow_model.observations.drift_duration(now)?;
        // last_intent は UserImeSetIntent / UserImeToggleIntent のみが設定する。
        // PanicReset / HwndCacheRestored は設定しないため、is_some() で十分。
        // SyncKey / PhysicalImeKey / Command は全て閾値 0 (即時補正) の対象。
        let is_strong_intent = self.shadow_model.last_intent.is_some();
        let threshold = if explicit_intent == Some(desired) && is_strong_intent {
            0
        } else {
            u128::from(crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS)
        };
        if dur.as_millis() < threshold {
            return None;
        }

        let max_age =
            std::time::Duration::from_millis(crate::tuning::DRIFT_CORRECTION_OBS_MAX_AGE_MS);
        let trusted = self.shadow_model.observations.most_recent_trusted(now)?;
        if trusted.age(now) > max_age {
            return None;
        }
        // ConvOpenInference（conv ビットからの間接推測、KatakanaShadowOff/
        // NativeToggleShadowOff 由来）は、明示的なユーザー意図が一度も無い間は単独で
        // drift correction を発火させない。desired_open のデフォルト値（起動直後等、
        // last_intent が一度も設定されていない状態）を conv 由来の推論だけで
        // actuate すると、ユーザーが望んでもいない ON/OFF の押し付けになりかねない。
        // 明示意図がある場合（BUG-19 再発の本来のシナリオ: ユーザーが OFF にした
        // 直後に conv がまだ native/katakana を示す）はこの gate を素通りし、
        // 既存の `desired`（ユーザーの意図した値）が正しく再適用される。
        if trusted.source == ObservationSource::ConvOpenInference && explicit_intent.is_none() {
            return None;
        }
        if trusted.open == desired {
            return None;
        }

        Some((desired, trusted.open, dur.as_millis() as u64))
    }

    /// IME apply 完了を記録する（C: mirror + D: generation 照合 dispatch）。
    ///
    /// `generation` がある場合は pending transition と一致する完了だけを受理する。
    /// 古い async 完了をここで弾くことで、GJI/Composition 側にも stale な
    /// `SetOpen(false)` 完了を伝播させない。
    ///
    /// 戻り値は、この完了を現在の IME apply として受理したかどうか。
    pub(crate) fn record_ime_apply_result(
        &mut self,
        open: bool,
        outcome: awase::platform::ImeOpenOutcome,
        generation: Option<u64>,
        ts: u64,
    ) -> bool {
        use awase::platform::ImeOpenOutcome;
        if let Some(generation) = generation {
            let pending = self.shadow_model.pending_generation();
            if pending != Some(generation) {
                log::debug!(
                    "[ime-apply] stale completion ignored: target={open} outcome={outcome:?} \
                     generation={generation} pending={pending:?}"
                );
                return false;
            }
        }

        let effective = match outcome {
            ImeOpenOutcome::Applied
            | ImeOpenOutcome::FallbackSent
            | ImeOpenOutcome::AlreadyMatched => open,
            ImeOpenOutcome::Failed => !open,
            ImeOpenOutcome::UnsafeToToggle => unreachable!(),
        };
        self.mirror_applied_open_with_ts(effective, ts);

        if let Some(generation) = generation {
            let event = ImeEvent::from_apply_outcome(open, outcome, generation);
            self.dispatch_event(event, TickMs(ts));
        }
        true
    }
}

// ── IME 操作ロジック ─────────────────────────────────────────────────────────
//
// PlatformState から委譲されるメソッド群。shadow_model / belief / event_log への
// 書き込みはすべてここに集約し、PlatformState からは直接 shadow_model を触らない。

impl ImeStateHub {
    /// `BrokenAppBootstrap` force-on ガードを追加する。
    pub(crate) fn set_force_on_broken_app_bootstrap(&mut self) {
        self.shadow_model.force_guards.add(ForceGuard {
            reason: ForceOnReason::BrokenAppBootstrap,
            expires_at: None,
            generation: self.event_log.next_seq(),
        });
    }

    /// observe_miss_monitor をリセットし、すべての force-on ガードを解除する。
    ///
    /// ユーザー操作（IME トグル・SetOpen 等）で「意図した状態」が確定したときに呼ぶ。
    pub(crate) fn reset_detect_state(&mut self) {
        self.shadow_model.observe_miss_monitor.record_success();
        self.shadow_model.force_guards.guards.clear();
    }

    /// IME トグルが実際に適用されたことを記録する。
    pub(crate) fn on_ime_toggled(&mut self) {
        self.reset_detect_state();
    }

    /// Engine の SetOpen リクエスト直後に呼ぶ。
    pub(crate) fn on_set_open_requested(&mut self) {
        self.reset_detect_state();
    }

    /// panic_reset 向け全面リセット。
    ///
    /// belief・shadow_model を初期化し `PanicReset` force guard を立てる。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    pub(crate) fn apply_panic_reset(&mut self, tick_ms: TickMs) {
        self.dispatch_event(
            ImeEvent::InputModeApplied {
                mode: InputModeState::ObservedRomaji,
                strategy: InputModeApplyStrategy::PanicReset,
                result: InputModeApplyResult::Applied,
                at: tick_ms,
            },
            tick_ms,
        );
        self.belief.is_japanese_ime = true;
        self.belief.prev_conversion_mode = None;
        self.shadow_model.observe_miss_monitor.record_success();
        self.shadow_model.force_guards.guards.clear();
        self.shadow_model.force_guards.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
            expires_at: None,
            generation: self.event_log.next_seq(),
        });
        // PanicReset は desired_open=true に戻すが last_intent を設定しない。
        // ForceGuard::PanicReset が IME ON を保証する。
        self.dispatch_event(ImeEvent::PanicReset { target: true }, tick_ms);
        // panic reset はフォーカスエポックを変えない（同じフォーカスコンテキスト内のリセット）。
        let cur_epoch = self.shadow_model.observations.current_focus_epoch;
        self.shadow_model
            .observations
            .clear_on_focus_change(cur_epoch);
    }

    /// `ImeUpdate` を belief / shadow_model に反映する。
    ///
    /// `observer::ime_observer::poll_and_classify_ime()` の結果を受け取り、
    /// 状態への書き込みをここに集約する。判断ロジックを持たない純粋適用関数。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    pub(crate) fn apply_ime_update(
        &mut self,
        update: &crate::observer::ime_observer::ImeUpdate,
        tick_ms: TickMs,
        accepted: crate::state::probe_admission::AcceptedObservation,
    ) {
        if let Some(is_jp) = update.is_japanese_ime {
            self.belief.is_japanese_ime = is_jp;
        }
        if let Some(obs) = update.observer_poll {
            self.dispatch_event(
                ImeEvent::ObserverReported {
                    open: obs.value,
                    source: ObservationSource::ObserverPoll,
                    hwnd: HwndId::NULL,
                    confidence: ObservationConfidence::Medium,
                    focus_epoch: accepted.focus_epoch,
                },
                tick_ms,
            );
        }
        if update.increment_miss_count {
            self.shadow_model
                .observe_miss_monitor
                .record_miss(std::time::Instant::now());
            let miss = self
                .shadow_model
                .observe_miss_monitor
                .consecutive_miss_count;
            if miss == crate::IME_DETECT_MISS_THRESHOLD {
                log::warn!("IME detection failed {miss} consecutive times, will force IME ON");
            }
        }
        if update.clear_force_on_broken_app_bootstrap {
            self.shadow_model
                .force_guards
                .remove(ForceOnReason::BrokenAppBootstrap);
        }
        if update.clear_force_on_panic_reset {
            self.shadow_model
                .force_guards
                .remove(ForceOnReason::PanicReset);
            self.shadow_model.observe_miss_monitor.record_success();
        }
        if let Some(mode) = update.new_input_mode {
            self.dispatch_event(
                ImeEvent::InputModeObserved {
                    mode,
                    source: ObservationSource::ObserverPoll,
                    confidence: ObservationConfidence::Medium,
                    at: tick_ms,
                },
                tick_ms,
            );
        }
        if let Some(conv) = update.new_prev_conversion_mode {
            self.belief.prev_conversion_mode = Some(conv);
        }
    }

    /// `hwnd_cache` の復元結果を belief / shadow_model に反映する。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    pub(crate) fn apply_hwnd_cache_restore(
        &mut self,
        snapshot: Option<crate::focus::hwnd_cache::HwndImeSnapshot>,
        tick_ms: TickMs,
    ) {
        if let Some(snap) = snapshot {
            // HwndCacheRestored は desired_open を回復するが last_intent を設定しない。
            // キャッシュ復元はユーザーの能動的操作ではなく、後続の実観測で上書き可能。
            self.dispatch_event(
                ImeEvent::HwndCacheRestored {
                    target: snap.ime_on,
                },
                tick_ms,
            );
            // キャッシュされた input_mode が ObservedEisu の場合、生の観測と同じ強さで
            // engine activation を塞がせない（cache_restore_eisu_guard 参照）。
            // 2026-07-09 MS Edge で実発生: Uwp⇔TsfNative フォーカス往復のたびに
            // 131 秒前の ObservedEisu キャッシュが復元され、eisu guard に阻まれて
            // engine が inactive のまま固着し続けた。
            let mode = crate::state::eisu_recovery::cache_restore_eisu_guard(snap.input_mode);
            self.dispatch_event(
                ImeEvent::InputModeApplied {
                    mode,
                    strategy: InputModeApplyStrategy::CacheRestore,
                    result: InputModeApplyResult::Applied,
                    at: tick_ms,
                },
                tick_ms,
            );
        }
    }

    /// Imm32Unavailable (Chrome/Teams 等) 入場時に stale な `desired_open=false` を IME ON へ寄せ直す。
    ///
    /// TsfNative と同様だが、Imm32Unavailable では awase が IME 状態を制御できないため
    /// キャッシュが carry-over で汚染されやすい。キャッシュ値が「ユーザー明示の OFF」に
    /// 由来しない場合にのみ呼ぶこと（呼び出し側が stale 判定を行う）。
    ///
    /// `reset_to_off_for_tsf_native_cache_miss` と同様、これも「観測が何もない」ことを
    /// 根拠にした安全デフォルトの推測にすぎないため `UserImeSetIntent` は使わず
    /// `ObserverReported`（`HeuristicDefault`, Low confidence）として記録する。
    /// `desired_open` は書き換えない。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    pub(crate) fn reset_stale_ime_on_for_imm_broken(&mut self, tick_ms: TickMs) {
        if !self.belief.is_japanese_ime() || self.shadow_model.effective_open() {
            return;
        }
        if let Some(intent) = self.shadow_model.last_intent.as_ref() {
            log::debug!(
                "Imm32Unavailable entry: preserving ime_on=false (intent source={:?})",
                intent.source
            );
            return;
        }
        log::info!(
            "Imm32Unavailable entry without trusted cache: 安全デフォルト ON を Low confidence \
             observation として記録 (no explicit intent, Japanese layout, IME state \
             uncontrollable in Imm32Unavailable)"
        );
        let focus_epoch = self.shadow_model.observations.current_focus_epoch;
        self.dispatch_event(
            ImeEvent::ObserverReported {
                open: true,
                source: ObservationSource::HeuristicDefault,
                hwnd: HwndId::NULL,
                confidence: ObservationConfidence::Low,
                focus_epoch,
            },
            tick_ms,
        );
    }

    pub(crate) fn set_is_japanese_ime(&mut self, value: bool) {
        self.belief.is_japanese_ime = value;
    }

    pub(crate) fn set_prev_conversion_mode(&mut self, value: Option<u32>) {
        self.belief.prev_conversion_mode = value;
    }

    // ── イベント dispatch ヘルパ ──

    pub(crate) fn write_observer_poll(
        &mut self,
        value: bool,
        tick_ms: TickMs,
        accepted: crate::state::probe_admission::AcceptedObservation,
    ) {
        self.dispatch_event(
            ImeEvent::ObserverReported {
                open: value,
                source: ObservationSource::ObserverPoll,
                hwnd: HwndId::NULL,
                confidence: ObservationConfidence::Medium,
                focus_epoch: accepted.focus_epoch,
            },
            tick_ms,
        );
    }

    pub(crate) fn write_sync_key(&mut self, value: bool, tick_ms: TickMs) {
        self.dispatch_event(
            ImeEvent::UserImeSetIntent {
                target: value,
                source: UserIntentSource::SyncKey,
            },
            tick_ms,
        );
    }

    pub(crate) fn write_physical_key(&mut self, value: bool, tick_ms: TickMs) {
        self.dispatch_event(
            ImeEvent::UserImeSetIntent {
                target: value,
                source: UserIntentSource::PhysicalImeKey,
            },
            tick_ms,
        );
    }

    pub(crate) fn write_set_open_request(&mut self, value: bool, tick_ms: TickMs) {
        self.dispatch_event(
            ImeEvent::UserImeSetIntent {
                target: value,
                source: UserIntentSource::Command,
            },
            tick_ms,
        );
    }

    pub(crate) fn write_focus_probe(
        &mut self,
        value: bool,
        tick_ms: TickMs,
        accepted: crate::state::probe_admission::AcceptedObservation,
    ) {
        self.dispatch_event(
            ImeEvent::ObserverReported {
                open: value,
                source: ObservationSource::FocusProbe,
                hwnd: HwndId::NULL,
                // Low: top-level hwnd の IMC を読むため Qt/GJI 等では child hwnd と異なる場合がある。
                // High confidence の ImmCrossProbe が後から上書きする。
                confidence: ObservationConfidence::Low,
                focus_epoch: accepted.focus_epoch,
            },
            tick_ms,
        );
    }

    /// ImmCross 非同期プローブ結果を記録する（High confidence）。
    ///
    /// `read_ime_state_full_async` が child hwnd の IMM32 状態を読んだ後に呼ぶ。
    /// High confidence のため `derive_open()` で即採用される。
    /// `accepted` は `ImmLikeTicket::admit()` が返した `AcceptedObservation`（epoch 照合済み）。
    pub(crate) fn write_imm_cross_probe(
        &mut self,
        value: bool,
        tick_ms: TickMs,
        accepted: crate::state::probe_admission::AcceptedObservation,
    ) {
        self.dispatch_event(
            ImeEvent::ObserverReported {
                open: value,
                source: ObservationSource::ImmCrossProbe,
                hwnd: HwndId::NULL,
                confidence: ObservationConfidence::High,
                focus_epoch: accepted.focus_epoch,
            },
            tick_ms,
        );
    }

    /// idle-conv-check の conv ビット推論から得た IME open 状態を観測として記録する
    /// (`KatakanaShadowOff` / `NativeToggleShadowOff`、`conv_classify::EngineSync::
    /// ReportOpenInference` 経由)。
    ///
    /// `desired_open` を直接書き換えない — `ObserverReported` として `observations`
    /// に記録するだけにとどめ、実際に補正が必要かどうかの判断は既存の drift
    /// correction 経路 (`check_drift_correction`) に委ねる。かつては
    /// `handle_engine_set_open(true)` を直接呼び `UserImeSetIntent{Command}` を偽装して
    /// `desired_open` を上書きしていたため、ユーザーの明示 OFF 直後でも engine が
    /// 勝手に ON へ戻る再発バグを起こした（2026-07-08, BUG-19 再発）。
    ///
    /// conv 由来の open 推論は間接観測（`ImmGetConversionStatus` の conv ビットから
    /// 「native/katakana ならおそらく open」と推測しているだけで、`ImmGetOpenStatus`
    /// を直接呼んでいるわけではない）のため confidence は `Medium` を上限とする
    /// (`GjiIoInference` と同じ「間接観測」区分)。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻。
    pub(crate) fn report_conv_open_inference(
        &mut self,
        open: bool,
        reason: crate::state::conv_classify::ConvSyncReason,
        tick_ms: TickMs,
    ) {
        log::debug!("[conv-open-inference] reason={reason:?} open={open}");
        let focus_epoch = self.shadow_model.observations.current_focus_epoch;
        self.dispatch_event(
            ImeEvent::ObserverReported {
                open,
                source: ObservationSource::ConvOpenInference,
                hwnd: HwndId::NULL,
                confidence: ObservationConfidence::Medium,
                focus_epoch,
            },
            tick_ms,
        );
    }
}

#[cfg(test)]
impl ImeStateHub {
    pub(crate) fn set_desired_open_for_test(&mut self, value: bool) {
        self.shadow_model.set_desired_open_for_test(value);
    }

    pub(crate) fn clear_last_intent_for_test(&mut self) {
        self.shadow_model.last_intent = None;
    }

    /// 現在呼び出し元がないが診断用アクセサとして残す。
    #[allow(dead_code)]
    pub(crate) fn last_intent_source(&self) -> Option<UserIntentSource> {
        self.shadow_model.last_intent.as_ref().map(|i| i.source)
    }
}

// ────────────────────────────────────────────────────────────────────────────
// FocusStore
// ────────────────────────────────────────────────────────────────────────────

/// フォーカスメタデータを集約する sub-struct。
///
/// `PlatformState` の Facade から内部委譲される。親を参照しない。
#[derive(Debug)]
pub(crate) struct FocusStore {
    pub app_kind: AppKind,
    pub focus_kind: FocusKind,
    /// 最後にフォアグラウンドプロセスが変わった時刻（ms, GetTickCount 系）。
    /// IME 診断ログで「フォーカス変更からの経過時間」を表示するために使う。
    pub last_focus_change_ms: u64,
    pub focus_debounce_ms: u32,
    pub ime_poll_interval_ms: u32,
    /// フォーカスプロセス変更のエポック番号。
    ///
    /// `on_focus_process_changed` のたびに `wrapping_add(1)` でインクリメントされる。
    /// probe の spawn 時にキャプチャし、完了時に照合することで「spawn 後にフォーカスが
    /// 変わったか」を時間ベースの競合なしに正確に判定できる（→ probe_admission モジュール）。
    pub focus_epoch: u64,
}

impl FocusStore {
    pub(crate) fn new() -> Self {
        Self {
            app_kind: AppKind::Win32,
            focus_kind: FocusKind::Undetermined,
            last_focus_change_ms: 0,
            focus_debounce_ms: 50,
            ime_poll_interval_ms: 500,
            focus_epoch: 0,
        }
    }
}

impl Default for FocusStore {
    fn default() -> Self {
        Self::new()
    }
}

// ────────────────────────────────────────────────────────────────────────────
// GateStore
// ────────────────────────────────────────────────────────────────────────────

/// フックゲート・バイパス関連状態を集約する sub-struct。
///
/// `PlatformState` の Facade から内部委譲される。親を参照しない。
#[derive(Debug)]
pub(crate) struct GateStore {
    pub last_hook_activity_ms: u64,
    /// Ctrl+key bypass 直後フラグ。
    ///
    /// Ctrl+非修飾キーが PassThrough として素通りした後、次の非修飾 non-Ctrl キー 1 つを
    /// NICOLA エンジンをスキップして直接 passthrough させる。
    /// tmux prefix (Ctrl+J) → コマンドキー (n/p) のように、
    /// prefix 直後のコマンドキーが NICOLA に横取りされる問題を防ぐ。
    pub post_bypass_passthrough: bool,
    /// IME 同期キー直後のキー保留バッファ（旧 `ime_gate`）。
    pub sync_key_gate: SyncKeyGate,
    /// 今回の左Shift downが単独タップ候補か（`kp_stage_shift_conv_guard`）。
    ///
    /// 左Shift KeyDownでtrueにセットし、Shift保持中に`VK_LSHIFT`/`VK_RSHIFT`以外の
    /// 非注入物理KeyDownが来たらfalseに倒す（チョード判定）。左Shift KeyUp時に
    /// これがtrueのままなら「本物の単独タップ」として半角英数トグルの対象にする。
    pub left_shift_tap_candidate: bool,
    /// 今回のShift downに対応する復元処理が必要か（`kp_stage_shift_conv_guard`）。
    ///
    /// Shift KeyDownで awase が conv=0x00000000（IME-ON 半角英数）へ切り替えたとき
    /// true。Shift KeyUpで`std::mem::take`し、trueならKeyUp側の復元/トグル判定を
    /// 走らせる。**`half_width_alnum_toggle_active`とは独立**（トグルON中の
    /// Shift downでも必ずtrueにする——立てないとKeyUp側でトグルOFF/右Shift緊急解除が
    /// 発火しなくなる、2026-07-11 codexレビューで発覚）。
    pub shift_conv_guard_pending: bool,
    /// 左Shift単独タップによる「IME-ON半角英数」持続トグルが有効か。
    ///
    /// `shift_conv_guard_pending`と違い、Shift keyup後も左Shiftの次の単独タップ
    /// （または右Shiftタップ/フォーカス変更による緊急解除）まで true であり続ける。
    /// true の間、`platform_state.ime.input_mode()`はObservedEisuへ誘導され
    /// Engineが`Inactive(NotRomajiInput)`で素通りになる（IMEはbelief上ONのまま）。
    /// idle-conv-check / ime_refresh の OS poll を凍結する（`shift_conv_guard_pending`
    /// と同じ理由: conv=0x0000は awase自身の意図的な状態のため）。
    pub half_width_alnum_toggle_active: bool,
    /// `kp_stage_idle_conv_check` の conv 読み取り（offload 済み、`SendMessageTimeoutW`
    /// ベース）が in-flight かどうか。
    ///
    /// GJI が本当にハングしている間に断続的なタイピングが続くと、idle ゲートを
    /// 通過するたびに新しい offload 呼び出しが積み上がりワーカースレッドが増え続ける。
    /// 1 件 in-flight の間は新規 spawn をスキップし、完了時（epoch 棄却時も含む）に
    /// `with_app` 内で必ず false へ戻す。
    pub idle_conv_check_in_flight: bool,
}

impl GateStore {
    pub(crate) fn new() -> Self {
        Self {
            last_hook_activity_ms: 0,
            post_bypass_passthrough: false,
            sync_key_gate: SyncKeyGate::new(),
            left_shift_tap_candidate: false,
            shift_conv_guard_pending: false,
            half_width_alnum_toggle_active: false,
            idle_conv_check_in_flight: false,
        }
    }
}

impl Default for GateStore {
    fn default() -> Self {
        Self::new()
    }
}

// ────────────────────────────────────────────────────────────────────────────
// KeymapStore
// ────────────────────────────────────────────────────────────────────────────

/// アクティブなキーマップルールを保持する sub-struct。
///
/// `PlatformState` の Facade から内部委譲される。親を参照しない。
#[derive(Debug, Default)]
pub(crate) struct KeymapStore {
    /// 現在のフォーカスアプリに適用されるキーマップルール
    pub active_keymaps: crate::keymap::KeymapTable,
}

// ────────────────────────────────────────────────────────────────────────────
// PlatformState
// ────────────────────────────────────────────────────────────────────────────

/// Platform 層の全状態を集約する Facade 構造体。
///
/// 各ドメインの状態は sub-struct（`FocusStore` / `GateStore` / `KeymapStore`）に委譲する。
/// `ImeStateHub` は IME 観測・判断を担う凝集ユニットとして引き続き `ime` フィールドで保持する。
///
/// シングルスレッド（メインスレッド＋フックコールバック）からのみアクセスされる。
/// `APP: SingleThreadCell<Runtime>` 経由で保持される。
#[derive(Debug)]
pub struct PlatformState {
    /// IME 観測・判断・belief 書き戻しを担う凝集ユニット（ImeStore 相当）。
    pub(crate) ime: ImeStateHub,
    /// フォーカスメタデータ（AppKind / FocusKind / タイムスタンプ / デバウンス設定）。
    pub(crate) focus: FocusStore,
    /// フックゲート・バイパス関連状態（アクティビティタイムスタンプ / post-bypass / sync_key_gate）。
    pub(crate) gate: GateStore,
    /// キーマップルール（フォーカスアプリ別アクティブルール）。
    pub(crate) keymap: KeymapStore,
}

impl PlatformState {
    /// デフォルト値で初期化する
    #[must_use]
    pub fn new() -> Self {
        Self {
            ime: ImeStateHub::new(),
            focus: FocusStore::new(),
            gate: GateStore::new(),
            keymap: KeymapStore::default(),
        }
    }
}

impl Default for PlatformState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// shadow_model を直接設定するヘルパ:
    /// `set_intent=Some(source)` なら UserImeSetIntent を dispatch し last_intent を設定する。
    /// `set_intent=None` なら desired_open のみ直接書き換え、last_intent は空のままにする
    /// (focus 変更後の carry-over シナリオを模擬)。
    fn ps_with_shadow(
        desired_open: bool,
        set_intent: Option<UserIntentSource>,
        is_japanese: bool,
    ) -> PlatformState {
        let mut ps = PlatformState::new();
        ps.ime.belief.is_japanese_ime = is_japanese;
        if let Some(source) = set_intent {
            ps.ime.dispatch_event(
                ImeEvent::UserImeSetIntent {
                    target: desired_open,
                    source,
                },
                TickMs(0),
            );
        } else {
            ps.ime.set_desired_open_for_test(desired_open);
            ps.ime.clear_last_intent_for_test();
        }
        ps
    }

    // reset_stale_ime_on_for_imm_broken も同様に desired_open を書き換えない。
    #[test]
    fn imm_broken_reset_does_not_touch_desired_open() {
        let mut ps = ps_with_shadow(false, None, true);
        ps.ime.reset_stale_ime_on_for_imm_broken(TickMs(0));
        assert!(
            !ps.ime.model().desired_open(),
            "desired_open はユーザーの真の意図のまま変更されない"
        );
        assert!(
            ps.ime.effective_open(),
            "実効値は Low confidence observation 経由で true になる"
        );
    }

    // ── handle_engine_set_open: focus_transition_was_pending フィルタ ──
    //
    // 2026-07-05: Alt+Tab 中の中間ウィンドウ（Alt+Tab スイッチャー等）への一瞬の
    // フォーカスで Engine が SetOpen を発行し、それが最終的な着地先ウィンドウとは
    // 無関係な SendInput として実行され、belief と実IME状態が乖離するバグの修正。

    // focus_transition_was_pending=true の場合、SetOpen 要求はフィルタされ
    // desired_open/last_explicit_ime_action_ms は変化しない。
    #[test]
    fn handle_engine_set_open_filters_when_focus_transition_was_pending() {
        let mut ps = ps_with_shadow(false, Some(UserIntentSource::SyncKey), true);
        let applied = ps
            .ime
            .handle_engine_set_open(true, false, true, 1, TickMs(0));
        assert!(!applied, "focus transition pending 中は適用されない");
        assert!(
            !ps.ime.model().desired_open(),
            "フィルタされた SetOpen は desired_open を書き換えない"
        );
    }

    // focus_transition_was_pending=false なら通常通り適用される（回帰防止）。
    #[test]
    fn handle_engine_set_open_applies_when_focus_transition_not_pending() {
        let mut ps = ps_with_shadow(false, Some(UserIntentSource::SyncKey), true);
        let applied = ps
            .ime
            .handle_engine_set_open(true, false, false, 1, TickMs(0));
        assert!(
            applied,
            "focus transition が pending でなければ通常通り適用される"
        );
        assert!(ps.ime.model().desired_open());
    }

    // 既存の CtrlImeChord フィルタが、focus_transition フィルタ追加後も
    // 引き続き機能することを確認する回帰テスト。
    #[test]
    fn handle_engine_set_open_ctrl_chord_filter_still_works() {
        let mut ps = ps_with_shadow(true, Some(UserIntentSource::SyncKey), true);
        // 1 回目: IME OFF 要求 + Ctrl 押下中 → chord transaction 開始。
        let first = ps
            .ime
            .handle_engine_set_open(false, true, false, 1, TickMs(0));
        assert!(first, "chord を開始する最初の要求は適用される");
        assert!(ps.ime.is_ctrl_ime_chord_active());
        // 2 回目: chord transaction 中の二次 IME OFF 要求 → フィルタされる。
        let second = ps
            .ime
            .handle_engine_set_open(false, true, false, 2, TickMs(0));
        assert!(
            !second,
            "chord transaction 中の二次 IME OFF 要求はフィルタされる"
        );
    }

    // ── report_conv_open_inference / check_drift_correction (BUG-19 再発対策) ──
    //
    // 2026-07-08 実機再発: ユーザーが IME OFF (last_intent=Some(false)) にした
    // 約1.6秒後、conv ビットが native/katakana を示したことを理由に
    // KatakanaShadowOff が UserImeSetIntent{Command} を偽装して desired_open を
    // true に書き換え、engine が勝手に ON へ戻った。修正後は ObserverReported
    // (ConvOpenInference) として記録するだけにとどめ、既存の drift correction が
    // 正しい方向（desired=false の再送）で解決することを、実時間 sleep を使わず
    // （drift.started_at / 観測の at を直接バックデートして）確認する。

    use super::super::observation_store::ImeDrift;
    use crate::state::conv_classify::ConvSyncReason;

    #[test]
    fn report_conv_open_inference_does_not_touch_desired_open_or_last_intent() {
        let mut ps = ps_with_shadow(false, Some(UserIntentSource::PhysicalImeKey), true);
        ps.ime
            .report_conv_open_inference(true, ConvSyncReason::KatakanaShadowOff, TickMs(0));
        assert!(
            !ps.ime.model().desired_open(),
            "conv 由来の open 推論は desired_open を書き換えない"
        );
        assert_eq!(
            ps.ime.explicit_intent(),
            Some(false),
            "last_intent (explicit_intent) も変更されない — ObserverReported は意図を偽装しない"
        );
    }

    // BUG-19 再発の実ログ相当: last_intent=Some(false) (explicit_intent==desired) なので
    // threshold=0 となり、conv の一発観測直後でも正しい方向 (false の再送) が返る。
    #[test]
    fn check_drift_correction_fires_immediately_when_explicit_off_intent_conflicts_with_conv_inference(
    ) {
        let mut ps = ps_with_shadow(false, Some(UserIntentSource::PhysicalImeKey), true);
        ps.ime
            .report_conv_open_inference(true, ConvSyncReason::KatakanaShadowOff, TickMs(0));
        let now = std::time::Instant::now();
        let explicit_intent = ps.ime.explicit_intent();
        match ps.ime.check_drift_correction(now, explicit_intent) {
            Some((desired, observed, _dur_ms)) => {
                assert!(!desired, "desired は false のまま保持されている");
                assert!(observed, "conv 推論が observed=true として記録されている");
            }
            None => panic!(
                "explicit intent が desired と一致する場合は即時 (threshold=0) で \
                 補正が返るべき"
            ),
        }
    }

    // 明示意図が一度も無い（起動直後等）状態では、conv 推論単独で drift correction
    // を発火させない — desired_open のデフォルト値をユーザーの意図なしに actuate
    // してしまうのを防ぐ。
    #[test]
    fn check_drift_correction_ignores_conv_inference_alone_without_explicit_intent() {
        let mut ps = ps_with_shadow(false, None, true);
        ps.ime
            .report_conv_open_inference(true, ConvSyncReason::KatakanaShadowOff, TickMs(0));
        // 明示意図が無いので threshold=DRIFT_CORRECTION_THRESHOLD_MS。実時間 sleep を
        // 避けるため drift.started_at を直接バックデートして閾値超過を模す。
        ps.ime.shadow_model.observations.drift = Some(ImeDrift {
            started_at: std::time::Instant::now()
                - std::time::Duration::from_millis(
                    crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS + 50,
                ),
        });
        let now = std::time::Instant::now();
        let explicit_intent = ps.ime.explicit_intent();
        assert_eq!(explicit_intent, None);
        assert_eq!(
            ps.ime.check_drift_correction(now, explicit_intent),
            None,
            "明示意図なしでは ConvOpenInference 単独で補正を発火させない"
        );
    }

    #[test]
    fn check_drift_correction_none_when_conv_inference_matches_desired() {
        let mut ps = ps_with_shadow(true, Some(UserIntentSource::PhysicalImeKey), true);
        ps.ime
            .report_conv_open_inference(true, ConvSyncReason::KatakanaShadowOff, TickMs(0));
        let now = std::time::Instant::now();
        let explicit_intent = ps.ime.explicit_intent();
        assert_eq!(
            ps.ime.check_drift_correction(now, explicit_intent),
            None,
            "desired と observed が一致していれば補正不要"
        );
    }

    // GJI 候補ポップアップの観測が古くなった場合 (DRIFT_CORRECTION_OBS_MAX_AGE_MS 超過)
    // は、明示意図があっても採用しない（BUG-20 の max_age ガードが ConvOpenInference
    // にも同じく効くことの確認）。
    #[test]
    fn check_drift_correction_ignores_stale_conv_inference_beyond_max_age() {
        let mut ps = ps_with_shadow(false, Some(UserIntentSource::PhysicalImeKey), true);
        ps.ime
            .report_conv_open_inference(true, ConvSyncReason::KatakanaShadowOff, TickMs(0));
        let stale_at = std::time::Instant::now()
            - std::time::Duration::from_millis(
                crate::tuning::DRIFT_CORRECTION_OBS_MAX_AGE_MS + 200,
            );
        ps.ime
            .shadow_model
            .observations
            .per_source
            .conv_open_inference
            .as_mut()
            .unwrap()
            .at = stale_at;
        let now = std::time::Instant::now();
        let explicit_intent = ps.ime.explicit_intent();
        assert_eq!(
            ps.ime.check_drift_correction(now, explicit_intent),
            None,
            "max_age を超えた観測は無視される"
        );
    }
}
