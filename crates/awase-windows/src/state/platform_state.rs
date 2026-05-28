use awase::engine::InputModeState;
use awase::types::{AppKind, FocusKind};

use super::belief::{ImeBelief, ShadowSource};
use super::hook_state::{HookRoutingState, HookConfig, SyncKeyGate};
use super::ime_event::{ImeEvent, ImeEventEnvelope, IntentSource};
use super::ime_event_log::ImeEventLog;
use super::ime_model::{ImeEffectiveState, ImeModel};

// ────────────────────────────────────────────────────────────────────────────
// ImeStateHub
// ────────────────────────────────────────────────────────────────────────────

/// IME 観測・判断・belief 書き戻しを担う凝集ユニット。
///
/// `PlatformState` から IME 関連フィールドを切り出すことで、
/// 「観測」「フォーカス状態」「フック設定」の混在を解消する。
///
/// - `belief`        : 観測値から導出した現在の IME 状態（Tick ごとに更新）
/// - `shadow_model`  : 新モデル (Phase 1-3 で導入)。force_guards / drift_monitor を持つ
#[derive(Debug)]
pub(crate) struct ImeStateHub {
    /// 観測値から導出した現在の IME 状態への「信念」
    pub(crate) belief: ImeBelief,
    /// 各ソースの最新観測値（Phase 2: 観測と判断の分離）。
    ///
    /// `ime_on` の最終値は `ImeObservations::resolve_and_clear()` で一括決定される。
    pub(crate) ime_observations: crate::ime_observations::ImeObservations,
    /// IME 状態変更 event のリングバッファ (Step 0)。
    pub(crate) event_log: ImeEventLog,

    /// Shadow IME モデル (Step 1)。Phase 3a で recovery 統合済。
    /// force_guards / drift_monitor を持つ SSOT。
    pub(crate) shadow_model: ImeModel,
}

