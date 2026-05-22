use windows::Win32::Foundation::HWND;

pub struct FocusProbe {
    hwnd_addr: usize,
    pub process_id: u32,
    pub class_name: String,
}

impl FocusProbe {
    pub fn hwnd(&self) -> HWND {
        HWND(self.hwnd_addr as *mut _)
    }
}

/// Win32 タイムアウト付きフォアグラウンドウィンドウ情報取得。
///
/// # Safety
/// Win32 API (GetGUIThreadInfo, GetWindowThreadProcessId, GetClassNameW) を呼ぶ。
pub unsafe fn run_focus_probe() -> Option<FocusProbe> {
    crate::win32::run_with_timeout(
        std::time::Duration::from_millis(300),
        || {
            let result = unsafe {
                crate::win32::get_gui_thread_info_with_timeout(
                    std::time::Duration::from_millis(150),
                )
            };
            let Some(valid_hwnd) = result.focused_hwnd else {
                return FocusProbe {
                    hwnd_addr: 0,
                    process_id: 0,
                    class_name: String::new(),
                };
            };
            let hwnd = valid_hwnd.as_hwnd();
            let process_id = crate::focus::classify::get_window_process_id(hwnd);
            let class_name = crate::focus::classify::get_class_name_string(hwnd);
            FocusProbe {
                hwnd_addr: hwnd.0 as usize,
                process_id,
                class_name,
            }
        },
    )
}
