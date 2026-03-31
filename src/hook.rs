use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, WH_KEYBOARD_LL,
    WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::output::INJECTED_MARKER;
use awase::types::{KeyEventType, RawKeyEvent, ScanCode, Timestamp, VkCode};

/// フックコールバックが最後に呼ばれた時刻（`GetTickCount64` ミリ秒）。
/// 0 はまだ一度も呼ばれていないことを意味する。
static LAST_HOOK_ACTIVITY: AtomicU64 = AtomicU64::new(0);

/// フックコールバックの最終活動時刻を返す（`GetTickCount64` ミリ秒、0 = 未活動）。
pub fn last_hook_activity_ms() -> u64 {
    LAST_HOOK_ACTIVITY.load(Ordering::Relaxed)
}

/// 現在時刻を `GetTickCount64` ミリ秒で返す。
pub fn current_tick_ms() -> u64 {
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
}

/// フックが応答しているかを判定する。
///
/// `timeout_ms` 以内にフックコールバックが呼ばれていれば `true`。
/// まだ一度も呼ばれていない場合も `true`（起動直後はキー入力がない）。
pub fn is_hook_responsive(timeout_ms: u64) -> bool {
    let last = LAST_HOOK_ACTIVITY.load(Ordering::Relaxed);
    if last == 0 {
        return true; // 起動直後: まだキー入力がない
    }
    let now = current_tick_ms();
    (now - last) < timeout_ms
}

/// フックを再登録する（OS に無言で削除された場合の自動復旧用）。
///
/// コールバックは既にグローバルに保持されているため、
/// `SetWindowsHookExW` を再度呼んでハンドルを差し替えるだけ。
/// UAC 昇格もプロセス再起動も不要。
pub fn reinstall_hook() -> bool {
    unsafe {
        // 旧ハンドルがあれば念のため解除（既に無効な可能性あり）
        let old = *HOOK_HANDLE.get_mut();
        if !old.0.is_null() {
            let _ = UnhookWindowsHookEx(old);
        }

        match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_callback), None, 0) {
            Ok(new_handle) => {
                HOOK_HANDLE.set(new_handle);
                LAST_HOOK_ACTIVITY.store(current_tick_ms(), Ordering::Relaxed);
                log::info!("Keyboard hook reinstalled successfully");
                true
            }
            Err(e) => {
                log::error!("Failed to reinstall keyboard hook: {e}");
                HOOK_HANDLE.set(HHOOK(std::ptr::null_mut()));
                false
            }
        }
    }
}

/// シングルスレッド専用のグローバルセル（main.rs と同じパターン）
struct SingleThreadCell<T>(UnsafeCell<T>);
unsafe impl<T> Sync for SingleThreadCell<T> {}

impl<T> SingleThreadCell<T> {
    const fn new(val: T) -> Self {
        Self(UnsafeCell::new(val))
    }

    unsafe fn get_mut(&self) -> &mut T {
        &mut *self.0.get()
    }

    unsafe fn set(&self, val: T) {
        *self.0.get() = val;
    }
}

/// グローバルなフックハンドル
static HOOK_HANDLE: SingleThreadCell<HHOOK> = SingleThreadCell::new(HHOOK(std::ptr::null_mut()));

/// フックコールバックで使うコールバック関数
static KEY_EVENT_CALLBACK: SingleThreadCell<Option<Box<dyn FnMut(RawKeyEvent) -> CallbackResult>>> =
    SingleThreadCell::new(None);

/// 再入ガード
static IN_CALLBACK: SingleThreadCell<bool> = SingleThreadCell::new(false);

/// コールバックの戻り値
pub enum CallbackResult {
    /// 元キーを握りつぶす（LRESULT(1)）
    Consumed,
    /// 元キーをそのまま通す
    PassThrough,
}

/// フック解除を保証する RAII ガード
///
/// スコープを抜けると自動的に `UnhookWindowsHookEx` を呼び出し、
/// コールバックもクリアする。
pub struct HookGuard {
    _private: (), // 外部から直接構築させない
}

impl Drop for HookGuard {
    fn drop(&mut self) {
        unsafe {
            let handle = *HOOK_HANDLE.get_mut();
            if !handle.0.is_null() {
                let _ = UnhookWindowsHookEx(handle);
                HOOK_HANDLE.set(HHOOK(std::ptr::null_mut()));
                log::info!("Keyboard hook uninstalled");
            }
            KEY_EVENT_CALLBACK.set(None);
        }
    }
}

/// フックを登録する
///
/// 返された `HookGuard` を保持している間フックが有効。
/// ドロップ時に自動解除される。
pub fn install_hook(
    callback: Box<dyn FnMut(RawKeyEvent) -> CallbackResult>,
) -> windows::core::Result<HookGuard> {
    unsafe {
        KEY_EVENT_CALLBACK.set(Some(callback));

        let handle = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_callback), None, 0)?;
        HOOK_HANDLE.set(handle);

        log::info!("Keyboard hook installed successfully");
    }
    Ok(HookGuard { _private: () })
}

/// WH_KEYBOARD_LL フックコールバック
unsafe extern "system" fn hook_callback(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let hook_handle = *HOOK_HANDLE.get_mut();

    if ncode >= 0 {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);

        // ── ハートビート更新（ウォッチドッグ用）──
        LAST_HOOK_ACTIVITY.store(current_tick_ms(), Ordering::Relaxed);

        // ── 自己注入チェック（無限ループ防止）──
        if kb.dwExtraInfo == INJECTED_MARKER {
            return CallNextHookEx(hook_handle, ncode, wparam, lparam);
        }

        // ── 再入ガード ──
        let in_callback = IN_CALLBACK.get_mut();
        if *in_callback {
            return CallNextHookEx(hook_handle, ncode, wparam, lparam);
        }
        *in_callback = true;

        let event_type = match wparam.0 as u32 {
            WM_KEYDOWN => KeyEventType::KeyDown,
            WM_KEYUP => KeyEventType::KeyUp,
            WM_SYSKEYDOWN => KeyEventType::SysKeyDown,
            WM_SYSKEYUP => KeyEventType::SysKeyUp,
            _ => {
                *in_callback = false;
                return CallNextHookEx(hook_handle, ncode, wparam, lparam);
            }
        };

        let event = RawKeyEvent {
            vk_code: VkCode(kb.vkCode as u16),
            scan_code: ScanCode(kb.scanCode),
            event_type,
            extra_info: kb.dwExtraInfo,
            timestamp: now_timestamp(),
        };

        log::trace!(
            "Hook: vk=0x{:02X} scan=0x{:04X} type={:?}",
            event.vk_code.0,
            event.scan_code.0,
            event.event_type
        );

        // ── コールバック呼び出し ──
        let result = KEY_EVENT_CALLBACK
            .get_mut()
            .as_mut()
            .map_or(CallbackResult::PassThrough, |callback| callback(event));

        *IN_CALLBACK.get_mut() = false;

        match result {
            CallbackResult::Consumed => {
                return LRESULT(1); // 元キーを握りつぶす
            }
            CallbackResult::PassThrough => {
                // 何もしない（元キーをそのまま通す）
            }
        }
    }

    CallNextHookEx(hook_handle, ncode, wparam, lparam)
}

/// 起動時点からの経過マイクロ秒を返す（`Instant` を内部的に使用）
fn now_timestamp() -> Timestamp {
    use std::sync::OnceLock;
    use std::time::Instant;
    static BASELINE: OnceLock<Instant> = OnceLock::new();
    let baseline = BASELINE.get_or_init(Instant::now);
    baseline.elapsed().as_micros() as u64
}
