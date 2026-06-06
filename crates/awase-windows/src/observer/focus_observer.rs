//! フォーカス変更の観測 — AppKind 判定と修飾キー状態取得。
//!
//! ADR 028 により、フォーカスイベントの分類・flush・IME refresh は
//! `refresh_ime_state_cache` (runtime.rs) でデバウンス後に一括処理される。
//! このモジュールは `detect_app_kind` と `read_os_modifiers` のみ提供する。

pub use crate::focus::class_names::detect_app_kind;

use awase::engine::ModifierState;
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

use crate::vk::{VK_LCONTROL, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RSHIFT, VK_RWIN};

/// 現在の修飾キー状態を取得する。
///
/// Ctrl/Shift は `PHYSICAL_KEY_STATE`（SendInput 非影響・LLKHF_INJECTED 非影響）を使う。
/// awase 自身が IME_KANJI_MARKER 付き synthetic Ctrl↑ を注入するため、
/// `GetAsyncKeyState(VK_CONTROL)` は汚染される。X サーバー等の外部 synthetic も同様。
/// Alt/Win は awase が操作しないため `GetAsyncKeyState` で問題ない。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
#[must_use]
pub unsafe fn read_os_modifiers() -> ModifierState {
    // GetAsyncKeyState: 最上位ビットが 1 なら押下中
    let pressed = |vk: i32| -> bool { (GetAsyncKeyState(vk).cast_unsigned() & 0x8000) != 0 };
    ModifierState {
        // Ctrl は PHYSICAL_KEY_STATE で判定（awase の synthetic Ctrl↑ / X サーバー synthetic に非影響）
        ctrl: crate::hook::is_physical_key_down(VK_LCONTROL)
            || crate::hook::is_physical_key_down(VK_RCONTROL),
        alt: pressed(i32::from(VK_MENU.0)),
        // Shift は物理状態を直接参照（GetAsyncKeyState は SendInput の LSHIFT↑ で汚染される）
        shift: crate::hook::is_physical_key_down(VK_LSHIFT)
            || crate::hook::is_physical_key_down(VK_RSHIFT),
        win: pressed(i32::from(VK_LWIN.0)) || pressed(i32::from(VK_RWIN.0)),
    }
}
