#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! Windows 自動起動管理（HKCU Run レジストリキー経由）
//!
//! `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run` に
//! 値を書き込む方式。schtasks より軽量で GPO 制限の影響を受けない。
//! 起動遅延は不要（シェル未起動時はトレイ登録に失敗しても TaskbarCreated で復元）。
//!
//! 旧バージョンとの互換: `migrate_from_schtasks()` が起動時に一度だけ呼ばれ、
//! 旧 Task Scheduler タスクが残っていれば自動削除する。

use std::os::windows::process::CommandExt;
use std::process::Command;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_DWORD, REG_SZ,
    RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};

const RUN_SUBKEY: windows::core::PCWSTR =
    windows::core::w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE_NAME: windows::core::PCWSTR = windows::core::w!("awase");

// `wix/main.wxs` が既に使っている awase 自身の設定キー（`Software\awase`）。
// Run キーとは無関係の単なる完了フラグなので、自動起動の書き込み系API
// （register_path/unregister）とは別の関数に隔離してある。
const APP_SUBKEY: windows::core::PCWSTR = windows::core::w!("Software\\awase");
const SCHTASKS_MIGRATED_VALUE_NAME: windows::core::PCWSTR = windows::core::w!("SchtasksMigrated");

/// HKCU Run キーに自動起動エントリを登録する（自プロセス自身のパスを使う）
///
/// `awase.exe` 自身から呼ぶ用途（トレイメニュー操作）向け。別プロセスから
/// 任意のパスで登録したい場合（`awase-settings.exe` から `awase.exe` を
/// 登録する等）は [`register_path`] を使う。
#[must_use]
pub fn register() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        tracing::error!("Failed to get current executable path");
        return false;
    };
    register_path(&exe)
}

/// HKCU Run キーに指定した実行ファイルパスで自動起動エントリを登録する。
///
/// `awase-settings.exe`（設定 GUI、別プロセス）から `awase.exe`（本体）を
/// 登録する用途向け。`current_exe()` は呼び出し元プロセス自身のパスしか
/// 返せないため、自プロセス以外を登録する場合は明示的にパスを渡す必要が
/// ある。
#[must_use]
pub fn register_path(exe: &std::path::Path) -> bool {
    let Some(exe_str) = exe.to_str() else {
        tracing::error!("Executable path contains non-UTF-8 characters");
        return false;
    };

    // パスにスペースが含まれる場合（例: `C:\Users\Taro Yamada\...`）に
    // CreateProcess の解釈が曖昧にならないよう、常にダブルクォートで囲む
    // （Opus敵対的レビュー指摘、2026-09-07）。
    let quoted = format!("\"{exe_str}\"");

    // REG_SZ は NUL 終端済み UTF-16 が必要
    let exe_wide: Vec<u16> = quoted.encode_utf16().chain(std::iter::once(0)).collect();
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

    if result == ERROR_SUCCESS {
        tracing::info!("Auto-start registered: {exe_str}");
        true
    } else {
        tracing::error!("Failed to register auto-start: {result:?}");
        false
    }
}

/// HKCU Run キーから自動起動エントリを削除する
///
/// 冪等: 値が元々存在しない（`ERROR_FILE_NOT_FOUND`）場合も成功として扱う。
/// 呼び出し元（設定画面のチェックボックス等）は「既に未登録の状態でオフに
/// する」を正常系として扱えるようにする必要があるため（Opus敵対的レビュー
/// 指摘、2026-09-07: 以前は not-found を失敗扱いしており、ズレた状態から
/// チェックボックス操作だけでは復旧できなかった）。
#[must_use]
pub fn unregister() -> bool {
    // SAFETY: HKEY_CURRENT_USER は擬似ハンドル。サブキー・値名は NUL 終端済み UTF-16。
    let result = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_SUBKEY, VALUE_NAME) };

    if result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND {
        tracing::info!("Auto-start unregistered");
        true
    } else {
        tracing::warn!("Failed to unregister auto-start: {result:?}");
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

/// 旧バージョンの Task Scheduler タスクが残っていれば削除する。
///
/// v1.4.x 以前は schtasks でタスク登録していた。以前は起動のたびに無条件で
/// `schtasks.exe /delete` を隠しウィンドウで spawn していたが、これ自体が
/// 「ログオン時起動 → 無操作で schtasks.exe を実行（persistence 系 LOLBin の
/// 代表格）→ グローバルキーフック設置」という、`RegSetKeyValueW` 単発より
/// 広く persistence ヒューリスティックに使われるパターンになっていた
/// （Opus敵対的レビュー指摘、2026-09-07。v1.4.x からは十分時間が経っており、
/// 移行完了後のユーザーにとっては無意味な繰り返しでもあった）。
/// `Software\awase\SchtasksMigrated` に完了マーカーを立て、以後の起動では
/// spawn 自体を行わない。
pub fn migrate_from_schtasks() {
    const TASK_NAME: &str = "awase";
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    if schtasks_migration_marker_is_set() {
        return;
    }

    let output = Command::new("schtasks")
        .args(["/delete", "/tn", TASK_NAME, "/f"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    match &output {
        Ok(o) if o.status.success() => {
            tracing::info!("Migration: removed legacy schtasks task '{TASK_NAME}'");
        }
        Ok(_) => {
            // タスクが存在しない場合は正常（ほとんどの実行はここを通る）
        }
        Err(e) => {
            tracing::warn!("Migration: failed to invoke schtasks: {e}");
        }
    }
    set_schtasks_migration_marker();
}

fn schtasks_migration_marker_is_set() -> bool {
    // SAFETY: data/size を None にして存在確認のみ行う。
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            APP_SUBKEY,
            SCHTASKS_MIGRATED_VALUE_NAME,
            RRF_RT_REG_DWORD,
            None,
            None,
            None,
        )
        .is_ok()
    }
}

fn set_schtasks_migration_marker() {
    let value: u32 = 1;
    // SAFETY: value はスタック上のローカル変数で呼び出し中有効。
    //         HKEY_CURRENT_USER は擬似ハンドルで CloseHandle 不要。
    let result = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            APP_SUBKEY,
            SCHTASKS_MIGRATED_VALUE_NAME,
            REG_DWORD.0,
            Some((&raw const value).cast()),
            u32::try_from(size_of::<u32>()).unwrap_or(4),
        )
    };
    if result != ERROR_SUCCESS {
        tracing::warn!("Failed to set schtasks migration marker: {result:?}");
    }
}
