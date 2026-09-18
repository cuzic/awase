//! ADR-176: 較正画面でユーザーが今のIME ON/OFF状態を確認・操作できるように
//! するための、awase-settings自身のウィンドウに対するIME状態の読み取りと、
//! awase.exeへの明示的なIME状態セットアップ要求。
//!
//! **読み取り**は`awase-windows::imm`の較正probeと同じ枯れた手法
//! （`ImmGetDefaultIMEWnd`と`SendMessageTimeoutW`による`WM_IME_CONTROL`
//! 経由）を、awase-settings自身のプロセス内で完結させて使う（IPC不要）。
//! 対象は必ず「現在最前面にある、かつ自分自身のプロセスのウィンドウ」に
//! 限定する。そうしないと、たまたま前面にある無関係な他アプリのIMEを
//! 誤って読み取ってしまう。
//!
//! **書き込み**（IME ON/OFFボタン）は、当初`IMC_SETOPENSTATUS`の直接送信を
//! 試みたが実機検証で効果が無いと判明した（GJIはTSFベースで、
//! `WM_IME_CONTROL`経由の書き込みはTSF側のCOMインタフェースを経由しない
//! ため反映されない）。次に物理キー（`VK_KANJI`）相当のSendInputを試みたが、
//! これもBUG-14（注入されたIMEモードキーをユーザー意図として扱わない
//! 安全策）に意図的に拒否されて効果が無かった（実機ログ`[shadow-toggle]
//! injected IMEキー vk=0x19 はユーザー意図に処理しない (BUG-14)`で確認）。
//! 最終的に`WM_CALIBRATION_SET_IME_OPEN`をawase.exeへ送り、
//! `UserIntentSource::Command`経由の正規のIME actuation
//! （`state/platform_state.rs::handle_engine_set_open`）を呼んでもらう
//! 方式に落ち着いた。

#[cfg(target_os = "windows")]
mod windows_impl {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetForegroundWindow, GetWindowThreadProcessId, PostMessageW, SMTO_ABORTIFHUNG,
        SendMessageTimeoutW,
    };
    use windows::core::w;

    const WM_IME_CONTROL: u32 = 0x0283;
    const IMC_GETOPENSTATUS: usize = 0x0005;
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

    /// awase.exeへ`WM_CALIBRATION_SET_IME_OPEN`を送り、IME ON/OFFの
    /// セットアップを依頼する。awase.exeが見つからない場合は`false`。
    pub(crate) fn set_ime_open(open: bool) -> bool {
        let payload = awase_windows::calibration_ipc::CalibrationSetImeOpenPayload {
            pid: std::process::id(),
            open,
        };
        // SAFETY: FindWindowW/PostMessageWはどちらも通常のWin32 API呼び出し。
        //         見つかったHWNDへ既定のIPCペイロードを送るだけで副作用は
        //         awase.exe側のハンドラに委ねる。
        unsafe {
            let Ok(hwnd) = FindWindowW(w!("awase_tray_window"), None) else {
                tracing::warn!(
                    "[calibration] IME状態セットアップ通知の送信先ウィンドウ \
                     (awase_tray_window) が見つかりません。awase.exe が起動して\
                     いない可能性があります。"
                );
                return false;
            };
            let wparam = WPARAM(awase_windows::calibration_ipc::pack_set_ime_open(payload));
            PostMessageW(
                hwnd,
                awase_windows::WM_CALIBRATION_SET_IME_OPEN,
                wparam,
                LPARAM(0),
            )
            .is_ok()
        }
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
