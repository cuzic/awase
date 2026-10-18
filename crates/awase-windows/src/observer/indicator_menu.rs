#![allow(unsafe_code)]
// Win32 / COM の呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9と同じ作法)
//! ADR-264 段階 1: タスクバーの入力インジケーターの右クリックメニューを、利用者が開いたときだけ読む**観測**。
//!
//! 流れ（すべて読むだけ。IME へ何も書かない・belief へ入れない）:
//!
//! 1. [`install_menu_hook`] の WinEvent（`EVENT_SYSTEM_MENUPOPUPSTART`）が、メニュー窓（クラス `#32768`）の表示を検知する
//!    （メインスレッドのメッセージループ上で実行される。ADR-264 決定 1。`WH_*` フック・マウスフックは足さない）。
//! 2. コールバックは `Runtime::indicator_menu_on_popup_start`（`runtime/indicator_menu_shadow.rs`）へ渡すだけで、
//!    前置きのフィルタ（カーソルがタスクバーの上か・IME の同定・間隔）はそちらが判断する。
//! 3. 通ったものは、**メニュー窓の hwnd を読み取りワーカーへ送る**（[`spawn_indicator_menu_worker`]。UI Automation は
//!    専用スレッド＝STA で呼ぶ。固まってもメインスレッドは止まらない。`focus/uia.rs` のワーカーと同じ作法）。
//! 4. ワーカーは `AutomationElement.FromHandle` でメニュー窓の配下の `MenuItem` だけを読み（デスクトップ全体の
//!    `FindAll` は使わない。実測で右クリックから 29〜109 ms で読める）、純粋な判定
//!    （`state::indicator_menu::classify_menu`）にかけ、結果を固定長に符号化して `WM_INDICATOR_MENU_READ` で返す。
//!
//! メニューの名前・`AutomationId` は、判定が終わったらその場で捨てる（記録するのは判定の結果と件数・所要だけ。決定 6）。

use std::sync::mpsc;
use std::time::Instant;

use windows::core::w;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationLegacyIAccessiblePattern, SetWinEventHook,
    TreeScope_Descendants, UIA_LegacyIAccessiblePatternId, UIA_MenuItemControlTypeId,
    UnhookWinEvent, HWINEVENTHOOK,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetCursorPos, GetWindowRect, EVENT_SYSTEM_MENUPOPUPSTART, WINEVENT_OUTOFCONTEXT,
};

use crate::focus::uia::SendableHwnd;
use crate::state::indicator_menu::{classify_menu, rect_contains, MenuIme, MenuItem, MenuOutcome};

/// 読み取りワーカーへの依頼。
#[derive(Clone, Copy, Debug)]
pub struct MenuReadRequest {
    /// `EVENT_SYSTEM_MENUPOPUPSTART` で受けたメニュー窓
    pub menu_hwnd: SendableHwnd,
    /// 読むときに使う IME の表（呼び出し側が同定する）
    pub ime: MenuIme,
}

/// `SetWinEventHook` の RAII ガード。Drop 時に `UnhookWinEvent` を呼ぶ。
pub struct MenuHookGuard(HWINEVENTHOOK);

impl std::fmt::Debug for MenuHookGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("MenuHookGuard").field(&self.0 .0).finish()
    }
}

impl Drop for MenuHookGuard {
    fn drop(&mut self) {
        // SAFETY: `self.0` は `SetWinEventHook` が返した有効なフックハンドル。`Drop` は一度しか呼ばれない。
        unsafe {
            let _ = UnhookWinEvent(self.0);
        }
        tracing::info!("[indicator-menu] MENUPOPUPSTART hook uninstalled");
    }
}

/// `EVENT_SYSTEM_MENUPOPUPSTART` のコールバックの型（`SetWinEventHook` の `WINEVENTPROC`）。
///
/// コールバックの本体は `with_app` を呼ぶので、`observer/` ではなくオーケストレーター側（`app/bootstrap.rs`）に置く
/// （`layer_boundary_guard` の B-1: `observer/` 内で `with_app` を直接呼ばない）。
pub type MenuEventProc = unsafe extern "system" fn(HWINEVENTHOOK, u32, HWND, i32, i32, u32, u32);

