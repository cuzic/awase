use awase::engine::InputModeState;
use awase::types::{AppKind, FocusKind};

use super::belief::ImeBelief;
use super::hook_state::{HookConfig, HookRoutingState, SyncKeyGate};
use super::ime_event::{ChordKind, HwndId, ImeEvent, ImeEventEnvelope, IntentSource};
use super::ime_event_log::ImeEventLog;
use super::ime_model::ImeModel;
use super::input_barrier::InputBarrier;

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
        let event_for_reduce = event.clone();
        let time = self.event_log.record(event);
        let envelope = ImeEventEnvelope {
            time,
            event: event_for_reduce,
        };
        self.shadow_model.reduce(&envelope);
    }

    /// shadow_model から派生した最新の explicit intent。
    ///
    /// (Step 2B 以降の SSOT。Priority 4-5 observer による上書きを block する根拠。)
    pub(crate) fn last_explicit_intent_compat(&self) -> Option<bool> {
        self.shadow_model.last_intent.as_ref().map(|i| i.target)
    }

    /// applied_open / applied_at_ms を更新する（apply 完了時の SSOT 更新）。
    ///
    /// ImeModel アクセス可能なサイトで `set_ime_apply_latch` の代わりに呼ぶ。
    /// executor 内部 (PlatformState 非アクセス) は ImeApplySucceeded event 経由で更新される。
    pub(crate) fn mirror_applied_open(&mut self, value: bool) {
        self.mirror_applied_open_with_ts(value, crate::hook::current_tick_ms());
    }

    /// `applied_open / applied_at_ms` を指定タイムスタンプで更新する。
    ///
    /// `ts = 0` は「楽観的未確認」（ImmCross async 送信直後など）を表す。
    /// `applied_at_ms > 0` が「apply 確認済み」の条件なので skip_override 等の判定に影響する。
    pub(crate) const fn mirror_applied_open_with_ts(&mut self, value: bool, ts: u64) {
        self.shadow_model.applied_open = Some(value);
        self.shadow_model.applied_at_ms = ts;
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

    // ── Applied state ──

    pub(crate) const fn applied_open(&self) -> Option<bool> {
        self.shadow_model.applied_open
    }

    pub(crate) fn applied_open_or_default(&self) -> bool {
        self.shadow_model.applied_open.unwrap_or(false)
    }

    pub(crate) const fn has_applied_state(&self) -> bool {
        self.shadow_model.applied_open.is_some()
    }

    pub(crate) fn applied_pair(&self) -> Option<(bool, u64)> {
        self.shadow_model.applied_pair()
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
    pub hook: HookRoutingState,
    pub hook_config: HookConfig,
    pub last_hook_activity_ms: u64,
    /// IME 同期キー直後のキー保留バッファ（旧 `ime_gate`）。
    pub sync_key_gate: SyncKeyGate,
    /// 現在のフォーカスアプリに適用されるキーマップルール
    pub active_keymaps: crate::keymap::KeymapTable,
    /// 直近に物理 IME キー or sync キーで IME OFF にした時刻 (ms, GetTickCount 系)。
    /// 0 = 未設定。TsfNative 入場時の reset_stale スキップ判定に使う。
    pub last_explicit_ime_off_ms: u64,
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
            hook: HookRoutingState::default(),
            hook_config: HookConfig {
                left_thumb_vk: crate::vk::VK_NONCONVERT,
                right_thumb_vk: crate::vk::VK_CONVERT,
            },
            last_hook_activity_ms: 0,
            sync_key_gate: SyncKeyGate::new(),
            active_keymaps: crate::keymap::KeymapTable::default(),
            last_explicit_ime_off_ms: 0,
        }
    }
}

impl Default for PlatformState {
    fn default() -> Self {
        Self::new()
    }
}

impl PlatformState {
    // ── ImeStateHub への参照アクセサ ──

    /// `ImeBelief` への共有参照を返す。
    ///
    /// `build_input_context(&ps.belief(), …)` のような呼び出し用。
    #[inline]
    #[must_use]
    pub const fn belief(&self) -> &ImeBelief {
        &self.ime.belief
    }

    // ── ImeBelief への便利読み取りメソッド ──
    //
    // `belief()` を直接使っても同等だが、呼び出しサイトを短くするために置く。
    // `build_input_context(&ps.belief(), …)` のような「構造体丸ごと」の渡し方は belief() を使う。

    /// IME が ON かどうかを返す (Phase 3e: shadow_model.effective_open() が SSOT)。
    #[inline]
    #[must_use]
    pub const fn ime_on(&self) -> bool {
        self.ime.shadow_model.effective_open()
    }

