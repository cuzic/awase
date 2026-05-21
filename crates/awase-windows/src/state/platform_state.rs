use awase::engine::InputModeState;
use awase::types::{AppKind, FocusKind};

use super::preconditions::{Preconditions, ShadowSource};
use super::hook_state::{HookRoutingState, HookConfig, ImeGuardState};

/// Platform 層の全状態を集約する構造体。
///
/// シングルスレッド（メインスレッド＋フックコールバック）からのみアクセスされる。
/// `APP: SingleThreadCell<Runtime>` 経由で保持される。
#[derive(Debug)]
pub struct PlatformState {
    pub preconditions: Preconditions,
    pub hook: HookRoutingState,
    pub hook_config: HookConfig,
    pub focus_kind: FocusKind,
    pub app_kind: AppKind,
    pub last_hook_activity_ms: u64,
    pub hook_event_count: u64,
    pub focus_debounce_ms: u32,
    pub ime_poll_interval_ms: u32,
    pub ime_guard: ImeGuardState,
    /// フォーカス切替直後フラグ。
    ///
    /// フォーカス変更を検知したときに `true` にセットされる。
    /// 次のキーストローク到着時に同期プローブ（高速 IME 状態検出）を実行し、
    /// preconditions を即座に更新してからキーを処理する。
    /// これにより「古い preconditions でキーが処理される」ギャップを解消する。
    pub focus_transition_pending: bool,
    /// 最後にフォアグラウンドプロセスが変わった時刻（ms, GetTickCount 系）。
    /// IME 診断ログで「フォーカス変更からの経過時間」を表示するために使う。
    pub last_focus_change_ms: u64,
    /// OS probe / observe から得た生の IME ON/OFF 観測値（ユーザー意図とは別管理）。
    ///
    /// `fast_ime_probe` や `observe()` の生の結果をここに記録する。
    /// `preconditions.ime_on`（engine_active の判断基準）とは意図的に分離してあり、
    /// `user_enabled=true` のとき OS probe が false を返しても
    /// engine を deactivate しないための根拠として使う。
    pub os_ime_on: Option<bool>,
    /// 各ソースの最新観測値（Phase 2: 観測と判断の分離）。
    ///
    /// `ime_on` の最終値は `ImeObservations::resolve_and_clear()` で一括決定される。
    pub ime_observations: crate::ime_observations::ImeObservations,
}

impl PlatformState {
    /// デフォルト値で初期化する
    pub fn new() -> Self {
        Self {
            preconditions: Preconditions {
                ime_on: true,        // 安全側: ON で初期化
                ime_on_source: ShadowSource::Init,
                input_mode: InputModeState::ObservedRomaji, // デフォルト: ローマ字入力
                is_japanese_ime: true, // デフォルト: 日本語
                prev_conversion_mode: None,
                ime_detect_miss_count: 0,
                ime_force_on_guard: false,
            },
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
            focus_kind: FocusKind::Undetermined,
            app_kind: AppKind::Win32,
            last_hook_activity_ms: 0,
            hook_event_count: 0,
            focus_debounce_ms: 50,
            ime_poll_interval_ms: 500,
            ime_guard: ImeGuardState { active: false, deferred_keys: Vec::new() },
            focus_transition_pending: false,
            last_focus_change_ms: 0,
            os_ime_on: None,
            ime_observations: crate::ime_observations::ImeObservations::default(),
        }
    }
}

impl Default for PlatformState {
    fn default() -> Self {
        Self::new()
    }
}

impl PlatformState {
    /// `ime_observations.resolve_and_clear()` を実行して `preconditions.ime_on` を更新する。
    ///
    /// 各観測ソースが値を書き込んだ直後に呼ぶ。これにより `preconditions.ime_on` は
    /// 常に最新の観測値を反映する。
    pub fn apply_ime_observations(&mut self, user_enabled: bool) {
        let current = self.preconditions.ime_on;
        let is_japanese = self.preconditions.is_japanese_ime;
        if let Some((val, src)) = self.ime_observations.resolve_and_clear(current, user_enabled, is_japanese) {
            self.preconditions.set_ime_on(val, src);
        }
    }
}
