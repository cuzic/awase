//! Windows 自動起動管理（Task Scheduler 経由）

use std::os::windows::process::CommandExt;
use std::process::Command;

const TASK_NAME: &str = "awase";

/// コンソールウィンドウを生成しない CreateProcess フラグ。
/// schtasks.exe はコンソールアプリのため、GUI アプリから起動すると
/// 一瞬コマンドプロンプト窓が出る。このフラグで非表示にする。
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Task Scheduler にログオン時自動起動タスクを登録する
pub fn register() -> bool {
    let exe = std::env::current_exe().ok();
    let Some(exe_path) = exe.as_ref().and_then(|p| p.to_str()) else {
        log::error!("Failed to get current executable path");
        return false;
    };

    let output = Command::new("schtasks")
        .args([
            "/create", "/tn", TASK_NAME, "/tr", exe_path, "/sc", "onlogon", "/rl", "limited", "/f",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            log::info!("Auto-start task registered: {TASK_NAME}");
            true
        }
        Ok(o) => {
            log::error!("schtasks failed: {}", String::from_utf8_lossy(&o.stderr));
            false
        }
        Err(e) => {
            log::error!("Failed to run schtasks: {e}");
            false
        }
    }
}

/// Task Scheduler から自動起動タスクを削除する
pub fn unregister() -> bool {
    let output = Command::new("schtasks")
        .args(["/delete", "/tn", TASK_NAME, "/f"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            log::info!("Auto-start task unregistered: {TASK_NAME}");
            true
        }
        Ok(_) | Err(_) => {
            log::warn!("Failed to unregister auto-start task (may not exist)");
            false
        }
    }
}

/// タスクが登録済みかどうかを確認する
pub fn is_registered() -> bool {
    Command::new("schtasks")
        .args(["/query", "/tn", TASK_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// ユーザーにダイアログで自動起動を確認する
/// Returns: true = Yes, false = No
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
