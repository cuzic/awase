//! IMM 能力の学習（ImmGetDefaultIMEWnd による初回判定）

use windows::Win32::Foundation::HWND;
use awase::types::AppKind;
use crate::focus::classifier::{AppKindClassifier, ImmCapability};

/// ImmGetDefaultIMEWnd=NULL の場合、そのアプリを Broken と記録する。
///
/// `new_app_kind` が `Win32` かつ `class_name` が未学習の場合にのみ
/// `ImmGetDefaultIMEWnd` を呼び出して結果をキャッシュに反映する。
///
/// # Safety
/// Win32 API (`ImmGetDefaultIMEWnd`) を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn learn_imm_capability_on_focus(
    classifier: &mut AppKindClassifier,
    hwnd: HWND,
    class_name: &str,
    new_app_kind: AppKind,
) {
    if new_app_kind != AppKind::Win32 {
        return;
    }
    if classifier.imm_learning.contains_key(class_name) {
        return;
    }

    if unsafe { crate::imm::get_ime_wnd(hwnd) }.is_none() {
        log::info!(
            "IMM capability: ImmGetDefaultIMEWnd=NULL, learning Broken (class={class_name})"
        );
        classifier.learn_imm_capability(class_name.to_string(), ImmCapability::Broken);
    }
}
