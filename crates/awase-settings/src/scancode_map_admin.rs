//! Caps(英数)⇔Ctrl 入れ替え / Caps(英数)→Ctrl 片方向複製プリセット
//! （ADR-111 / ADR-126）の Scancode Map 適用フロー。
//!
//! `awase-settings.exe` は既定では非昇格で起動する（BUG-79対策、
//! `asInvoker`）。この機能の有効化/無効化ボタンが押されたときだけ、
//! 自分自身を `--scancode-map swap` 等で `runas` 起動し、
//! 昇格側プロセス（[`run_elevated_worker`]）がレジストリの読み取り・
//! マージ・書き込み・読み戻し検証（`awase_windows::scancode_map` 参照、
//! 決定3）を行う。非昇格側（[`request_elevated_change`]）は
//! `ShellExecuteExW`+`SEE_MASK_NOCLOSEPROCESS` で起動したプロセスの終了を
//! 待って終了コードを見る——`ShellExecuteW`（`tray.rs::restart_as_admin`が
//! 使う方）はプロセスハンドルを返さず成否を確実に取得できないため、
//! こちらは意図的に別の API を使う（ADR-111決定4）。

/// 昇格側プロセスのエントリポイント。`--scancode-map <selection>` を
/// 検出したら GUI を起動せずこの関数を呼び、終了コードで結果を返す
/// （`main()` の `--bug-report` と同型のヘッドレス分岐パターン）。
///
/// 戻り値: 成功時 `0`、失敗時 `1`（`awase-settings.exe` のプロセス終了
/// コードとして使う。`ShellExecuteExW` 側が `GetExitCodeProcess` で読む）。
#[must_use]
pub fn run_elevated_worker(selection: awase_windows::scancode_map::ScancodeMapSelection) -> i32 {
    #[cfg(windows)]
    {
        run_elevated_worker_windows(selection)
    }
    #[cfg(not(windows))]
    {
        let _ = selection;
        log::error!("[scancode-map] このプラットフォームでは未対応");
        1
    }
}

#[cfg(windows)]
fn run_elevated_worker_windows(
    selection: awase_windows::scancode_map::ScancodeMapSelection,
) -> i32 {
    use awase_windows::scancode_map as sm;

    let existing = match read_entries() {
        Ok(entries) => entries,
        Err(e) => {
            log::error!("[scancode-map] 既存値の読み取りに失敗: {e}");
            return 1;
        }
    };
    let new_entries = sm::compute_new_entries(&existing, selection);
    let write_result = match sm::build_bytes(&new_entries) {
        Some(bytes) => sm::write(&bytes),
        None => sm::delete(),
    };
    if let Err(e) = write_result {
        log::error!("[scancode-map] 書き込みに失敗: {e}");
        return 1;
    }

    // 決定3: 書き込み後に必ず読み戻し検証する。
    match read_entries() {
        Ok(verified) if verified == new_entries => 0,
        Ok(_) => {
            log::error!("[scancode-map] 読み戻し検証で内容が一致しない");
            1
        }
        Err(e) => {
            log::error!("[scancode-map] 読み戻し検証に失敗: {e}");
            1
        }
    }
}

#[cfg(windows)]
fn read_entries() -> Result<Vec<(u16, u16)>, String> {
    use awase_windows::scancode_map as sm;
    match sm::read()? {
        Some(bytes) => Ok(sm::parse_entries(&bytes)),
        None => Ok(Vec::new()),
    }
}

/// 現在の Scancode Map の状態（GUI 表示用）。昇格不要（読み取りのみ）。
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum ScancodeMapStatus {
    /// プリセットが有効（他の無関係なエントリが `extra_entries` 件ある）。
    Active {
        preset: awase_windows::scancode_map::ScancodeMapPreset,
        extra_entries: usize,
    },
    /// プリセットは無効（値自体が未設定、または他のエントリのみ存在）。
    Inactive { extra_entries: usize },
    /// レジストリ読み取りに失敗。
    ReadError(String),
}

