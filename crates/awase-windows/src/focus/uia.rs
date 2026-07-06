//! Phase 3: UIA (UI Automation) パターンベース非同期判定

use std::sync::mpsc;

use crate::focus::{AppKind, FocusKind};
use windows::Win32::Foundation::HWND;
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

/// `HWND` を `Send` 可能にするラッパー
///
/// `HWND` は `*mut c_void` を含むため `Send` を実装していないが、
/// ウィンドウハンドルの値自体はスレッド間で安全に受け渡せる。
/// UIA ワーカースレッドへの HWND 送信専用。
#[derive(Clone, Copy)]
pub struct SendableHwnd(pub HWND);

impl std::fmt::Debug for SendableHwnd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SendableHwnd").finish()
    }
}
// Safety: HWND の値（ポインタ値）はスレッド間で安全に共有できる。
// ウィンドウハンドルはプロセス内でグローバルに有効であり、
// 別スレッドから参照しても問題ない。
unsafe impl Send for SendableHwnd {}

/// UIA 非同期判定の結果（FocusKind + AppKind）
#[derive(Debug)]
pub struct UiaClassifyResult {
    pub focus_kind: FocusKind,
    pub app_kind: Option<AppKind>,
}

/// `FrameworkId` から `AppKind` を推定する（CC≤3）
///
/// # Safety
/// `element` は有効な COM オブジェクトでなければならない。
unsafe fn resolve_app_kind(element: &IUIAutomationElement) -> Option<AppKind> {
    match element.CurrentFrameworkId() {
        Ok(fid) => {
            let fid_str = fid.to_string();
            let kind = match fid_str.as_str() {
                "Win32" | "WinForm" => Some(AppKind::Win32),
                "DirectUI" | "XAML" | "WPF" => Some(AppKind::Uwp),
                _ => None,
            };
            log::debug!("UIA: FrameworkId=\"{fid_str}\" → app_kind={kind:?}");
            kind
        }
        Err(e) => {
            log::trace!("UIA: CurrentFrameworkId failed: {e:?}");
            None
        }
    }
}

/// `ValuePattern` の `IsReadOnly` を確認して `FocusKind` を返す（CC≤4）
///
/// - `IsReadOnly=false` → `Some(TextInput)`
/// - `IsReadOnly=true`  → `Some(NonText)`
/// - パターン取得失敗・`IsReadOnly` 取得失敗 → `None`
///
/// # Safety
/// `element` は有効な COM オブジェクトでなければならない。
unsafe fn check_value_pattern(element: &IUIAutomationElement) -> Option<FocusKind> {
    let pattern = element
        .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
        .ok()?;
    match pattern.CurrentIsReadOnly() {
        Ok(read_only) if !read_only.as_bool() => {
            log::debug!("UIA: ValuePattern(IsReadOnly=false) → TextInput");
            Some(FocusKind::TextInput)
        }
        Ok(_) => {
            log::debug!("UIA: ValuePattern(IsReadOnly=true) → NonText");
            Some(FocusKind::NonText)
        }
        Err(_) => None,
    }
}

/// `TextPattern` の有無を確認して `FocusKind` を返す（CC≤2）
///
/// - パターンあり → `Some(TextInput)`
/// - パターンなし → `None`
///
/// # Safety
/// `element` は有効な COM オブジェクトでなければならない。
unsafe fn check_text_pattern(element: &IUIAutomationElement) -> Option<FocusKind> {
    if element
        .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
        .is_ok()
    {
        log::debug!("UIA: TextPattern available → TextInput");
        Some(FocusKind::TextInput)
    } else {
        None
    }
}

/// `ControlType` からテキスト入力可否を判定して `FocusKind` を返す（CC≤4）
///
/// - `Edit` / `Document` → `Some(TextInput)`
/// - 既知の非テキスト型   → `Some(NonText)`
/// - それ以外・取得失敗   → `None`
///
/// # Safety
/// `element` は有効な COM オブジェクトでなければならない。
unsafe fn check_control_type(element: &IUIAutomationElement) -> Option<FocusKind> {
    let control_type = element.CurrentControlType().ok()?;

    if control_type == UIA_EditControlTypeId || control_type == UIA_DocumentControlTypeId {
        log::debug!("UIA: ControlType={control_type:?} → TextInput");
        return Some(FocusKind::TextInput);
    }

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
        return Some(FocusKind::NonText);
    }

    None
}

