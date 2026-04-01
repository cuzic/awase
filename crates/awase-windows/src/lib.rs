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

use std::sync::atomic::{AtomicBool, AtomicU8};

use awase::types::RawKeyEvent;

// ── クロススレッド共有グローバル状態 ──
//
// フック（メインスレッド）とメッセージループ間、または Ctrl+C ハンドラ（別スレッド）
// からアクセスされるため、Atomic 型でなければならない。

/// フォーカス中コントロールの種別キャッシュ（Undetermined=2 で初期化）
pub static FOCUS_KIND: AtomicU8 = AtomicU8::new(2); // FocusKind::Undetermined

/// キャッシュされた IME ON/OFF 状態。0=OFF, 1=ON, 2=Unknown（初期状態）
pub static IME_STATE_CACHE: AtomicU8 = AtomicU8::new(2);

/// IME 検出の信頼度キャッシュ（UIA 非同期判定で更新）
pub static IME_RELIABILITY: AtomicU8 = AtomicU8::new(2); // ImeReliability::Unknown

/// メインスレッド ID（Ctrl+C ハンドラから WM_QUIT を送るため）
pub static MAIN_THREAD_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Ctrl+C 受信フラグ
pub static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// 管理者権限フラグ（起動時に設定、メニュー表示で参照）
pub static ELEVATED: AtomicBool = AtomicBool::new(false);

/// フォーカス遷移デバウンス時間（ミリ秒、config から初期化）
pub static FOCUS_DEBOUNCE_MS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(50);

/// IME 状態ポーリング間隔（ミリ秒、config から初期化）
pub static IME_POLL_INTERVAL_MS: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(500);

/// APP グローバル — シングルスレッド専用
pub static APP: SingleThreadCell<Runtime> = SingleThreadCell::new();

/// フォーカス遷移デバウンスタイマー ID
pub const TIMER_FOCUS_DEBOUNCE: usize = 103;

/// IME 状態ポーリング用タイマー ID（安全ネット: マウスで言語バー操作した場合等）
pub const TIMER_IME_POLL: usize = 101;

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
