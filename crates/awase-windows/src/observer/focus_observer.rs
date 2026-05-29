//! フォーカス変更の観測 — AppKind 判定と修飾キー状態取得。
//!
//! ADR 028 により、フォーカスイベントの分類・flush・IME refresh は
//! `refresh_ime_state_cache` (runtime.rs) でデバウンス後に一括処理される。
//! このモジュールは `detect_app_kind` と `read_os_modifiers` のみ提供する。

pub use crate::focus::class_names::detect_app_kind;

use awase::engine::ModifierState;
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

use crate::vk::{VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT};

/// `GetAsyncKeyState` で現在の修飾キー状態を取得する。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
#[must_use]
pub unsafe fn read_os_modifiers() -> ModifierState {
    // GetAsyncKeyState: 最上位ビットが 1 なら押下中
    let pressed = |vk: i32| -> bool { (GetAsyncKeyState(vk).cast_unsigned() & 0x8000) != 0 };
    ModifierState {
        ctrl:  pressed(VK_CONTROL.0 as i32),
        alt:   pressed(VK_MENU.0 as i32),
        shift: pressed(VK_SHIFT.0 as i32),
        win:   pressed(VK_LWIN.0 as i32) || pressed(VK_RWIN.0 as i32),
    }
}