impl ImeStateHub {
    /// デフォルト値で初期化する。
    pub(crate) fn new() -> Self {
        Self {
            belief: ImeBelief {
                ime_on: true,                              // 安全側: ON で初期化
                ime_on_source: ShadowSource::Init,
                input_mode: InputModeState::ObservedRomaji, // デフォルト: ローマ字入力
                is_japanese_ime: true,                     // デフォルト: 日本語
                prev_conversion_mode: None,
            },
            ime_observations: crate::ime_observations::ImeObservations::default(),
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

    /// Phase 3c: applied_open の mirror 設定。
    ///
    /// 既存 `Output::set_ime_apply_latch` 呼び出しサイトで platform_state にアクセスできる
    /// 場合、この関数も同時に呼び出して shadow_model.applied_open を同期する。
    /// executor 内部 (PlatformState 非アクセス) のみ async path で
    /// ImeApplySucceeded event 経由で更新される。
    pub(crate) fn mirror_applied_open(&mut self, value: bool) {
        self.shadow_model.applied_open = Some(value);
        // 同じ apply が完了した扱いなので pending も clear
        if let Some(p) = &self.shadow_model.pending {
            if p.target == value {
                self.shadow_model.pending = None;
            }
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// FocusPlatformState
// ────────────────────────────────────────────────────────────────────────────

/// フォーカス追跡に関する Platform 層の状態を集約する構造体。
///
/// app_kind・focus_kind・タイミング・ポーリング間隔を保持する。
///
/// Step 5: focus_transition_pending: bool は InputBarrier::FocusTransition に置換済。
#[derive(Debug)]
pub struct FocusPlatformState {
    pub app_kind: AppKind,
    pub focus_kind: FocusKind,
    /// 最後にフォアグラウンドプロセスが変わった時刻（ms, GetTickCount 系）。
    /// IME 診断ログで「フォーカス変更からの経過時間」を表示するために使う。
    pub last_focus_change_ms: u64,
    pub focus_debounce_ms: u32,
    pub ime_poll_interval_ms: u32,
}

impl FocusPlatformState {
    const fn new() -> Self {
        Self {
            app_kind: AppKind::Win32,
            focus_kind: FocusKind::Undetermined,
            last_focus_change_ms: 0,
            focus_debounce_ms: 50,
            ime_poll_interval_ms: 500,
        }
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
    /// フォーカス追跡に関する状態を集約するユニット。
    pub focus: FocusPlatformState,
    pub hook: HookRoutingState,
    pub hook_config: HookConfig,
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
            focus: FocusPlatformState::new(),
            hook: HookRoutingState::default(),
            hook_config: HookConfig {
                left_thumb_vk: crate::vk::VK_NONCONVERT,
                right_thumb_vk: crate::vk::VK_CONVERT,
            },
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

    /// IME が ON かどうかを返す。
    #[inline]
    #[must_use]
    pub const fn ime_on(&self) -> bool {
        self.ime.belief.ime_on()
    }

    /// `ime_on` を最後に更新したソースを返す。
    #[inline]
    #[must_use]
    pub const fn ime_on_source(&self) -> ShadowSource {
        self.ime.belief.ime_on_source()
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

    /// IME 状態検出の連続失敗回数を返す (Phase 3a: shadow_model.drift_monitor 由来)。
    #[inline]
    #[must_use]
    pub const fn ime_detect_miss_count(&self) -> u32 {
        self.ime.shadow_model.drift_monitor.consecutive_miss_count
    }

    /// いずれかの強制 ON ガードが立っているかを返す (Phase 3a: shadow_model.force_guards 由来)。
    #[inline]
    #[must_use]
    pub fn is_force_on_guard_active(&self) -> bool {
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

    // ── ForceGuardSet / DriftMonitor への書き込みメソッド (Phase 3a) ──

    /// `BrokenAppBootstrap` ガードをセットする。
    #[inline]
    pub fn set_force_on_broken_app_bootstrap(&mut self) {
        self.ime.shadow_model.force_guards.add(
            super::force_guard::ForceGuard {
                reason: super::force_guard::ForceOnReason::BrokenAppBootstrap,
                expires_at: None,
                generation: self.ime.event_log.next_seq(),
            },
        );
    }

    /// drift_monitor を reset し、すべての force-on ガードを解除する。
    ///
    /// ユーザー操作（Shadow IME トグル・SetOpen 等）で「ユーザーが意図した状態」が
    /// 確定したときに呼ぶ。
    #[inline]
    pub fn reset_ime_detect_state(&mut self) {
        self.ime.shadow_model.drift_monitor.record_success();
        self.ime.shadow_model.force_guards.guards.clear();
    }

    /// panic_reset 向け全面リセット。
    ///
    /// belief / recovery のすべてのフィールドをまとめて設定する。
    /// `ime_observations` もクリアして stale な観測値が残らないようにする。
    pub fn apply_panic_reset(&mut self) {
        self.ime.belief.input_mode = InputModeState::ObservedRomaji;
        self.ime.belief.set_ime_on(true, ShadowSource::PanicReset);
        self.ime.belief.is_japanese_ime = true;
        self.ime.belief.prev_conversion_mode = None;
        // Phase 3a: drift_monitor + force_guards に置換
        self.ime.shadow_model.drift_monitor.record_success();
        self.ime.shadow_model.force_guards.guards.clear();
        self.ime.shadow_model.force_guards.add(super::force_guard::ForceGuard {
            reason: super::force_guard::ForceOnReason::PanicReset,
            expires_at: None,
            generation: self.ime.event_log.next_seq(),
        });
        // パニックリセット後は全観測スロットと明示的意図をクリア。
        self.ime.ime_observations.clear_on_focus_change();
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
    /// `ime_observations.resolve_and_clear()` を実行して `belief.ime_on` を更新する。
    ///
    /// ## 呼び出し規約
    ///
    /// 通常は `write_*` ヘルパーや `apply_ime_update` が内部で呼ぶため、
    /// 外部から直接呼ぶ必要はほとんどない。
    ///
    /// - `write_sync_key`, `write_physical_key`, `write_set_open_request`,
    ///   `write_focus_probe`, `write_observer_poll` — 書き込みと同時に自動解決する。
    /// - `apply_ime_update` — `ImeUpdate` の全フィールド適用後に自動解決する。
    ///
    /// 複数スロットを 1 tick 内で書き込みたい場合（将来の拡張）にのみ直接使用する。
    pub fn apply_ime_observations(&mut self, user_enabled: bool) {
        let current = self.ime.belief.ime_on;
        let is_japanese = self.ime.belief.is_japanese_ime;
        let obs = &self.ime.ime_observations;
        log::trace!(
            "[apply-obs] slots: sync={:?} phys={:?} req={:?} fp={:?} op={:?} \
             belief_on={} is_jp={} user_en={}",
            obs.sync_key.map(|o| o.value),
            obs.physical_key.map(|o| o.value),
            obs.set_open_request.map(|o| o.value),
            obs.focus_probe.map(|o| o.value),
            obs.observer_poll.map(|o| o.value),
            current, is_japanese, user_enabled,
        );
        if let Some((val, src)) = self.ime.ime_observations.resolve_and_clear(current, user_enabled, is_japanese) {
            // ObserverPoll / FocusSnapshot が明示的意図と矛盾する値を返した場合はブロックする。
            // タイマーではなく「最後の明示的操作の意図」を根拠にするため、
            // フォーカス変更でクリアされるまで有効（時間切れなし）。
            // 明示的操作（SyncKey / PhysicalImeKey / SetOpenRequest, Priority 1-3）は
            // intent スロットを直接更新するので、ここには到達しない。
            if matches!(src, ShadowSource::ObserverPoll | ShadowSource::FocusSnapshot) {
                // intent guard: shadow_model.last_intent 由来の compat 値で判定。
                if let Some(intent) = self.ime.last_explicit_intent_compat() {
                    if val != intent {
                        log::debug!(
                            "[explicit-intent] belief→{val} blocked (intent={intent}, src={src:?})"
                        );
                        match src {
                            ShadowSource::ObserverPoll => {
                                self.ime.ime_observations.observer_poll = None;
                            }
                            ShadowSource::FocusSnapshot => {
                                self.ime.ime_observations.focus_probe = None;
                            }
                            _ => {}
                        }
                        return;
                    }
                }
            }
            log::debug!(
                "[apply-obs] belief update: {}→{} src={:?} intent={:?}",
                current, val, src, self.ime.last_explicit_intent_compat(),
            );
            self.ime.belief.set_ime_on(val, src);
        }
        // Step 1: Shadow Reducer の effective state と既存 belief を比較し diff log を出力。
        // 本番判定には影響しない (shadow_model の値は使わない)。
        self.log_shadow_diff_if_any();
    }

    /// Shadow Reducer (Step 1) の effective state と既存 belief の diff を log 出力。
    /// 1 週間モニタで Expected / Suspicious / Regression を分類する材料にする。
    fn log_shadow_diff_if_any(&self) {
        let old_ime_on = self.ime.belief.ime_on;
        let new_effective = self.ime.shadow_model.effective_state();
        let Some(severity) =
            ImeEffectiveState::classify_diff(old_ime_on, new_effective.ime_target_open)
        else {
            return;
        };
        let last_intent = self.ime.shadow_model.last_intent.as_ref();
        let drift_seq = self.ime.shadow_model.observations.drift.map(|d| d.first_drift_seq);
        log::debug!(
            "[shadow-diff seq~{}] severity={:?} old.ime_on={} new.target={} \
             last_intent={:?} drift_seq={:?}",
            self.ime.event_log.next_seq().saturating_sub(1),
            severity,
            old_ime_on,
            new_effective.ime_target_open,
            last_intent.map(|i| (i.target, i.source, i.at_seq)),
            drift_seq,
        );
    }

    /// 最後の明示的 IME 操作の意図を返す（ログ・診断用）。
    ///
    /// 最後の明示的 IME 操作の意図を返す (Step 2B: shadow_model.last_intent 由来)。
    pub fn explicit_intent(&self) -> Option<bool> {
        self.ime.last_explicit_intent_compat()
    }

    /// `observer_poll` スロットに観測値を書き込み、即座に judgement を通す。
    ///
    /// 外部観測（GJI I/O 等）を `belief.ime_on` に反映する正規ルート。
    pub fn write_observer_poll(&mut self, value: bool, ms: u64, user_enabled: bool) {
        self.ime.dispatch_event(ImeEvent::ObserverReported {
            open: value,
            source: super::ime_event::ObservationSource::ObserverPoll,
            hwnd: super::ime_event::HwndId::NULL,
            confidence: super::ime_event::ObservationConfidence::Medium,
        });
        self.ime.ime_observations.observer_poll =
            Some(crate::ime_observations::ImeObs { value, ms });
        self.apply_ime_observations(user_enabled);
    }

    /// フォーカス変更時に `ime_observations` の全スロットと明示的意図をクリアする。
    pub fn clear_ime_observations_on_focus_change(&mut self) {
        self.ime.ime_observations.clear_on_focus_change();
        // Step 3: shadow_model の last_intent / observations を clear (SSOT)。
        // desired_open は維持 (フォーカス変更でユーザー意図を捨てない)。
        self.ime.shadow_model.last_intent = None;
        self.ime.shadow_model.observations.clear_on_focus_change();
        log::debug!("[explicit-intent] cleared (focus change)");
    }

    /// `sync_key` スロットに観測値を書き込み、即座に judgement を通す。
    pub fn write_sync_key(&mut self, value: bool, ms: u64, user_enabled: bool) {
        self.ime.dispatch_event(ImeEvent::UserImeSetIntent {
            target: value,
            source: IntentSource::SyncKey,
        });
        self.ime.ime_observations.sync_key =
            Some(crate::ime_observations::ImeObs { value, ms });
        self.apply_ime_observations(user_enabled);
    }

    /// `physical_key` スロットに観測値を書き込み、即座に judgement を通す。
    pub fn write_physical_key(&mut self, value: bool, ms: u64, user_enabled: bool) {
        self.ime.dispatch_event(ImeEvent::UserImeSetIntent {
            target: value,
            source: IntentSource::PhysicalImeKey,
        });
        self.ime.ime_observations.physical_key =
            Some(crate::ime_observations::ImeObs { value, ms });
        self.apply_ime_observations(user_enabled);
    }

    /// `set_open_request` スロットに観測値を書き込み、即座に judgement を通す。
    pub fn write_set_open_request(&mut self, value: bool, ms: u64, user_enabled: bool) {
        log::debug!(
            "[write-set-open-req] value={value} user_en={user_enabled} \
             belief_on={} op={:?} fp={:?}",
            self.ime.belief.ime_on,
            self.ime.ime_observations.observer_poll.map(|o| o.value),
            self.ime.ime_observations.focus_probe.map(|o| o.value),
        );
        self.ime.dispatch_event(ImeEvent::UserImeSetIntent {
            target: value,
            source: IntentSource::Command,
        });
        self.ime.ime_observations.set_open_request =
            Some(crate::ime_observations::ImeObs { value, ms });
        self.apply_ime_observations(user_enabled);
    }

    /// `focus_probe` スロットに観測値を書き込み、即座に judgement を通す。
    pub fn write_focus_probe(&mut self, value: bool, ms: u64, user_enabled: bool) {
        self.ime.dispatch_event(ImeEvent::ObserverReported {
            open: value,
            source: super::ime_event::ObservationSource::FocusProbe,
            hwnd: super::ime_event::HwndId::NULL,
            confidence: super::ime_event::ObservationConfidence::Medium,
        });
        self.ime.ime_observations.focus_probe =
            Some(crate::ime_observations::ImeObs { value, ms });
        self.apply_ime_observations(user_enabled);
    }

    /// `ImeUpdate` を `ImeBelief` / `ImeRecoveryState` / `ImeObservations` に反映し、
    /// 即座に judgement を通す。
    ///
    /// `observer::ime_observer::poll_and_classify_ime()` / `classify_fetched_snapshot()` の結果を受け取り、
    /// 状態への書き込みと解決をここに集約する。判断ロジックを持たない純粋適用関数。
    pub fn apply_ime_update(
        &mut self,
        update: &crate::observer::ime_observer::ImeUpdate,
        user_enabled: bool,
    ) {
        // is_japanese_ime: 検出成功時のみ更新
        if let Some(is_jp) = update.is_japanese_ime {
            self.ime.belief.is_japanese_ime = is_jp;
        }

        // observer_poll スロット
        if let Some(obs) = update.observer_poll {
            self.ime.ime_observations.observer_poll = Some(obs);
        }

        // miss_count (Phase 3a: drift_monitor 経由)
        if update.increment_miss_count {
            self.ime.shadow_model.drift_monitor.record_miss(std::time::Instant::now());
            let miss = self.ime.shadow_model.drift_monitor.consecutive_miss_count;
            if miss == crate::IME_DETECT_MISS_THRESHOLD {
                log::warn!(
                    "IME detection failed {miss} consecutive times, will force IME ON"
                );
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
            self.ime.shadow_model.drift_monitor.record_success();
        }

        // input_mode
        if let Some(mode) = update.new_input_mode {
            self.ime.belief.input_mode = mode;
        }

        // prev_conversion_mode
        if let Some(conv) = update.new_prev_conversion_mode {
            self.ime.belief.prev_conversion_mode = Some(conv);
        }

        self.apply_ime_observations(user_enabled);
    }

    /// `hwnd_cache::restore_on_focus_enter()` の結果を `ImeBelief` に反映する。
    ///
    /// キャッシュヒット（`Some`）の場合のみ適用する。`None` の場合は何もしない。
    pub const fn apply_hwnd_cache_restore(
        &mut self,
        snapshot: Option<crate::focus::hwnd_cache::HwndImeSnapshot>,
    ) {
        if let Some(snap) = snapshot {
            self.ime.belief.set_ime_on(snap.ime_on, ShadowSource::HwndCache);
            self.ime.belief.input_mode = snap.input_mode;
        }
    }

    /// TsfNative ウィンドウへのフォーカス入場時、`HwndCache` ミスで前ウィンドウから
    /// carry over した `ime_on=false` を IME ON へ寄せ直す（Japanese 文脈の安全側既定）。
    ///
    /// TsfNative では IMM クロスプロセス取得もポーリングも skip されるため、
    /// stale な `false` が ObserverPoll でも復旧せず Engine が活性化不能になる。
    /// 日本語レイアウト時のみ実行する。
    ///
    /// # 4 層モデルとの整合: Layer 1 観測を尊重する
    ///
    /// 本処理は Layer 2 (`ImeBelief`) を直接書き換える「ヒューリスティック修復」だが、
    /// `belief.ime_on=false` の出所が Layer 1 の検証済み観測やユーザ明示操作である場合、
    /// その値を上書きするとユーザ意図に反した IME ON 発火を招く
    /// （例: ユーザが Ctrl+無変換 で IME OFF した直後に Windows Terminal へ切替）。
    ///
    /// よって `ime_on_source` を確認し、以下の「Layer 1 由来の信頼できる false」は保護する:
    /// - `ObserverPoll`    : IMM クロスプロセス読みで verified
    /// - `PhysicalImeKey`  : ユーザの直接操作（半角/全角等）
    /// - `SyncKey`         : config 由来の同期キー（ユーザ設定）
    /// - `SetOpenRequest`  : Engine の判断（special-key 等、ユーザ起点）
    /// - `FocusSnapshot`   : フォーカス変更直後のフレッシュなプローブ
    ///
    /// 上書き対象は「観測由来でない値」のみ:
    /// - `Init`       : 起動時の既定値（通常は ON 初期化なので発火しない）
    /// - `HwndCache`  : 別 HWND キャッシュからの復元（本関数は cache miss 時のみ呼ばれるため
    ///   実際には到達しないが、再入時の保護として記載）
    /// - `PanicReset` : 強制リセット由来
    pub fn reset_stale_ime_on_for_tsf_native(&mut self) {
        if !self.ime.belief.is_japanese_ime() || self.ime.belief.ime_on() {
            return;
        }
        let source = self.ime.belief.ime_on_source();
        if Self::is_layer1_verified_source(source) {
            log::debug!(
                "TsfNative entry: preserving ime_on=false (source={source:?}, Layer 1 verified/explicit)"
            );
            return;
        }
        log::info!(
            "TsfNative entry without cache: reset stale ime_on=false → true \
             (source={source:?}, Japanese layout, IME state untrackable in TSF-native)"
        );
        self.ime.belief.set_ime_on(true, ShadowSource::HwndCache);
    }

    /// `ime_on` の出所が Layer 1 の検証済み観測またはユーザ明示操作かを返す。
    ///
    /// `true` のとき、その `ime_on` 値は Layer 2 ヒューリスティックで上書きしてはならない
    /// （`reset_stale_ime_on_for_tsf_native` 等の保護判定で使用）。
    const fn is_layer1_verified_source(source: ShadowSource) -> bool {
        matches!(
            source,
            ShadowSource::ObserverPoll
                | ShadowSource::PhysicalImeKey
                | ShadowSource::SyncKey
                | ShadowSource::SetOpenRequest
                | ShadowSource::FocusSnapshot
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ps_with_belief(ime_on: bool, source: ShadowSource, is_japanese: bool) -> PlatformState {
        let mut ps = PlatformState::new();
        ps.ime.belief.ime_on = ime_on;
        ps.ime.belief.ime_on_source = source;
        ps.ime.belief.is_japanese_ime = is_japanese;
        ps
    }

    // Layer 1 由来の検証済み false は保護される（4 層モデル尊重）。
    // ユーザが直前に Ctrl+無変換 等で IME OFF した状態が、TsfNative ウィンドウへの
    // 切替で勝手に ON に戻されてはいけない。
    #[test]
    fn reset_stale_preserves_observer_poll_false() {
        let mut ps = ps_with_belief(false, ShadowSource::ObserverPoll, true);
        ps.reset_stale_ime_on_for_tsf_native();
        assert!(!ps.ime.belief.ime_on());
        assert_eq!(ps.ime.belief.ime_on_source(), ShadowSource::ObserverPoll);
    }

    #[test]
    fn reset_stale_preserves_physical_key_false() {
        let mut ps = ps_with_belief(false, ShadowSource::PhysicalImeKey, true);
        ps.reset_stale_ime_on_for_tsf_native();
        assert!(!ps.ime.belief.ime_on());
    }

    #[test]
    fn reset_stale_preserves_sync_key_false() {
        let mut ps = ps_with_belief(false, ShadowSource::SyncKey, true);
        ps.reset_stale_ime_on_for_tsf_native();
        assert!(!ps.ime.belief.ime_on());
    }

    #[test]
    fn reset_stale_preserves_set_open_request_false() {
        let mut ps = ps_with_belief(false, ShadowSource::SetOpenRequest, true);
        ps.reset_stale_ime_on_for_tsf_native();
        assert!(!ps.ime.belief.ime_on());
    }

    #[test]
    fn reset_stale_preserves_focus_snapshot_false() {
        let mut ps = ps_with_belief(false, ShadowSource::FocusSnapshot, true);
        ps.reset_stale_ime_on_for_tsf_native();
        assert!(!ps.ime.belief.ime_on());
    }

    // 観測由来でない false (PanicReset) は従来通り上書きされる。
    #[test]
    fn reset_stale_overrides_panic_reset_false() {
        let mut ps = ps_with_belief(false, ShadowSource::PanicReset, true);
        ps.reset_stale_ime_on_for_tsf_native();
        assert!(ps.ime.belief.ime_on());
        assert_eq!(ps.ime.belief.ime_on_source(), ShadowSource::HwndCache);
    }

    // 既に ON なら何もしない（早期 return）。
    #[test]
    fn reset_stale_noop_when_already_on() {
        let mut ps = ps_with_belief(true, ShadowSource::ObserverPoll, true);
        ps.reset_stale_ime_on_for_tsf_native();
        assert!(ps.ime.belief.ime_on());
        assert_eq!(ps.ime.belief.ime_on_source(), ShadowSource::ObserverPoll);
    }

    // 非日本語レイアウトでは何もしない。
    #[test]
    fn reset_stale_noop_when_not_japanese() {
        let mut ps = ps_with_belief(false, ShadowSource::PanicReset, false);
        ps.reset_stale_ime_on_for_tsf_native();
        assert!(!ps.ime.belief.ime_on());
    }
}
