//! Phase 1: 同期フォーカス判定（クラス名 + IMM + スタイル + MSAA）

use awase::types::FocusKind;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmReleaseContext};
use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetWindowLongW, GetWindowThreadProcessId, GWL_EXSTYLE, GWL_STYLE};

use super::msaa::msaa_classify;

/// `WS_EX_NOIME` (0x0040_0000) — IME 入力を受け付けないウィンドウスタイル
const WS_EX_NOIME: i32 = 0x0040_0000;

/// `ES_READONLY` (0x0800) — 読み取り専用 Edit コントロール
const ES_READONLY: i32 = 0x0800;

/// フォーカス中のウィンドウがテキスト入力を受け付けるかを判定する
///
/// deny-first（バイパスを優先）、allow は確信がある場合のみ。
/// 判定不能なら `Undetermined` を返す。
pub unsafe fn classify_focus(hwnd: HWND) -> FocusKind {
    if hwnd == HWND::default() {
        return FocusKind::NonText;
    }

    // 1. ImmGetContext == NULL → IME 入力不可
    let himc = ImmGetContext(hwnd);
    if himc.is_invalid() {
        return FocusKind::NonText;
    }
    let _ = ImmReleaseContext(hwnd, himc);

    // 2. WS_EX_NOIME ウィンドウスタイル
    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
    if ex_style & WS_EX_NOIME != 0 {
        return FocusKind::NonText;
    }

    // 3. クラス名による判定
    let class_name = get_class_name_string(hwnd);
    if !class_name.is_empty() {

        // 既知のテキスト入力コントロール
        if matches!(
            class_name.as_str(),
            "Edit"
                | "RichEdit"
                | "RichEdit20A"
                | "RichEdit20W"
                | "RICHEDIT50W"
                | "Scintilla"
                | "ConsoleWindowClass"
        ) {
            // Edit コントロールの読み取り専用チェック
            if class_name == "Edit" {
                let style = GetWindowLongW(hwnd, GWL_STYLE);
                if style & ES_READONLY != 0 {
                    return FocusKind::NonText;
                }
            }
            return FocusKind::TextInput;
        }

        // 既知の非テキストコントロール
        if matches!(
            class_name.as_str(),
            "Button"
                | "Static"
                | "SysListView32"
                | "SysTreeView32"
                | "SysHeader32"
                | "ToolbarWindow32"
                | "msctls_statusbar32"
                | "SysTabControl32"
                | "msctls_trackbar32"
                | "msctls_progress32"
        ) {
            return FocusKind::NonText;
        }
    }

    // 4. MSAA (IAccessible) role による判定
    msaa_classify(hwnd)
}

/// ウィンドウハンドルからクラス名を取得する
pub unsafe fn get_class_name_string(hwnd: HWND) -> String {
    let mut class_buf = [0u16; 256];
    let len = GetClassNameW(hwnd, &mut class_buf);
    if len > 0 {
        String::from_utf16_lossy(&class_buf[..len as usize])
    } else {
        String::new()
    }
}

/// ウィンドウハンドルからプロセス ID を取得する
pub(crate) unsafe fn get_window_process_id(hwnd: HWND) -> u32 {
    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    pid
}

/// プロセス ID から実行ファイル名を取得する
pub(crate) unsafe fn get_process_name(process_id: u32) -> String {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) else {
        return String::new();
    };
    let mut buf = [0u16; 260];
    let mut len = buf.len() as u32;
    let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len);
    let _ = CloseHandle(handle);
    if ok.is_ok() && len > 0 {
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit('\\').next().unwrap_or(&path).to_string()
    } else {
        String::new()
    }
}
