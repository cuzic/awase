//! ADR-176: 較正画面でユーザーが今のIME ON/OFF状態を確認・操作できるように
//! するための、awase-settings自身のウィンドウに対するIME状態の読み取りと、
//! IME ON/OFF切り替えの複数手法。
//!
//! **読み取り**は`awase-windows::imm`の較正probeと同じ枯れた手法
//! （`ImmGetDefaultIMEWnd`と`SendMessageTimeoutW`による`WM_IME_CONTROL`
//! 経由）を、awase-settings自身のプロセス内で完結させて使う（IPC不要）。
//! 対象は必ず「現在最前面にある、かつ自分自身のプロセスのウィンドウ」に
//! 限定する。そうしないと、たまたま前面にある無関係な他アプリのIMEを
//! 誤って読み取ってしまう。
//!
//! **書き込み**は実機検証で複数の手法が効かないことが判明したため
//! （`IMC_SETOPENSTATUS`直接送信・`VK_KANJI`のSendInput・
//! `WM_CALIBRATION_SET_IME_OPEN`経由のCommandアクチュエーション、いずれも
//! 実機で無効果）、比較検証のため複数手法を並べて実装している
//! （`set_ime_open_*`各関数）。効果があった手法が分かり次第、他は削除する
//! 想定の一時的な診断コード。

#[cfg(target_os = "windows")]
mod windows_impl {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::Input::Ime::{
        ImmGetContext, ImmGetDefaultIMEWnd, ImmReleaseContext, ImmSetOpenStatus,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
        VIRTUAL_KEY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetForegroundWindow, GetWindowThreadProcessId, PostMessageW, SMTO_ABORTIFHUNG,
        SendMessageTimeoutW,
    };
    use windows::core::w;

    const WM_IME_CONTROL: u32 = 0x0283;
    const IMC_GETOPENSTATUS: usize = 0x0005;
    const IMC_SETOPENSTATUS: usize = 0x0006;
    const PROBE_TIMEOUT_MS: u32 = 200;
    /// 漢字キー（IME ON/OFFの伝統的なトグルキー、`keys.ime_toggle`既定値）。
    const VK_KANJI: u16 = 0x19;
    /// Windows標準の冪等IME ONキー。GJIがネイティブに処理する
    /// （`ime_controller.rs::GjiDirectStrategy`が実際に使う本番の手法）。
    const VK_IME_ON: u16 = 0x16;
    /// Windows標準の冪等IME OFFキー（同上）。
    const VK_IME_OFF: u16 = 0x1A;

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

    /// 手法A: `ImmGetContext`/`ImmSetOpenStatus`/`ImmReleaseContext`を
    /// 自分自身のウィンドウに対して直接呼ぶ、最も素朴なIMM32 API。
    pub(crate) fn set_ime_open_immset(open: bool) -> bool {
        let Some(hwnd) = own_foreground_hwnd() else {
            return false;
        };
        // SAFETY: hwnd は own_foreground_hwnd が返した有効なハンドル。
        //         ImmReleaseContextをImmGetContextと対で呼ぶ。
        unsafe {
            let himc = ImmGetContext(hwnd);
            if himc.is_invalid() {
                return false;
            }
            let ok = ImmSetOpenStatus(himc, open).as_bool();
            let _ = ImmReleaseContext(hwnd, himc);
            ok
        }
    }

    /// 手法B: `ImmGetDefaultIMEWnd`が返すIMEウィンドウへ`WM_IME_CONTROL`+
    /// `IMC_SETOPENSTATUS`を送る（awase-windows側のクロスプロセスprobeと
    /// 同じ経路を書き込みにも使う）。
    pub(crate) fn set_ime_open_wm_control(open: bool) -> bool {
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

    /// 手法C: `VK_IME_ON`/`VK_IME_OFF`（一方向の冪等キー）をSendInputで
    /// 送る。`ime_controller.rs::GjiDirectStrategy`が実際に使う本番の
    /// 手法と同じVKコード。
    pub(crate) fn set_ime_open_dedicated_vk(open: bool) -> bool {
        let vk = if open { VK_IME_ON } else { VK_IME_OFF };
        send_vk_pair(vk)
    }

    /// 手法D: `VK_KANJI`（トグルキー）をSendInputで送る。既に目的の状態で
    /// あれば送らない。
    pub(crate) fn set_ime_open_kanji_toggle(open: bool) -> bool {
        if current_ime_open() == Some(open) {
            return true;
        }
        send_vk_pair(VK_KANJI)
    }

    /// 手法E: awase.exeへ`WM_CALIBRATION_SET_IME_OPEN`を送り、
    /// `UserIntentSource::Command`経由の正規のIME actuationを依頼する。
    pub(crate) fn set_ime_open_command_ipc(open: bool) -> bool {
        let payload = awase_windows::calibration_ipc::CalibrationSetImeOpenPayload {
            pid: std::process::id(),
            open,
        };
        // SAFETY: FindWindowW/PostMessageWはどちらも通常のWin32 API呼び出し。
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

    fn send_vk_pair(vk: u16) -> bool {
        let inputs = [key_input(vk, false), key_input(vk, true)];
        let size = i32::try_from(size_of::<INPUT>())
            .expect("size_of::<INPUT>() is a small constant that always fits in i32");
        // SAFETY: inputs はスタック上の有効な配列で、size は要素の実サイズと一致する。
        let sent = unsafe { SendInput(&inputs, size) };
        sent as usize == inputs.len()
    }

    const fn key_input(vk: u16, is_keyup: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: 0,
                    dwFlags: if is_keyup {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) use windows_impl::{
    current_ime_open, set_ime_open_command_ipc, set_ime_open_dedicated_vk, set_ime_open_immset,
    set_ime_open_kanji_toggle, set_ime_open_wm_control,
};

#[cfg(not(target_os = "windows"))]
pub(crate) fn current_ime_open() -> Option<bool> {
    None
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn set_ime_open_immset(_open: bool) -> bool {
    false
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn set_ime_open_wm_control(_open: bool) -> bool {
    false
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn set_ime_open_dedicated_vk(_open: bool) -> bool {
    false
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn set_ime_open_kanji_toggle(_open: bool) -> bool {
    false
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn set_ime_open_command_ipc(_open: bool) -> bool {
    false
}