    /// 入力モードを返す。
    #[inline]
    #[must_use]
    pub const fn input_mode(&self) -> InputModeState {
        self.ime.belief.input_mode()
    }

    /// 日本語 IME がアクティブかを返す。
    #[inline]
    #[must_use]
    pub const fn is_japanese_ime(&self) -> bool {
        self.ime.belief.is_japanese_ime()
    }

    /// 直前の conversion_mode を返す。
    #[inline]
    #[must_use]
    pub const fn prev_conversion_mode(&self) -> Option<u32> {
        self.ime.belief.prev_conversion_mode()
    }

    /// IME 状態検出の連続失敗回数を返す (Phase 3a: shadow_model.observe_miss_monitor 由来)。
    #[inline]
    #[must_use]
    pub const fn ime_detect_miss_count(&self) -> u32 {
        self.ime
            .shadow_model
            .observe_miss_monitor
            .consecutive_miss_count
    }

    /// いずれかの強制 ON ガードが立っているかを返す (Phase 3a: shadow_model.force_guards 由来)。
    #[inline]
    #[must_use]
    pub const fn is_force_on_guard_active(&self) -> bool {
        self.ime.shadow_model.force_guards.requires_on()
    }

    // ── ImeBelief への書き込みメソッド ──

    /// `input_mode` を設定する。
    #[inline]
    pub const fn set_input_mode(&mut self, mode: InputModeState) {
        self.ime.belief.input_mode = mode;
    }

    /// `is_japanese_ime` を設定する。
    #[inline]
    pub const fn set_is_japanese_ime(&mut self, value: bool) {
        self.ime.belief.is_japanese_ime = value;
    }

    /// `prev_conversion_mode` を設定する。
    #[inline]
    pub const fn set_prev_conversion_mode(&mut self, value: Option<u32>) {
        self.ime.belief.prev_conversion_mode = value;
    }

    // ── ForceGuardSet / ObserveMissMonitor への書き込みメソッド (Phase 3a) ──

    /// `BrokenAppBootstrap` ガードをセットする。
    #[inline]
    pub fn set_force_on_broken_app_bootstrap(&mut self) {
        self.ime
            .shadow_model
            .force_guards
            .add(super::force_guard::ForceGuard {
                reason: super::force_guard::ForceOnReason::BrokenAppBootstrap,
                expires_at: None,
                generation: self.ime.event_log.next_seq(),
            });
    }

    /// observe_miss_monitor を reset し、すべての force-on ガードを解除する。
    ///
    /// ユーザー操作（Shadow IME トグル・SetOpen 等）で「ユーザーが意図した状態」が
    /// 確定したときに呼ぶ。
    #[inline]
    pub fn reset_ime_detect_state(&mut self) {
        self.ime.shadow_model.observe_miss_monitor.record_success();
        self.ime.shadow_model.force_guards.guards.clear();
    }

    /// Shadow IME トグルによって IME 状態が実際に変化したときに呼ぶ。
    ///
    /// 意図的なトグル後は drift 検出カウンタを無効化する。
    pub fn on_shadow_ime_toggled(&mut self) {
        self.reset_ime_detect_state();
    }

    /// SetOpen リクエストを shadow_model に書き込んだときに呼ぶ。
    ///
    /// Engine が IME ON/OFF を要求した直後は drift 検出カウンタを無効化する。
    pub fn on_set_open_requested(&mut self) {
        self.reset_ime_detect_state();
    }

    /// panic_reset 向け全面リセット。
    ///
    /// belief (input_mode, is_japanese_ime, prev_conversion_mode) と shadow_model を初期化する。
    pub fn apply_panic_reset(&mut self) {
        self.ime.belief.input_mode = InputModeState::ObservedRomaji;
        self.ime.belief.is_japanese_ime = true;
        self.ime.belief.prev_conversion_mode = None;
        // Phase 3a: observe_miss_monitor + force_guards に置換
        self.ime.shadow_model.observe_miss_monitor.record_success();
        self.ime.shadow_model.force_guards.guards.clear();
        self.ime
            .shadow_model
            .force_guards
            .add(super::force_guard::ForceGuard {
                reason: super::force_guard::ForceOnReason::PanicReset,
                expires_at: None,
                generation: self.ime.event_log.next_seq(),
            });
        // Step 2B: shadow_model を直接 reset (event 記録は残しつつ intent はクリア)。
        self.ime.dispatch_event(ImeEvent::UserImeSetIntent {
            target: true,
            source: IntentSource::Recovery,
        });
        self.ime.shadow_model.last_intent = None;
        self.ime.shadow_model.observations.clear_on_focus_change();
    }
}

