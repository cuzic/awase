// Windows 専用クレート — 非 Windows では純粋モジュールのみコンパイルされる
// Win32 API (フック, SendInput, SetTimer 等) の使用に unsafe が必須
#![allow(unsafe_code)]
#![warn(unused_qualifications)]
// Win32 API の型キャスト (usize → i32 等) は OS の ABI 制約により不可避
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    // hook.rs 内の局所 SingleThreadCell が &self → &mut T を使用（シングルスレッド保証下で安全）
    clippy::mut_from_ref,
    // コールバック型定義が複雑になるのは Win32 API の設計上避けられない
    clippy::type_complexity
)]

//! Windows 固有のプラットフォーム実装クレート。
//!
//! キーボードフック、出力、IME 制御、システムトレイ、フォーカス判定など
//! すべての Win32 API 依存コードを集約する。
//! 非 Windows では `focus/{cache,class_names}`, `scanmap`, `single_thread_cell`, `tuning`
//! などの純粋モジュールのみコンパイルされる。

// ── 純粋モジュール（全プラットフォーム）──────────────────────────────────────────
pub mod focus;
pub mod scanmap;
pub mod single_thread_cell;
pub mod state;
pub mod tuning;

// ── Windows 専用モジュール ───────────────────────────────────────────────────────
#[cfg(windows)]
pub mod autostart;
#[cfg(windows)]
pub mod hook;
#[cfg(windows)]
pub mod ime;
#[cfg(windows)]
pub(crate) mod ime_controller;
#[cfg(windows)]
pub mod ime_diagnostic;
#[cfg(windows)]
pub(crate) mod imm;
#[cfg(windows)]
pub mod input_defer;
#[cfg(windows)]
pub mod journal;
#[cfg(windows)]
pub mod keymap;
#[cfg(windows)]
pub mod observer;
#[cfg(windows)]
pub mod output;
#[cfg(windows)]
pub mod panic_detect;
#[cfg(windows)]
pub mod platform;
#[cfg(windows)]
pub mod runtime;
#[cfg(windows)]
pub mod timer;
#[cfg(windows)]
pub mod tray;
#[cfg(windows)]
pub mod tsf;
#[cfg(windows)]
pub mod vk;
#[cfg(windows)]
pub mod win32;

#[cfg(windows)]
pub(crate) mod app;
#[cfg(windows)]
pub use app::run;

#[cfg(windows)]
pub use runtime::{LayoutEntry, Runtime};
pub use single_thread_cell::SingleThreadCell;

#[cfg(windows)]
use awase::types::RawKeyEvent;

pub use crate::state::{HookConfig, ImeBelief};
#[cfg(windows)]
pub use crate::state::PlatformState;
pub use crate::tuning::IME_DETECT_MISS_THRESHOLD;

#[cfg(windows)]
pub use crate::tsf::probe_bridge::{OUTPUT_GATE, WM_DRAIN_OUTPUT_QUEUE};

#[cfg(windows)]
pub use crate::input_defer::{InputDeferQueue, INPUT_DEFER};

// ── クロススレッド共有グローバル状態 ──
//
// Ctrl+C ハンドラ（別スレッド）からアクセスされるため、Atomic 型でなければならない。

use std::sync::atomic::{AtomicBool, Ordering};

static MAIN_THREAD_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub fn main_thread_id() -> u32 { MAIN_THREAD_ID.load(Ordering::SeqCst) }
#[cfg(windows)]
pub(crate) fn set_main_thread_id(tid: u32) { MAIN_THREAD_ID.store(tid, Ordering::SeqCst); }

#[cfg(windows)]
static ENGINE_THREAD_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
#[cfg(windows)]
pub(crate) fn engine_thread_id() -> u32 { ENGINE_THREAD_ID.load(Ordering::Relaxed) }
#[cfg(windows)]
pub(crate) fn set_engine_thread_id(tid: u32) { ENGINE_THREAD_ID.store(tid, Ordering::Relaxed); }

static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);
pub fn is_quit_requested() -> bool { QUIT_REQUESTED.load(Ordering::SeqCst) }
#[cfg(windows)]
pub(crate) fn request_quit() { QUIT_REQUESTED.store(true, Ordering::SeqCst); }

