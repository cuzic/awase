//! Windows 自動起動管理（HKCU Run レジストリキー経由）
//!
//! `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run` に
//! 値を書き込む方式。schtasks より軽量で GPO 制限の影響を受けない。
//! 起動遅延は不要（シェル未起動時はトレイ登録に失敗しても TaskbarCreated で復元）。

use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ, REG_SZ,
};

const RUN_SUBKEY: windows::core::PCWSTR =
    windows::core::w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE_NAME: windows::core::PCWSTR = windows::core::w!("awase");

/// HKCU Run キーに自動起動エントリを登録する
#[must_use]
pub fn register() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        log::error!("Failed to get current executable path");
        return false;
    };
    let Some(exe_str) = exe.to_str() else {
        log::error!("Executable path contains non-UTF-8 characters");
        return false;
    };

    // REG_SZ は NUL 終端済み UTF-16 が必要
    let exe_wide: Vec<u16> = exe_str.encode_utf16().chain(std::iter::once(0)).collect();
    let byte_len = u32::try_from(exe_wide.len() * 2).unwrap_or(u32::MAX);

    // SAFETY: exe_wide は NUL 終端済み UTF-16 文字列。ポインタは呼び出し中有効。
    //         HKEY_CURRENT_USER は擬似ハンドルで CloseHandle 不要。
    let result = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN_SUBKEY,
            VALUE_NAME,
            REG_SZ.0,
            Some(exe_wide.as_ptr().cast()),
            byte_len,
        )
    };

    if result.is_ok() {
        log::info!("Auto-start registered: {exe_str}");
        true
    } else {
        log::error!("Failed to register auto-start: {result:?}");
        false
    }
}

/// HKCU Run キーから自動起動エントリを削除する
#[must_use]
pub fn unregister() -> bool {
    // SAFETY: HKEY_CURRENT_USER は擬似ハンドル。サブキー・値名は NUL 終端済み UTF-16。
    let result = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_SUBKEY, VALUE_NAME) };

    if result.is_ok() {
        log::info!("Auto-start unregistered");
        true
    } else {
        log::warn!("Failed to unregister auto-start (may not exist): {result:?}");
        false
    }
}

/// HKCU Run キーに自動起動エントリが存在するか確認する
#[must_use]
pub fn is_registered() -> bool {
    // SAFETY: data/size を None にして存在確認のみ行う。
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_SUBKEY,
            VALUE_NAME,
            RRF_RT_REG_SZ,
            None,
            None,
            None,
        )
        .is_ok()
    }
}

/// ユーザーにダイアログで自動起動を確認する
/// Returns: true = Yes, false = No
#[must_use]
pub fn ask_user() -> bool {
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONQUESTION, MB_YESNO};

    let result = unsafe {
        MessageBoxW(
            None,
            w!("awase をログオン時に自動起動しますか？\n\n後から config.toml の auto_start で変更できます。"),
            w!("awase - 自動起動設定"),
            MB_YESNO | MB_ICONQUESTION,
        )
    };

    result == IDYES
}