/// `EVENT_SYSTEM_MENUPOPUPSTART` の WinEvent フックを登録する（段階 1 はこのイベントだけ。ADR-264 決定 1 の範囲
/// 0x4〜0x7 のうち、メニュー窓の表示だけを受ける）。失敗したら `None`（観測できないだけで、挙動は変わらない）。
#[must_use]
pub fn install_menu_hook(proc: MenuEventProc) -> Option<MenuHookGuard> {
    // SAFETY: `proc` は `'static` な extern "system" fn ポインタ（呼び出し側が渡す）。`WINEVENT_OUTOFCONTEXT` により
    //         コールバックはメッセージループスレッド（メインスレッド）で実行される。返されたフックは
    //         `WinEventHookGuard::drop` で `UnhookWinEvent` される。
    let hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_MENUPOPUPSTART,
            EVENT_SYSTEM_MENUPOPUPSTART,
            None,
            Some(proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if hook.is_invalid() {
        tracing::warn!("[indicator-menu] failed to install MENUPOPUPSTART hook");
        None
    } else {
        tracing::info!(
            "[indicator-menu] MENUPOPUPSTART hook installed (ADR-264 stage 1, observe only)"
        );
        Some(MenuHookGuard(hook))
    }
}

/// この hwnd がポップアップメニュー窓（クラス `#32768`）か。
#[must_use]
pub fn is_popup_menu_window(hwnd: HWND) -> bool {
    crate::focus::class_names::is_popup_menu_window_class(
        &crate::focus::classify::get_class_name_string(hwnd),
    )
}

/// カーソルがタスクバー（`Shell_TrayWnd`）の矩形の上にあるか。
///
/// ADR-264 決定 1 の前置きフィルタの段階 1 版: 入力インジケーターの矩形ではなくタスクバー全体で判定する
/// （インジケーターの矩形の取得は ADR-261 の実装に依存するため。ここで通ったメニューも、読んだ項目が IME の表と
/// **完全一致**しなければ `Miss` になるので、通知領域・タスクバーの別のメニューを誤って採ることはない）。
/// 矩形で判定するのは、`WindowFromPoint` だと、メニューがカーソルの直上に開いて境界で窓が入れ替わりうるため
/// （CI の実測: メニューの下端 y=744 とカーソルの y=744 が一致した）。補助タスクバーは対象外（未確認）。
#[must_use]
pub fn cursor_over_taskbar() -> bool {
    let mut pt = POINT::default();
    // SAFETY: `pt` はローカルの有効な POINT。
    if unsafe { GetCursorPos(&raw mut pt) }.is_err() {
        return false;
    }
    // SAFETY: 定数の窓クラス名で検索するだけ。
    let Ok(tray) = (unsafe { FindWindowW(w!("Shell_TrayWnd"), windows::core::PCWSTR::null()) })
    else {
        return false;
    };
    let mut r = RECT::default();
    // SAFETY: `tray` は FindWindowW が返した窓、`r` はローカルの有効な RECT。
    if unsafe { GetWindowRect(tray, &raw mut r) }.is_err() {
        return false;
    }
    rect_contains(r.left, r.top, r.right, r.bottom, pt.x, pt.y)
}

/// メニュー窓の配下の `MenuItem`（`AutomationId`・選択の有無）を読む。読めなければ空。
///
/// # Safety
/// `automation` は有効な `IUIAutomation`（COM 初期化済みのスレッドで作ったもの）でなければならない。
unsafe fn read_menu_items(automation: &IUIAutomation, menu: HWND) -> Vec<MenuItem> {
    let mut items = Vec::new();
    let Ok(root) = automation.ElementFromHandle(menu) else {
        return items;
    };
    let Ok(cond) = automation.CreateTrueCondition() else {
        return items;
    };
    let Ok(all) = root.FindAll(TreeScope_Descendants, &cond) else {
        return items;
    };
    let len = all.Length().unwrap_or(0);
    for i in 0..len {
        let Ok(el) = all.GetElement(i) else {
            continue;
        };
        // 区切り線（Separator）などは読まない。判定に使うのは MenuItem の `Item N` だけ。
        if el.CurrentControlType().ok() != Some(UIA_MenuItemControlTypeId) {
            continue;
        }
        let automation_id = el
            .CurrentAutomationId()
            .map(|b| b.to_string())
            .unwrap_or_default();
        // STATE_SYSTEM_CHECKED（0x10）。実機では SelectionItemPattern が取れず、このビットに現在のモードが出る。
        let selected = el
            .GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(
                UIA_LegacyIAccessiblePatternId,
            )
            .ok()
            .and_then(|p| p.CurrentState().ok())
            .is_some_and(|st| st & 0x10 != 0);
        items.push(MenuItem {
            automation_id,
            selected,
        });
    }
    items
}

/// 1 件を読んで判定し、固定長に符号化したもの `(wParam, lParam)`。
///
/// # Safety
/// `automation` は有効な `IUIAutomation`（COM 初期化済みのスレッドで作ったもの）でなければならない。
unsafe fn read_and_classify(automation: &IUIAutomation, req: MenuReadRequest) -> (usize, isize) {
    let started = Instant::now();
    let items = read_menu_items(automation, req.menu_hwnd.0);
    let read_ms = u16::try_from(started.elapsed().as_millis()).unwrap_or(u16::MAX);
    let outcome = MenuOutcome {
        ime: req.ime,
        result: classify_menu(req.ime, &items),
        item_count: u16::try_from(items.len()).unwrap_or(u16::MAX),
        read_ms,
    };
    outcome.pack()
}

/// 読み取りワーカーを起動する。専用スレッドで COM（STA）と `IUIAutomation` を持ち、依頼を 1 件ずつ処理して
/// `WM_INDICATOR_MENU_READ`（`wParam`/`lParam` = [`MenuOutcome::pack`]）をメインスレッドへ返す。
///
/// 戻り値の `WorkerThread` をアプリ終了まで保持すること（drop 時に停止・join される）。
#[must_use]
pub fn spawn_indicator_menu_worker() -> (win32_worker::WorkerThread, mpsc::Sender<MenuReadRequest>)
{
    let (tx, rx) = mpsc::channel::<MenuReadRequest>();
    let worker = win32_worker::WorkerThread::spawn("indicator-menu-worker", move |token| {
        // SAFETY: CoInitializeEx はスレッド開始直後に一度だけ呼ぶ。STA を初期化し、同スレッド内の COM 操作を安全にする。
        unsafe {
            let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            if hr.is_err() {
                tracing::warn!("[indicator-menu] CoInitializeEx failed: {hr:?}");
            }
        }
        // SAFETY: CoInitializeEx のあとに呼ぶ。失敗時は ok() が None を返す。
        let automation: Option<IUIAutomation> =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok() };
        let Some(automation) = automation else {
            tracing::warn!("[indicator-menu] failed to create IUIAutomation; shadow disabled");
            return;
        };
        tracing::info!("[indicator-menu] worker thread started");

        loop {
            // recv_timeout でシャットダウン通知も定期的に確認する
            match rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(req) => {
                    // SAFETY: `automation` は上で作った有効な COM オブジェクトで、同じスレッドから呼ぶ。
                    let (w, l) = unsafe { read_and_classify(&automation, req) };
                    crate::win32::post_to_main_thread_with(crate::WM_INDICATOR_MENU_READ, w, l);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if token.is_shutdown() {
                        tracing::info!(
                            "[indicator-menu] worker: shutdown signal received, exiting"
                        );
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    tracing::info!("[indicator-menu] worker: channel closed, exiting");
                    break;
                }
            }
        }
    });
    (worker, tx)
}
