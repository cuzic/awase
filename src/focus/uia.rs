//! Phase 3: UIA (UI Automation) パターンベース非同期判定

use std::sync::mpsc;

use awase::types::FocusKind;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
    IUIAutomationValuePattern, UIA_ButtonControlTypeId,
    UIA_DocumentControlTypeId, UIA_EditControlTypeId, UIA_HyperlinkControlTypeId,
    UIA_ImageControlTypeId, UIA_ListItemControlTypeId, UIA_MenuBarControlTypeId,
    UIA_MenuControlTypeId, UIA_MenuItemControlTypeId, UIA_ProgressBarControlTypeId,
    UIA_ScrollBarControlTypeId, UIA_SeparatorControlTypeId, UIA_SliderControlTypeId,
    UIA_StatusBarControlTypeId, UIA_TabControlTypeId, UIA_TabItemControlTypeId,
    UIA_TextControlTypeId, UIA_TextPatternId, UIA_TitleBarControlTypeId,
    UIA_ToolBarControlTypeId, UIA_TreeItemControlTypeId, UIA_ValuePatternId,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// `HWND` を `Send` 可能にするラッパー
///
/// `HWND` は `*mut c_void` を含むため `Send` を実装していないが、
/// ウィンドウハンドルの値自体はスレッド間で安全に受け渡せる。
/// UIA ワーカースレッドへの HWND 送信専用。
#[derive(Clone, Copy)]
pub struct SendableHwnd(pub HWND);
// Safety: HWND の値（ポインタ値）はスレッド間で安全に共有できる。
// ウィンドウハンドルはプロセス内でグローバルに有効であり、
// 別スレッドから参照しても問題ない。
unsafe impl Send for SendableHwnd {}

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
            log::trace!("UIA: GetFocusedElement failed: {e:?}");
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
        if control_type == UIA_EditControlTypeId || control_type == UIA_DocumentControlTypeId {
            log::debug!("UIA: ControlType={control_type:?} → TextInput");
            return FocusKind::TextInput;
        }

        // 非テキスト系
        let non_text_types = [
            UIA_ButtonControlTypeId,
            UIA_MenuItemControlTypeId,
            UIA_TreeItemControlTypeId,
            UIA_ListItemControlTypeId,
            UIA_TabControlTypeId,
            UIA_TabItemControlTypeId,
            UIA_ToolBarControlTypeId,
            UIA_StatusBarControlTypeId,
            UIA_ProgressBarControlTypeId,
            UIA_SliderControlTypeId,
            UIA_ScrollBarControlTypeId,
            UIA_HyperlinkControlTypeId,
            UIA_ImageControlTypeId,
            UIA_MenuBarControlTypeId,
            UIA_MenuControlTypeId,
            UIA_TitleBarControlTypeId,
            UIA_SeparatorControlTypeId,
            UIA_TextControlTypeId,
        ];
        if non_text_types.contains(&control_type) {
            log::debug!("UIA: ControlType={control_type:?} → NonText");
            return FocusKind::NonText;
        }
    }

    // 4. 確定的なシグナルなし
    log::debug!("UIA: no definitive signal → Undetermined");
    FocusKind::Undetermined
}

/// UIA 非同期判定ワーカースレッドを起動する
///
/// 専用スレッドで COM を初期化し、`IUIAutomation` インスタンスを保持する。
/// チャネル経由で HWND を受け取り、`GetFocusedElement` でコントロール種別を判定して
/// `FOCUS_KIND` を更新する。Phase 1-2 で `Undetermined` だったコントロールの解像度を上げる。
pub fn spawn_uia_worker() -> mpsc::Sender<SendableHwnd> {
    let (tx, rx) = mpsc::channel::<SendableHwnd>();
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }

        let automation: Option<IUIAutomation> = unsafe {
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
        };

        let Some(automation) = automation else {
            log::warn!("UIA: Failed to create IUIAutomation, Phase 3 disabled");
            return;
        };

        log::info!("UIA worker thread started");

        while let Ok(SendableHwnd(hwnd)) = rx.recv() {
            let state = unsafe { uia_classify_focus(&automation, hwnd) };
            if state != FocusKind::Undetermined {
                log::debug!("UIA async: hwnd={hwnd:?} → {state:?}");

                // メインスレッドに結果を送信（FOCUS_KIND への書き込みはメインスレッドで行う）
                unsafe {
                    let _ = PostMessageW(
                        HWND::default(),
                        crate::WM_FOCUS_KIND_UPDATE,
                        WPARAM(state as u8 as usize),
                        LPARAM(hwnd.0 as isize),
                    );
                }
            }
        }
    });
    tx
}
