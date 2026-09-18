//! ADR-176: 較正画面でユーザーが今のIME ON/OFF状態を確認・操作できるように
//! するための、awase-settings自身のウィンドウに対するIME状態の読み書き。
//!
//! `awase-windows::imm`の較正probe（`ImmGetDefaultIMEWnd`と`SendMessageTimeoutW`
//! による`WM_IME_CONTROL`経由の読み書き）と同じ枯れた手法を、awase-settings
//! 自身のプロセス内で完結させて使う（awase.exeとのIPCは不要）。
//!
//! 読み書きの対象は必ず「現在最前面にある、かつ自分自身のプロセスのウィンドウ」に
//! 限定する。そうしないと、たまたま前面にある無関係な他アプリのIMEを誤って
//! 読み書きしてしまう。

#[cfg(target_os = "windows")]
mod windows_impl {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, SMTO_ABORTIFHUNG, SendMessageTimeoutW,
    };

    const WM_IME_CONTROL: u32 = 0x0283;
    const IMC_GETOPENSTATUS: usize = 0x0005;
    const IMC_SETOPENSTATUS: usize = 0x0006;
    const PROBE_TIMEOUT_MS: u32 = 200;

    /// 現在の最前面ウィンドウが自分自身（awase-settings.exe）のものであれば
    /// そのHWNDを返す。他アプリが前面にある場合は`None`。
    fn own_foreground_hwnd() -> Option<HWND> {
        // SAFETY: 引数を取らない単純な照会API、副作用なし。
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        // SAFETY: hwnd は直前に取得した有効なハンドル（NULLチェック済み）。
        unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
        // SAFETY: 引数を取らない単純な照会API。
        let current_pid = unsafe { GetCurrentProcessId() };
        (pid == current_pid).then_some(hwnd)
    }

    /// 現在のIME ON/OFF状態を照会する。自分自身のウィンドウが最前面に
    /// 無い、またはIMEウィンドウが取得できない場合は`None`
    /// （呼び出し元は「不明」として表示すること）。
    pub(crate) fn current_ime_open() -> Option<bool> {
        let hwnd = own_foreground_hwnd()?;
        // SAFETY: hwnd は own_foreground_hwnd が返した有効なハンドル。
        let ime_wnd = unsafe { ImmGetDefaultIMEWnd(hwnd) };
        if ime_wnd.0.is_null() {
            return None;
        }
        let mut result = 0usize;
        // SAFETY: ime_wnd は直前に取得した有効なIMEウィンドウハンドル。
        //         SMTO_ABORTIFHUNGによりハングした相手で無期限ブロックしない。
        let ok = unsafe {
            SendMessageTimeoutW(
                ime_wnd,
                WM_IME_CONTROL,
                WPARAM(IMC_GETOPENSTATUS),
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                PROBE_TIMEOUT_MS,
                Some(&raw mut result),
            )
        };
        (ok.0 != 0).then_some(result != 0)
    }

    /// IME ON/OFF状態を設定する。自分自身のウィンドウが最前面に無い場合は
    /// 何もせず`false`を返す。
    pub(crate) fn set_ime_open(open: bool) -> bool {
        let Some(hwnd) = own_foreground_hwnd() else {
            return false;
        };
        // SAFETY: hwnd は own_foreground_hwnd が返した有効なハンドル。
        let ime_wnd = unsafe { ImmGetDefaultIMEWnd(hwnd) };
        if ime_wnd.0.is_null() {
            return false;
        }
        let mut result = 0usize;
        // SAFETY: ime_wnd は直前に取得した有効なIMEウィンドウハンドル。
        let ok = unsafe {
            SendMessageTimeoutW(
                ime_wnd,
                WM_IME_CONTROL,
                WPARAM(IMC_SETOPENSTATUS),
                LPARAM(isize::from(open)),
                SMTO_ABORTIFHUNG,
                PROBE_TIMEOUT_MS,
                Some(&raw mut result),
            )
        };
        ok.0 != 0
    }
}

#[cfg(target_os = "windows")]
pub(crate) use windows_impl::{current_ime_open, set_ime_open};

#[cfg(not(target_os = "windows"))]
pub(crate) fn current_ime_open() -> Option<bool> {
    None
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn set_ime_open(_open: bool) -> bool {
    false
}
