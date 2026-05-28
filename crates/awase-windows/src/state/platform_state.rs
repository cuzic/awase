use awase::engine::InputModeState;
use awase::types::{AppKind, FocusKind};

use super::belief::{ImeBelief, ImeRecoveryState, ShadowSource};
use super::hook_state::{HookRoutingState, HookConfig, SyncKeyGate};

// ────────────────────────────────────────────────────────────────────────────
// ImeStateHub
// ────────────────────────────────────────────────────────────────────────────

/// IME 観測・判断・belief 書き戻しを担う凝集ユニット。
///
/// `PlatformState` から IME 関連フィールドを切り出すことで、
/// 「観測」「フォーカス状態」「フック設定」の混在を解消する。
///
/// - `belief`   : 観測値から導出した現在の IME 状態（Tick ごとに更新）
/// - `recovery` : IME 回復ロジック用 FSM 状態（複数 Tick にまたがる累積値）
#[derive(Debug)]
pub(crate) struct ImeStateHub {
    /// 観測値から導出した現在の IME 状態への「信念」
    pub(crate) belief: ImeBelief,
    /// IME 回復ロジック用 FSM 累積状態
    pub(crate) recovery: ImeRecoveryState,
    /// 各ソースの最新観測値（Phase 2: 観測と判断の分離）。
    ///
    /// `ime_on` の最終値は `ImeObservations::resolve_and_clear()` で一括決定される。
    pub(crate) ime_observations: crate::ime_observations::ImeObservations,
    /// IME 明示指示後の観測値抑制タイムスタンプ（0 = 非アクティブ）。
    ///
    /// `write_set_open_request` / `write_sync_key` / `write_physical_key` でセットされ、
    /// この ms 値を超えると失効する。ガード中は `ObserverPoll` / `FocusSnapshot` が
    /// `ime_intent_guard_on` と矛盾する値で belief を上書きするのを防ぐ。
    ///
    /// - IME OFF 要求 (guard_on=false): observer=true をブロック
    /// - IME ON 要求 (guard_on=true): observer=false をブロック
    pub(crate) ime_intent_guard_until_ms: u64,
    /// ガード中に保護する IME 状態（`write_set_open_request` 等の `value` と同じ）。
    pub(crate) ime_intent_guard_on: bool,
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
            recovery: ImeRecoveryState {
                ime_detect_miss_count: 0,
                force_on_broken_app_bootstrap: false,
                force_on_panic_reset: false,
            },
            ime_observations: crate::ime_observations::ImeObservations::default(),
            ime_intent_guard_until_ms: 0,
            ime_intent_guard_on: false,
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// FocusPlatformState
// ────────────────────────────────────────────────────────────────────────────

/// フォーカス追跡に関する Platform 層の状態を集約する構造体。
///
/// app_kind・focus_kind・focus_transition_pending・タイミング・ポーリング間隔を保持する。
#[derive(Debug)]
pub struct FocusPlatformState {
    pub app_kind: AppKind,
    pub focus_kind: FocusKind,
    /// フォーカス切替直後フラグ。
    ///
    /// フォーカス変更を検知したときに `true` にセットされる。
    /// 次のキーストローク到着時に同期プローブ（高速 IME 状態検出）を実行し、
    /// belief を即座に更新してからキーを処理する。
    /// これにより「古い belief でキーが処理される」ギャップを解消する。
    pub focus_transition_pending: bool,
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
            focus_transition_pending: false,
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
            hook: HookRoutingState {
                ctrl_bypass_hold: false,
            },
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

    /// `ImeRecoveryState` への共有参照を返す。
    #[inline]
    #[must_use]
    pub const fn recovery(&self) -> &ImeRecoveryState {
        &self.ime.recovery
    }

    // ── ImeBelief / ImeRecoveryState への便利読み取りメソッド ──
    //
    // `belief()` / `recovery()` を直接使っても同等だが、呼び出しサイトを短くするために置く。
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

    /// IME 状態検出の連続失敗回数を返す。
    #[inline]
    #[must_use]
    pub const fn ime_detect_miss_count(&self) -> u32 {
        self.ime.recovery.ime_detect_miss_count()
    }

