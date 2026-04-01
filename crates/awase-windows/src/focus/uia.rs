//! Phase 3: UIA (UI Automation) パターンベース非同期判定

use std::sync::mpsc;

use awase::types::{AppKind, FocusKind, ImeReliability};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
    IUIAutomationValuePattern, UIA_ButtonControlTypeId, UIA_DocumentControlTypeId,
    UIA_EditControlTypeId, UIA_HyperlinkControlTypeId, UIA_ImageControlTypeId,
    UIA_ListItemControlTypeId, UIA_MenuBarControlTypeId, UIA_MenuControlTypeId,
    UIA_MenuItemControlTypeId, UIA_ProgressBarControlTypeId, UIA_ScrollBarControlTypeId,
    UIA_SeparatorControlTypeId, UIA_SliderControlTypeId, UIA_StatusBarControlTypeId,
    UIA_TabControlTypeId, UIA_TabItemControlTypeId, UIA_TextControlTypeId, UIA_TextPatternId,
    UIA_TitleBarControlTypeId, UIA_ToolBarControlTypeId, UIA_TreeItemControlTypeId,
    UIA_ValuePatternId,
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

/// UIA 非同期判定の結果（FocusKind + ImeReliability + AppKind）
pub struct UiaClassifyResult {
    pub focus_kind: FocusKind,
    pub ime_reliability: ImeReliability,
    pub app_kind: Option<AppKind>,
}

/// UIA `FrameworkId` から IME 状態取得の信頼度を推定する
///
/// Win32 / WinForms は従来の IMM32 パスを使用するためクロスプロセス検出が正確。
/// WPF は IMM32 互換レイヤーを持つが TSF 寄りなので Unknown とする。
/// DirectUI / XAML / その他の Modern UI は TSF のみで IMM が不正確。
fn classify_framework_id(framework_id: &str) -> ImeReliability {
    match framework_id {
        "Win32" | "WinForm" => ImeReliability::Reliable,
        // WPF は TSF 統合だが IMM 互換もあるため安全側に倒す
        "DirectUI" | "XAML" | "WPF" => ImeReliability::Unreliable,
        // Chrome/Electron 等は独自の IME 実装
        _ => ImeReliability::Unknown,
    }
}

/// UIA `FrameworkId` から `AppKind` を推定する
fn classify_framework_app_kind(framework_id: &str) -> Option<AppKind> {
    match framework_id {
        "Win32" | "WinForm" => Some(AppKind::Win32),
        "DirectUI" | "XAML" | "WPF" => Some(AppKind::Uwp),
        // Chrome/Electron は FrameworkId が空文字列のことが多い → class name で判定済み
        _ => None,
    }
}

