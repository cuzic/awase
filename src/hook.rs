use std::cell::UnsafeCell;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, WH_KEYBOARD_LL,
    WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::output::INJECTED_MARKER;
use awase::types::{KeyEventType, RawKeyEvent, Timestamp};

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

/// フックを登録する
pub fn install_hook(
    callback: Box<dyn FnMut(RawKeyEvent) -> CallbackResult>,
) -> windows::core::Result<()> {
    unsafe {
        KEY_EVENT_CALLBACK.set(Some(callback));

        let handle = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_callback), None, 0)?;
        HOOK_HANDLE.set(handle);

        log::info!("Keyboard hook installed successfully");
    }
    Ok(())
}

/// フックを解除する
pub fn uninstall_hook() {
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

/// WH_KEYBOARD_LL フックコールバック
unsafe extern "system" fn hook_callback(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let hook_handle = *HOOK_HANDLE.get_mut();

    if ncode >= 0 {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);

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
            vk_code: kb.vkCode as u16,
            scan_code: kb.scanCode,
            event_type,
            extra_info: kb.dwExtraInfo,
            timestamp: now_timestamp(),
        };

        log::trace!(
            "Hook: vk=0x{:02X} scan=0x{:04X} type={:?}",
            event.vk_code,
            event.scan_code,
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
