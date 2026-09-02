#![allow(unsafe_code)]
//! OS のかな入力ロック状態を読む observer。

use awase::engine::kana_input_warn::KanaLockReading;

/// OS のかな入力ロック状態を読む。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッド（Runtime のメッセージループスレッド）から呼ぶこと。
#[must_use]
pub unsafe fn read_kana_lock() -> KanaLockReading {
    let state = unsafe {
        windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(i32::from(crate::vk::VK_KANA.0))
    };
    if state & 1 != 0 {
        KanaLockReading::On
    } else {
        KanaLockReading::Off
    }
}

/// 現在のフォアグラウンドウィンドウクラス名を診断ログ用に読む。
///
/// # Safety
/// Win32 UI API を呼び出す。メインスレッドから呼ぶこと。
#[must_use]
pub unsafe fn foreground_class_name() -> String {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return String::from("<none>");
    }
    let name = crate::focus::classify::get_class_name_string(hwnd);
    if name.is_empty() {
        String::from("<unknown>")
    } else {
        name
    }
}
