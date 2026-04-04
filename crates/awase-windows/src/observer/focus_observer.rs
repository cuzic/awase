//! フォーカス変更の観測 — AppKind 判定と修飾キー状態取得。
//!
//! ADR 028 により、フォーカスイベントの分類・flush・IME refresh は
//! `refresh_ime_state_cache` (runtime.rs) でデバウンス後に一括処理される。
//! このモジュールは `detect_app_kind` と `read_os_modifiers` のみ提供する。

use awase::engine::ModifierState;
use awase::types::AppKind;
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

/// `GetAsyncKeyState` で現在の修飾キー状態を取得する。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn read_os_modifiers() -> ModifierState {
    // GetAsyncKeyState: 最上位ビットが 1 なら押下中
    let pressed = |vk: i32| -> bool { (GetAsyncKeyState(vk).cast_unsigned() & 0x8000) != 0 };
    ModifierState {
        ctrl: pressed(0x11),                 // VK_CONTROL
        alt: pressed(0x12),                  // VK_MENU
        shift: pressed(0x10),                // VK_SHIFT
        win: pressed(0x5B) || pressed(0x5C), // VK_LWIN / VK_RWIN
    }
}

/// ウィンドウクラス名からアプリの UI フレームワーク種別を判定する。
///
/// - `Chrome_WidgetWin_1`: Chromium 系（Chrome, Edge, Electron, VS Code 等）
/// - `MozillaWindowClass`: Firefox（Chromium と同様の入力処理）
/// - `Windows.UI.Core.CoreWindow`: UWP / XAML 系
/// - その他: Win32 クラシック
pub fn detect_app_kind(class_name: &str) -> AppKind {
    let class_lower = class_name.to_ascii_lowercase();
    if class_lower.starts_with("chrome_") || class_lower == "mozillawindowclass" {
        AppKind::Chrome
    } else if class_lower == "windows.ui.core.corewindow"
        || class_lower == "applicationframewindow"
        || class_lower.starts_with("windows.ui.input.")
    {
        AppKind::Uwp
    } else {
        AppKind::Win32
    }
}