    /// いずれかの強制 ON ガードが立っているかを返す。
    #[inline]
    #[must_use]
    pub const fn is_force_on_guard_active(&self) -> bool {
        self.ime.recovery.is_force_on_guard_active()
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

    // ── ImeRecoveryState への書き込みメソッド ──

    /// `force_on_broken_app_bootstrap` ガードをセットする。
    #[inline]
    pub const fn set_force_on_broken_app_bootstrap(&mut self) {
        self.ime.recovery.force_on_broken_app_bootstrap = true;
    }

    /// `ime_detect_miss_count` と両強制 ON ガードを同時にリセットする。
    ///
    /// ユーザー操作（Shadow IME トグル・SetOpen 等）で「ユーザーが意図した状態」が
    /// 確定したときに呼ぶ。
    #[inline]
    pub const fn reset_ime_detect_state(&mut self) {
        self.ime.recovery.ime_detect_miss_count = 0;
        self.ime.recovery.force_on_broken_app_bootstrap = false;
        self.ime.recovery.force_on_panic_reset = false;
    }

    /// panic_reset 向け全面リセット。
    ///
    /// belief / recovery のすべてのフィールドをまとめて設定する。
    /// `ime_observations` もクリアして stale な観測値が残らないようにする。
    pub const fn apply_panic_reset(&mut self) {
        self.ime.belief.input_mode = InputModeState::ObservedRomaji;
        self.ime.belief.set_ime_on(true, ShadowSource::PanicReset);
        self.ime.belief.is_japanese_ime = true;
        self.ime.belief.prev_conversion_mode = None;
        self.ime.recovery.ime_detect_miss_count = 0;
        self.ime.recovery.force_on_broken_app_bootstrap = false;
        self.ime.recovery.force_on_panic_reset = true;
        // パニックリセット後は全観測スロットと IME 指示ガードをクリア。
        self.ime.ime_observations.clear_on_focus_change();
        self.ime.ime_intent_guard_until_ms = 0;
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
            // IME 明示指示後のガード中は ObserverPoll / FocusSnapshot が指示と矛盾する値で
            // belief を上書きするのを防ぐ。
            //   - IME OFF 後 (guard_on=false): observer=true をブロック（stale ON で再アクティベート防止）
            //   - IME ON 後 (guard_on=true): observer=false をブロック（LINE 等の処理遅延で即座に非活性化防止）
            // 明示的操作 (SyncKey / PhysicalImeKey / SetOpenRequest, Priority 1-3) はガードを通過させる。
            if val != self.ime.ime_intent_guard_on
                && matches!(src, ShadowSource::ObserverPoll | ShadowSource::FocusSnapshot)
                && self.is_ime_intent_guarded()
            {
                log::debug!(
                    "[ime_intent_guard] belief→{val} blocked (intent={}, src={:?}, remaining={}ms)",
                    self.ime.ime_intent_guard_on,
                    src,
                    self.ime.ime_intent_guard_until_ms
                        .saturating_sub(crate::hook::current_tick_ms()),
                );
                // ガード中にブロックした観測スロットを即座にクリアしてガード失効後の stale flip を防ぐ。
                // IME ON ガード (guard_on=true): false 観測をクリア → +1000ms 後の誤 deactivation を防止。
                // IME OFF ガード (guard_on=false): true 観測をクリア → +1000ms 後の誤 re-activation を防止。
                match src {
                    ShadowSource::ObserverPoll => {
                        log::debug!(
                            "[ime_intent_guard] clear stale observer_poll={val} (guard_on={}, prevent post-guard flip)",
                            self.ime.ime_intent_guard_on,
                        );
                        self.ime.ime_observations.observer_poll = None;
                    }
                    ShadowSource::FocusSnapshot => {
                        log::debug!(
                            "[ime_intent_guard] clear stale focus_probe={val} (guard_on={}, prevent post-guard flip)",
                            self.ime.ime_intent_guard_on,
                        );
                        self.ime.ime_observations.focus_probe = None;
                    }
                    _ => {}
                }
                return;
            }
            log::debug!(
                "[apply-obs] belief update: {}→{} src={:?} guard_on={} guarded={}",
                current, val, src, self.ime.ime_intent_guard_on, self.is_ime_intent_guarded(),
            );
            self.ime.belief.set_ime_on(val, src);
        }
    }

