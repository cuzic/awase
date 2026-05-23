//! フォーカス変更の観測 — AppKind 判定と修飾キー状態取得。
//!
//! ADR 028 により、フォーカスイベントの分類・flush・IME refresh は
//! `refresh_ime_state_cache` (runtime.rs) でデバウンス後に一括処理される。
//! このモジュールは `detect_app_kind` と `read_os_modifiers` のみ提供する。

pub use crate::focus::class_names::detect_app_kind;

use awase::engine::ModifierState;
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

