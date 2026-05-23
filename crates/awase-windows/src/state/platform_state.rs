use awase::engine::InputModeState;
use awase::types::{AppKind, FocusKind};

use super::preconditions::{Preconditions, ShadowSource};
use super::hook_state::{HookRoutingState, HookConfig, ImeGuardState};

// ────────────────────────────────────────────────────────────────────────────
// ImeStateHub
// ────────────────────────────────────────────────────────────────────────────

/// IME 観測・判断・preconditions 書き戻しを担う凝集ユニット。
///
/// `PlatformState` から IME 関連フィールドを切り出すことで、
/// 「観測」「フォーカス状態」「フック設定」の混在を解消する。
#[derive(Debug)]
pub(crate) struct ImeStateHub {
    /// 環境前提条件（IME 状態・入力方式・日本語判定）
    pub(crate) preconditions: Preconditions,
    /// OS probe / observe から得た生の IME ON/OFF 観測値（ユーザー意図とは別管理）。
    ///
    /// `fast_ime_probe` や `observe()` の生の結果をここに記録する。
    /// `preconditions.ime_on`（engine_active の判断基準）とは意図的に分離してあり、
    /// `user_enabled=true` のとき OS probe が false を返しても
    /// engine を deactivate しないための根拠として使う。
    pub(crate) os_ime_on: Option<bool>,
    /// 各ソースの最新観測値（Phase 2: 観測と判断の分離）。
    ///
    /// `ime_on` の最終値は `ImeObservations::resolve_and_clear()` で一括決定される。
    pub(crate) ime_observations: crate::ime_observations::ImeObservations,
}

