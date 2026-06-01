//! GJI (Google 日本語入力) config1.db パッチおよびプロセス管理。
#![allow(unsafe_code)]

use std::{fs, path::PathBuf};

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

// ── キーバインドパッチ ──

const MARKER: &[u8] = b"status\tkey\tcommand\n";

pub const ENTRIES: &[&str] = &[
    "Precomposition\tF13\tIMEOn\n",
    "Precomposition\tF14\tIMEOff\n",
    "Composition\tF13\tIMEOn\n",
    "Composition\tF14\tIMEOff\n",
    "Conversion\tF13\tIMEOn\n",
    "Conversion\tF14\tIMEOff\n",
];

pub fn find_block(data: &[u8]) -> Result<(usize, usize, usize), String> {
    let block_start = data
        .windows(MARKER.len())
        .position(|w| w == MARKER)
        .ok_or("key binding marker not found in config1.db")?;

    if block_start < 2 {
        return Err("unexpected file layout: marker too close to start".to_string());
    }
    let varint_offset = block_start - 2;

    let b0 = data[varint_offset];
    let b1 = data[varint_offset + 1];
    if b0 & 0x80 == 0 {
        return Err(format!(
            "unexpected varint format at offset {varint_offset:#x}: \
             expected 2-byte varint (MSB set), got {b0:#04x}"
        ));
    }
    let length = ((b0 & 0x7F) as usize) | (((b1 & 0x7F) as usize) << 7);

    Ok((varint_offset, block_start, length))
}

pub fn encode_varint2(value: usize) -> Result<[u8; 2], String> {
    if value >= 16384 {
        return Err(format!(
            "new block length {value} exceeds 2-byte protobuf varint capacity (16383)"
        ));
    }
    Ok([
        u8::try_from(value & 0x7F).unwrap_or(0x7F) | 0x80,
        u8::try_from((value >> 7) & 0x7F).unwrap_or(0x7F),
    ])
}

/// `Ok(None)` = パッチ不要、`Ok(Some((bytes, names)))` = パッチ済み。
pub type PatchResult = Result<Option<(Vec<u8>, Vec<&'static str>)>, String>;

pub fn patch(data: &[u8]) -> PatchResult {
    let (varint_offset, block_start, block_length) = find_block(data)?;

    let block_end = block_start
        .checked_add(block_length)
        .filter(|&e| e <= data.len())
        .ok_or_else(|| {
            format!(
                "block length {block_length} overflows file size {}",
                data.len()
            )
        })?;

    let block = std::str::from_utf8(&data[block_start..block_end])
        .map_err(|e| format!("key binding block is not valid UTF-8: {e}"))?;

    let missing: Vec<&'static str> = ENTRIES
        .iter()
        .copied()
        .filter(|entry| !block.contains(entry))
        .collect();

    if missing.is_empty() {
        return Ok(None);
    }

    let new_bytes: Vec<u8> = missing.iter().flat_map(|s| s.as_bytes()).copied().collect();
    let new_length = block_length + new_bytes.len();
    let new_varint = encode_varint2(new_length)?;

    let mut result = Vec::with_capacity(data.len() + new_bytes.len());
    result.extend_from_slice(&data[..block_end]);
    result.extend_from_slice(&new_bytes);
    result.extend_from_slice(&data[block_end..]);
    result[varint_offset] = new_varint[0];
    result[varint_offset + 1] = new_varint[1];

    Ok(Some((result, missing)))
}

/// `%LOCALAPPDATA%\..\LocalLow\Google\Google Japanese Input\config1.db`
pub fn default_config_path() -> Option<PathBuf> {
    let local = std::env::var("LOCALAPPDATA").ok()?;
    Some(
        PathBuf::from(local)
            .parent()?
            .join("LocalLow")
            .join("Google")
            .join("Google Japanese Input")
            .join("config1.db"),
    )
}

// ── プロセス管理 ──

const GJI_EXE: &str = "GoogleIMEJaConverter.exe";

/// GJI コンバータプロセスの PID を返す。見つからなければ `None`。
fn find_gji_pid() -> Option<u32> {
    // SAFETY: `CreateToolhelp32Snapshot` と `Process32FirstW`/`Process32NextW` は
    //         スナップショットハンドルと有効な `PROCESSENTRY32W` ポインタを渡す標準的な呼び出し。
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut entry = PROCESSENTRY32W {
            dwSize: u32::try_from(size_of::<PROCESSENTRY32W>()).unwrap_or(0),
            ..Default::default()
        };
        if Process32FirstW(snap, &raw mut entry).is_err() {
            let _ = CloseHandle(snap);
            return None;
        }
        loop {
            let name_end = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..name_end]);
            if name.eq_ignore_ascii_case(GJI_EXE) {
                let pid = entry.th32ProcessID;
                let _ = CloseHandle(snap);
                return Some(pid);
            }
            if Process32NextW(snap, &raw mut entry).is_err() {
                break;
            }
        }
        let _ = CloseHandle(snap);
        None
    }
}

