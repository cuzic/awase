//! Phase 2: MSAA (IAccessible) によるロールベース判定

use awase::types::FocusKind;
use windows::core::{Interface, VARIANT};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{AccessibleObjectFromWindow, IAccessible};

/// `OBJID_CLIENT` — クライアント領域のアクセシブルオブジェクト
const OBJID_CLIENT: i32 = -4;

/// MSAA アクセシビリティロール
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
enum MsaaRole {
    TitleBar = 1,
    MenuBar = 2,
    ScrollBar = 3,
    MenuPopup = 11,
    MenuItem = 12,
    Document = 15,
    ToolBar = 22,
    StatusBar = 23,
    List = 33,
    ListItem = 34,
    Outline = 35,
    OutlineItem = 36,
    PageTab = 37,
    Indicator = 39,
    Graphic = 40,
    StaticText = 41,
    Text = 42,
    PushButton = 43,
    ProgressBar = 48,
    Slider = 51,
}

impl MsaaRole {
    fn from_u32(v: u32) -> Option<Self> {
        match v {
            1 => Some(Self::TitleBar),
            2 => Some(Self::MenuBar),
            3 => Some(Self::ScrollBar),
            11 => Some(Self::MenuPopup),
            12 => Some(Self::MenuItem),
            15 => Some(Self::Document),
            22 => Some(Self::ToolBar),
            23 => Some(Self::StatusBar),
            33 => Some(Self::List),
            34 => Some(Self::ListItem),
            35 => Some(Self::Outline),
            36 => Some(Self::OutlineItem),
            37 => Some(Self::PageTab),
            39 => Some(Self::Indicator),
            40 => Some(Self::Graphic),
            41 => Some(Self::StaticText),
            42 => Some(Self::Text),
            43 => Some(Self::PushButton),
            48 => Some(Self::ProgressBar),
            51 => Some(Self::Slider),
            _ => None,
        }
    }

    fn is_text_input(self) -> bool {
        matches!(self, Self::Text | Self::Document)
    }

    fn is_non_text(self) -> bool {
        matches!(
            self,
            Self::TitleBar
                | Self::MenuBar
                | Self::ScrollBar
                | Self::MenuPopup
                | Self::MenuItem
                | Self::ToolBar
                | Self::StatusBar
                | Self::List
                | Self::ListItem
                | Self::Outline
                | Self::OutlineItem
                | Self::PageTab
                | Self::Indicator
                | Self::Graphic
                | Self::StaticText
                | Self::PushButton
                | Self::ProgressBar
                | Self::Slider
        )
    }
}

/// MSAA ロールに基づくフォーカス判定
///
/// テキスト入力ロール（Text, Document）なら TextInput、
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

            if let Some(role) = MsaaRole::from_u32(role_id) {
                if role.is_text_input() {
                    log::debug!("MSAA: {:?} → TextInput", role);
                    return FocusKind::TextInput;
                }
                if role.is_non_text() {
                    log::debug!("MSAA: {:?} → NonText", role);
                    return FocusKind::NonText;
                }
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