static ELEVATED: AtomicBool = AtomicBool::new(false);
pub fn is_elevated() -> bool { ELEVATED.load(Ordering::Relaxed) }
#[cfg(windows)]
pub(crate) fn set_elevated(v: bool) { ELEVATED.store(v, Ordering::Relaxed); }

/// raw TSF literal 検出後の回収ペイロード。
///
/// バックスペース数とローマ字再送文字列を一括管理する。
/// WM_DRAIN_OUTPUT_QUEUE ハンドラが `flush_raw_tsf_literal_recovery()` で消費する。
#[cfg(windows)]
#[derive(Debug)]
pub struct RawTsfLiteralPending {
    /// 送信すべきバックスペースの数
    pub(crate) backs: std::sync::atomic::AtomicUsize,
    /// 再送すべきローマ字文字列（空文字列 = 再送なし）
    pub(crate) romaji: std::sync::Mutex<String>,
}

#[cfg(windows)]
impl RawTsfLiteralPending {
    const fn new() -> Self {
        Self {
            backs: std::sync::atomic::AtomicUsize::new(0),
            romaji: std::sync::Mutex::new(String::new()),
        }
    }

    /// バックスペース数とローマ字を一括セットする。
    ///
    /// # Panics
    /// Mutex が poison された場合（通常発生しない）。
    pub fn set_pending(&self, backs: usize, romaji: String) {
        use std::sync::atomic::Ordering::Relaxed;
        self.backs.store(backs, Relaxed);
        *self.romaji.lock().unwrap() = romaji;
    }

    /// バックスペース数とローマ字を一括取り出しする（backs は 0 にリセット、romaji は空にリセット）。
    ///
    /// # Panics
    /// Mutex が poison された場合（通常発生しない）。
    pub fn take_pending(&self) -> (usize, String) {
        use std::sync::atomic::Ordering::Relaxed;
        let backs = self.backs.swap(0, Relaxed);
        let romaji = std::mem::take(&mut *self.romaji.lock().unwrap());
        (backs, romaji)
    }
}

#[cfg(windows)]
pub static RAW_TSF_LITERAL: RawTsfLiteralPending = RawTsfLiteralPending::new();

/// RUNTIME グローバル — シングルスレッド専用
#[cfg(windows)]
pub static RUNTIME: SingleThreadCell<Runtime> = SingleThreadCell::new();

/// `RUNTIME` グローバルへの集約アクセスポイント。
///
/// `RefCell` の実行時借用チェックにより再入を安全に検出する。
/// 再入を検出した場合は `log::warn!` を出力して `None` を返す（UB なし）。
#[cfg(windows)]
#[must_use = "再入時は None を返す。消えてはいけないメッセージには with_app_or_repost を、\
意図的に捨てる場合は `let _ = with_app(...)` を使うこと"]
pub fn with_app<R>(f: impl FnOnce(&mut Runtime) -> R) -> Option<R> {
    RUNTIME.try_borrow_mut().map_or_else(
        || {
            log::warn!(
                "with_app re-entry detected — returning None (caller should re-post if needed)"
            );
            None
        },
        |mut guard| guard.as_mut().map(f),
    )
}

/// `RUNTIME` グローバルへの読み取り専用アクセスファサード。
#[cfg(windows)]
pub fn with_app_ref<R>(f: impl FnOnce(&Runtime) -> R) -> Option<R> {
    RUNTIME.with(f)
}

/// `with_app` を呼び、再入で `None` が返った場合は `msg` を自スレッドのキューに再 post する。
#[cfg(windows)]
pub fn with_app_or_repost(msg: u32, f: impl FnOnce(&mut Runtime)) {
    if with_app(f).is_none() {
        win32::post_to_main_thread(msg);
    }
}

/// `with_app_or_repost` の wparam / lparam 付きバリアント。
#[cfg(windows)]
pub fn with_app_or_repost_with(
    msg: u32,
    wparam: usize,
    lparam: isize,
    f: impl FnOnce(&mut Runtime),
) {
    if with_app(f).is_none() {
        win32::post_to_main_thread_with(msg, wparam, lparam);
    }
}

