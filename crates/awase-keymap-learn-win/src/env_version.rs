#![allow(unsafe_code)]
//! GJI Converter 本体のファイル版取得
//! ([ADR-196](../../../docs/adr/196-keymap-learn-truth-priority.md) 決定3b、
//! [ADR196-T5](../../../docs/tasks/adr196-t5-revalidation-not-invalidation.md))。
//!
//! [`file_version`] は `VS_FIXEDFILEINFO` の版を読む共有関数で、T5 が所有する
//! ([ADR196-T3](../../../docs/tasks/adr196-t3-bundled-table-versioning.md) は利用側)。
//! Toolhelp・`OpenProcess`・`GetFileVersionInfoW` はブロックしうるため、UI スレッドや
//! 学習プロセスのメインループからは [`probe_gji_env_version_with_timeout`] を使うこと。

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use awase_keymap_learn::revalidation::{classify_converter_version, EnvVersion, EnvVersionProbe};
use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

/// `awase-windows` の `tsf/gji_monitor.rs::GJI_PROCESS_PREFIXES` と同じ Converter 候補
/// (Renderer/CacheService は除外)。
const GJI_CONVERTER_PREFIXES: &[&str] = &["GoogleIMEJaConverter", "GoogleJapaneseInputConverter"];

/// `VS_FIXEDFILEINFO` の `dwFileVersionMS`/`dwFileVersionLS` を4値へ分解する。
#[must_use]
const fn split_file_version(ms: u32, ls: u32) -> EnvVersion {
    EnvVersion([ms >> 16, ms & 0xFFFF, ls >> 16, ls & 0xFFFF])
}

/// 実行ファイルの `VS_FIXEDFILEINFO` ファイル版を返す。版リソースが無い/読めない場合は `None`。
#[must_use]
pub fn file_version(path: &Path) -> Option<EnvVersion> {
    let wide: Vec<u16> = path
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let name = PCWSTR(wide.as_ptr());
    // SAFETY: `name` は NUL 終端の有効な UTF-16 文字列(`wide` が関数末尾まで生存)。
    let size = unsafe { GetFileVersionInfoSizeW(name, None) };
    if size == 0 {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    // SAFETY: `buf` は `size` バイトの有効な書き込み先。
    unsafe { GetFileVersionInfoW(name, None, size, buf.as_mut_ptr().cast()) }.ok()?;
    let mut info: *mut core::ffi::c_void = std::ptr::null_mut();
    let mut len = 0u32;
    // SAFETY: `buf` は直上で埋めた版情報ブロック。`info` は `buf` 内を指す(`buf` は下の
    //         読み出しまで生存)。ルートブロック "\\" の照会は VS_FIXEDFILEINFO を返す。
    let ok = unsafe { VerQueryValueW(buf.as_ptr().cast(), w!("\\"), &raw mut info, &raw mut len) };
    if !ok.as_bool() || info.is_null() || (len as usize) < size_of::<VS_FIXEDFILEINFO>() {
        return None;
    }
    // SAFETY: `info` は `buf` 内の有効な VS_FIXEDFILEINFO を指し、長さも上で確認済み。
    //         アラインは保証されないため `read_unaligned` で読む。
    let fixed = unsafe { info.cast::<VS_FIXEDFILEINFO>().read_unaligned() };
    Some(split_file_version(
        fixed.dwFileVersionMS,
        fixed.dwFileVersionLS,
    ))
}

fn session_of(pid: u32) -> Option<u32> {
    let mut session = 0u32;
    // SAFETY: `session` は有効な書き込み先。
    unsafe { ProcessIdToSessionId(pid, &raw mut session) }.ok()?;
    Some(session)
}

fn process_image_path(pid: u32) -> Option<PathBuf> {
    // SAFETY: 最小権限で開く。失敗時は Err。
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: `handle` は有効、`buf` は `len` 要素の書き込み先。
    let ok = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &raw mut len,
        )
    };
    // SAFETY: `handle` は OpenProcess が返した有効なハンドルで、ここで1回だけ閉じる。
    let _ = unsafe { CloseHandle(handle) };
    if ok.is_err() || len == 0 {
        return None;
    }
    Some(PathBuf::from(String::from_utf16_lossy(
        &buf[..len as usize],
    )))
}

/// 自セッションの GJI Converter 実行ファイルのフルパスを探す。無ければ `None`。
#[must_use]
pub fn find_gji_converter_path() -> Option<PathBuf> {
    // SAFETY: 全プロセス対象のスナップショット。返るハンドルは下で必ず閉じる。
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;
    let own_session = session_of(unsafe { GetCurrentProcessId() });
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut found = None;
    // SAFETY: `snapshot` は有効、`entry` は `dwSize` 設定済み。
    let mut more = unsafe { Process32FirstW(snapshot, &raw mut entry) }.is_ok();
    while more {
        let end = entry
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..end]).to_ascii_lowercase();
        let is_converter = GJI_CONVERTER_PREFIXES
            .iter()
            .any(|p| name.starts_with(&p.to_ascii_lowercase()));
        let same_session = own_session.is_none_or(|s| session_of(entry.th32ProcessID) == Some(s));
        if is_converter && same_session {
            if let Some(path) = process_image_path(entry.th32ProcessID) {
                found = Some(path);
                break;
            }
        }
        // SAFETY: `snapshot`/`entry` は上と同じく有効。
        more = unsafe { Process32NextW(snapshot, &raw mut entry) }.is_ok();
    }
    // SAFETY: `snapshot` は有効なハンドルで、ここで1回だけ閉じる。
    let _ = unsafe { CloseHandle(snapshot) };
    found
}

/// 現在の GJI Converter の版を [`EnvVersionProbe`] として返す(ブロックしうる)。
/// `process_start` は呼び出し側プロセス(学習プロセス/awase-settings)の起動時刻。
#[must_use]
pub fn probe_gji_env_version(process_start: SystemTime) -> EnvVersionProbe {
    let Some(path) = find_gji_converter_path() else {
        return EnvVersionProbe::Unknown;
    };
    let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    classify_converter_version(file_version(&path), modified, process_start)
}

/// [`probe_gji_env_version`] を別スレッドで走らせ、`timeout` 内に返らなければ
/// [`EnvVersionProbe::Unknown`](fail open)を返す。応答しないスレッドは切り離すだけで
/// 回収しないため、短周期で繰り返し呼ぶ用途には向かない(現状は学習プロセスが1回だけ呼ぶ)。
#[must_use]
pub fn probe_gji_env_version_with_timeout(
    process_start: SystemTime,
    timeout: Duration,
) -> EnvVersionProbe {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(probe_gji_env_version(process_start));
    });
    rx.recv_timeout(timeout).unwrap_or(EnvVersionProbe::Unknown)
}