/// UIA を使用してフォーカス中コントロールの種別と IME 信頼度を判定する
///
/// Pattern-first アプローチ:
/// 1. `ValuePattern` → `IsReadOnly` で編集可能なテキストフィールドを検出
/// 2. `TextPattern` の有無でテキスト編集能力を検出
/// 3. `CurrentControlType` をフォールバックとして使用
///
/// さらに `FrameworkId` から IME 状態取得の信頼度を推定する。
///
/// Chrome/WPF/UWP など Win32 クラス名では判定できないコントロールに有効。
///
/// COM が初期化済みのスレッドから呼び出すこと
#[allow(unused_variables)] // hwnd はデバッグ用に保持
pub fn uia_classify_focus(automation: &IUIAutomation, hwnd: HWND) -> UiaClassifyResult {
    let element: IUIAutomationElement = match unsafe { automation.GetFocusedElement() } {
        Ok(el) => el,
        Err(e) => {
            log::trace!("UIA: GetFocusedElement failed: {e:?}");
            return UiaClassifyResult {
                focus_kind: FocusKind::Undetermined,
                ime_reliability: ImeReliability::Unknown,
                app_kind: None,
            };
        }
    };

    // FrameworkId を取得して IME 信頼度と AppKind を判定
    let (ime_reliability, app_kind) = match unsafe { element.CurrentFrameworkId() } {
        Ok(fid) => {
            let fid_str = fid.to_string();
            let reliability = classify_framework_id(&fid_str);
            let kind = classify_framework_app_kind(&fid_str);
            log::debug!("UIA: FrameworkId=\"{fid_str}\" → {reliability:?}, app_kind={kind:?}");
            (reliability, kind)
        }
        Err(e) => {
            log::trace!("UIA: CurrentFrameworkId failed: {e:?}");
            (ImeReliability::Unknown, None)
        }
    };

    // マクロ的ヘルパー: FocusKind, ime_reliability, app_kind をまとめて返す
    macro_rules! result {
        ($kind:expr) => {
            UiaClassifyResult {
                focus_kind: $kind,
                ime_reliability,
                app_kind,
            }
        };
    }

    // 1. ValuePattern → IsReadOnly チェック
    //    「編集可能な値を持つ」が最も強いシグナル
    if let Ok(pattern) =
        unsafe { element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) }
    {
        match unsafe { pattern.CurrentIsReadOnly() } {
            Ok(read_only) if !read_only.as_bool() => {
                log::debug!("UIA: ValuePattern(IsReadOnly=false) → TextInput");
                return result!(FocusKind::TextInput);
            }
            Ok(_) => {
                log::debug!("UIA: ValuePattern(IsReadOnly=true) → NonText");
                return result!(FocusKind::NonText);
            }
            Err(_) => {} // fall through
        }
    }

    // 2. TextPattern チェック
    //    TextPattern をサポートする要素はテキスト編集能力を持つ
    if unsafe {
        element
            .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
            .is_ok()
    } {
        log::debug!("UIA: TextPattern available → TextInput");
        return result!(FocusKind::TextInput);
    }

    // 3. フォールバック: ControlType で確定的な非テキストコントロールを判別
    if let Ok(control_type) = unsafe { element.CurrentControlType() } {
        // テキスト入力系（補助的な確認のみ）
        if control_type == UIA_EditControlTypeId || control_type == UIA_DocumentControlTypeId {
            log::debug!("UIA: ControlType={control_type:?} → TextInput");
            return result!(FocusKind::TextInput);
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
            return result!(FocusKind::NonText);
        }
    }

    // 4. 確定的なシグナルなし
    log::debug!("UIA: no definitive signal → Undetermined");
    result!(FocusKind::Undetermined)
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
            let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            if hr.is_err() {
                log::warn!("UIA: CoInitializeEx failed: {hr:?}");
            }
        }

        let automation: Option<IUIAutomation> =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok() };

        let Some(automation) = automation else {
            log::warn!("UIA: Failed to create IUIAutomation, Phase 3 disabled");
            return;
        };

        log::info!("UIA worker thread started");

        while let Ok(SendableHwnd(hwnd)) = rx.recv() {
            // GetFocusedElement はシステムのフォーカス要素を取得するため hwnd を直接使用しない。
            // hwnd は WM_FOCUS_KIND_UPDATE の LPARAM で返し、メインスレッド側で検証に使う。
            let result = uia_classify_focus(&automation, hwnd);
            let has_info = result.focus_kind != FocusKind::Undetermined
                || result.ime_reliability != ImeReliability::Unknown
                || result.app_kind.is_some();

            if has_info {
                log::debug!(
                    "UIA async: hwnd={hwnd:?} → {:?} (ime_reliability={:?}, app_kind={:?})",
                    result.focus_kind,
                    result.ime_reliability,
                    result.app_kind,
                );

                // メインスレッドに結果を送信
                // wParam: 下位 8 bit = FocusKind, 次の 8 bit = ImeReliability,
                //         次の 8 bit = AppKind (0xFF = なし)
                let app_kind_val = result.app_kind.map_or(0xFF_usize, |k| k as u8 as usize);
                let wparam_val = (result.focus_kind as u8 as usize)
                    | ((result.ime_reliability as u8 as usize) << 8)
                    | (app_kind_val << 16);
                unsafe {
                    let _ = PostMessageW(
                        HWND::default(),
                        crate::WM_FOCUS_KIND_UPDATE,
                        WPARAM(wparam_val),
                        LPARAM(hwnd.0 as isize),
                    );
                }
            }
        }
    });
    tx
}
