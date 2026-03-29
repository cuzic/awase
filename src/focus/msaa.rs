//! Phase 2: MSAA (IAccessible) によるロールベース判定

use awase::types::FocusKind;
use windows::core::{Interface, VARIANT};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{AccessibleObjectFromWindow, IAccessible};

/// `OBJID_CLIENT` — クライアント領域のアクセシブルオブジェクト
const OBJID_CLIENT: i32 = -4;

/// MSAA ロールに基づくフォーカス判定
///
/// テキスト入力ロール（ROLE_SYSTEM_TEXT, ROLE_SYSTEM_DOCUMENT）なら TextInput、
/// 非テキストロール（ツールバー、メニュー等）なら NonText、
/// 判定不能なら Undetermined を返す。
pub unsafe fn msaa_classify(hwnd: HWND) -> FocusKind {
    let mut acc: *mut std::ffi::c_void = std::ptr::null_mut();
    let ok = AccessibleObjectFromWindow(
        hwnd,
        OBJID_CLIENT as u32,
        &IAccessible::IID,
        &mut acc,
    );
    if ok.is_ok() && !acc.is_null() {
        let accessible: IAccessible = IAccessible::from_raw(acc);
        let child_self = VARIANT::from(0i32); // CHILDID_SELF
        if let Ok(role) = accessible.get_accRole(&child_self) {
            let role_id = role.as_raw().Anonymous.Anonymous.Anonymous.lVal as u32;

            // テキスト入力ロール
            const ROLE_SYSTEM_TEXT: u32 = 42; // 0x2A — editable text
            const ROLE_SYSTEM_DOCUMENT: u32 = 15; // 0x0F — document window

            if matches!(role_id, ROLE_SYSTEM_TEXT | ROLE_SYSTEM_DOCUMENT) {
                log::debug!("MSAA: role={} → TextInput", role_id);
                return FocusKind::TextInput;
            }

            // 非テキストロール
            const ROLE_SYSTEM_TITLEBAR: u32 = 1;
            const ROLE_SYSTEM_MENUBAR: u32 = 2;
            const ROLE_SYSTEM_SCROLLBAR: u32 = 3;
            const ROLE_SYSTEM_MENUPOPUP: u32 = 11;
            const ROLE_SYSTEM_MENUITEM: u32 = 12;
            const ROLE_SYSTEM_TOOLBAR: u32 = 22;
            const ROLE_SYSTEM_STATUSBAR: u32 = 23;
            const ROLE_SYSTEM_LIST: u32 = 33;
            const ROLE_SYSTEM_LISTITEM: u32 = 34;
            const ROLE_SYSTEM_OUTLINE: u32 = 35; // tree view
            const ROLE_SYSTEM_OUTLINEITEM: u32 = 36;
            const ROLE_SYSTEM_PAGETAB: u32 = 37;
            const ROLE_SYSTEM_INDICATOR: u32 = 39;
            const ROLE_SYSTEM_GRAPHIC: u32 = 40;
            const ROLE_SYSTEM_STATICTEXT: u32 = 41;
            const ROLE_SYSTEM_PUSHBUTTON: u32 = 43;
            const ROLE_SYSTEM_PROGRESSBAR: u32 = 48;
            const ROLE_SYSTEM_SLIDER: u32 = 51;

            if matches!(
                role_id,
                ROLE_SYSTEM_TITLEBAR
                    | ROLE_SYSTEM_MENUBAR
                    | ROLE_SYSTEM_SCROLLBAR
                    | ROLE_SYSTEM_MENUPOPUP
                    | ROLE_SYSTEM_MENUITEM
                    | ROLE_SYSTEM_TOOLBAR
                    | ROLE_SYSTEM_STATUSBAR
                    | ROLE_SYSTEM_LIST
                    | ROLE_SYSTEM_LISTITEM
                    | ROLE_SYSTEM_OUTLINE
                    | ROLE_SYSTEM_OUTLINEITEM
                    | ROLE_SYSTEM_PAGETAB
                    | ROLE_SYSTEM_INDICATOR
                    | ROLE_SYSTEM_GRAPHIC
                    | ROLE_SYSTEM_STATICTEXT
                    | ROLE_SYSTEM_PUSHBUTTON
                    | ROLE_SYSTEM_PROGRESSBAR
                    | ROLE_SYSTEM_SLIDER
            ) {
                log::debug!("MSAA: role={} → NonText", role_id);
                return FocusKind::NonText;
            }

            log::debug!(
                "MSAA: role={} → Undetermined (not in allow/deny list)",
                role_id
            );
        }
    }

    // 判定不能 → Undetermined
    FocusKind::Undetermined
}
