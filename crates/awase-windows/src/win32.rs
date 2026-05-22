//! Windows API の安全ラッパー

use std::ptr::NonNull;
use std::time::Duration;

use windows::Win32::Foundation::HWND;

/// 非 null が保証された HWND ラッパー。
///
/// Win32 API の HWND は null を返すことがある（フォーカスなし等）。
/// `ValidHwnd` は境界で null チェックを行い、内部では非 null を型保証する。
///
/// # 使い方
/// ```ignore
/// let hwnd: HWND = unsafe { GetForegroundWindow() };
/// if let Some(valid) = ValidHwnd::new(hwnd) {
///     // valid は非 null 保証済み
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidHwnd(NonNull<std::ffi::c_void>);

// Safety: HWND の値（ポインタ値）はスレッド間で安全に共有できる。
// ウィンドウハンドルはプロセス内でグローバルに有効であり、
// 別スレッドから参照しても問題ない。
unsafe impl Send for ValidHwnd {}
unsafe impl Sync for ValidHwnd {}

impl ValidHwnd {
    /// null チェックを行い、非 null なら `Some(ValidHwnd)` を返す。
    pub fn new(hwnd: HWND) -> Option<Self> {
        NonNull::new(hwnd.0).map(ValidHwnd)
    }

    /// 内部の `HWND` を返す。非 null が保証されている。
    pub fn as_hwnd(self) -> HWND {
        HWND(self.0.as_ptr())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::HWND;

    #[test]
    fn valid_hwnd_null_returns_none() {
        let null_hwnd = HWND(std::ptr::null_mut());
        assert!(ValidHwnd::new(null_hwnd).is_none());
    }

    #[test]
    fn valid_hwnd_nonnull_roundtrips() {
        let fake_ptr = 0x1234usize as *mut std::ffi::c_void;
        let hwnd = HWND(fake_ptr);
        let valid = ValidHwnd::new(hwnd).expect("non-null pointer should be Some");
        assert_eq!(valid.as_hwnd().0, fake_ptr);
    }
}
use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
};

/// タイムアウト付きで任意の処理をワーカースレッドで実行する。
///
/// `win32_async::run_with_timeout` の re-export。
pub use win32_async::run_with_timeout;

/// メインスレッドのメッセージキューにカスタムメッセージを POST する。
///
/// `PostMessageW(None, msg, WPARAM(0), LPARAM(0))` の簡潔なラッパー。
/// `None` はメッセージループを持つスレッド（= メインスレッド）を意味する。
pub fn post_to_main_thread(msg: u32) {
    let _ = unsafe {
        windows::Win32::UI::WindowsAndMessaging::PostMessageW(
            None,
            msg,
            windows::Win32::Foundation::WPARAM(0),
            windows::Win32::Foundation::LPARAM(0),
        )
    };
}

/// メインスレッドのメッセージキューにパラメータ付きでカスタムメッセージを POST する。
pub fn post_to_main_thread_with(msg: u32, wparam: usize, lparam: isize) {
    let _ = unsafe {
        windows::Win32::UI::WindowsAndMessaging::PostMessageW(
            None,
            msg,
            windows::Win32::Foundation::WPARAM(wparam),
            windows::Win32::Foundation::LPARAM(lparam),
        )
    };
}

/// `SendInput` の安全ラッパー（`size_of` キャストを安全に処理）
pub fn send_input_safe(inputs: &[INPUT]) -> u32 {
    let size = i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32");
    unsafe { SendInput(inputs, size) }
}

/// `GetGUIThreadInfo` の結果
#[derive(Debug, Clone, Copy)]
pub struct GuiThreadResult {
    /// フォーカスを持つウィンドウ。null（フォーカスなし）の場合は `None`。
    pub focused_hwnd: Option<ValidHwnd>,
    /// ウィンドウが属するスレッド ID（0 = 取得失敗）
    pub thread_id: u32,
}

/// `GetGUIThreadInfo(0, ...)` のラッパー — ブロッキングが一定時間を超えたら
/// フォールバックとして `GetForegroundWindow()` を返す。
///
/// `GetGUIThreadInfo` はフォアグラウンドウィンドウの GUI スレッドにメッセージを送るため、
/// 対象スレッドがハングしていると無期限にブロックする。
/// `run_with_timeout` でワーカースレッドで実行し、タイムアウト時は
/// 非ブロッキングな `GetForegroundWindow` にフォールバックする。
///
/// # Safety
/// Win32 API を呼び出す。
pub unsafe fn get_gui_thread_info_with_timeout(timeout: Duration) -> GuiThreadResult {
    // HWND はポインタだが、スレッド間で安全に送信可能
    // （Win32 ウィンドウハンドルはプロセス内で有効なグローバルリソース）
    struct SendableResult(Option<ValidHwnd>, u32);
    unsafe impl Send for SendableResult {}

    let result = run_with_timeout(timeout, || {
        let mut info = GUITHREADINFO {
            cbSize: u32::try_from(size_of::<GUITHREADINFO>()).unwrap(),
            ..Default::default()
        };
        unsafe {
            if GetGUIThreadInfo(0, &raw mut info).is_ok() {
                // hwndFocus が null なら hwndActive を使う
                let hwnd = ValidHwnd::new(info.hwndFocus)
                    .or_else(|| ValidHwnd::new(info.hwndActive));
                let tid = if let Some(h) = hwnd {
                    let mut pid = 0u32;
                    GetWindowThreadProcessId(h.as_hwnd(), Some(&raw mut pid))
                } else {
                    0
                };
                SendableResult(hwnd, tid)
            } else {
                SendableResult(ValidHwnd::new(GetForegroundWindow()), 0)
            }
        }
    });

    match result {
        Some(SendableResult(hwnd, tid)) => GuiThreadResult { focused_hwnd: hwnd, thread_id: tid },
        None => {
            // フォールバック: GetForegroundWindow は非ブロッキング
            GuiThreadResult {
                focused_hwnd: ValidHwnd::new(unsafe { GetForegroundWindow() }),
                thread_id: 0,
            }
        }
    }
}
