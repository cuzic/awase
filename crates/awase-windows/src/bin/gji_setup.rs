//! GJI (Google Japanese Input) config1.db パッチツール
//!
//! GJI の config1.db に F14 キーバインドを追加する。
//!
//! 追加エントリ:
//!   Precomposition  F14  IMEOff
//!   Composition     F14  IMEOff
//!   Conversion      F14  IMEOff
//!
//! これにより awase が Ctrl+Shift+Delete の代わりに F14 で IME OFF を制御できるようになる。
//! F14 は実キーボードに存在せず、ブラウザショートカットとも衝突しない。
//!
//! 使用法:
//!   awase-gji-setup              # デフォルトパス自動検出
//!   awase-gji-setup <path>       # パス指定

use std::{fs, path::PathBuf, process::ExitCode};

const MARKER: &[u8] = b"status\tkey\tcommand\n";

const ENTRIES: &[&str] = &[
    "Precomposition\tF14\tIMEOff\n",
    "Composition\tF14\tIMEOff\n",
    "Conversion\tF14\tIMEOff\n",
];

fn main() -> ExitCode {
    let path = match resolve_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!("target: {}", path.display());

    let data = match fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: cannot read '{}': {e}", path.display());
            return ExitCode::FAILURE;
        }
    };

    match patch(&data) {
        Ok(None) => {
            println!("already up-to-date: all F14 entries are present");
            ExitCode::SUCCESS
        }
        Ok(Some((patched, added))) => {
            let backup = path.with_extension("db.bak");
            if let Err(e) = fs::write(&backup, &data) {
                eprintln!("warning: backup failed ({}): {e}", backup.display());
            } else {
                println!("backup: {}", backup.display());
            }
            match fs::write(&path, &patched) {
                Ok(()) => {
                    println!("patched: {} bytes -> {} bytes", data.len(), patched.len());
                    for entry in &added {
                        println!("  added: {}", entry.trim_end());
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: cannot write '{}': {e}", path.display());
                    ExitCode::FAILURE
                }
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn resolve_path() -> Result<PathBuf, String> {
    let mut args = std::env::args().skip(1);
    if let Some(arg) = args.next() {
        return Ok(PathBuf::from(arg));
    }
    default_path().ok_or_else(|| {
        "cannot determine default path; LOCALAPPDATA not set. \
         Specify path explicitly: awase-gji-setup <path>"
            .to_string()
    })
}

/// `%LOCALAPPDATA%\..\LocalLow\Google\Google Japanese Input\config1.db`
fn default_path() -> Option<PathBuf> {
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

/// キーバインドブロックの位置と長さを返す。
/// 戻り値: (varint_offset, block_start, block_length)
fn find_block(data: &[u8]) -> Result<(usize, usize, usize), String> {
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

fn encode_varint2(value: usize) -> Result<[u8; 2], String> {
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

/// `patch` の戻り値型エイリアス: `Ok(None)` = パッチ不要、`Ok(Some((bytes, names)))` = パッチ済み。
type PatchResult = Result<Option<(Vec<u8>, Vec<&'static str>)>, String>;

/// 不足エントリを末尾に追加したバイト列と追加したエントリ名を返す。
/// すでに全エントリが存在する場合は `Ok(None)`。
fn patch(data: &[u8]) -> PatchResult {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_db(extra: &str) -> Vec<u8> {
        // varint(len) + block
        let block = format!("{}{extra}", std::str::from_utf8(MARKER).unwrap());
        let len = block.len();
        let varint = [(len & 0x7F) as u8 | 0x80, ((len >> 7) & 0x7F) as u8];
        let mut data = vec![0u8; 2]; // dummy prefix
        data[0] = varint[0];
        data[1] = varint[1];
        data.extend_from_slice(block.as_bytes());
        data
    }

    #[test]
    fn patch_adds_missing_entries() {
        let db = make_test_db("DirectInput\tF13\tIMEOn\n");
        let result = patch(&db).unwrap().unwrap();
        let (patched, added) = result;
        assert_eq!(added.len(), 3);
        let text = std::str::from_utf8(&patched).unwrap();
        for entry in ENTRIES {
            assert!(text.contains(entry), "missing: {entry}");
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
        let db = make_test_db("Precomposition\tF14\tIMEOff\n");
        let (_, added) = patch(&db).unwrap().unwrap();
        assert_eq!(added.len(), 2);
        assert!(!added.contains(&"Precomposition\tF14\tIMEOff\n"));
    }

    #[test]
    fn varint_roundtrip() {
        for v in [0usize, 127, 128, 5274, 5345, 16383] {
            if v < 128 {
                continue; // 1-byte case, not covered by encode_varint2
            }
            let [b0, b1] = encode_varint2(v).unwrap();
            let decoded = ((b0 & 0x7F) as usize) | (((b1 & 0x7F) as usize) << 7);
            assert_eq!(decoded, v, "roundtrip failed for {v}");
        }
    }
}
