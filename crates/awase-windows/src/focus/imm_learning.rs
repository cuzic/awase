//! IMM32 クロスプロセス制御能力の学習（ImmGetDefaultIMEWnd による初回判定）

use crate::focus::classifier::ImmCapability;
use crate::focus::AppKind;
use windows::Win32::Foundation::HWND;

/// ImmGetDefaultIMEWnd=NULL の場合、そのアプリの IMM32 制御を `Unavailable` と記録する。
///
/// `new_app_kind` が `Win32` かつ `class_name` が未学習の場合にのみ
/// `ImmGetDefaultIMEWnd` を呼び出して結果をキャッシュに反映する。
///
/// # Safety
/// Win32 API (`ImmGetDefaultIMEWnd`) を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn learn_imm_capability_on_focus(
    platform: &mut crate::platform::WindowsPlatform,
    hwnd: HWND,
    class_name: &str,
    new_app_kind: AppKind,
) {
    if new_app_kind != AppKind::Win32 {
        return;
    }
    if platform.focus.imm_capability(class_name).is_some() {
        return;
    }

    if unsafe { crate::imm::get_ime_wnd(hwnd) }.is_none() {
        log::info!(
            "IMM32 capability: ImmGetDefaultIMEWnd=NULL, learning Unavailable (class={class_name})"
        );
        platform.learn_imm_capability(class_name.to_string(), ImmCapability::Unavailable);
    }
}
