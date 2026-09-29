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

/// 窓を探し直す回数と間隔(起動直後や再アクティブ化直後は UIA ツリーがまだ空・作り直し中のことがある)。
const RETRIES: usize = 10;
const RETRY_MS: u64 = 300;

/// `top` 配下の Edit 要素を走査順に集めて `pick` に渡し、`Some` を返すまで [`RETRIES`] 回まで探し直す。
/// 「Edit が 1 つでもあれば十分」なのか「目的の Edit が現れるまで待つ」のかは `pick` が決める。
pub(crate) fn wait_edits<T>(
    top: HWND,
    pick: impl Fn(Vec<IUIAutomationElement>) -> Option<T>,
) -> Option<T> {
    // SAFETY: UIA の COM 呼び出しのみ。戻り値の要素は呼び出し側が保持する。
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(ua) =
            CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
        else {
            log("[uia] UIA 初期化に失敗");
            return None;
        };
        for _ in 0..RETRIES {
            let scanned = (|| {
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
            if let Some(found) = scanned.and_then(&pick) {
                return Some(found);
            }
            sleep_ms(RETRY_MS);
        }
        None
    }
}

/// Edit 要素の名前(アクセシビリティ名)。取れなければ空文字列。
pub(crate) fn name_of(el: &IUIAutomationElement) -> String {
    // SAFETY: UIA プロパティの読み取りのみ。
    unsafe { el.CurrentName().map(|b| b.to_string()).unwrap_or_default() }
}

/// ValuePattern で値を読む。読めなければ `None`(空の値は `Some("")`)。
pub(crate) fn try_read_value(el: &IUIAutomationElement) -> Option<String> {
    // SAFETY: UIA パターンの取得と値の読み取りのみ。
    unsafe {
        el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .ok()?
            .CurrentValue()
            .ok()
            .map(|v| v.to_string())
    }
}

/// [`try_read_value`] の、読めない理由をセンチネル文字列にした版
/// (チェッカーが「入力欄が読めなかった」と「空だった」を区別できるようにするため)。
pub(crate) fn read_value(el: &IUIAutomationElement) -> String {
    try_read_value(el).unwrap_or_else(|| "<uia-no-value>".into())
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
