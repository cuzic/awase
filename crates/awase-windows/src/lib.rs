// Windows 専用クレート — 非 Windows では空クレートとしてコンパイルされる
#![cfg(windows)]
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
pub mod ime_diagnostic;
pub mod ime_observations;
pub mod observer;
pub mod output;
pub mod platform;
pub mod runtime;
pub mod scanmap;
pub(crate) mod state;
pub(crate) mod timing;
pub mod timer;
pub mod tray;
pub mod tsf;
pub mod vk;
pub mod win32;

pub use runtime::{LayoutEntry, Runtime};
pub use win32_async::SingleThreadCell;

use std::sync::atomic::AtomicBool;

use awase::types::RawKeyEvent;

pub use crate::state::{
    HookConfig, HookRoutingState, ImeForceOnGuard, ImeGuardState, IME_DETECT_MISS_THRESHOLD,
    Preconditions, PlatformState, ShadowSource,
};

pub use crate::tsf::probe_bridge::{
    OUTPUT_ACTIVE, OUTPUT_PENDING_QUEUE, WM_DRAIN_OUTPUT_QUEUE, post_drain_output_queue,
};

// ── クロススレッド共有グローバル状態 ──
//
// Ctrl+C ハンドラ（別スレッド）からアクセスされるため、Atomic 型でなければならない。

/// メインスレッド ID（Ctrl+C ハンドラから WM_QUIT を送るため）
pub static MAIN_THREAD_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Ctrl+C 受信フラグ
pub static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// 管理者権限フラグ（起動時に設定、メニュー表示で参照）
pub static ELEVATED: AtomicBool = AtomicBool::new(false);

// OBS_* グローバルは tsf/observer.rs で定義。後方互換のため re-export する。
pub use crate::tsf::observer::{
    COMPOSITION_PROBE_SEQ, OBS_FOCUS_NAMECHANGE_SEQ, OBS_GJI_CANDIDATE_SHOW_SEQ,
    OBS_GJI_CANDIDATE_VISIBLE,
};

/// raw TSF literal 検出後の回収ペイロード。
///
/// バックスペース数とローマ字再送文字列を一括管理する。
/// WM_DRAIN_OUTPUT_QUEUE ハンドラが `flush_raw_tsf_literal_recovery()` で消費する。
#[derive(Debug)]
pub struct RawTsfLiteralPending {
    /// 送信すべきバックスペースの数
    pub backs: std::sync::atomic::AtomicUsize,
    /// 再送すべきローマ字文字列（空文字列 = 再送なし）
    pub romaji: std::sync::Mutex<String>,
}

impl RawTsfLiteralPending {
    const fn new() -> Self {
        Self {
            backs: std::sync::atomic::AtomicUsize::new(0),
            romaji: std::sync::Mutex::new(String::new()),
        }
    }
}

pub static RAW_TSF_LITERAL: RawTsfLiteralPending = RawTsfLiteralPending::new();

// ── PlatformState: シングルスレッド上の全状態を集約 ──
// PlatformState は crate::state::platform_state に移動済み。re-export は上部の pub use で行う。

/// APP グローバル — シングルスレッド専用
pub static APP: SingleThreadCell<Runtime> = SingleThreadCell::new();

/// `APP` グローバルへの集約アクセスポイント。
///
/// `APP.get_mut()` の呼び出しをすべてここに集約し、unsafe 契約を一元管理する。
/// 呼び出し側では `unsafe` ブロックが不要になる。
///
/// # Safety (module-level contract)
/// awase-windows はすべての呼び出しが Windows メッセージループスレッドからのみ行われる。
/// この保証により `SingleThreadCell::with_mut` の unsafe 要件が満たされる。
pub fn with_app<R>(f: impl FnOnce(&mut Runtime) -> R) -> Option<R> {
    // Safety: Windows メッセージループはシングルスレッドである
    unsafe { APP.with_mut(f) }
}

/// `APP` グローバルへの読み取り専用アクセスファサード。
pub fn with_app_ref<R>(f: impl FnOnce(&Runtime) -> R) -> Option<R> {
    // Safety: Windows メッセージループはシングルスレッドである
    unsafe { APP.with(f) }
}

/// 統合 IME リフレッシュタイマー ID
///
/// フォーカスデバウンス (50ms) と定期ポーリング (500ms) を統合。
/// `schedule_ime_refresh(delay_ms)` で遅延を指定してリセットする。
/// refresh 完了後に自動的に `ime_poll_interval_ms` で再スケジュールされる。
pub const TIMER_IME_REFRESH: usize = 101;

/// フック消失ウォッチドッグタイマー ID（IME ポーリングとは独立）
pub const TIMER_HOOK_WATCHDOG: usize = 102;

/// スリープ復帰 / セッションアンロック後の遅延リカバリタイマー ID
///
/// 復帰直後は OS や IME サービスがまだ回復途中で、ブロッキング Win32 API が
/// メッセージループをハングさせる恐れがある。2秒遅延して安全に復帰処理を行う。
pub const TIMER_POWER_RESUME: usize = 103;

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

/// 多重起動検出時に新インスタンスから既存インスタンスに送る通知
///
/// 既存インスタンスはこのメッセージを受けるとタスクトレイにバルーン通知を表示し、
/// ユーザーに「すでに起動している」ことを知らせる。
pub const WM_DUPLICATE_INSTANCE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 17;

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
