// Win32 API (フック, SendInput, SetTimer 等) の使用に unsafe が必須
#![allow(unsafe_code)]
// Win32 API の型キャスト (usize → i32 等) は OS の ABI 制約により不可避
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    // SingleThreadCell は &self → &mut T を返すが、シングルスレッド保証下で安全
    clippy::mut_from_ref,
    // コールバック型定義が複雑になるのは Win32 API の設計上避けられない
    clippy::type_complexity
)]

//! Windows 固有のプラットフォーム実装クレート。
//!
//! キーボードフック、出力、IME 制御、システムトレイ、フォーカス判定など
//! すべての Win32 API 依存コードを集約する。

pub mod autostart;
pub mod executor;
pub mod focus;
pub mod hook;
pub mod ime;
pub mod observer;
pub mod output;
pub mod platform;
pub mod runtime;
pub mod scanmap;
pub mod single_thread_cell;
pub mod timer;
pub mod tray;
pub mod vk;
pub mod win32;

pub use runtime::{LayoutEntry, Runtime};
pub use single_thread_cell::SingleThreadCell;

use std::sync::atomic::AtomicBool;

use awase::types::{AppKind, FocusKind, RawKeyEvent};

// ── クロススレッド共有グローバル状態 ──
//
// Ctrl+C ハンドラ（別スレッド）からアクセスされるため、Atomic 型でなければならない。

/// メインスレッド ID（Ctrl+C ハンドラから WM_QUIT を送るため）
pub static MAIN_THREAD_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Ctrl+C 受信フラグ
pub static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// 管理者権限フラグ（起動時に設定、メニュー表示で参照）
pub static ELEVATED: AtomicBool = AtomicBool::new(false);

// ── PlatformState: シングルスレッド上の全状態を集約 ──

/// 環境前提条件（IME 状態・入力方式・日本語判定）
#[derive(Debug)]
pub struct Preconditions {
    /// IME が ON か（shadow 追跡含む、Observer ポーリングで実際の OS 状態に収束）
    pub ime_on: bool,
    /// ローマ字入力方式か（false = かな入力、フックはすべてのキーをパススルー）
    pub is_romaji: bool,
    /// 日本語 IME がアクティブか
    pub is_japanese_ime: bool,
    /// 直前の conversion_mode（ROMAN ビット消失によるかな切替検出用）
    pub prev_conversion_mode: u32,
}

/// フックルーティング状態（キーペア追跡・再入ガード）
#[derive(Debug)]
pub struct HookRoutingState {
    /// Engine に送った KeyDown を記録するビットセット（VK 0-255）
    pub sent_to_engine: [u64; 4],
    /// TrackOnly で送った KeyDown を記録するビットセット
    pub track_only_keys: [u64; 4],
    /// 再入ガード
    pub in_callback: bool,
    /// IME 制御コンボ直後の Ctrl バイパス抑制フラグ。
    /// Ctrl+Henkan/Muhenkan 消費後、Ctrl がまだ押されている間の文字キーを
    /// ショートカットとして Bypass しない。Ctrl KeyUp で解除。
    pub suppress_ctrl_bypass: bool,
}

/// フック設定（親指キー VK コード）
#[derive(Debug)]
pub struct HookConfig {
    pub left_thumb_vk: u16,
    pub right_thumb_vk: u16,
}

/// IME 遷移ガード状態（IME トグルキー押下中のキーバッファリング）
#[derive(Debug)]
pub struct ImeGuardState {
    pub active: bool,
    pub deferred_keys: Vec<(RawKeyEvent, awase::engine::input_tracker::PhysicalKeyState)>,
}

/// 修飾キーのフック追跡状態（同時押し判定用）
///
/// `GetAsyncKeyState` はフックコールバック内でタイミングにより
/// Ctrl の押下を検出できないことがある。フックが受け取った
/// KeyDown/KeyUp イベントから独自に追跡することで、
/// Ctrl+Henkan 等のコンボキーを確実に検出する。
#[derive(Debug)]
pub struct ModifierTiming {
    pub ctrl_down: bool,
    pub ctrl_up_tick: u64,
    pub alt_down: bool,
    pub alt_up_tick: u64,
}