/// レジストリから GJI の実行ファイルパスを取得する。
///
/// `HKLM\SOFTWARE\Google\Google Japanese Input` の `InstalledPath` 値を読み、
/// `GoogleIMEJaConverter.exe` を付加して返す。
pub fn get_gji_exe_path() -> Option<PathBuf> {
    let subkey = crate::win32::to_wide("SOFTWARE\\Google\\Google Japanese Input");
    let value_name = crate::win32::to_wide("InstalledPath");

    let mut buf = vec![0u16; 512];
    let mut buf_bytes = u32::try_from(buf.len() * 2).unwrap_or(u32::MAX);

    // SAFETY: レジストリキー・値名は NUL 終端済み UTF-16。buf は十分なサイズで確保済み。
    //         RegGetValueW は書き込みバイト数を buf_bytes に返す。HKEY_LOCAL_MACHINE は
    //         擬似ハンドルで CloseHandle 不要。
    let result = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            windows::core::PCWSTR(subkey.as_ptr()),
            windows::core::PCWSTR(value_name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&raw mut buf_bytes),
        )
    };

    if result.is_err() {
        return None;
    }

    // buf_bytes はバイト数。u16 要素数に変換し NUL を除く。
    let chars = (buf_bytes as usize) / 2;
    let s = String::from_utf16_lossy(&buf[..chars.saturating_sub(1)]);
    if s.is_empty() {
        return None;
    }
    Some(PathBuf::from(s).join(GJI_EXE))
}

/// GJI プロセスを終了して exe パスから再起動する。
///
/// - `exe_path`: レジストリから取得した `GoogleIMEJaConverter.exe` の絶対パス
/// - GJI が起動していない場合は kill をスキップして再起動のみ行う
pub fn kill_and_restart_gji(exe_path: &std::path::Path) -> Result<(), String> {
    if let Some(pid) = find_gji_pid() {
        // SAFETY: PID は直前の列挙で得た有効な値。PROCESS_TERMINATE 権限で開く。
        //         TerminateProcess 後すぐ CloseHandle してリークを防ぐ。
        unsafe {
            match OpenProcess(PROCESS_TERMINATE, false, pid) {
                Ok(h) => {
                    let _ = TerminateProcess(h, 1);
                    let _ = CloseHandle(h);
                    log::info!("GJI process {pid} terminated");
                }
                Err(e) => {
                    return Err(format!("Failed to open GJI process {pid}: {e}"));
                }
            }
        }
        // 少し待ってからプロセスが消えるのを待つ（kill は非同期）
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    match std::process::Command::new(exe_path).spawn() {
        Ok(_) => {
            log::info!("GJI restarted: {}", exe_path.display());
            Ok(())
        }
        Err(e) => Err(format!("Failed to start GJI ({}): {e}", exe_path.display())),
    }
}

/// トレイメニューから呼ばれるワンショット GJI セットアップ。
///
/// 1. config1.db をパッチ（すでに完了なら `Ok(false)`）
/// 2. GJI を kill → 再起動
///
/// 戻り値: `Ok(true)` = パッチ実施、`Ok(false)` = 既適用、`Err` = エラーメッセージ
pub fn run_gji_setup() -> Result<bool, String> {
    let path = default_config_path()
        .ok_or("LOCALAPPDATA が設定されていないため config1.db のパスを特定できません")?;

    let data = fs::read(&path)
        .map_err(|e| format!("config1.db を読み込めません ({}): {e}", path.display()))?;

    match patch(&data)? {
        None => Ok(false),
        Some((patched, added)) => {
            let backup = path.with_extension("db.bak");
            if let Err(e) = fs::write(&backup, &data) {
                log::warn!("GJI config backup failed: {e}");
            }
            fs::write(&path, &patched)
                .map_err(|e| format!("config1.db の書き込みに失敗しました: {e}"))?;
            log::info!(
                "GJI config patched: {} entries added",
                added.len()
            );
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_db(extra: &str) -> Vec<u8> {
        let block = format!("{}{extra}", std::str::from_utf8(MARKER).unwrap());
        let len = block.len();
        let varint = [(len & 0x7F) as u8 | 0x80, ((len >> 7) & 0x7F) as u8];
        let mut data = vec![0u8; 2];
        data[0] = varint[0];
        data[1] = varint[1];
        data.extend_from_slice(block.as_bytes());
        data
    }

    #[test]
    fn patch_adds_missing_entries() {
        let db = make_test_db("DirectInput\tF15\tIMEOn\n");
        let (patched, added) = patch(&db).unwrap().unwrap();
        assert_eq!(added.len(), 6);
        for entry in ENTRIES {
            assert!(
                patched.windows(entry.len()).any(|w| w == entry.as_bytes()),
                "missing: {entry}"
            );
        }
    }

    #[test]
    fn patch_skips_when_all_present() {
        let existing = ENTRIES.join("");
        let db = make_test_db(&existing);
        assert!(patch(&db).unwrap().is_none());
    }

    #[test]
    fn patch_adds_only_missing() {
        let db = make_test_db("Precomposition\tF13\tIMEOn\nPrecomposition\tF14\tIMEOff\n");
        let (_, added) = patch(&db).unwrap().unwrap();
        assert_eq!(added.len(), 4);
        assert!(!added.contains(&"Precomposition\tF13\tIMEOn\n"));
        assert!(!added.contains(&"Precomposition\tF14\tIMEOff\n"));
    }

    #[test]
    fn varint_roundtrip() {
        for v in [128usize, 5274, 5345, 16383] {
            let [b0, b1] = encode_varint2(v).unwrap();
            let decoded = ((b0 & 0x7F) as usize) | (((b1 & 0x7F) as usize) << 7);
            assert_eq!(decoded, v, "roundtrip failed for {v}");
        }
    }
}
