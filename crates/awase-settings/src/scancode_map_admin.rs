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
        tracing::error!("[scancode-map] このプラットフォームでは未対応");
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
            tracing::error!("[scancode-map] 既存値の読み取りに失敗: {e}");
            return 1;
        }
    };
    let new_entries = sm::compute_new_entries(&existing, selection);
    let write_result = match sm::build_bytes(&new_entries) {
        Some(bytes) => sm::write(&bytes),
        None => sm::delete(),
    };
    if let Err(e) = write_result {
        tracing::error!("[scancode-map] 書き込みに失敗: {e}");
        return 1;
    }

    // 決定3: 書き込み後に必ず読み戻し検証する。
    match read_entries() {
        Ok(verified) if verified == new_entries => 0,
        Ok(_) => {
            tracing::error!("[scancode-map] 読み戻し検証で内容が一致しない");
            1
        }
        Err(e) => {
            tracing::error!("[scancode-map] 読み戻し検証に失敗: {e}");
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

/// 昇格側プロセスのエントリポイント（ペア集合の適用、ADR-230 決定5）。`--scancode-pairs <spec>` を検出したら
/// GUI を起動せずこの関数を呼び、[`awase_windows::scancode_apply::WorkerExit`] の終了コードで結果を返す。
///
/// 非昇格側が読んだ状態（比較交換の期待値）と、書く直前に読み直した値を比べ、食い違えば書かない。
/// 壊れた既存値は上書きしない。書いたあと読み戻して一致しなければ、書く前の生のバイト列（無ければ削除）へ戻す。
#[must_use]
pub fn run_elevated_pairs_worker(spec: &str) -> i32 {
    #[cfg(windows)]
    {
        run_elevated_pairs_worker_windows(spec)
    }
    #[cfg(not(windows))]
    {
        let _ = spec;
        tracing::error!("[scancode-pairs] このプラットフォームでは未対応");
        awase_windows::scancode_apply::WorkerExit::Failed.code()
    }
}

#[cfg(windows)]
fn run_elevated_pairs_worker_windows(spec: &str) -> i32 {
    use awase_windows::scancode_apply::{
        ApplyRequest, Decision, WorkerExit, decide, read_back_matches,
    };
    use awase_windows::scancode_map as sm;

    let request = match ApplyRequest::from_spec(spec) {
        Ok(request) => request,
        Err(e) => {
            tracing::error!("[scancode-pairs] 引数を解釈できない: {e:?}");
            return WorkerExit::BadArguments.code();
        }
    };
    let original = match sm::read() {
        Ok(raw) => raw,
        Err(e) => {
            tracing::error!("[scancode-pairs] 既存値の読み取りに失敗: {e}");
            return WorkerExit::Failed.code();
        }
    };
    let entries = match decide(original.as_deref(), &request) {
        Decision::Stop(exit) => {
            if exit != WorkerExit::Ok {
                tracing::error!("[scancode-pairs] 書き込まずに終了: {exit:?}");
            }
            return exit.code();
        }
        Decision::Write { entries } => entries,
    };
    let write_result = match sm::build_bytes(&entries) {
        Some(bytes) => sm::write(&bytes),
        None => sm::delete(),
    };
    if let Err(e) = write_result {
        tracing::error!("[scancode-pairs] 書き込みに失敗: {e}");
        return WorkerExit::Failed.code();
    }
    match sm::read() {
        Ok(raw) if read_back_matches(&entries, raw.as_deref()) => WorkerExit::Ok.code(),
        other => {
            tracing::error!("[scancode-pairs] 読み戻し検証が一致しない({other:?})。元の値へ戻す");
            rollback_windows(original.as_deref()).code()
        }
    }
}

/// 書く前の生の値へ戻し、戻せたかを読み直して確かめる。
#[cfg(windows)]
fn rollback_windows(original: Option<&[u8]>) -> awase_windows::scancode_apply::WorkerExit {
    use awase_windows::scancode_apply::WorkerExit;
    use awase_windows::scancode_map as sm;

    let restore_result = match original {
        Some(bytes) => sm::write(bytes),
        None => sm::delete(),
    };
    if let Err(e) = restore_result {
        tracing::error!("[scancode-pairs] 巻き戻しの書き込みに失敗: {e}");
        return WorkerExit::RollbackFailed;
    }
    match sm::read() {
        Ok(now) if now.as_deref() == original => WorkerExit::RolledBack,
        _ => WorkerExit::RollbackFailed,
    }
}

/// 自己昇格フローの結果（GUI 側の表示分岐用）。
#[derive(Debug, Clone)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum ElevationOutcome {
    /// 昇格・書き込み・読み戻し検証まで成功。
    Success,
    /// 昇格プロセスは起動したが処理に失敗した（終了コード非0。ペア適用では、下の `Rejected` に当たらない失敗）。
    Failed,
    /// ペア集合の適用を、昇格側が理由つきで書かずに止めた、または元へ戻した（ADR-230 決定5）。
    /// `RolledBack`/`RollbackFailed` は書き込みを試みたあとの結果で、画面は今のレジストリを読み直して表示すること。
    Rejected(awase_windows::scancode_apply::WorkerExit),
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
    match launch_elevated_windows(&format!("--scancode-map {}", selection.as_cli_arg())) {
        Ok(0) => ElevationOutcome::Success,
        Ok(_) => ElevationOutcome::Failed,
        Err(outcome) => outcome,
    }
}

/// ペア集合の適用を、自分自身を `--scancode-pairs <spec>` で `runas` 起動して依頼する（ADR-230 決定5）。
// UI（ADR-230 段階4）から呼ぶまで未使用。
#[allow(dead_code)]
#[must_use]
pub fn request_elevated_pairs_change(
    request: &awase_windows::scancode_apply::ApplyRequest,
) -> ElevationOutcome {
    #[cfg(windows)]
    {
        use awase_windows::scancode_apply::WorkerExit;
        match launch_elevated_windows(&format!("--scancode-pairs {}", request.to_spec())) {
            Ok(code) => match WorkerExit::from_code(i32::try_from(code).unwrap_or(i32::MAX)) {
                Some(WorkerExit::Ok) => ElevationOutcome::Success,
                Some(WorkerExit::Failed) | None => ElevationOutcome::Failed,
                Some(exit) => ElevationOutcome::Rejected(exit),
            },
            Err(outcome) => outcome,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = request;
        ElevationOutcome::LaunchError("このプラットフォームでは未対応".to_string())
    }
}

/// 自分自身を `params` で `runas` 起動し、終了を待って終了コードを返す。UAC のキャンセル・起動失敗は `Err`。
#[cfg(windows)]
fn launch_elevated_windows(params: &str) -> Result<u32, ElevationOutcome> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, GetLastError};
    use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
    use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::PCWSTR;

    let Ok(exe) = std::env::current_exe() else {
        return Err(ElevationOutcome::LaunchError(
            "current_exe の取得に失敗".to_string(),
        ));
    };
    let exe_wide = to_wide(&exe.to_string_lossy());
    let verb_wide = to_wide("runas");
    let params_wide = to_wide(params);

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
        return Err(if err == ERROR_CANCELLED {
            ElevationOutcome::Cancelled
        } else {
            ElevationOutcome::LaunchError(format!("ShellExecuteExW failed: {err:?}"))
        });
    }
    if sei.hProcess.is_invalid() {
        return Err(ElevationOutcome::LaunchError(
            "プロセスハンドルを取得できませんでした".to_string(),
        ));
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
    Ok(if got_exit_code.is_ok() { exit_code } else { 1 })
}