impl ModifierTiming {
    /// 猶予期間（ミリ秒）: KeyUp 後この期間内なら「まだ押されている」と判定
    pub const GRACE_MS: u64 = 150;

    pub fn new() -> Self {
        Self { ctrl_down: false, ctrl_up_tick: 0, alt_down: false, alt_up_tick: 0 }
    }

    pub fn is_ctrl_active(&self, now_tick: u64) -> bool {
        self.ctrl_down || now_tick.saturating_sub(self.ctrl_up_tick) < Self::GRACE_MS
    }

    pub fn is_alt_active(&self, now_tick: u64) -> bool {
        self.alt_down || now_tick.saturating_sub(self.alt_up_tick) < Self::GRACE_MS
    }
}

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
    pub modifier_timing: ModifierTiming,
}

impl PlatformState {
    /// デフォルト値で初期化する
    pub fn new() -> Self {
        Self {
            preconditions: Preconditions {
                ime_on: true,        // 安全側: ON で初期化
                is_romaji: true,     // デフォルト: ローマ字入力
                is_japanese_ime: true, // デフォルト: 日本語
                prev_conversion_mode: 0,
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
            modifier_timing: ModifierTiming::new(),
        }
    }
}

impl Default for PlatformState {
    fn default() -> Self {
        Self::new()
    }
}

/// APP グローバル — シングルスレッド専用
pub static APP: SingleThreadCell<Runtime> = SingleThreadCell::new();

/// 統合 IME リフレッシュタイマー ID
///
/// フォーカスデバウンス (50ms) と定期ポーリング (500ms) を統合。
/// `schedule_ime_refresh(delay_ms)` で遅延を指定してリセットする。
/// refresh 完了後に自動的に `ime_poll_interval_ms` で再スケジュールされる。
pub const TIMER_IME_REFRESH: usize = 101;

/// フック消失ウォッチドッグタイマー ID（IME ポーリングとは独立）
pub const TIMER_HOOK_WATCHDOG: usize = 102;

/// 設定リロード用カスタムメッセージ（設定 GUI から `PostMessageW` で送信される）
pub const WM_RELOAD_CONFIG: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 10;

/// IME 制御キー後の遅延キー再処理用カスタムメッセージ
pub const WM_PROCESS_DEFERRED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 11;

/// UIA 非同期判定完了通知用カスタムメッセージ
pub const WM_FOCUS_KIND_UPDATE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 12;

/// フックで IME 制御キーを検出した際の即時キャッシュ更新要求
pub const WM_IME_KEY_DETECTED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 14;

/// フックコールバックからキューされた Effects の実行要求
pub const WM_EXECUTE_EFFECTS: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 15;

/// パニックリセット要求（IME 関連キー連打検出時にフックから PostMessage）
pub const WM_PANIC_RESET: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 16;

/// キーイベントを SendInput で再注入する（IME OFF 時の遅延キー用）
///
/// INJECTED_MARKER 付きなのでフックに再捕捉されない。
///
/// # Safety
/// Win32 API (`send_input_safe`) を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn reinject_key(event: &RawKeyEvent) {
    use crate::output::INJECTED_MARKER;
    use awase::types::KeyEventType;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE,
        VIRTUAL_KEY,
    };

    let is_keyup = matches!(event.event_type, KeyEventType::KeyUp);

    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(event.vk_code.0),
                wScan: event.scan_code.0 as u16,
                dwFlags: if is_keyup {
                    KEYEVENTF_KEYUP | KEYEVENTF_SCANCODE
                } else {
                    KEYEVENTF_SCANCODE
                },
                time: 0,
                dwExtraInfo: INJECTED_MARKER,
            },
        },
    };
    win32::send_input_safe(&[input]);
}