/// UIA を使用してフォーカス中コントロールの種別を判定する（CC≤6）
///
/// Pattern-first アプローチ:
/// 1. `ValuePattern` → `IsReadOnly` で編集可能なテキストフィールドを検出
/// 2. `TextPattern` の有無でテキスト編集能力を検出
/// 3. `CurrentControlType` をフォールバックとして使用
///
/// さらに `FrameworkId` から `AppKind` を推定する。
///
/// Chrome/WPF/UWP など Win32 クラス名では判定できないコントロールに有効。
///
/// COM が初期化済みのスレッドから呼び出すこと
#[must_use]
pub fn uia_classify_focus(automation: &IUIAutomation, _hwnd: HWND) -> UiaClassifyResult {
    // SAFETY: automation は CoCreateInstance が返した有効な IUIAutomation COM オブジェクト。
    //         GetFocusedElement は COM が初期化済みのスレッドから呼び出されることが
    //         呼出元のコメントで保証されている。
    let element: IUIAutomationElement = match unsafe { automation.GetFocusedElement() } {
        Ok(el) => el,
        Err(e) => {
            log::trace!("UIA: GetFocusedElement failed: {e:?}");
            return UiaClassifyResult {
                focus_kind: FocusKind::Undetermined,
                app_kind: None,
            };
        }
    };

    // SAFETY: element は GetFocusedElement が返した有効な IUIAutomationElement COM オブジェクト。
    //         COM は初期化済みであり、AddRef 済みのポインタを保持している。
    let app_kind = unsafe { resolve_app_kind(&element) };

    // SAFETY: element は GetFocusedElement が返した有効な IUIAutomationElement COM オブジェクト。
    //         COM は初期化済みであり、AddRef 済みのポインタを保持している。
    if let Some(kind) = unsafe { check_value_pattern(&element) } {
        return UiaClassifyResult {
            focus_kind: kind,
            app_kind,
        };
    }

    // SAFETY: element は GetFocusedElement が返した有効な IUIAutomationElement COM オブジェクト。
    //         COM は初期化済みであり、AddRef 済みのポインタを保持している。
    if let Some(kind) = unsafe { check_text_pattern(&element) } {
        return UiaClassifyResult {
            focus_kind: kind,
            app_kind,
        };
    }

    // SAFETY: element は GetFocusedElement が返した有効な IUIAutomationElement COM オブジェクト。
    //         COM は初期化済みであり、AddRef 済みのポインタを保持している。
    if let Some(kind) = unsafe { check_control_type(&element) } {
        return UiaClassifyResult {
            focus_kind: kind,
            app_kind,
        };
    }

    log::debug!("UIA: no definitive signal → Undetermined");
    UiaClassifyResult {
        focus_kind: FocusKind::Undetermined,
        app_kind,
    }
}

/// UIA 非同期判定ワーカースレッドを起動する
///
/// 専用スレッドで COM を初期化し、`IUIAutomation` インスタンスを保持する。
/// チャネル経由で HWND を受け取り、`GetFocusedElement` でコントロール種別を判定して
/// `FOCUS_KIND` を更新する。Phase 1-2 で `Undetermined` だったコントロールの解像度を上げる。
///
/// 戻り値の `WorkerThread` をアプリ終了まで保持すること（drop 時に停止・join される）。
#[must_use]
pub fn spawn_uia_worker() -> (win32_worker::WorkerThread, mpsc::Sender<SendableHwnd>) {
    let (tx, rx) = mpsc::channel::<SendableHwnd>();
    let worker = win32_worker::WorkerThread::spawn("uia-worker", move |token| {
        // SAFETY: CoInitializeEx はスレッド開始直後に一度だけ呼ばれる。
        //         COINIT_APARTMENTTHREADED でシングルスレッドアパートメントを初期化し、
        //         同スレッド内の COM 操作が安全に行えるようになる。
        unsafe {
            let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            if hr.is_err() {
                log::warn!("UIA: CoInitializeEx failed: {hr:?}");
            }
        }

        // SAFETY: CoInitializeEx 呼び出し後に CoCreateInstance を実行する。
        //         CLSCTX_INPROC_SERVER でインプロセスサーバーを指定し、
        //         失敗時は ok() が None を返すため安全。
        let automation: Option<IUIAutomation> =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok() };

        let Some(automation) = automation else {
            log::warn!("UIA: Failed to create IUIAutomation, Phase 3 disabled");
            return;
        };

        log::info!("UIA worker thread started");

        loop {
            // recv_timeout でシャットダウン通知も定期的に確認する
            match rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(SendableHwnd(hwnd)) => {
                    // GetFocusedElement はシステムのフォーカス要素を取得するため hwnd を直接使用しない。
                    // hwnd は WM_FOCUS_KIND_UPDATE の LPARAM で返し、メインスレッド側で検証に使う。
                    let result = uia_classify_focus(&automation, hwnd);
                    let has_info =
                        result.focus_kind != FocusKind::Undetermined || result.app_kind.is_some();

                    if has_info {
                        log::debug!(
                            "UIA async: hwnd={hwnd:?} → {:?} (app_kind={:?})",
                            result.focus_kind,
                            result.app_kind,
                        );

                        // メインスレッドに結果を送信
                        // wParam: 下位 8 bit = FocusKind, 次の 8 bit = AppKind (FOCUS_KIND_UPDATE_NO_APP_KIND = なし)
                        let app_kind_val = result.app_kind.map_or_else(
                            || usize::from(crate::FOCUS_KIND_UPDATE_NO_APP_KIND),
                            |k| k as u8 as usize,
                        );
                        let wparam_val = (result.focus_kind as u8 as usize) | (app_kind_val << 8);
                        crate::win32::post_to_main_thread_with(
                            crate::WM_FOCUS_KIND_UPDATE,
                            wparam_val,
                            hwnd.0 as isize,
                        );
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if token.is_shutdown() {
                        log::info!("UIA worker: shutdown signal received, exiting");
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    log::info!("UIA worker: channel closed, exiting");
                    break;
                }
            }
        }
    });
    (worker, tx)
}
