//! Windows API の安全ラッパー

use std::time::Duration;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
};

/// タイムアウト付きで任意の処理をワーカースレッドで実行する。
///
/// `win32_async::run_with_timeout` の re-export。
pub use win32_async::run_with_timeout;

/// null チェックを行い、非 null なら `Some(hwnd)` を返す。
///
/// Win32 API が返す `HWND` は null（フォーカスなし・失敗）を示すことがある。
/// 境界でこの関数を使い、以降は `Option<HWND>` として処理する。
pub fn non_null_hwnd(hwnd: HWND) -> Option<HWND> {
    (!hwnd.0.is_null()).then_some(hwnd)
}

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
    pub focused_hwnd: Option<HWND>,
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
    struct SendableResult(Option<HWND>, u32);
    unsafe impl Send for SendableResult {}

    let result = run_with_timeout(timeout, || {
        let mut info = GUITHREADINFO {
            cbSize: u32::try_from(size_of::<GUITHREADINFO>()).unwrap(),
            ..Default::default()
        };
        unsafe {
            if GetGUIThreadInfo(0, &raw mut info).is_ok() {
                // hwndFocus が null なら hwndActive を使う
                let hwnd = non_null_hwnd(info.hwndFocus)
                    .or_else(|| non_null_hwnd(info.hwndActive));
                let tid = if let Some(h) = hwnd {
                    let mut pid = 0u32;
                    GetWindowThreadProcessId(h, Some(&raw mut pid))
                } else {
                    0
                };
                SendableResult(hwnd, tid)
            } else {
                SendableResult(non_null_hwnd(GetForegroundWindow()), 0)
            }
        }
    });

    match result {
        Some(SendableResult(hwnd, tid)) => GuiThreadResult { focused_hwnd: hwnd, thread_id: tid },
        None => {
            // フォールバック: GetForegroundWindow は非ブロッキング
            GuiThreadResult {
                focused_hwnd: non_null_hwnd(unsafe { GetForegroundWindow() }),
                thread_id: 0,
            }
        }
    }
}