/// 現在の Scancode Map の状態を読み取る（昇格不要）。
#[must_use]
pub fn read_status() -> ScancodeMapStatus {
    #[cfg(windows)]
    {
        use awase_windows::scancode_map as sm;
        match read_entries() {
            Ok(entries) => {
                let (preset, extra_entries) = sm::detect_status(&entries);
                if let Some(preset) = preset {
                    ScancodeMapStatus::Active {
                        preset,
                        extra_entries,
                    }
                } else {
                    ScancodeMapStatus::Inactive { extra_entries }
                }
            }
            Err(e) => ScancodeMapStatus::ReadError(e),
        }
    }
    #[cfg(not(windows))]
    {
        ScancodeMapStatus::ReadError("このプラットフォームでは未対応".to_string())
    }
}

/// 自己昇格フローの結果（GUI 側の表示分岐用）。
#[derive(Debug, Clone)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum ElevationOutcome {
    /// 昇格・書き込み・読み戻し検証まで成功。
    Success,
    /// 昇格プロセスは起動したが処理に失敗した（終了コード非0）。
    Failed,
    /// ユーザーが UAC プロンプトをキャンセルした。
    Cancelled,
    /// 昇格プロセス自体の起動に失敗した（キャンセル以外）。
    LaunchError(String),
}

/// 自分自身を `--scancode-map <selection>` で `runas` 起動し、完了を待って
/// 結果を返す（ADR-111決定4）。
#[must_use]
pub fn request_elevated_change(
    selection: awase_windows::scancode_map::ScancodeMapSelection,
) -> ElevationOutcome {
    #[cfg(windows)]
    {
        request_elevated_change_windows(selection)
    }
    #[cfg(not(windows))]
    {
        let _ = selection;
        ElevationOutcome::LaunchError("このプラットフォームでは未対応".to_string())
    }
}

#[cfg(windows)]
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn request_elevated_change_windows(
    selection: awase_windows::scancode_map::ScancodeMapSelection,
) -> ElevationOutcome {
    use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, GetLastError};
    use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
    use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::PCWSTR;

    let Ok(exe) = std::env::current_exe() else {
        return ElevationOutcome::LaunchError("current_exe の取得に失敗".to_string());
    };
    let exe_wide = to_wide(&exe.to_string_lossy());
    let verb_wide = to_wide("runas");
    let params = format!("--scancode-map {}", selection.as_cli_arg());
    let params_wide = to_wide(&params);

    let mut sei = SHELLEXECUTEINFOW {
        cbSize: u32::try_from(std::mem::size_of::<SHELLEXECUTEINFOW>()).unwrap_or_default(),
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(verb_wide.as_ptr()),
        lpFile: PCWSTR(exe_wide.as_ptr()),
        lpParameters: PCWSTR(params_wide.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };

    // SAFETY: `exe_wide`/`verb_wide`/`params_wide` は NUL 終端済み UTF-16
    // で、この呼び出しが完了するまでスコープ内で生存している。
    let launch_result = unsafe { ShellExecuteExW(&raw mut sei) };
    if launch_result.is_err() {
        // SAFETY: 直前の失敗したAPI呼び出し直後のエラーコード取得。
        let err = unsafe { GetLastError() };
        return if err == ERROR_CANCELLED {
            ElevationOutcome::Cancelled
        } else {
            ElevationOutcome::LaunchError(format!("ShellExecuteExW failed: {err:?}"))
        };
    }
    if sei.hProcess.is_invalid() {
        return ElevationOutcome::LaunchError("プロセスハンドルを取得できませんでした".to_string());
    }

    // SAFETY: `sei.hProcess` は直前に取得した有効なハンドル。
    unsafe {
        WaitForSingleObject(sei.hProcess, INFINITE);
    }
    let mut exit_code: u32 = 1;
    // SAFETY: `sei.hProcess` は有効、`exit_code` は書き込み先として有効。
    let got_exit_code = unsafe { GetExitCodeProcess(sei.hProcess, &raw mut exit_code) };
    // SAFETY: `sei.hProcess` はこの後使わない。
    unsafe {
        let _ = CloseHandle(sei.hProcess);
    }

    if got_exit_code.is_ok() && exit_code == 0 {
        ElevationOutcome::Success
    } else {
        ElevationOutcome::Failed
    }
}
