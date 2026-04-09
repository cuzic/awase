//! Windows API の安全ラッパー

use std::time::Duration;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
};

/// `SendInput` の安全ラッパー（`size_of` キャストを安全に処理）
pub fn send_input_safe(inputs: &[INPUT]) -> u32 {
    let size = i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32");
    unsafe { SendInput(inputs, size) }
}

/// `GetGUIThreadInfo` の結果
#[derive(Debug, Clone, Copy)]
pub struct GuiThreadResult {
    /// フォーカスを持つウィンドウ（フォールバック時は `GetForegroundWindow` の結果）
    pub focused_hwnd: HWND,
    /// ウィンドウが属するスレッド ID（0 = 取得失敗）
    pub thread_id: u32,
}

/// `GetGUIThreadInfo(0, ...)` のラッパー — ブロッキングが一定時間を超えたら
/// フォールバックとして `GetForegroundWindow()` を返す。
///
/// `GetGUIThreadInfo` はフォアグラウンドウィンドウの GUI スレッドにメッセージを送るため、
/// 対象スレッドがハングしていると無期限にブロックする。
/// ワーカースレッドで実行し、タイムアウトした場合は非ブロッキングな
/// `GetForegroundWindow` にフォールバックする。
///
/// # Safety
/// Win32 API を呼び出す。
pub unsafe fn get_gui_thread_info_with_timeout(timeout: Duration) -> GuiThreadResult {
    // HWND はポインタだが、スレッド間で安全に送信可能
    // （Win32 ウィンドウハンドルはプロセス内で有効なグローバルリソース）
    struct SendableResult(HWND, u32);
    unsafe impl Send for SendableResult {}

    let handle = std::thread::spawn(|| {
        let mut info = GUITHREADINFO {
            cbSize: u32::try_from(size_of::<GUITHREADINFO>()).unwrap(),
            ..Default::default()
        };
        unsafe {
            if GetGUIThreadInfo(0, &raw mut info).is_ok() {
                let hwnd = if info.hwndFocus.0.is_null() {
                    info.hwndActive
                } else {
                    info.hwndFocus
                };
                let mut pid = 0u32;
                let tid = GetWindowThreadProcessId(hwnd, Some(&raw mut pid));
                SendableResult(hwnd, tid)
            } else {
                let hwnd = GetForegroundWindow();
                SendableResult(hwnd, 0)
            }
        }
    });

    // タイムアウト付き join: park_timeout で待機
    let start = std::time::Instant::now();
    loop {
        if handle.is_finished() {
            match handle.join() {
                Ok(SendableResult(hwnd, tid)) => {
                    return GuiThreadResult {
                        focused_hwnd: hwnd,
                        thread_id: tid,
                    };
                }
                Err(_) => {
                    // ワーカースレッドがパニックした場合はフォールバック
                    log::error!("GetGUIThreadInfo worker thread panicked");
                    break;
                }
            }
        }
        if start.elapsed() >= timeout {
            log::warn!(
                "GetGUIThreadInfo timed out after {}ms, falling back to GetForegroundWindow",
                timeout.as_millis()
            );
            // ワーカースレッドは放置（OS がスレッド終了時に回収）
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    // フォールバック: GetForegroundWindow は非ブロッキング
    let hwnd = unsafe { GetForegroundWindow() };
    GuiThreadResult {
        focused_hwnd: hwnd,
        thread_id: 0,
    }
}
