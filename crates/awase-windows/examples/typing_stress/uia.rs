//! UI Automation で別プロセスの入力欄を読み書きする共通部品(Chrome・awase-settings など、
//! `WM_GETTEXT` が届かない入力先用)。入力先ごとに「どの Edit を選ぶか」だけ述語で渡す。

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationValuePattern,
    TreeScope_Descendants, UIA_EditControlTypeId, UIA_ValuePatternId,
};

use crate::{log, press, send_key, sleep_ms};

/// `find_edit` が窓を探し直す回数と間隔(起動直後は UIA ツリーがまだ空のことがある)。
const RETRIES: usize = 10;
const RETRY_MS: u64 = 300;

/// `top` 配下の Edit 要素を走査順に返す。空なら [`RETRIES`] 回まで待って探し直す。
pub(crate) fn edit_elements(top: HWND) -> Vec<IUIAutomationElement> {
    // SAFETY: UIA の COM 呼び出しのみ。戻り値の要素は呼び出し側が保持する。
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(ua) =
            CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
        else {
            log("[uia] UIA 初期化に失敗");
            return Vec::new();
        };
        for _ in 0..RETRIES {
            let found = (|| {
                let root = ua.ElementFromHandle(top).ok()?;
                let cond = ua.CreateTrueCondition().ok()?;
                let all = root.FindAll(TreeScope_Descendants, &cond).ok()?;
                let mut edits = Vec::new();
                for i in 0..all.Length().unwrap_or(0) {
                    let Ok(el) = all.GetElement(i) else { continue };
                    if el.CurrentControlType().ok() == Some(UIA_EditControlTypeId) {
                        edits.push(el);
                    }
                }
                Some(edits)
            })();
            if let Some(edits) = found.filter(|e| !e.is_empty()) {
                return edits;
            }
            sleep_ms(RETRY_MS);
        }
        Vec::new()
    }
}

/// Edit 要素の名前(アクセシビリティ名)。取れなければ空文字列。
pub(crate) fn name_of(el: &IUIAutomationElement) -> String {
    // SAFETY: UIA プロパティの読み取りのみ。
    unsafe { el.CurrentName().map(|b| b.to_string()).unwrap_or_default() }
}

/// ValuePattern で値を読む。読めない理由はセンチネル文字列にして返す
/// (チェッカーが「入力欄が読めなかった」と「空だった」を区別できるようにするため)。
pub(crate) fn read_value(el: &IUIAutomationElement) -> String {
    // SAFETY: UIA パターンの取得と値の読み取りのみ。
    unsafe {
        el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .map_or_else(
                |_| "<uia-no-value-pattern>".into(),
                |vp| vp.CurrentValue().map(|v| v.to_string()).unwrap_or_default(),
            )
    }
}

pub(crate) const NOT_FOUND: &str = "<uia-not-found>";

/// フォーカス済みの入力欄を Ctrl+A → Backspace で空にする。ValuePattern の SetValue は、
/// 外部からの値書き換えに対応していない入力先(egui/accesskit など)があるため使わない。
pub(crate) fn clear_focused() {
    send_key(0x11, 0x1D, true);
    sleep_ms(20);
    press(0x41, 0x1E, 30);
    send_key(0x11, 0x1D, false);
    sleep_ms(50);
    press(0x08, 0x0E, 30);
    sleep_ms(150);
}