// ── タイマー ID 定数（純粋 usize、全プラットフォーム）─────────────────────────────

/// 統合 IME リフレッシュタイマー ID
pub const TIMER_IME_REFRESH: usize = 101;
/// フック消失ウォッチドッグタイマー ID
pub const TIMER_HOOK_WATCHDOG: usize = 102;
/// スリープ復帰 / セッションアンロック後の遅延リカバリタイマー ID
pub const TIMER_POWER_RESUME: usize = 103;
/// ReinjectKey の output guard 解除待ちタイマー ID
pub const TIMER_OUTPUT_GUARD: usize = 104;
/// TSF ウォームアッププローブのポーリングタイマー ID
pub const TIMER_TSF_PROBE: usize = 105;
/// TsfGate の PendingWarmup フォールバックタイマー ID
pub const TIMER_TSF_GATE: usize = 106;
/// Ctrl+無変換 IME OFF ミスタイプ救済の先読みタイマー ID
pub const TIMER_IME_OFF_RESCUE: usize = 107;
/// GjiFsm の LongIdle タイムアウトタイマー ID
pub const TIMER_GJI_LONG_IDLE: usize = 108;

// ── Windows メッセージ定数 ──────────────────────────────────────────────────────

/// `WM_FOCUS_KIND_UPDATE` の wParam 上位 8bit が「AppKind 不明」を示すセンチネル値。
pub const FOCUS_KIND_UPDATE_NO_APP_KIND: u8 = 0xFF;

/// 設定リロード用カスタムメッセージ
#[cfg(windows)]
pub const WM_RELOAD_CONFIG: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 10;
/// IME 制御キー後の遅延キー再処理用カスタムメッセージ
#[cfg(windows)]
pub const WM_PROCESS_DEFERRED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 11;
/// UIA 非同期判定完了通知用カスタムメッセージ
#[cfg(windows)]
pub const WM_FOCUS_KIND_UPDATE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 12;
/// フックで IME 制御キーを検出した際の即時キャッシュ更新要求
#[cfg(windows)]
pub const WM_IME_KEY_DETECTED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 14;
/// フックコールバックからキューされた Effects の実行要求
#[cfg(windows)]
pub const WM_EXECUTE_EFFECTS: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 15;
/// パニックリセット要求
#[cfg(windows)]
pub const WM_PANIC_RESET: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 16;
/// 多重起動検出時の通知
#[cfg(windows)]
pub const WM_DUPLICATE_INSTANCE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 17;
/// フックスレッドからエンジンスレッドへのキーイベント転送メッセージ
#[cfg(windows)]
pub const WM_KEY_FROM_HOOK: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 19;
/// ジャーナルダンプ要求
#[cfg(windows)]
pub const WM_DUMP_JOURNAL: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 20;
/// IME 種別変化通知
#[cfg(windows)]
pub const WM_IME_KIND_CHANGED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 21;

// ── RawKeyEventExt ───────────────────────────────────────────────────────────────

/// `RawKeyEvent` の SendInput 再注入ヘルパー。
#[cfg(windows)]
pub trait RawKeyEventExt {
    /// キーイベントを SendInput で再注入する（IME OFF 時の遅延キー用）。
    ///
    /// # Safety
    /// Win32 API (`send_input_safe`) を呼び出す。メインスレッドから呼ぶこと。
    unsafe fn reinject(&self);
}

#[cfg(windows)]
impl RawKeyEventExt for RawKeyEvent {
    unsafe fn reinject(&self) {
        use crate::output::INJECTED_MARKER;
        use awase::types::KeyEventType;
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
            VIRTUAL_KEY,
        };

        let is_keyup = matches!(self.event_type, KeyEventType::KeyUp);

        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(self.vk_code.0),
                    wScan: 0,
                    dwFlags: if is_keyup {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                    time: 0,
                    dwExtraInfo: INJECTED_MARKER,
                },
            },
        };
        let _ = win32::send_input_safe(&[input]);
    }
}