impl PlatformState {
    /// 最後の明示的 IME 操作の意図を返す（ログ・診断用）。
    ///
    /// shadow_model.last_intent.target を返す。
    #[must_use]
    pub fn explicit_intent(&self) -> Option<bool> {
        self.ime.last_explicit_intent_compat()
    }

    /// `observer_poll` 観測を shadow_model へ dispatch する。
    ///
    /// 外部観測（GJI I/O 等）の正規ルート。`user_enabled` / `ms` は現状未使用だが、
    /// 既存呼び出しサイトの API 互換のため引数は保持する。
    pub fn write_observer_poll(&mut self, value: bool, _ms: u64, _user_enabled: bool) {
        self.ime.dispatch_event(ImeEvent::ObserverReported {
            open: value,
            source: super::ime_event::ObservationSource::ObserverPoll,
            hwnd: HwndId::NULL,
            confidence: super::ime_event::ObservationConfidence::Medium,
        });
    }

    /// 同期キー由来の意図を shadow_model へ dispatch する。
    pub fn write_sync_key(&mut self, value: bool, ms: u64, _user_enabled: bool) {
        if !value {
            self.last_explicit_ime_off_ms = ms;
        }
        self.ime.dispatch_event(ImeEvent::UserImeSetIntent {
            target: value,
            source: IntentSource::SyncKey,
        });
    }

    /// 物理 IME キー由来の意図を shadow_model へ dispatch する。
    pub fn write_physical_key(&mut self, value: bool, ms: u64, _user_enabled: bool) {
        if !value {
            self.last_explicit_ime_off_ms = ms;
        }
        self.ime.dispatch_event(ImeEvent::UserImeSetIntent {
            target: value,
            source: IntentSource::PhysicalImeKey,
        });
    }

    /// Engine の SetOpen 要求由来の意図を shadow_model へ dispatch する。
    pub fn write_set_open_request(&mut self, value: bool, _ms: u64, _user_enabled: bool) {
        self.ime.dispatch_event(ImeEvent::UserImeSetIntent {
            target: value,
            source: IntentSource::Command,
        });
    }

    /// フォーカス変更直後の同期プローブ観測を shadow_model へ dispatch する。
    pub fn write_focus_probe(&mut self, value: bool, _ms: u64, _user_enabled: bool) {
        self.ime.dispatch_event(ImeEvent::ObserverReported {
            open: value,
            source: super::ime_event::ObservationSource::FocusProbe,
            hwnd: HwndId::NULL,
            confidence: super::ime_event::ObservationConfidence::Medium,
        });
    }

    /// `ImeUpdate` を shadow_model / belief (is_japanese_ime, input_mode, prev_conversion_mode) に反映する。
    ///
    /// `observer::ime_observer::poll_and_classify_ime()` / `classify_fetched_snapshot()` の結果を受け取り、
    /// 状態への書き込みをここに集約する。判断ロジックを持たない純粋適用関数。
    pub fn apply_ime_update(
        &mut self,
        update: &crate::observer::ime_observer::ImeUpdate,
        _user_enabled: bool,
    ) {
        // is_japanese_ime: 検出成功時のみ更新
        if let Some(is_jp) = update.is_japanese_ime {
            self.ime.belief.is_japanese_ime = is_jp;
        }

        // observer_poll 観測 → shadow_model へ dispatch
        if let Some(obs) = update.observer_poll {
            self.ime.dispatch_event(ImeEvent::ObserverReported {
                open: obs.value,
                source: super::ime_event::ObservationSource::ObserverPoll,
                hwnd: HwndId::NULL,
                confidence: super::ime_event::ObservationConfidence::Medium,
            });
        }

        // miss_count (Phase 3a: observe_miss_monitor 経由)
        if update.increment_miss_count {
            self.ime
                .shadow_model
                .observe_miss_monitor
                .record_miss(std::time::Instant::now());
            let miss = self
                .ime
                .shadow_model
                .observe_miss_monitor
                .consecutive_miss_count;
            if miss == crate::IME_DETECT_MISS_THRESHOLD {
                log::warn!("IME detection failed {miss} consecutive times, will force IME ON");
            }
        }

        // force_on_broken_app_bootstrap のリセット（検出成功時、Phase 3a: ForceGuardSet 経由）
        if update.clear_force_on_broken_app_bootstrap {
            self.ime
                .shadow_model
                .force_guards
                .remove(super::force_guard::ForceOnReason::BrokenAppBootstrap);
        }

        // force_on_panic_reset と miss_count のリセット（検出成功時、Phase 3a）
        if update.clear_force_on_panic_reset {
            self.ime
                .shadow_model
                .force_guards
                .remove(super::force_guard::ForceOnReason::PanicReset);
            self.ime.shadow_model.observe_miss_monitor.record_success();
        }

        // input_mode
        if let Some(mode) = update.new_input_mode {
            self.ime.belief.input_mode = mode;
        }

        // prev_conversion_mode
        if let Some(conv) = update.new_prev_conversion_mode {
            self.ime.belief.prev_conversion_mode = Some(conv);
        }
    }

