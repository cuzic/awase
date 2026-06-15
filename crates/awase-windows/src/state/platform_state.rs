use awase::engine::InputModeState;
use awase::types::{AppKind, FocusKind};

use super::belief::ImeBelief;
use super::force_guard::{ForceGuard, ForceOnReason};
use super::hook_state::SyncKeyGate;
use super::ime_event::{ChordKind, HwndId, ImeEvent, ImeEventEnvelope, IntentSource};
use super::ime_event_log::ImeEventLog;
use super::ime_model::ImeModel;
use super::input_barrier::InputBarrier;
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
}

impl ImeStateHub {
    /// デフォルト値で初期化する。
    pub(crate) fn new() -> Self {
        Self {
            belief: ImeBelief {
                input_mode: InputModeState::ObservedRomaji, // デフォルト: ローマ字入力
                is_japanese_ime: true,                      // デフォルト: 日本語
                prev_conversion_mode: None,
            },
            event_log: ImeEventLog::default(),
            journal: UnifiedJournal::default(),
            shadow_model: ImeModel::default(),
        }
    }
}

impl ImeStateHub {
    /// Event を log に記録し、shadow_model にも reduce する (Step 1)。
    ///
    /// `event_log.record()` だけを呼ぶより、こちらを使うと record + reduce が
    /// 同一 envelope で進む。write_* メソッドはこちらを使う。
    pub(crate) fn dispatch_event(&mut self, event: ImeEvent) {
        let description = format!("{event:?}");
        let event_for_reduce = event.clone();
        let time = self.event_log.record(event);
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
    pub(crate) fn mirror_applied_open(&mut self, value: bool) {
        self.mirror_applied_open_with_ts(value, crate::hook::current_tick_ms());
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

    /// 最後に明示的 IME-OFF（target=false）を行った時刻 (tick_ms)。
    ///
    /// `last_intent` が `target=false` であればその `at_ms` を返す。
    /// 未設定・target=true の場合は 0 を返す。
    pub(crate) fn last_explicit_off_ms(&self) -> u64 {
        match &self.shadow_model.last_intent {
            Some(i) if !i.target => i.at_ms,
            _ => 0,
        }
    }

    pub(crate) fn effective_open(&self) -> bool {
        self.shadow_model.effective_open()
    }

    pub(crate) fn detect_miss_count(&self) -> u32 {
        self.shadow_model
            .observe_miss_monitor
            .consecutive_miss_count
    }

    pub(crate) fn is_force_on_guard_active(&self) -> bool {
        self.shadow_model.force_guards.requires_on()
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
        let desired = self.shadow_model.desired_open;

        let dur = self.shadow_model.observations.drift_duration(now)?;
        let threshold = if explicit_intent == Some(desired) {
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
        if trusted.open == desired {
            return None;
        }

        Some((desired, trusted.open, dur.as_millis() as u64))
    }

    /// IME apply 完了を記録する（C: mirror + D: generation 照合 dispatch）。
    ///
    /// `mirror_applied_open_with_ts` と `pending_generation` チェックを一体化し、
    /// 呼び出し元が generation を個別に取得する必要をなくす。
    pub(crate) fn record_ime_apply_result(
        &mut self,
        open: bool,
        outcome: awase::platform::ImeOpenOutcome,
        ts: u64,
    ) {
        use awase::platform::ImeOpenOutcome;
        let effective = match outcome {
            ImeOpenOutcome::Applied
            | ImeOpenOutcome::FallbackSent
            | ImeOpenOutcome::AlreadyMatched => open,
            ImeOpenOutcome::Failed => !open,
            ImeOpenOutcome::UnsafeToToggle => unreachable!(),
        };
        self.mirror_applied_open_with_ts(effective, ts);

        if let Some(generation) = self.shadow_model.pending_generation() {
            let event = ImeEvent::from_apply_outcome(open, outcome, generation);
            self.dispatch_event(event);
        }
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
    pub(crate) fn apply_panic_reset(&mut self) {
        self.belief.input_mode = InputModeState::ObservedRomaji;
        self.belief.is_japanese_ime = true;
        self.belief.prev_conversion_mode = None;
        self.shadow_model.observe_miss_monitor.record_success();
        self.shadow_model.force_guards.guards.clear();
        self.shadow_model.force_guards.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
            expires_at: None,
            generation: self.event_log.next_seq(),
        });
        self.dispatch_event(ImeEvent::UserImeSetIntent {
            target: true,
            source: IntentSource::Recovery,
        });
        self.shadow_model.last_intent = None;
        self.shadow_model.observations.clear_on_focus_change();
    }

    /// `ImeUpdate` を belief / shadow_model に反映する。
    ///
    /// `observer::ime_observer::poll_and_classify_ime()` の結果を受け取り、
    /// 状態への書き込みをここに集約する。判断ロジックを持たない純粋適用関数。
    pub(crate) fn apply_ime_update(&mut self, update: &crate::observer::ime_observer::ImeUpdate) {
        if let Some(is_jp) = update.is_japanese_ime {
            self.belief.is_japanese_ime = is_jp;
        }
        if let Some(obs) = update.observer_poll {
            self.dispatch_event(ImeEvent::ObserverReported {
                open: obs.value,
                source: super::ime_event::ObservationSource::ObserverPoll,
                hwnd: HwndId::NULL,
                confidence: super::ime_event::ObservationConfidence::Medium,
            });
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
            self.belief.input_mode = mode;
        }
        if let Some(conv) = update.new_prev_conversion_mode {
            self.belief.prev_conversion_mode = Some(conv);
        }
    }

    /// `hwnd_cache` の復元結果を belief / shadow_model に反映する。
    pub(crate) fn apply_hwnd_cache_restore(
        &mut self,
        snapshot: Option<crate::focus::hwnd_cache::HwndImeSnapshot>,
    ) {
        if let Some(snap) = snapshot {
            self.dispatch_event(ImeEvent::UserImeSetIntent {
                target: snap.ime_on,
                source: IntentSource::HwndCache,
            });
            self.belief.input_mode = snap.input_mode;
        }
    }

    /// TsfNative 入場時に stale な `desired_open=false` を IME ON へ寄せ直す。
    ///
    /// 日本語レイアウトかつ `last_intent` がない（前ウィンドウからの carry-over）
    /// 場合のみ実行する。`last_intent` があれば現フォーカス文脈の意図として保護する。
    pub(crate) fn reset_stale_ime_on_for_tsf_native(&mut self) {
        if !self.belief.is_japanese_ime() || self.shadow_model.effective_open() {
            return;
        }
        if let Some(intent) = self.shadow_model.last_intent.as_ref() {
            log::debug!(
                "TsfNative entry: preserving ime_on=false (intent source={:?})",
                intent.source
            );
            return;
        }
        log::info!(
            "TsfNative entry without cache: reset stale ime_on=false → true \
             (no intent, Japanese layout, IME state untrackable in TSF-native)"
        );
        self.dispatch_event(ImeEvent::UserImeSetIntent {
            target: true,
            source: IntentSource::Recovery,
        });
    }

    /// Imm32Unavailable (Chrome/Teams 等) 入場時に stale な `desired_open=false` を IME ON へ寄せ直す。
    ///
    /// TsfNative と同様だが、Imm32Unavailable では awase が IME 状態を制御できないため
    /// キャッシュが carry-over で汚染されやすい。キャッシュ値が「ユーザー明示の OFF」に
    /// 由来しない場合にのみ呼ぶこと（呼び出し側が stale 判定を行う）。
    pub(crate) fn reset_stale_ime_on_for_imm_broken(&mut self) {
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
            "Imm32Unavailable entry without trusted cache: reset stale ime_on=false → true \
             (no explicit intent, Japanese layout, IME state uncontrollable in Imm32Unavailable)"
        );
        self.dispatch_event(ImeEvent::UserImeSetIntent {
            target: true,
            source: IntentSource::Recovery,
        });
    }

    pub(crate) fn set_is_japanese_ime(&mut self, value: bool) {
        self.belief.is_japanese_ime = value;
    }

    pub(crate) fn set_prev_conversion_mode(&mut self, value: Option<u32>) {
        self.belief.prev_conversion_mode = value;
    }

    // ── イベント dispatch ヘルパ ──

    pub(crate) fn write_observer_poll(&mut self, value: bool) {
        self.dispatch_event(ImeEvent::ObserverReported {
            open: value,
            source: super::ime_event::ObservationSource::ObserverPoll,
            hwnd: HwndId::NULL,
            confidence: super::ime_event::ObservationConfidence::Medium,
        });
    }

    pub(crate) fn write_sync_key(&mut self, value: bool) {
        self.dispatch_event(ImeEvent::UserImeSetIntent {
            target: value,
            source: IntentSource::SyncKey,
        });
    }

    pub(crate) fn write_physical_key(&mut self, value: bool) {
        self.dispatch_event(ImeEvent::UserImeSetIntent {
            target: value,
            source: IntentSource::PhysicalImeKey,
        });
    }

    pub(crate) fn write_set_open_request(&mut self, value: bool) {
        self.dispatch_event(ImeEvent::UserImeSetIntent {
            target: value,
            source: IntentSource::Command,
        });
    }

    pub(crate) fn write_focus_probe(&mut self, value: bool) {
        self.dispatch_event(ImeEvent::ObserverReported {
            open: value,
            source: super::ime_event::ObservationSource::FocusProbe,
            hwnd: HwndId::NULL,
            confidence: super::ime_event::ObservationConfidence::Medium,
        });
    }
}

#[cfg(test)]
impl ImeStateHub {
    pub(crate) fn set_desired_open_for_test(&mut self, value: bool) {
        self.shadow_model.desired_open = value;
    }

