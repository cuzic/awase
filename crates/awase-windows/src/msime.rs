//! Microsoft IME キー割り当て自動設定
//!
//! NICOLA 親指シフト入力に最適化するため、MSIME の無変換/変換キーに
//! IME-OFF/ON を割り当てる。
//!
//! 設定するレジストリ値:
//!   HKCU\Software\Microsoft\IME\15.0\IMEJP\MSIME
//!     IsKeyAssignmentEnabled = DWORD:1
//!     KeyAssignmentMuhenkan  = DWORD:1  (無変換 → IME オフ)
//!     KeyAssignmentHenkan    = DWORD:0  (変換   → ひらがな/IME オン)

use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::System::Registry::{
    RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD, REG_DWORD,
};

const MSIME_SUBKEY: windows::core::PCWSTR =
    windows::core::w!("Software\\Microsoft\\IME\\15.0\\IMEJP\\MSIME");

const KEY_ENABLED: windows::core::PCWSTR = windows::core::w!("IsKeyAssignmentEnabled");
const KEY_MUHENKAN: windows::core::PCWSTR = windows::core::w!("KeyAssignmentMuhenkan");
const KEY_HENKAN: windows::core::PCWSTR = windows::core::w!("KeyAssignmentHenkan");

/// 起動時に一度だけ確認ダイアログを出すためのフラグ
static STARTUP_CHECK_DONE: AtomicBool = AtomicBool::new(false);

fn read_dword(value_name: windows::core::PCWSTR) -> Option<u32> {
    let mut val: u32 = 0;
    let mut size = u32::try_from(size_of::<u32>()).unwrap();
    // SAFETY: HKEY_CURRENT_USER は擬似ハンドル。val は u32 でサイズ一致。
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            MSIME_SUBKEY,
            value_name,
            RRF_RT_REG_DWORD,
            None,
            Some(std::ptr::addr_of_mut!(val).cast()),
            Some(&mut size),
        )
    };
    ok.ok().map(|_| val)
}

fn write_dword(value_name: windows::core::PCWSTR, data: u32) -> bool {
    // SAFETY: HKEY_CURRENT_USER は擬似ハンドル。data は u32 でサイズ一致。
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            MSIME_SUBKEY,
            value_name,
            REG_DWORD.0,
            Some(std::ptr::addr_of!(data).cast()),
            u32::try_from(size_of::<u32>()).unwrap(),
        )
    }
    .is_ok()
}

/// 目的の設定が既に書き込まれているか確認する
#[must_use]
pub fn is_configured() -> bool {
    read_dword(KEY_ENABLED) == Some(1)
        && read_dword(KEY_MUHENKAN) == Some(1)
        && read_dword(KEY_HENKAN) == Some(0)
}

/// MSIME キー割り当てを書き込む。失敗した値の数を返す（0 = 全成功）。
pub fn configure() -> u32 {
    let mut failures = 0u32;
    if !write_dword(KEY_ENABLED, 1) {
        log::error!("[msime] failed to write IsKeyAssignmentEnabled");
        failures += 1;
    }
    if !write_dword(KEY_MUHENKAN, 1) {
        log::error!("[msime] failed to write KeyAssignmentMuhenkan");
        failures += 1;
    }
    if !write_dword(KEY_HENKAN, 0) {
        log::error!("[msime] failed to write KeyAssignmentHenkan");
        failures += 1;
    }
    if failures == 0 {
        log::info!("[msime] key assignment configured (無変換→IME-OFF, 変換→IME-ON)");
    }
    failures
}

/// 確認ダイアログを表示する。OK なら true。
#[must_use]
fn ask_user() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDCANCEL, MB_ICONINFORMATION, MB_OKCANCEL,
    };

    let caption = crate::win32::to_wide("Microsoft IME のセットアップ");
    let message = crate::win32::to_wide(concat!(
        "Microsoft IME のキー割り当てを NICOLA 向けに設定します。\r\n\r\n",
        "  無変換 → IME オフ（直接入力）\r\n",
        "  変換   → ひらがな（IME オン）\r\n\r\n",
        "設定を上書きしてよろしいですか？",
    ));

    let response = unsafe {
        MessageBoxW(
            None,
            windows::core::PCWSTR(message.as_ptr()),
            windows::core::PCWSTR(caption.as_ptr()),
            MB_OKCANCEL | MB_ICONINFORMATION,
        )
    };

    response != IDCANCEL
}

/// 起動時に MSIME が検出されたときに呼ぶ。一度だけダイアログを出して設定する。
pub fn on_startup_msime_detected() {
    if STARTUP_CHECK_DONE.swap(true, Ordering::SeqCst) {
        return;
    }

    if is_configured() {
        log::info!("[msime] key assignment already configured, skip");
        return;
    }

    if ask_user() {
        let failures = configure();
        if failures > 0 {
            let msg = crate::win32::to_wide(
                "Microsoft IME の設定書き込みに一部失敗しました。\r\nログを確認してください。",
            );
            let cap = crate::win32::to_wide("awase - エラー");
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{
                    MessageBoxW, MB_ICONWARNING, MB_OK,
                };
                MessageBoxW(
                    None,
                    windows::core::PCWSTR(msg.as_ptr()),
                    windows::core::PCWSTR(cap.as_ptr()),
                    MB_OK | MB_ICONWARNING,
                );
            }
        }
    }
}

const fn size_of<T>() -> usize {
    std::mem::size_of::<T>()
}
