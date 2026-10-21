#![allow(unsafe_code)]
// Win32 API（レジストリ）呼び出しに unsafe が必須
//! Windows Scancode Map のレジストリ I/O（`HKLM\SYSTEM\CurrentControlSet\Control\Keyboard Layout\Scancode Map` の読み書き）。
//!
//! 以前は `scancode_map.rs` の `#[cfg(windows)] mod registry` だった。純粋部（エントリ列とバイト列の変換）を
//! OS 非依存の crate へ移す（ADR-274 段階 1）ために、Windows 専用のこの殻のファイルへ分けた。
//! 何をどう書くかの判断は `scancode_pairs`（ペア）・`scancode_apply`（昇格側の適用）が持つ。

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_LOCAL_MACHINE, REG_BINARY,
    RRF_RT_REG_BINARY,
};

const SUBKEY: PCWSTR = w!("SYSTEM\\CurrentControlSet\\Control\\Keyboard Layout");
const VALUE_NAME: PCWSTR = w!("Scancode Map");

/// 現在の Scancode Map の値を読み取る。値が存在しなければ `Ok(None)`。
/// 昇格不要（`HKEY_LOCAL_MACHINE` は既定で誰でも読める）。
pub fn read() -> Result<Option<Vec<u8>>, String> {
    let mut size: u32 = 0;
    // SAFETY: 出力バッファ引数は None（サイズ取得のみ）。他の引数は
    // 静的な NUL 終端済み UTF-16 文字列。
    let result = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            SUBKEY,
            VALUE_NAME,
            RRF_RT_REG_BINARY,
            None,
            None,
            Some(&raw mut size),
        )
    };
    if result != windows::Win32::Foundation::ERROR_SUCCESS {
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        return Err(format!("Scancode Map 読み取り失敗(サイズ取得): {result:?}"));
    }
    if size == 0 {
        return Ok(Some(Vec::new()));
    }
    let mut buf = vec![0u8; size as usize];
    let mut actual_size = size;
    // SAFETY: buf は size バイト確保済みで、呼び出し中有効。
    let result = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            SUBKEY,
            VALUE_NAME,
            RRF_RT_REG_BINARY,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&raw mut actual_size),
        )
    };
    if result == windows::Win32::Foundation::ERROR_SUCCESS {
        buf.truncate(actual_size as usize);
        Ok(Some(buf))
    } else {
        Err(format!("Scancode Map 読み取り失敗: {result:?}"))
    }
}

/// キーボードクラスのフィルタドライバ名（`UpperFilters` か `LowerFilters`）を読む（ADR-248。Ctrl2cap 等の検出用）。
/// 値が無ければ空。昇格不要。
pub fn read_keyboard_class_filters(value_name: &str) -> Result<Vec<String>, String> {
    use windows::Win32::System::Registry::RRF_RT_REG_MULTI_SZ;
    const CLASS_KEY: PCWSTR =
        w!("SYSTEM\\CurrentControlSet\\Control\\Class\\{4D36E96B-E325-11CE-BFC1-08002BE10318}");
    let value: Vec<u16> = value_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut size: u32 = 0;
    // SAFETY: 出力バッファは None（サイズ取得のみ）。value は NUL 終端済みで呼び出し中有効。
    let result = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            CLASS_KEY,
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_MULTI_SZ,
            None,
            None,
            Some(&raw mut size),
        )
    };
    if result == ERROR_FILE_NOT_FOUND {
        return Ok(Vec::new());
    }
    if result != windows::Win32::Foundation::ERROR_SUCCESS {
        return Err(format!("{value_name} 読み取り失敗(サイズ取得): {result:?}"));
    }
    let mut buf = vec![0u8; size as usize];
    let mut actual_size = size;
    // SAFETY: buf は size バイト確保済みで、呼び出し中有効。
    let result = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            CLASS_KEY,
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_MULTI_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&raw mut actual_size),
        )
    };
    if result == windows::Win32::Foundation::ERROR_SUCCESS {
        buf.truncate(actual_size as usize);
        Ok(crate::scancode_map::parse_multi_sz(&buf))
    } else {
        Err(format!("{value_name} 読み取り失敗: {result:?}"))
    }
}

/// Scancode Map に値を書き込む。管理者権限が必要（呼び出し元は
/// 昇格済みであること、`awase-settings` の自己昇格フロー参照）。
pub fn write(bytes: &[u8]) -> Result<(), String> {
    // SAFETY: bytes は呼び出し中有効なスライス。
    let result = unsafe {
        RegSetKeyValueW(
            HKEY_LOCAL_MACHINE,
            SUBKEY,
            VALUE_NAME,
            REG_BINARY.0,
            Some(bytes.as_ptr().cast()),
            u32::try_from(bytes.len()).unwrap_or(u32::MAX),
        )
    };
    if result == windows::Win32::Foundation::ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("Scancode Map 書き込み失敗: {result:?}"))
    }
}

/// Scancode Map の値自体を削除する。管理者権限が必要。
pub fn delete() -> Result<(), String> {
    // SAFETY: `HKEY_LOCAL_MACHINE` は擬似ハンドルで CloseHandle 不要。
    //         SUBKEY/VALUE_NAME は静的な NUL 終端済み UTF-16 文字列。
    let result = unsafe { RegDeleteKeyValueW(HKEY_LOCAL_MACHINE, SUBKEY, VALUE_NAME) };
    if result == windows::Win32::Foundation::ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(format!("Scancode Map 削除失敗: {result:?}"))
    }
}
