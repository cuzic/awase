#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! Phase 1: 同期フォーカス判定（クラス名 + IMM + スタイル + MSAA）

use crate::focus::FocusKind;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowLongW, GetWindowThreadProcessId, GWL_EXSTYLE, GWL_STYLE,
};

use super::msaa::msaa_classify;
use crate::state::focus_classify_plan::{decide_by_class, decide_by_ex_style, needs_edit_style};
use crate::win32::HwndExt as _;

pub use super::class_names::AppImeProfile;

pub use crate::state::focus_classify_plan::{ClassifyReason, ClassifyResult};

/// フォーカス中のウィンドウがテキスト入力を受け付けるかを判定する
///
/// deny-first（バイパスを優先）、allow は確信がある場合のみ。
/// 判定不能なら `Undetermined` を返す。
#[must_use]
#[tracing::instrument(level = "debug", skip_all)]
pub fn classify_focus(hwnd: HWND) -> ClassifyResult {
    if hwnd.non_null().is_none() {
        return ClassifyResult {
            kind: FocusKind::NonText,
            reason: ClassifyReason::NullHwnd,
        };
    }

    // 1. ImmGetContext — NULL でも NonText 確定にしない。
    // Windows 11 のメモ帳 (RichEditD2DPT) 等、TSF のみで IMM コンテキストを
    // 持たないテキストコントロールがあるため、Phase 2/3 に判断を委ねる。
    // SAFETY: hwnd は呼出元で NULL チェック済み。ImmContextGuard::new は IMM コンテキストを
    //         取得する Win32 API (ImmGetContext) のラッパーで、有効な HWND を渡せば安全。
    let _has_imm_context = unsafe { crate::imm::ImmContextGuard::new(hwnd).is_some() };

    // 2. WS_EX_NOIME ウィンドウスタイル
    // SAFETY: hwnd は呼出元で NULL チェック済み。GWL_EXSTYLE は有効な nIndex 値であり、
    //         GetWindowLongW は有効な HWND に対して安全に呼び出せる。
    let ex_style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) };
    if let Some(result) = decide_by_ex_style(ex_style) {
        return result;
    }

    // 3. クラス名（と Edit のときの GWL_STYLE）による判定
    let class_name = get_class_name_string(hwnd);
    let edit_style = needs_edit_style(&class_name).then(|| {
        // SAFETY: hwnd は呼出元で NULL チェック済み。GWL_STYLE は有効な nIndex 値で、
        //         GetWindowLongW は有効な HWND に対して安全に呼び出せる。
        unsafe { GetWindowLongW(hwnd, GWL_STYLE) }
    });
    if let Some(result) = decide_by_class(class_name, edit_style) {
        return result;
    }

    // 4. MSAA (IAccessible) role による判定
    msaa_classify(hwnd)
}

/// ウィンドウハンドルからクラス名を取得する
#[must_use]
pub fn get_class_name_string(hwnd: HWND) -> String {
    let mut class_buf = [0u16; 256];
    // SAFETY: hwnd は呼出元から渡された値。class_buf はスタック上の有効なバッファで、
    //         スライス長を上限として GetClassNameW に渡すため書き込み範囲は保証される。
    let len = unsafe { GetClassNameW(hwnd, &mut class_buf) };
    if len > 0 {
        #[expect(clippy::cast_sign_loss)] // len is guaranteed non-negative by GetClassNameW
        String::from_utf16_lossy(&class_buf[..len as usize])
    } else {
        String::new()
    }
}

/// ウィンドウハンドルからプロセス ID を取得する
#[must_use]
pub fn get_window_process_id(hwnd: HWND) -> u32 {
    let mut pid: u32 = 0;
    // SAFETY: hwnd は呼出元から渡された値。pid はスタック上の u32 変数で有効なポインタ。
    //         GetWindowThreadProcessId は NULL hwnd でも安全に 0 を返す Win32 API。
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
    pid
}

/// `hwnd` の top-level 祖先ウィンドウの hwnd を返す（`GetAncestor(hwnd, GA_ROOT)`）。
///
/// PR 109 コードレビュー指摘1 Step1: `CurrentFocus::hwnd` はフォーカス中コントロール
/// （`hwndFocus` 等）であり、必ずしも top-level ウィンドウとは限らない。この値は
/// 計測専用（`docs/known-bugs.md` 参照）であり、ADR-106 決定3 の判定ロジックには
/// 使わない。
#[must_use]
pub fn root_hwnd_of(hwnd: usize) -> usize {
    use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GA_ROOT};

    // SAFETY: hwnd は呼出元から渡された値。GetAncestor は無効な hwnd でも
    //         安全に NULL を返す Win32 API。
    let root = unsafe { GetAncestor(HWND(hwnd as *mut _), GA_ROOT) };
    root.0 as usize
}

/// プロセス ID から実行ファイル名を取得する
#[must_use]
pub fn get_process_name(process_id: u32) -> String {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // SAFETY: process_id は GetWindowThreadProcessId が返した有効な PID。
    //         OpenProcess に PROCESS_QUERY_LIMITED_INFORMATION を指定することで
    //         最小限の権限でハンドルを取得する。失敗時は Err を返すため安全。
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) })
    else {
        return String::new();
    };
    let mut buf = [0u16; 260];
    let mut len = buf.len() as u32;
    // SAFETY: handle は OpenProcess が返した有効なプロセスハンドル。
    //         buf はスタック上の有効なバッファで、len に容量を渡すため書き込み範囲が保証される。
    let ok = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &raw mut len,
        )
    };
    // SAFETY: handle は OpenProcess が返した有効なハンドル。CloseHandle は1回のみ呼ばれる。
    let _ = unsafe { CloseHandle(handle) };
    if ok.is_ok() && len > 0 {
        let path = String::from_utf16_lossy(&buf[..len as usize]); // len is non-negative
        path.rsplit('\\').next().unwrap_or(&path).to_string()
    } else {
        String::new()
    }
}