impl ImeStateHub {
    /// デフォルト値で初期化する。
    pub(crate) fn new() -> Self {
        Self {
            preconditions: Preconditions {
                ime_on: true,        // 安全側: ON で初期化
                ime_on_source: ShadowSource::Init,
                input_mode: InputModeState::ObservedRomaji, // デフォルト: ローマ字入力
                is_japanese_ime: true, // デフォルト: 日本語
                prev_conversion_mode: None,
                ime_detect_miss_count: 0,
                force_on_broken_app_bootstrap: false,
                force_on_panic_reset: false,
            },
            os_ime_on: None,
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
pub(crate) struct FocusPlatformState {
    pub(crate) app_kind: AppKind,
    pub(crate) focus_kind: FocusKind,
    /// フォーカス切替直後フラグ。
    ///
    /// フォーカス変更を検知したときに `true` にセットされる。
    /// 次のキーストローク到着時に同期プローブ（高速 IME 状態検出）を実行し、
    /// preconditions を即座に更新してからキーを処理する。
    /// これにより「古い preconditions でキーが処理される」ギャップを解消する。
    pub(crate) focus_transition_pending: bool,
    /// 最後にフォアグラウンドプロセスが変わった時刻（ms, GetTickCount 系）。
    /// IME 診断ログで「フォーカス変更からの経過時間」を表示するために使う。
    pub(crate) last_focus_change_ms: u64,
    pub(crate) focus_debounce_ms: u32,
    pub(crate) ime_poll_interval_ms: u32,
}

impl FocusPlatformState {
    fn new() -> Self {
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
    /// IME 観測・判断・preconditions 書き戻しを担う凝集ユニット。
    pub(crate) ime: ImeStateHub,
    /// フォーカス追跡に関する状態を集約するユニット。
    pub(crate) focus: FocusPlatformState,
    pub hook: HookRoutingState,
    pub hook_config: HookConfig,
    pub last_hook_activity_ms: u64,
    pub hook_event_count: u64,
    pub ime_guard: ImeGuardState,
}

impl PlatformState {
    /// デフォルト値で初期化する
    pub fn new() -> Self {
        Self {
            ime: ImeStateHub::new(),
            focus: FocusPlatformState::new(),
            hook: HookRoutingState {
                sent_to_engine: [0u64; 4],
                track_only_keys: [0u64; 4],
                in_callback: false,
                suppress_ctrl_bypass: false,
            },
            hook_config: HookConfig {
                left_thumb_vk: 0x1D,  // VK_NONCONVERT
                right_thumb_vk: 0x1C, // VK_CONVERT
            },
            last_hook_activity_ms: 0,
            hook_event_count: 0,
            ime_guard: ImeGuardState { active: false, deferred_keys: Vec::new() },
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

    /// `preconditions` への共有参照を返す。
    ///
    /// `build_input_context(&ps.preconditions(), …)` のような呼び出し用。
    #[inline]
    pub(crate) fn preconditions(&self) -> &Preconditions {
        &self.ime.preconditions
    }

    /// `os_ime_on` を設定する。
    #[inline]
    pub(crate) fn set_os_ime_on(&mut self, value: Option<bool>) {
        self.ime.os_ime_on = value;
    }

    // ── Preconditions への pub(crate) 読み取りパススルー ──

    /// IME が ON かどうかを返す。
    #[inline]
    pub(crate) fn ime_on(&self) -> bool {
        self.ime.preconditions.ime_on()
    }

    /// `ime_on` を最後に更新したソースを返す。
    #[inline]
    pub(crate) fn ime_on_source(&self) -> ShadowSource {
        self.ime.preconditions.ime_on_source()
    }

    /// 入力モードを返す。
    #[inline]
    pub(crate) fn input_mode(&self) -> InputModeState {
        self.ime.preconditions.input_mode()
    }

    /// 日本語 IME がアクティブかを返す。
    #[inline]
    pub(crate) fn is_japanese_ime(&self) -> bool {
        self.ime.preconditions.is_japanese_ime()
    }

    /// 直前の conversion_mode を返す。
    #[inline]
    pub(crate) fn prev_conversion_mode(&self) -> Option<u32> {
        self.ime.preconditions.prev_conversion_mode()
    }

    /// IME 状態検出の連続失敗回数を返す。
    #[inline]
    pub(crate) fn ime_detect_miss_count(&self) -> u32 {
        self.ime.preconditions.ime_detect_miss_count()
    }

    /// いずれかの強制 ON ガードが立っているかを返す。
    #[inline]
    pub(crate) fn is_force_on_guard_active(&self) -> bool {
        self.ime.preconditions.is_force_on_guard_active()
    }

    // ── Preconditions への書き込みメソッド（`apply_*` / `write_*`）──

    /// `ime_on` を設定する（ソース付き）。
    #[inline]
    pub(crate) fn set_ime_on(&mut self, value: bool, source: ShadowSource) {
        self.ime.preconditions.set_ime_on(value, source);
    }

    /// `input_mode` を設定する。
    #[inline]
    pub(crate) fn set_input_mode(&mut self, mode: InputModeState) {
        self.ime.preconditions.input_mode = mode;
    }

    /// `is_japanese_ime` を設定する。
    #[inline]
    pub(crate) fn set_is_japanese_ime(&mut self, value: bool) {
        self.ime.preconditions.is_japanese_ime = value;
    }

    /// `prev_conversion_mode` を設定する。
    #[inline]
    pub(crate) fn set_prev_conversion_mode(&mut self, value: Option<u32>) {
        self.ime.preconditions.prev_conversion_mode = value;
    }

    /// `ime_detect_miss_count` をインクリメントする（saturating）。
    #[inline]
    pub(crate) fn increment_ime_detect_miss_count(&mut self) {
        self.ime.preconditions.ime_detect_miss_count =
            self.ime.preconditions.ime_detect_miss_count.saturating_add(1);
    }

    /// `ime_detect_miss_count` を 0 にリセットする。
    #[inline]
    pub(crate) fn reset_ime_detect_miss_count(&mut self) {
        self.ime.preconditions.ime_detect_miss_count = 0;
    }

    /// `force_on_broken_app_bootstrap` ガードをセットする。
    #[inline]
    pub(crate) fn set_force_on_broken_app_bootstrap(&mut self) {
        self.ime.preconditions.force_on_broken_app_bootstrap = true;
    }

    /// `ime_detect_miss_count` と両強制 ON ガードを同時にリセットする。
    ///
    /// ユーザー操作（Shadow IME トグル・SetOpen 等）で「ユーザーが意図した状態」が
    /// 確定したときに呼ぶ。
    #[inline]
    pub(crate) fn reset_ime_detect_state(&mut self) {
        self.ime.preconditions.ime_detect_miss_count = 0;
        self.ime.preconditions.force_on_broken_app_bootstrap = false;
        self.ime.preconditions.force_on_panic_reset = false;
    }

    /// panic_reset 向け全面リセット。
    ///
    /// input_mode / ime_on / is_japanese_ime / prev_conversion_mode /
    /// ime_detect_miss_count / force_on_broken_app_bootstrap / force_on_panic_reset をまとめて設定する。
    /// `ime_observations` もクリアして stale な観測値が残らないようにする。
    pub(crate) fn apply_panic_reset(&mut self) {
        self.ime.preconditions.input_mode = InputModeState::ObservedRomaji;
        self.ime.preconditions.set_ime_on(true, ShadowSource::PanicReset);
        self.ime.preconditions.is_japanese_ime = true;
        self.ime.preconditions.prev_conversion_mode = None;
        self.ime.preconditions.ime_detect_miss_count = 0;
        self.ime.preconditions.force_on_broken_app_bootstrap = false;
        self.ime.preconditions.force_on_panic_reset = true;
        // パニックリセット後は全観測スロットをクリア:
        // stale な観測値が次の apply_ime_observations で即座に上書きするのを防ぐ。
        self.ime.ime_observations.clear_on_focus_change();
    }
}

impl PlatformState {
    /// `ime_observations.resolve_and_clear()` を実行して `preconditions.ime_on` を更新する。
    ///
    /// 各観測ソースが値を書き込んだ直後に呼ぶ。これにより `preconditions.ime_on` は
    /// 常に最新の観測値を反映する。
    pub fn apply_ime_observations(&mut self, user_enabled: bool) {
        let current = self.ime.preconditions.ime_on;
        let is_japanese = self.ime.preconditions.is_japanese_ime;
        if let Some((val, src)) = self.ime.ime_observations.resolve_and_clear(current, user_enabled, is_japanese) {
            self.ime.preconditions.set_ime_on(val, src);
        }
    }

    /// `observer_poll` スロットに観測値を書き込み、即座に judgement を通す。
    ///
    /// 外部観測（GJI I/O 等）を `preconditions.ime_on` に反映する正規ルート。
    /// 直接 `ime_observations.observer_poll` に書くのではなくこのメソッドを使うことで、
    /// 「書いたら必ず judgement が走る」不変条件が保たれる。
    pub fn write_observer_poll(&mut self, value: bool, ms: u64, user_enabled: bool) {
        self.ime.ime_observations.observer_poll =
            Some(crate::ime_observations::ImeObs { value, ms });
        self.apply_ime_observations(user_enabled);
    }

    /// `observer_poll` スロットの最新観測値を読み取る。
    ///
    /// `Some(value)` = 最後に観測された値。`None` = 未観測。
    /// `is_japanese_ime` との AND 結果を `os_ime_on` 計算に使う。
    pub(crate) fn observer_poll_value(&self) -> Option<bool> {
        self.ime.ime_observations.observer_poll.as_ref().map(|obs| obs.value)
    }

    /// フォーカス変更時に `ime_observations` の全スロットをクリアする。
    ///
    /// `ImeObservations::clear_on_focus_change()` への委譲ラッパー。
    pub(crate) fn clear_ime_observations_on_focus_change(&mut self) {
        self.ime.ime_observations.clear_on_focus_change();
    }

    /// `sync_key` スロットに観測値を書き込む。
    ///
    /// 書き込み後に `apply_ime_observations` を呼ぶことで judgement を走らせること。
    pub(crate) fn write_sync_key(&mut self, value: bool, ms: u64) {
        self.ime.ime_observations.sync_key =
            Some(crate::ime_observations::ImeObs { value, ms });
    }

    /// `physical_key` スロットに観測値を書き込む。
    ///
    /// 書き込み後に `apply_ime_observations` を呼ぶことで judgement を走らせること。
    pub(crate) fn write_physical_key(&mut self, value: bool, ms: u64) {
        self.ime.ime_observations.physical_key =
            Some(crate::ime_observations::ImeObs { value, ms });
    }

    /// `set_open_request` スロットに観測値を書き込む。
    ///
    /// 書き込み後に `apply_ime_observations` を呼ぶことで judgement を走らせること。
    pub(crate) fn write_set_open_request(&mut self, value: bool, ms: u64) {
        self.ime.ime_observations.set_open_request =
            Some(crate::ime_observations::ImeObs { value, ms });
    }

    /// `focus_probe` スロットに観測値を書き込む。
    ///
    /// 書き込み後に `apply_ime_observations` を呼ぶことで judgement を走らせること。
    pub(crate) fn write_focus_probe(&mut self, value: bool, ms: u64) {
        self.ime.ime_observations.focus_probe =
            Some(crate::ime_observations::ImeObs { value, ms });
    }

    /// `ImeObserverOutput` を `Preconditions` と `ImeObservations` に反映する。
    ///
    /// `observer::ime_observer::observe()` / `apply_snapshot()` の結果を受け取り、
    /// `Preconditions` への書き込みをここに集約する。
    pub fn apply_ime_observer_output(
        &mut self,
        out: &crate::observer::ime_observer::ImeObserverOutput,
    ) {
        // is_japanese_ime: 検出成功時のみ更新
        if let Some(is_jp) = out.is_japanese_ime {
            self.ime.preconditions.is_japanese_ime = is_jp;
        }

        // observer_poll スロット
        if let Some(obs) = out.observer_poll {
            self.ime.ime_observations.observer_poll = Some(obs);
        }

        // miss_count
        if out.increment_miss_count {
            self.ime.preconditions.ime_detect_miss_count =
                self.ime.preconditions.ime_detect_miss_count.saturating_add(1);
            if self.ime.preconditions.ime_detect_miss_count == crate::IME_DETECT_MISS_THRESHOLD {
                log::warn!(
                    "IME detection failed {} consecutive times, will force IME ON",
                    self.ime.preconditions.ime_detect_miss_count
                );
            }
        }

        // force_on_guard のリセット（検出成功時）
        if out.clear_force_on_guard {
            self.ime.preconditions.ime_detect_miss_count = 0;
            self.ime.preconditions.force_on_broken_app_bootstrap = false;
            self.ime.preconditions.force_on_panic_reset = false;
        }

        // input_mode
        if let Some(mode) = out.new_input_mode {
            self.ime.preconditions.input_mode = mode;
        }

        // prev_conversion_mode
        if let Some(conv) = out.new_prev_conversion_mode {
            self.ime.preconditions.prev_conversion_mode = Some(conv);
        }

        log::debug!(
            "IME snapshot: japanese={:?} ime_on={:?} romaji={:?} conv={:?} guard={}",
            out.snap_is_japanese_ime,
            out.snap_ime_on,
            out.snap_is_romaji,
            out.snap_conversion_mode.map(|v| format!("0x{v:08X}")),
            out.guard_was_active,
        );
    }

    /// `hwnd_cache::restore_on_focus_enter()` の結果を `Preconditions` に反映する。
    ///
    /// キャッシュヒット（`Some`）の場合のみ適用する。`None` の場合は何もしない。
    pub fn apply_hwnd_cache_restore(
        &mut self,
        snapshot: Option<crate::focus::hwnd_cache::HwndImeSnapshot>,
    ) {
        if let Some(snap) = snapshot {
            self.ime.preconditions.set_ime_on(snap.ime_on, ShadowSource::HwndCache);
            self.ime.preconditions.input_mode = snap.input_mode;
        }
    }
}
