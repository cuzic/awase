//! ADR-176 176-T9b: 較正結果（`WM_CALIBRATION_RESULT`）を受信するための
//! メッセージ専用ウィンドウ。
//!
//! awase.exe本体は`FindWindowW`で固定クラス名
//! （`awase_windows::calibration_ipc::CALIBRATION_RESULT_WINDOW_CLASS_NAME`）
//! のウィンドウを探し、見つかれば`WM_CALIBRATION_RESULT`を`PostMessageW`
//! する（`awase-windows`側の`notify_calibration_result`参照）。
//!
//! opus-adversarial-consultレビュー（round8）で確定した設計:
//! `winit::platform::windows::EventLoopBuilderExtWindows::with_msg_hook`は
//! winitの`dispatch_peeked_messages`という特定のPeekMessage呼び出しの中
//! でしか呼ばれず、OS由来のモーダルループ（ウィンドウのサイズ変更・
//! システムメニュー等）に対して構造的に脆い。生のWin32メッセージ専用
//! ウィンドウ（`HWND_MESSAGE`）を自前で作り、独自の`WndProc`で受ける方式
//! なら、Windowsのメッセージキューがスレッド単位（ウィンドウ単位ではない）
//! であることを利用して、**このスレッド上のどんなメッセージループから
//! `DispatchMessage`されても**正しくメッセージを受け取れる（winit/eframe
//! のイベントループが同じスレッドで回る限り、追加のメッセージポンプは
//! 不要）。
//!
//! 作成タイミングはeframeのイベントループ開始前・早期exit分岐
//! （`--bug-report`等）の後（`main()`参照）。`run_with_fallback`が
//! glow→wgpuの順で`eframe::run_native`を最大2回呼ぶが、このウィンドウは
//! それより前に一度だけ作成するため、二重登録の心配はない。

use awase_windows::calibration_ipc::CALIBRATION_RESULT_WINDOW_CLASS_NAME;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, HWND_MESSAGE, RegisterClassW, WINDOW_EX_STYLE,
    WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::PCWSTR;

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == awase_windows::WM_CALIBRATION_RESULT {
        if let Some(payload) = awase_windows::calibration_ipc::unpack_result(wparam.0) {
            // T9b時点ではログに残すのみ——T10の較正パネルUIがこの結果を
            // 画面表示に反映する（進捗表示/確定表示等）。
            tracing::info!(
                "[calibration] 結果を受信: vk={:?} kind={:?}",
                payload.vk,
                payload.kind
            );
        } else {
            tracing::warn!(
                "[calibration] 結果通知のペイロードを解釈できませんでした \
                 (wparam=0x{:X})",
                wparam.0
            );
        }
        return LRESULT(0);
    }
    // SAFETY: hwnd/msg/wparam/lparam はOSがWndProcへ渡した値をそのまま
    //         転送するだけであり、DefWindowProcWは未処理メッセージの
    //         標準的な処理先。
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// 較正結果受信用のメッセージ専用ウィンドウを作成する。`main()`冒頭、
/// `startup_failure::run_with_fallback`呼び出しより前に1回だけ呼ぶこと。
/// 失敗しても致命的ではない（較正結果を受け取れなくなるだけで、
/// awase-settings自体の起動は継続する）ため、戻り値は無く警告ログのみ
/// 残す。
pub(crate) fn create() {
    let class_name_wide = awase_windows::win32::to_wide(CALIBRATION_RESULT_WINDOW_CLASS_NAME);
    // SAFETY: GetModuleHandleW(None) は現在のモジュール自身のハンドルを
    //         返すだけで副作用が無い。
    let hinstance = match unsafe { GetModuleHandleW(None) } {
        Ok(h) => h,
        Err(err) => {
            tracing::warn!(
                "[calibration] 結果受信ウィンドウ用のモジュールハンドル取得に\
                 失敗しました: {err}"
            );
            return;
        }
    };

    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: hinstance.into(),
        lpszClassName: PCWSTR(class_name_wide.as_ptr()),
        ..Default::default()
    };
    // SAFETY: wc は上で構築した有効なWNDCLASSW。class_name_wideはこの
    //         呼び出しが完了するまでスコープ内で生存する。
    let atom = unsafe { RegisterClassW(&raw const wc) };
    if atom == 0 {
        tracing::warn!(
            "[calibration] 結果受信ウィンドウのクラス登録に失敗しました\
             （較正結果を受け取れません）"
        );
        return;
    }

    // SAFETY: atom は直前の RegisterClassW が返した有効なクラスアトム。
    //         HWND_MESSAGE はメッセージ専用ウィンドウの標準的な親指定。
    let result = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class_name_wide.as_ptr()),
            PCWSTR::null(),
            WS_OVERLAPPED,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            HWND_MESSAGE,
            None,
            hinstance,
            None,
        )
    };
    if let Err(err) = result {
        tracing::warn!(
            "[calibration] 結果受信ウィンドウの作成に失敗しました\
             （較正結果を受け取れません）: {err}"
        );
    }
}
