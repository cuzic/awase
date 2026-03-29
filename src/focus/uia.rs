//! Phase 3: UIA (UI Automation) パターンベース非同期判定

use awase::types::FocusKind;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{
    IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern, IUIAutomationValuePattern,
    UIA_ButtonControlTypeId, UIA_DocumentControlTypeId,
    UIA_EditControlTypeId, UIA_HyperlinkControlTypeId, UIA_ImageControlTypeId,
    UIA_ListItemControlTypeId, UIA_MenuBarControlTypeId, UIA_MenuControlTypeId,
    UIA_MenuItemControlTypeId, UIA_ProgressBarControlTypeId, UIA_ScrollBarControlTypeId,
    UIA_SeparatorControlTypeId, UIA_SliderControlTypeId, UIA_StatusBarControlTypeId,
    UIA_TabControlTypeId, UIA_TabItemControlTypeId, UIA_TextControlTypeId,
    UIA_TextPatternId, UIA_TitleBarControlTypeId, UIA_ToolBarControlTypeId,
    UIA_TreeItemControlTypeId, UIA_ValuePatternId,
};

/// UIA を使用してフォーカス中コントロールの種別を判定する
///
/// Pattern-first アプローチ:
/// 1. `ValuePattern` → `IsReadOnly` で編集可能なテキストフィールドを検出
/// 2. `TextPattern` の有無でテキスト編集能力を検出
/// 3. `CurrentControlType` をフォールバックとして使用
///
/// Chrome/WPF/UWP など Win32 クラス名では判定できないコントロールに有効。
///
/// Safety: COM が初期化済みのスレッドから呼び出すこと
#[allow(unused_variables)] // hwnd はデバッグ用に保持
pub unsafe fn uia_classify_focus(automation: &IUIAutomation, hwnd: HWND) -> FocusKind {
    let element: IUIAutomationElement = match automation.GetFocusedElement() {
        Ok(el) => el,
        Err(e) => {
            log::trace!("UIA: GetFocusedElement failed: {:?}", e);
            return FocusKind::Undetermined;
        }
    };

    // 1. ValuePattern → IsReadOnly チェック
    //    「編集可能な値を持つ」が最も強いシグナル
    if let Ok(pattern) = element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
        match pattern.CurrentIsReadOnly() {
            Ok(read_only) if !read_only.as_bool() => {
                log::debug!("UIA: ValuePattern(IsReadOnly=false) → TextInput");
                return FocusKind::TextInput;
            }
            Ok(_) => {
                log::debug!("UIA: ValuePattern(IsReadOnly=true) → NonText");
                return FocusKind::NonText;
            }
            Err(_) => {} // fall through
        }
    }

    // 2. TextPattern チェック
    //    TextPattern をサポートする要素はテキスト編集能力を持つ
    if element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId).is_ok() {
        log::debug!("UIA: TextPattern available → TextInput");
        return FocusKind::TextInput;
    }

    // 3. フォールバック: ControlType で確定的な非テキストコントロールを判別
    if let Ok(control_type) = element.CurrentControlType() {
        // テキスト入力系（補助的な確認のみ）
        if matches!(control_type, UIA_EditControlTypeId | UIA_DocumentControlTypeId) {
            log::debug!("UIA: ControlType={:?} → TextInput", control_type);
            return FocusKind::TextInput;
        }

        // 非テキスト系
        if matches!(
            control_type,
            UIA_ButtonControlTypeId
                | UIA_MenuItemControlTypeId
                | UIA_TreeItemControlTypeId
                | UIA_ListItemControlTypeId
                | UIA_TabControlTypeId
                | UIA_TabItemControlTypeId
                | UIA_ToolBarControlTypeId
                | UIA_StatusBarControlTypeId
                | UIA_ProgressBarControlTypeId
                | UIA_SliderControlTypeId
                | UIA_ScrollBarControlTypeId
                | UIA_HyperlinkControlTypeId
                | UIA_ImageControlTypeId
                | UIA_MenuBarControlTypeId
                | UIA_MenuControlTypeId
                | UIA_TitleBarControlTypeId
                | UIA_SeparatorControlTypeId
                | UIA_TextControlTypeId
        ) {
            log::debug!("UIA: ControlType={:?} → NonText", control_type);
            return FocusKind::NonText;
        }
    }

    // 4. 確定的なシグナルなし
    log::debug!("UIA: no definitive signal → Undetermined");
    FocusKind::Undetermined
}
