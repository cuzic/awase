//! Phase 1: 同期フォーカス判定（クラス名 + IMM + スタイル + MSAA）

use awase::types::FocusKind;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmReleaseContext};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowLongW, GetWindowThreadProcessId, GWL_EXSTYLE, GWL_STYLE,
};

use super::msaa::msaa_classify;

/// `WS_EX_NOIME` (0x0040_0000) — IME 入力を受け付けないウィンドウスタイル
const WS_EX_NOIME: i32 = 0x0040_0000;

/// `ES_READONLY` (0x0800) — 読み取り専用 Edit コントロール
const ES_READONLY: i32 = 0x0800;

/// フォーカス判定の結果と根拠
#[derive(Debug)]
pub struct ClassifyResult {
    pub kind: FocusKind,
    pub reason: ClassifyReason,
}

/// 判定根拠
#[derive(Debug)]
pub enum ClassifyReason {
    /// hwnd が NULL
    NullHwnd,
    /// ImmGetContext が NULL を返した（IME コンテキストなし）
    NoImeContext,
    /// WS_EX_NOIME ウィンドウスタイル
    NoImeStyle,
    /// Edit コントロールの ES_READONLY
    ReadOnlyEdit,
    /// 既知のテキスト入力クラス名
    KnownTextClass(String),
    /// 既知の非テキストクラス名
    KnownNonTextClass(String),
    /// MSAA ロールによる判定
    MsaaRole(String),
    /// Phase 1-2 で判定不能
    Undetermined,
}

impl std::fmt::Display for ClassifyReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NullHwnd => write!(f, "NullHwnd"),
            Self::NoImeContext => write!(f, "NoImeContext"),
            Self::NoImeStyle => write!(f, "NoImeStyle"),
            Self::ReadOnlyEdit => write!(f, "ReadOnlyEdit"),
            Self::KnownTextClass(c) => write!(f, "KnownTextClass({c})"),
            Self::KnownNonTextClass(c) => write!(f, "KnownNonTextClass({c})"),
            Self::MsaaRole(r) => write!(f, "MsaaRole({r})"),
            Self::Undetermined => write!(f, "Undetermined"),
        }
    }
}

/// フォーカス中のウィンドウがテキスト入力を受け付けるかを判定する
///
/// deny-first（バイパスを優先）、allow は確信がある場合のみ。
/// 判定不能なら `Undetermined` を返す。
pub unsafe fn classify_focus(hwnd: HWND) -> ClassifyResult {
    if hwnd == HWND::default() {
        return ClassifyResult {
            kind: FocusKind::NonText,
            reason: ClassifyReason::NullHwnd,
        };
    }

    // 1. ImmGetContext == NULL → IME 入力不可
    let himc = ImmGetContext(hwnd);
    if himc.is_invalid() {
        return ClassifyResult {
            kind: FocusKind::NonText,
            reason: ClassifyReason::NoImeContext,
        };
    }
    let _ = ImmReleaseContext(hwnd, himc);

    // 2. WS_EX_NOIME ウィンドウスタイル
    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
    if ex_style & WS_EX_NOIME != 0 {
        return ClassifyResult {
            kind: FocusKind::NonText,
            reason: ClassifyReason::NoImeStyle,
        };
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
                    return ClassifyResult {
                        kind: FocusKind::NonText,
                        reason: ClassifyReason::ReadOnlyEdit,
                    };
                }
            }
            return ClassifyResult {
                kind: FocusKind::TextInput,
                reason: ClassifyReason::KnownTextClass(class_name),
            };
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
            return ClassifyResult {
                kind: FocusKind::NonText,
                reason: ClassifyReason::KnownNonTextClass(class_name),
            };
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
        #[allow(clippy::cast_sign_loss)] // len is guaranteed non-negative by GetClassNameW
        String::from_utf16_lossy(&class_buf[..len as usize])
    } else {
        String::new()
    }
}

/// ウィンドウハンドルからプロセス ID を取得する
pub unsafe fn get_window_process_id(hwnd: HWND) -> u32 {
    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&raw mut pid));
    pid
}

/// プロセス ID から実行ファイル名を取得する
pub unsafe fn get_process_name(process_id: u32) -> String {
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
    let ok = QueryFullProcessImageNameW(
        handle,
        PROCESS_NAME_WIN32,
        windows::core::PWSTR(buf.as_mut_ptr()),
        &raw mut len,
    );
    let _ = CloseHandle(handle);
    if ok.is_ok() && len > 0 {
        let path = String::from_utf16_lossy(&buf[..len as usize]); // len is non-negative
        path.rsplit('\\').next().unwrap_or(&path).to_string()
    } else {
        String::new()
    }
}