    /// IME 明示指示ガードがアクティブかを返す。
    #[inline]
    fn is_ime_intent_guarded(&self) -> bool {
        let guard = self.ime.ime_intent_guard_until_ms;
        guard > 0 && crate::hook::current_tick_ms() < guard
    }

    /// IME ON 方向のガードがアクティブか（`notify_engine_refresh` の診断用）。
    pub fn is_ime_on_intent_guarded(&self) -> bool {
        self.ime.ime_intent_guard_on && self.is_ime_intent_guarded()
    }

    /// IME OFF 方向のガードがアクティブか（`notify_engine_refresh` の診断用）。
    pub fn is_ime_off_intent_guarded(&self) -> bool {
        !self.ime.ime_intent_guard_on && self.is_ime_intent_guarded()
    }

    /// `observer_poll` スロットに観測値を書き込み、即座に judgement を通す。
    ///
    /// 外部観測（GJI I/O 等）を `belief.ime_on` に反映する正規ルート。
    pub fn write_observer_poll(&mut self, value: bool, ms: u64, user_enabled: bool) {
        self.ime.ime_observations.observer_poll =
            Some(crate::ime_observations::ImeObs { value, ms });
        self.apply_ime_observations(user_enabled);
    }

    /// フォーカス変更時に `ime_observations` の全スロットをクリアする。
    pub const fn clear_ime_observations_on_focus_change(&mut self) {
        self.ime.ime_observations.clear_on_focus_change();
    }

    /// `sync_key` スロットに観測値を書き込み、即座に judgement を通す。
    pub fn write_sync_key(&mut self, value: bool, ms: u64, user_enabled: bool) {
        self.set_ime_intent_guard(value, ms);
        self.ime.ime_observations.sync_key =
            Some(crate::ime_observations::ImeObs { value, ms });
        self.apply_ime_observations(user_enabled);
    }

    /// `physical_key` スロットに観測値を書き込み、即座に judgement を通す。
    pub fn write_physical_key(&mut self, value: bool, ms: u64, user_enabled: bool) {
        self.set_ime_intent_guard(value, ms);
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
        self.set_ime_intent_guard(value, ms);
        self.ime.ime_observations.set_open_request =
            Some(crate::ime_observations::ImeObs { value, ms });
        self.apply_ime_observations(user_enabled);
    }

    /// IME 明示指示ガードをセットする。
    ///
    /// `write_set_open_request` / `write_sync_key` / `write_physical_key` から呼ばれる。
    /// 1000ms 間、ObserverPoll / FocusSnapshot が `value` と矛盾する観測で belief を
    /// 上書きするのをブロックする。
    #[inline]
    fn set_ime_intent_guard(&mut self, value: bool, ms: u64) {
        self.ime.ime_intent_guard_until_ms = ms.saturating_add(1000);
        self.ime.ime_intent_guard_on = value;
        log::debug!(
            "[ime_intent_guard] set (intent={value}, until +1000ms from now)"
        );
    }

    /// `focus_probe` スロットに観測値を書き込み、即座に judgement を通す。
    pub fn write_focus_probe(&mut self, value: bool, ms: u64, user_enabled: bool) {
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

        // miss_count
        if update.increment_miss_count {
            self.ime.recovery.ime_detect_miss_count =
                self.ime.recovery.ime_detect_miss_count.saturating_add(1);
            if self.ime.recovery.ime_detect_miss_count == crate::IME_DETECT_MISS_THRESHOLD {
                log::warn!(
                    "IME detection failed {} consecutive times, will force IME ON",
                    self.ime.recovery.ime_detect_miss_count
                );
            }
        }

        // force_on_broken_app_bootstrap のリセット（検出成功時）
        if update.clear_force_on_broken_app_bootstrap {
            self.ime.recovery.force_on_broken_app_bootstrap = false;
        }

        // force_on_panic_reset と miss_count のリセット（検出成功時）
        if update.clear_force_on_panic_reset {
            self.ime.recovery.force_on_panic_reset = false;
            self.ime.recovery.ime_detect_miss_count = 0;
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
