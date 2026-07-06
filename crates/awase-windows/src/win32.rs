//! Windows API の安全ラッパー

use std::time::Duration;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
};

/// タイムアウト付きで任意の処理をワーカースレッドで実行する。
///
/// `win32_async::run_with_timeout` の re-export。
pub use win32_async::run_with_timeout;

/// `HWND` の null チェック拡張トレイト。
pub trait HwndExt {
    /// null なら `None`、非 null なら `Some(self)` を返す。
    ///
    /// Win32 API が返す `HWND` は null（フォーカスなし・失敗）を示すことがある。
    /// 境界でこのメソッドを使い、以降は `Option<HWND>` として処理する。
    #[must_use]
    fn non_null(self) -> Option<HWND>;
}

impl HwndExt for HWND {
    fn non_null(self) -> Option<HWND> {
        (!self.0.is_null()).then_some(self)
    }
}

/// メインスレッド（エンジンスレッド）のメッセージキューにカスタムメッセージを POST する。
///
/// `PostThreadMessageW(engine_thread_id(), ..)` のラッパー。
///
/// 旧実装は `PostMessageW(None, ..)` を使っていたが、hwnd=NULL の `PostMessageW` は
/// 「**呼び出しスレッド自身**への `PostThreadMessage`」と等価（Microsoft docs）であり、
/// ワーカースレッド（gji-io-monitor / UIA worker 等）から呼ぶとメッセージが誰にも
/// 処理されず消失していた。これにより `WM_IME_KIND_CHANGED` が main に一度も届かず、
/// MS-IME 環境でも warmup 戦略がデフォルトの GjiFsm のまま走り続けた
/// （docs/known-bugs.md BUG-09）。`WM_FOCUS_KIND_UPDATE`（UIA worker 発）も同罪だった。
pub fn post_to_main_thread(msg: u32) {
    post_to_main_thread_with(msg, 0, 0);
}

/// メインスレッドのメッセージキューにパラメータ付きでカスタムメッセージを POST する。
///
/// スレッド安全: どのスレッドから呼んでも main（エンジン）スレッドに届く。
pub fn post_to_main_thread_with(msg: u32, wparam: usize, lparam: isize) {
    let tid = crate::engine_thread_id();
    if tid == 0 {
        // メッセージループ開始前（run_message_loop が TID を設定する前）。
        // この時点で呼び出せるのは初期化中の main スレッド自身に限られるため、
        // 自スレッドのキューへの投函（旧動作）が正しい。キューは PostMessageW 自身が
        // 生成し、ループ開始後に取り出される。
        // SAFETY: msg はプロセス定義のカスタムメッセージ ID。
        let _ = unsafe {
            windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                None,
                msg,
                windows::Win32::Foundation::WPARAM(wparam),
                windows::Win32::Foundation::LPARAM(lparam),
            )
        };
        return;
    }
    // SAFETY: tid は run_message_loop 先頭で設定された有効なスレッド ID。
    //         msg はプロセス定義のカスタムメッセージ ID。
    if unsafe {
        windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
            tid,
            msg,
            windows::Win32::Foundation::WPARAM(wparam),
            windows::Win32::Foundation::LPARAM(lparam),
        )
    }
    .is_err()
    {
        log::warn!("[post-main] PostThreadMessageW failed msg=0x{msg:X}");
    }
}

/// `SendInput` の安全ラッパー（`size_of` キャストを安全に処理）
///
/// # Panics
/// `INPUT` のサイズが `i32` に収まらない場合（実際には起こらない）。
#[must_use]
pub(crate) fn send_input_safe(inputs: &[INPUT]) -> u32 {
    let size = i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32");
    // SAFETY: inputs スライスは呼び出し中有効であり、size は sizeof::<INPUT>() の正確な値。
    //         SendInput はスライスの範囲外を読まない。
    unsafe { SendInput(inputs, size) }
}

/// `&str` を NUL 終端 UTF-16 `Vec<u16>` に変換する。
///
/// Win32 API に渡す `PCWSTR` を作るときの定型句を集約する。
#[must_use]
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `GetGUIThreadInfo` の結果
#[derive(Debug, Clone, Copy)]
pub struct GuiThreadResult {
    /// フォーカスを持つウィンドウ。null（フォーカスなし）の場合は `None`。
    pub focused_hwnd: Option<HWND>,
    /// ウィンドウが属するスレッド ID（0 = 取得失敗）
    pub thread_id: u32,
}

/// `GetGUIThreadInfo(0, ...)` のラッパー — ブロッキングが一定時間を超えたら
/// フォールバックとして `GetForegroundWindow()` を返す。
///
/// `GetGUIThreadInfo` はフォアグラウンドウィンドウの GUI スレッドにメッセージを送るため、
/// 対象スレッドがハングしていると無期限にブロックする。
/// `run_with_timeout` でワーカースレッドで実行し、タイムアウト時は
/// 非ブロッキングな `GetForegroundWindow` にフォールバックする。
///
/// # Panics
/// `GUITHREADINFO` のサイズが `u32` に収まらない場合（実際には起こらない）。
///
/// # Safety
/// Win32 API を呼び出す。
#[must_use]
pub unsafe fn get_gui_thread_info_with_timeout(timeout: Duration) -> GuiThreadResult {
    // HWND はポインタだが、スレッド間で安全に送信可能
    // （Win32 ウィンドウハンドルはプロセス内で有効なグローバルリソース）
    struct SendableResult(Option<HWND>, u32);
    unsafe impl Send for SendableResult {}

    let result = run_with_timeout(timeout, || {
        let mut info = GUITHREADINFO {
            cbSize: u32::try_from(size_of::<GUITHREADINFO>())
                .expect("GUITHREADINFO size is a small constant that always fits in u32"),
            ..Default::default()
        };
        // SAFETY: info は cbSize を正しく設定したスタック上の有効な構造体。
        //         GetGUIThreadInfo(0, ...) はフォアグラウンドスレッドの情報を取得する。
        //         GetForegroundWindow / GetWindowThreadProcessId はどのスレッドからも安全に呼べる。
        unsafe {
            if GetGUIThreadInfo(0, &raw mut info).is_ok() {
                // hwndFocus が null なら hwndActive を使う
                let hwnd = info
                    .hwndFocus
                    .non_null()
                    .or_else(|| info.hwndActive.non_null());
                let tid = hwnd.map_or(0, |h| {
                    let mut pid = 0u32;
                    GetWindowThreadProcessId(h, Some(&raw mut pid))
                });
                SendableResult(hwnd, tid)
            } else {
                SendableResult(GetForegroundWindow().non_null(), 0)
            }
        }
    });

    match result {
        Some(SendableResult(hwnd, tid)) => GuiThreadResult {
            focused_hwnd: hwnd,
            thread_id: tid,
        },
        None => {
            // フォールバック: GetForegroundWindow は非ブロッキング
            // SAFETY: GetForegroundWindow はどのスレッドからも安全に呼べる非ブロッキング API。
            GuiThreadResult {
                focused_hwnd: unsafe { GetForegroundWindow() }.non_null(),
                thread_id: 0,
            }
        }
    }
}
