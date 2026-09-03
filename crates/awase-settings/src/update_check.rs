/// 更新確認を実行する。
///
/// # Panics
///
/// panic させない設計。失敗はログに落として静かに return する。
pub fn run() {
    #[cfg(target_os = "windows")]
    {
        run_windows();
    }
    #[cfg(not(target_os = "windows"))]
    {
        log::warn!("[update-check] Windows以外では更新確認は利用できません");
    }
}

#[cfg(target_os = "windows")]
fn run_windows() {
    let Some(_guard) = UpdateCheckMutex::acquire() else {
        return;
    };

    // ★M-1対応: last_attempted_at は「この呼び出しが実際に何かした」ことの記録ではなく
    // 「呼び出し自体が起きた」ことの記録なので、config読み取り失敗・無効設定を含む
    // 全ての早期returnより前に書く。ここより後で return するパス（config読み取り
    // 失敗・update_check=false）でも記帳を飛ばすと、should_attempt()が永久にtrueの
    // ままになり、右クリックのたびにこのプロセスが無制限にspawnされ続ける。
    let path = awase::update_state::default_path();
    let mut state = awase::update_state::load(&path);
    let attempted_at = awase::update_state::now_unix();
    state.last_attempted_at = Some(attempted_at);
    if let Err(e) = awase::update_state::save(&path, &state) {
        log::warn!(
            "[update-check] 試行時刻を保存できませんでした: {} ({e})",
            path.display()
        );
    }

    let config_path = crate::find_config_path();
    let config = match awase::config::AppConfig::load(&config_path) {
        Ok(config) => config,
        Err(e) => {
            log::warn!(
                "[update-check] config.tomlを読めないため更新確認をスキップします: {} ({e})",
                config_path.display()
            );
            return;
        }
    };
    if !config.general.update_check {
        log::info!("[update-check] 更新確認は無効です");
        return;
    }

    match fetch_latest_version() {
        Ok(latest_version) => {
            let Some(parsed) = awase::version::parse(&latest_version) else {
                log::warn!(
                    "[update-check] latest_versionをSemVerとして解釈できません: {latest_version}"
                );
                return;
            };
            let now = awase::update_state::now_unix();
            state.last_success_at = Some(now);
            state.last_seen_latest = Some(parsed.to_string());
            if let Err(e) = awase::update_state::save(&path, &state) {
                log::warn!(
                    "[update-check] 成功状態を保存できませんでした: {} ({e})",
                    path.display()
                );
                return;
            }
            log::info!("[update-check] 最新バージョンを確認しました: {parsed}");
        }
        Err(e) => {
            log::warn!("[update-check] 最新バージョンを確認できませんでした: {e}");
        }
    }
}

#[cfg(target_os = "windows")]
struct UpdateCheckMutex(windows::Win32::Foundation::HANDLE);

