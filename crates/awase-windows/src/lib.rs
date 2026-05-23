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
pub(crate) mod ime_controller;
pub mod ime_diagnostic;
pub(crate) mod imm;
pub mod ime_observations;
pub mod input_defer;
pub mod observer;
pub mod output;
pub mod platform;
pub mod runtime;
pub mod scanmap;
pub mod single_thread_cell;
pub(crate) mod state;
pub(crate) mod timing;
pub mod timer;
pub mod tuning;
pub mod tray;
pub mod tsf;
pub mod vk;
pub mod win32;

pub use runtime::{LayoutEntry, Runtime};
pub use single_thread_cell::SingleThreadCell;

use std::sync::atomic::AtomicBool;

use awase::types::RawKeyEvent;

pub use crate::state::{
    HookConfig, HookRoutingState, ImeGuardState,
    Preconditions, PlatformState, ShadowSource,
};
pub use crate::tuning::IME_DETECT_MISS_THRESHOLD;

pub use crate::tsf::probe_bridge::{OUTPUT_GATE, WM_DRAIN_OUTPUT_QUEUE};

pub use crate::input_defer::{INPUT_DEFER, InputDeferQueue};

// ── クロススレッド共有グローバル状態 ──
//
// Ctrl+C ハンドラ（別スレッド）からアクセスされるため、Atomic 型でなければならない。

/// メインスレッド ID（Ctrl+C ハンドラから WM_QUIT を送るため）
pub static MAIN_THREAD_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Ctrl+C 受信フラグ
pub static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// 管理者権限フラグ（起動時に設定、メニュー表示で参照）
pub static ELEVATED: AtomicBool = AtomicBool::new(false);

// TSF_OBS は tsf/observer.rs で定義。クレート内 re-export。
// 外部クレートからの直接参照が不要なため pub(crate) とする。
pub(crate) use crate::tsf::observer::TSF_OBS;
pub(crate) use crate::tsf::observer::with_tsf_obs;

/// raw TSF literal 検出後の回収ペイロード。
///
/// バックスペース数とローマ字再送文字列を一括管理する。
/// WM_DRAIN_OUTPUT_QUEUE ハンドラが `flush_raw_tsf_literal_recovery()` で消費する。
#[derive(Debug)]
pub struct RawTsfLiteralPending {
    /// 送信すべきバックスペースの数
    pub(crate) backs: std::sync::atomic::AtomicUsize,
    /// 再送すべきローマ字文字列（空文字列 = 再送なし）
    pub(crate) romaji: std::sync::Mutex<String>,
}

impl RawTsfLiteralPending {
    const fn new() -> Self {
        Self {
            backs: std::sync::atomic::AtomicUsize::new(0),
            romaji: std::sync::Mutex::new(String::new()),
        }
    }

    /// バックスペース数とローマ字を一括セットする。
    pub fn set_pending(&self, backs: usize, romaji: String) {
        use std::sync::atomic::Ordering::Relaxed;
        self.backs.store(backs, Relaxed);
        *self.romaji.lock().unwrap() = romaji;
    }

    /// バックスペース数とローマ字を一括取り出しする（backs は 0 にリセット、romaji は空にリセット）。
    pub fn take_pending(&self) -> (usize, String) {
        use std::sync::atomic::Ordering::Relaxed;
        let backs = self.backs.swap(0, Relaxed);
        let romaji = std::mem::take(&mut *self.romaji.lock().unwrap());
        (backs, romaji)
    }
}

pub static RAW_TSF_LITERAL: RawTsfLiteralPending = RawTsfLiteralPending::new();

// ── PlatformState: シングルスレッド上の全状態を集約 ──
// PlatformState は crate::state::platform_state に移動済み。re-export は上部の pub use で行う。

/// APP グローバル — シングルスレッド専用
pub static APP: SingleThreadCell<Runtime> = SingleThreadCell::new();

// with_app の再入検出フラグ。
// ネストした GetMessageA ループ（block_on 等）経由での再入を検出し、UB を回避する。
std::thread_local! {
    static IN_WITH_APP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// `APP` グローバルへの集約アクセスポイント。
///
/// `APP.get_mut()` の呼び出しをすべてここに集約し、unsafe 契約を一元管理する。
/// 呼び出し側では `unsafe` ブロックが不要になる。
///
/// 再入を検出した場合は `log::error!` を出力して `None` を返す（UB を回避）。
///
/// # Safety (module-level contract)
/// awase-windows はすべての呼び出しが Windows メッセージループスレッドからのみ行われる。
/// この保証により `SingleThreadCell::with_mut` の unsafe 要件が満たされる。
pub fn with_app<R>(f: impl FnOnce(&mut Runtime) -> R) -> Option<R> {
    let already_in = IN_WITH_APP.with(|flag| flag.replace(true));
    if already_in {
        // SendMessage (cross-process IME) や block_on のネストメッセージループ経由で
        // win_event_proc などの外部コールバックから再呼び出しされた場合。
        // debug_assert はここに置かない: win_event_proc は extern "system" FFI 境界であり、
        // panic を FFI 越えに伝播させると UB / プロセスクラッシュになる。
        log::warn!("with_app re-entry detected — returning None (caller should re-post if needed)");
        return None;
    }
    // Safety: Windows メッセージループはシングルスレッドであり、再入ガード済み
    let result = unsafe { APP.with_mut(f) };
    IN_WITH_APP.with(|flag| flag.set(false));
    result
}

/// `APP` グローバルへの読み取り専用アクセスファサード。
pub fn with_app_ref<R>(f: impl FnOnce(&Runtime) -> R) -> Option<R> {
    // Safety: Windows メッセージループはシングルスレッドである
    unsafe { APP.with(f) }
}

/// `with_app` が現在アクティブかどうかを返す。
///
/// `hook_callback` が `SendMessageTimeoutW` 等のメッセージポンプ経由で再呼び出しされた際に
/// `APP.get_mut()` を呼ぶと `&mut Runtime` が二重に存在し UB となる。
/// このガードで早期リターンすることで UB を防ぐ。
pub fn in_with_app() -> bool {
    IN_WITH_APP.with(|flag| flag.get())
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

/// TSF ウォームアッププローブのポーリングタイマー ID
///
/// block_on ネストメッセージループを回避するため、
/// 10ms 間隔で GJI 静止・NAMECHANGE・リテラル検出を行う。
pub const TIMER_TSF_PROBE: usize = 105;

/// TsfGate の PendingWarmup フォールバックタイマー ID
///
/// フォーカス変更後 `WARMUP_TIMEOUT_MS`（500ms）以内にプローブが完了しない場合、
/// Bypass へ強制遷移して保留キーをドレインする。
pub const TIMER_TSF_GATE: usize = 106;

/// ReinjectKey の output guard 解除待ちタイマー ID
///
/// SendInput 直後 50ms は OS キューに出力イベントが残っており、
/// その間に passthrough キーを reinject すると IME composition が
/// キャンセルされる race が起きる。
/// block_on(sleep) を排除するため、SetTimer で待機してから drain_deferred を再実行する。
pub const TIMER_OUTPUT_GUARD: usize = 104;

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
        KEYBD_EVENT_FLAGS, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
        VIRTUAL_KEY,
    };

    let is_keyup = matches!(event.event_type, KeyEventType::KeyUp);

    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(event.vk_code.0),
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
    win32::send_input_safe(&[input]);
}
