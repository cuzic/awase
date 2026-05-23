use windows::Win32::Foundation::HWND;

#[derive(Debug)]
pub struct FocusSnapshot {
    hwnd_addr: usize,
    pub process_id: u32,
    pub class_name: String,
}

// SAFETY: hwnd_addr は usize として保存しており、ポインタとして同期アクセスはしない。
// process_id と class_name は自明に Send。
unsafe impl Send for FocusSnapshot {}

impl FocusSnapshot {
    #[must_use] 
    pub const fn hwnd(&self) -> HWND {
        HWND(self.hwnd_addr as *mut _)
    }
}

/// `read_focus_snapshot` の async 版。
/// ワーカースレッドで実行し、メッセージループに制御を返しながら待つ。
#[allow(clippy::future_not_send)]
pub async fn run_focus_probe_async() -> Option<FocusSnapshot> {
    win32_async::offload(move || {
        let result = unsafe {
            crate::win32::get_gui_thread_info_with_timeout(
                std::time::Duration::from_millis(150),
            )
        };
        let Some(hwnd) = result.focused_hwnd else {
            return Some(FocusSnapshot {
                hwnd_addr: 0,
                process_id: 0,
                class_name: String::new(),
            });
        };
        let process_id = crate::focus::classify::get_window_process_id(hwnd);
        let class_name = crate::focus::classify::get_class_name_string(hwnd);
        Some(FocusSnapshot {
            hwnd_addr: hwnd.0 as usize,
            process_id,
            class_name,
        })
    })
    .await
}

/// Win32 タイムアウト付きフォアグラウンドウィンドウ情報取得。
///
/// # Safety
/// Win32 API (GetGUIThreadInfo, GetWindowThreadProcessId, GetClassNameW) を呼ぶ。
#[must_use] 
pub unsafe fn read_focus_snapshot() -> Option<FocusSnapshot> {
    crate::win32::run_with_timeout(
        std::time::Duration::from_millis(300),
        || {
            let result = unsafe {
                crate::win32::get_gui_thread_info_with_timeout(
                    std::time::Duration::from_millis(150),
                )
            };
            let Some(hwnd) = result.focused_hwnd else {
                return FocusSnapshot {
                    hwnd_addr: 0,
                    process_id: 0,
                    class_name: String::new(),
                };
            };
            let process_id = crate::focus::classify::get_window_process_id(hwnd);
            let class_name = crate::focus::classify::get_class_name_string(hwnd);
            FocusSnapshot {
                hwnd_addr: hwnd.0 as usize,
                process_id,
                class_name,
            }
        },
    )
}