    pub(crate) fn clear_last_intent_for_test(&mut self) {
        self.shadow_model.last_intent = None;
    }

    pub(crate) fn last_intent_source(&self) -> Option<IntentSource> {
        self.shadow_model.last_intent.as_ref().map(|i| i.source)
    }
}

// ────────────────────────────────────────────────────────────────────────────
// PlatformState
// ────────────────────────────────────────────────────────────────────────────

/// Platform 層の全状態を集約する構造体。
///
/// シングルスレッド（メインスレッド＋フックコールバック）からのみアクセスされる。
/// `APP: SingleThreadCell<Runtime>` 経由で保持される。
#[derive(Debug)]
pub struct PlatformState {
    /// IME 観測・判断・belief 書き戻しを担う凝集ユニット。
    pub(crate) ime: ImeStateHub,
    // ── フォーカス追跡フィールド（旧 FocusPlatformState から直接展開）──
    pub app_kind: AppKind,
    pub focus_kind: FocusKind,
    /// 最後にフォアグラウンドプロセスが変わった時刻（ms, GetTickCount 系）。
    /// IME 診断ログで「フォーカス変更からの経過時間」を表示するために使う。
    pub last_focus_change_ms: u64,
    pub focus_debounce_ms: u32,
    pub ime_poll_interval_ms: u32,
    pub last_hook_activity_ms: u64,
    /// IME 同期キー直後のキー保留バッファ（旧 `ime_gate`）。
    pub sync_key_gate: SyncKeyGate,
    /// 現在のフォーカスアプリに適用されるキーマップルール
    pub active_keymaps: crate::keymap::KeymapTable,
}

impl PlatformState {
    /// デフォルト値で初期化する
    #[must_use]
    pub fn new() -> Self {
        Self {
            ime: ImeStateHub::new(),
            app_kind: AppKind::Win32,
            focus_kind: FocusKind::Undetermined,
            last_focus_change_ms: 0,
            focus_debounce_ms: 50,
            ime_poll_interval_ms: 500,
            last_hook_activity_ms: 0,
            sync_key_gate: SyncKeyGate::new(),
            active_keymaps: crate::keymap::KeymapTable::default(),
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
    /// `set_intent=Some(target)` なら UserImeSetIntent を dispatch し last_intent を設定する。
    /// `set_intent=None` なら desired_open のみ直接書き換え、last_intent は空のままにする
    /// (focus 変更後の carry-over シナリオを模擬)。
    fn ps_with_shadow(
        desired_open: bool,
        set_intent: Option<IntentSource>,
        is_japanese: bool,
    ) -> PlatformState {
        let mut ps = PlatformState::new();
        ps.ime.belief.is_japanese_ime = is_japanese;
        if let Some(source) = set_intent {
            ps.ime.dispatch_event(ImeEvent::UserImeSetIntent {
                target: desired_open,
                source,
            });
        } else {
            ps.ime.set_desired_open_for_test(desired_open);
            ps.ime.clear_last_intent_for_test();
        }
        ps
    }

    // フレッシュな intent が backing する false は保護される。
    // ユーザが直前に Ctrl+無変換 等で IME OFF した状態が、TsfNative ウィンドウへの
    // 切替で勝手に ON に戻されてはいけない。
    #[test]
    fn reset_stale_preserves_intent_backed_false_sync_key() {
        let mut ps = ps_with_shadow(false, Some(IntentSource::SyncKey), true);
        ps.ime.reset_stale_ime_on_for_tsf_native();
        assert!(!ps.ime.effective_open());
    }

    #[test]
    fn reset_stale_preserves_intent_backed_false_physical_key() {
        let mut ps = ps_with_shadow(false, Some(IntentSource::PhysicalImeKey), true);
        ps.ime.reset_stale_ime_on_for_tsf_native();
        assert!(!ps.ime.effective_open());
    }

    #[test]
    fn reset_stale_preserves_intent_backed_false_command() {
        let mut ps = ps_with_shadow(false, Some(IntentSource::Command), true);
        ps.ime.reset_stale_ime_on_for_tsf_native();
        assert!(!ps.ime.effective_open());
    }

    #[test]
    fn reset_stale_preserves_intent_backed_false_hwnd_cache() {
        let mut ps = ps_with_shadow(false, Some(IntentSource::HwndCache), true);
        ps.ime.reset_stale_ime_on_for_tsf_native();
        assert!(!ps.ime.effective_open());
    }

    // last_intent=None (前ウィンドウからの carry-over) は ON へ寄せ直す。
    #[test]
    fn reset_stale_overrides_carry_over_false() {
        let mut ps = ps_with_shadow(false, None, true);
        ps.ime.reset_stale_ime_on_for_tsf_native();
        assert!(ps.ime.effective_open());
        assert_eq!(ps.ime.last_intent_source(), Some(IntentSource::Recovery),);
    }

    // 既に ON なら何もしない（早期 return）。
    #[test]
    fn reset_stale_noop_when_already_on() {
        let mut ps = ps_with_shadow(true, Some(IntentSource::SyncKey), true);
        ps.ime.reset_stale_ime_on_for_tsf_native();
        assert!(ps.ime.effective_open());
    }

    // 非日本語レイアウトでは何もしない。
    #[test]
    fn reset_stale_noop_when_not_japanese() {
        let mut ps = ps_with_shadow(false, None, false);
        ps.ime.reset_stale_ime_on_for_tsf_native();
        assert!(!ps.ime.effective_open());
    }
}
