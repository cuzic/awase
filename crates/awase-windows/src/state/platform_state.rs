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
    pub hook_event_count: u64,
    /// IME 同期キー直後のキー保留バッファ（旧 `ime_gate`）。
    pub sync_key_gate: SyncKeyGate,
    /// 現在のフォーカスアプリに適用されるキーマップルール
    pub active_keymaps: Vec<crate::keymap::CompiledKeymap>,
}

impl PlatformState {
    /// デフォルト値で初期化する
    #[must_use]
    pub fn new() -> Self {
        Self {
            ime: ImeStateHub::new(),
            focus: FocusPlatformState::new(),
            hook: HookRoutingState {
                sent_to_engine: [0u64; 4],
                track_only_keys: [0u64; 4],
                intercept_consumed: [0u64; 4],
                in_callback: false,
                ctrl_bypass_hold: false,
            },
            hook_config: HookConfig {
                left_thumb_vk: 0x1D,  // VK_NONCONVERT
                right_thumb_vk: 0x1C, // VK_CONVERT
            },
            last_hook_activity_ms: 0,
            hook_event_count: 0,
            sync_key_gate: SyncKeyGate::new(),
            active_keymaps: Vec::new(),
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
        // パニックリセット後は全観測スロットをクリア:
        // stale な観測値が次の apply_ime_observations で即座に上書きするのを防ぐ。
        self.ime.ime_observations.clear_on_focus_change();
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
        if let Some((val, src)) = self.ime.ime_observations.resolve_and_clear(current, user_enabled, is_japanese) {
            self.ime.belief.set_ime_on(val, src);
        }
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
        self.ime.ime_observations.sync_key =
            Some(crate::ime_observations::ImeObs { value, ms });
        self.apply_ime_observations(user_enabled);
    }

    /// `physical_key` スロットに観測値を書き込み、即座に judgement を通す。
    pub fn write_physical_key(&mut self, value: bool, ms: u64, user_enabled: bool) {
        self.ime.ime_observations.physical_key =
            Some(crate::ime_observations::ImeObs { value, ms });
        self.apply_ime_observations(user_enabled);
    }

    /// `set_open_request` スロットに観測値を書き込み、即座に judgement を通す。
    pub fn write_set_open_request(&mut self, value: bool, ms: u64, user_enabled: bool) {
        self.ime.ime_observations.set_open_request =
            Some(crate::ime_observations::ImeObs { value, ms });
        self.apply_ime_observations(user_enabled);
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
    /// `false` の起源は別プロファイルでの SetOpenRequest/ObserverPoll であり、
    /// 新ウィンドウの実態と一致する保証がない。日本語レイアウト時のみ実行する。
    pub fn reset_stale_ime_on_for_tsf_native(&mut self) {
        if !self.ime.belief.is_japanese_ime() || self.ime.belief.ime_on() {
            return;
        }
        log::info!(
            "TsfNative entry without cache: reset stale ime_on=false → true \
             (Japanese layout, IME state untrackable in TSF-native)"
        );
        self.ime.belief.set_ime_on(true, ShadowSource::HwndCache);
    }
}