    /// `hwnd_cache::restore_on_focus_enter()` の結果を shadow_model に反映する。
    ///
    /// キャッシュヒット（`Some`）の場合のみ適用する。`None` の場合は何もしない。
    /// per-HWND キャッシュは「前回 focus 時のユーザ意図」を保存しているため、
    /// `UserImeSetIntent { source: HwndCache }` として dispatch する。
    pub fn apply_hwnd_cache_restore(
        &mut self,
        snapshot: Option<crate::focus::hwnd_cache::HwndImeSnapshot>,
    ) {
        if let Some(snap) = snapshot {
            self.ime.dispatch_event(ImeEvent::UserImeSetIntent {
                target: snap.ime_on,
                source: IntentSource::HwndCache,
            });
            self.ime.belief.input_mode = snap.input_mode;
        }
    }

    /// TsfNative ウィンドウへのフォーカス入場時、`HwndCache` ミスで前ウィンドウから
    /// carry over した `desired_open=false` を IME ON へ寄せ直す（Japanese 文脈の安全側既定）。
    ///
    /// TsfNative では IMM クロスプロセス取得もポーリングも skip されるため、
    /// stale な `false` が ObserverPoll でも復旧せず Engine が活性化不能になる。
    /// 日本語レイアウト時のみ実行する。
    ///
    /// # 4 層モデルとの整合: フレッシュな intent は尊重する
    ///
    /// `FocusChanged` event は `last_intent` を clear するが `desired_open` は保持する。
    /// よって本関数が呼ばれた時点で `last_intent.is_none()` なら、その `false` は前ウィンドウ
    /// からの carry over でしかありえない（cache miss なので HwndCache 由来の intent もない）。
    /// その場合のみ ON へ寄せ直す。`last_intent.is_some()` なら現フォーカス文脈の新しい
    /// 意図（HwndCache 経由含む）なので保護する。
    pub fn reset_stale_ime_on_for_tsf_native(&mut self) {
        if !self.ime.belief.is_japanese_ime() || self.ime_on() {
            return;
        }
        if let Some(intent) = self.ime.shadow_model.last_intent.as_ref() {
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
        self.ime.dispatch_event(ImeEvent::UserImeSetIntent {
            target: true,
            source: IntentSource::Recovery,
        });
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
        ps.reset_stale_ime_on_for_tsf_native();
        assert!(!ps.ime_on());
    }

    #[test]
    fn reset_stale_preserves_intent_backed_false_physical_key() {
        let mut ps = ps_with_shadow(false, Some(IntentSource::PhysicalImeKey), true);
        ps.reset_stale_ime_on_for_tsf_native();
        assert!(!ps.ime_on());
    }

    #[test]
    fn reset_stale_preserves_intent_backed_false_command() {
        let mut ps = ps_with_shadow(false, Some(IntentSource::Command), true);
        ps.reset_stale_ime_on_for_tsf_native();
        assert!(!ps.ime_on());
    }

    #[test]
    fn reset_stale_preserves_intent_backed_false_hwnd_cache() {
        let mut ps = ps_with_shadow(false, Some(IntentSource::HwndCache), true);
        ps.reset_stale_ime_on_for_tsf_native();
        assert!(!ps.ime_on());
    }

    // last_intent=None (前ウィンドウからの carry-over) は ON へ寄せ直す。
    #[test]
    fn reset_stale_overrides_carry_over_false() {
        let mut ps = ps_with_shadow(false, None, true);
        ps.reset_stale_ime_on_for_tsf_native();
        assert!(ps.ime_on());
        assert_eq!(ps.ime.last_intent_source(), Some(IntentSource::Recovery),);
    }

    // 既に ON なら何もしない（早期 return）。
    #[test]
    fn reset_stale_noop_when_already_on() {
        let mut ps = ps_with_shadow(true, Some(IntentSource::SyncKey), true);
        ps.reset_stale_ime_on_for_tsf_native();
        assert!(ps.ime_on());
    }

    // 非日本語レイアウトでは何もしない。
    #[test]
    fn reset_stale_noop_when_not_japanese() {
        let mut ps = ps_with_shadow(false, None, false);
        ps.reset_stale_ime_on_for_tsf_native();
        assert!(!ps.ime_on());
    }
}