#[cfg(target_os = "windows")]
impl UpdateCheckMutex {
    fn acquire() -> Option<Self> {
        use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
        use windows::Win32::System::Threading::CreateMutexW;
        use windows::core::w;

        // SAFETY: CreateMutexW receives no security attributes, does not take ownership of the
        // name pointer, and the w! literal is a valid NUL-terminated wide string.
        let handle = unsafe { CreateMutexW(None, false, w!("Global\\awase_update_check")) };
        match handle {
            Ok(handle) => {
                // SAFETY: GetLastError is read immediately after CreateMutexW on this thread.
                if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                    log::info!("[update-check] 既に実行中のためスキップします");
                    // SAFETY: handle was returned by CreateMutexW and is closed exactly once here.
                    unsafe {
                        let _ = windows::Win32::Foundation::CloseHandle(handle);
                    }
                    None
                } else {
                    Some(Self(handle))
                }
            }
            Err(e) => {
                log::warn!("[update-check] Mutexを取得できませんでした: {e}");
                None
            }
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for UpdateCheckMutex {
    fn drop(&mut self) {
        // SAFETY: self.0 is a valid mutex handle returned by CreateMutexW and owned by this guard.
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(target_os = "windows")]
fn fetch_latest_version() -> Result<String, String> {
    let body = winhttp_get_latest_release()?;
    let value: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("JSONを解釈できません: {e}"))?;
    if value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
    {
        return Err("schema_versionが1ではありません".to_owned());
    }
    value
        .get("latest_version")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| "latest_versionがありません".to_owned())
}

#[cfg(target_os = "windows")]
fn winhttp_get_latest_release() -> Result<String, String> {
    use windows::Win32::Networking::WinHttp::{
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
        WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen,
        WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReceiveResponse, WinHttpSendRequest,
        WinHttpSetTimeouts,
    };
    use windows::core::{PCWSTR, w};

    struct Handle(*mut core::ffi::c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
    impl Handle {
        fn new(handle: *mut core::ffi::c_void, label: &str) -> Result<Self, String> {
            if handle.is_null() {
                Err(format!("{label} に失敗しました"))
            } else {
                Ok(Self(handle))
            }
        }
    }

    unsafe {
        let session = Handle::new(
            WinHttpOpen(
                w!("awase-update-check/1"),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            ),
            "WinHttpOpen",
        )?;
        WinHttpSetTimeouts(session.0, 5_000, 5_000, 10_000, 10_000)
            .map_err(|e| format!("WinHttpSetTimeouts: {e}"))?;
        let connect = Handle::new(
            WinHttpConnect(session.0, w!("report.awase.cc"), 443, 0),
            "WinHttpConnect",
        )?;
        let request = Handle::new(
            WinHttpOpenRequest(
                connect.0,
                w!("GET"),
                w!("/v1/latest-release"),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            ),
            "WinHttpOpenRequest",
        )?;
        WinHttpSendRequest(request.0, None, None, 0, 0, 0)
            .map_err(|e| format!("WinHttpSendRequest: {e}"))?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())
            .map_err(|e| format!("WinHttpReceiveResponse: {e}"))?;

        let mut status_code = 0_u32;
        let mut status_len = u32::try_from(std::mem::size_of::<u32>()).unwrap_or(4);
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&raw mut status_code).cast()),
            &raw mut status_len,
            std::ptr::null_mut(),
        )
        .map_err(|e| format!("WinHttpQueryHeaders: {e}"))?;

        let response = read_response_body_limited(request.0, 64 * 1024)?;
        if status_code != 200 {
            return Err(format!("HTTP {status_code}: {response}"));
        }
        Ok(response)
    }
}

#[cfg(target_os = "windows")]
unsafe fn read_response_body_limited(
    request: *mut core::ffi::c_void,
    max_bytes: usize,
) -> Result<String, String> {
    use windows::Win32::Networking::WinHttp::{WinHttpQueryDataAvailable, WinHttpReadData};

    let mut out = Vec::new();
    loop {
        let mut available = 0_u32;
        unsafe {
            WinHttpQueryDataAvailable(request, &raw mut available)
                .map_err(|e| format!("WinHttpQueryDataAvailable: {e}"))?;
        }
        if available == 0 {
            break;
        }
        let available_usize =
            usize::try_from(available).map_err(|_| "レスポンスが大きすぎます".to_owned())?;
        let Some(remaining) = max_bytes.checked_sub(out.len()) else {
            return Err("レスポンスが大きすぎます".to_owned());
        };
        if available_usize > remaining {
            return Err("レスポンスが大きすぎます".to_owned());
        }
        let mut buf = vec![0_u8; available_usize];
        let mut read = 0_u32;
        unsafe {
            WinHttpReadData(request, buf.as_mut_ptr().cast(), available, &raw mut read)
                .map_err(|e| format!("WinHttpReadData: {e}"))?;
        }
        buf.truncate(usize::try_from(read).unwrap_or(0));
        out.extend_from_slice(&buf);
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}
