#![cfg_attr(windows, allow(unsafe_code))]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! Windows Scancode Map（`HKLM\SYSTEM\CurrentControlSet\Control\Keyboard
//! Layout\Scancode Map`）の読み書きと、Caps(英数)⇔Left Ctrl 入れ替え
//! プリセット用のバイト列生成・マージロジック（ADR-111）。
//!
//! バイナリ形式は Windows のドキュメント化された固定フォーマット
//! （4バイトヘッダ×2 + エントリ数(null終端込み) + エントリ配列 + null終端）
//! に従う。エントリはリトルエンディアン `u16` ペア `[to_scancode,
//! from_scancode]` の順。パース・生成・マージのロジックは純粋関数として
//! 全プラットフォームでコンパイル・テストできる（`awase-settings` から
//! Linux上でも単体テストするため）。レジストリ I/O のみ `#[cfg(windows)]`。

/// JIS「英数」キー / US「CapsLock」キーの物理スキャンコード（Set 1）。
/// 両者は物理的に同一の位置・同一のスキャンコードを共有し、レイアウト
/// ドライバが Shift 状態で異なる VK に翻訳する（ADR-111 決定1）。
pub const SCANCODE_CAPS_EISU: u16 = 0x003A;
/// Left Ctrl のスキャンコード（Set 1、非拡張）。
pub const SCANCODE_LEFT_CTRL: u16 = 0x001D;

/// このプリセットが書き込む2エントリ（`(from, to)` の順）。
#[must_use]
pub const fn preset_entries() -> [(u16, u16); 2] {
    [
        (SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL),
        (SCANCODE_LEFT_CTRL, SCANCODE_CAPS_EISU),
    ]
}

/// Scancode Map の `REG_BINARY` 値をパースし `(from, to)` のリストを返す。
/// 不正な形式（短すぎる等）は空リストを返す（既存値が壊れていた場合に
/// 安全側で「未設定」扱いにするため、エラーにはしない）。
#[must_use]
pub fn parse_entries(bytes: &[u8]) -> Vec<(u16, u16)> {
    if bytes.len() < 12 {
        return Vec::new();
    }
    let count = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    let mut out = Vec::new();
    let mut offset = 12;
    // count にはnull終端エントリ自身も含まれるので count-1 件読む。
    for _ in 0..count.saturating_sub(1) {
        let Some(chunk) = bytes.get(offset..offset + 4) else {
            break;
        };
        let to = u16::from_le_bytes([chunk[0], chunk[1]]);
        let from = u16::from_le_bytes([chunk[2], chunk[3]]);
        if from == 0 && to == 0 {
            break;
        }
        out.push((from, to));
        offset += 4;
    }
    out
}

/// `(from, to)` のリストから Scancode Map の `REG_BINARY` 値を構築する。
/// `entries` が空なら `None`（値自体を削除すべきことを示す）。
#[must_use]
pub fn build_bytes(entries: &[(u16, u16)]) -> Option<Vec<u8>> {
    if entries.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(12 + entries.len() * 4 + 4);
    out.extend_from_slice(&0u32.to_le_bytes()); // Header: Version
    out.extend_from_slice(&0u32.to_le_bytes()); // Header: Flags
    let count = u32::try_from(entries.len() + 1).unwrap_or(u32::MAX);
    out.extend_from_slice(&count.to_le_bytes());
    for &(from, to) in entries {
        out.extend_from_slice(&to.to_le_bytes());
        out.extend_from_slice(&from.to_le_bytes());
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // null 終端エントリ
    Some(out)
}

/// `entries` にこのプリセットの2エントリが両方含まれているか。
#[must_use]
pub fn is_preset_active(entries: &[(u16, u16)]) -> bool {
    preset_entries().iter().all(|e| entries.contains(e))
}

/// 有効化時: 既存エントリから `from` がこのプリセットのスキャンコードと
/// 重複するものを除去し、プリセットの2エントリを追加する（ADR-111決定3）。
#[must_use]
pub fn merge_for_enable(existing: &[(u16, u16)]) -> Vec<(u16, u16)> {
    let mut out: Vec<(u16, u16)> = remove_preset(existing);
    out.extend(preset_entries());
    out
}

/// 無効化時: プリセットの2エントリ（`from`基準）のみを除去する
/// （ADR-111決定3）。他の（awaseと無関係な）エントリはそのまま残る。
#[must_use]
pub fn remove_preset(existing: &[(u16, u16)]) -> Vec<(u16, u16)> {
    existing
        .iter()
        .copied()
        .filter(|&(from, _)| from != SCANCODE_CAPS_EISU && from != SCANCODE_LEFT_CTRL)
        .collect()
}

#[cfg(windows)]
mod registry {
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
        if result == windows::Win32::Foundation::ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("Scancode Map 削除失敗: {result:?}"))
        }
    }
}

#[cfg(windows)]
pub use registry::{delete, read, write};

#[cfg(test)]
mod tests {
    use super::{
        build_bytes, is_preset_active, merge_for_enable, parse_entries, preset_entries,
        remove_preset, SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL,
    };

    #[test]
    fn build_then_parse_roundtrips_preset() {
        let entries = preset_entries();
        let bytes = build_bytes(&entries).expect("non-empty entries");
        assert_eq!(parse_entries(&bytes), entries.to_vec());
    }

    #[test]
    fn build_bytes_matches_documented_layout_for_preset() {
        let bytes = build_bytes(&preset_entries()).unwrap();
        // ヘッダ(Version=0, Flags=0) + count=3(2エントリ+null終端)
        assert_eq!(&bytes[0..4], &0u32.to_le_bytes());
        assert_eq!(&bytes[4..8], &0u32.to_le_bytes());
        assert_eq!(&bytes[8..12], &3u32.to_le_bytes());
        // エントリ1: to=0x001D, from=0x003A
        assert_eq!(&bytes[12..14], &0x001Du16.to_le_bytes());
        assert_eq!(&bytes[14..16], &0x003Au16.to_le_bytes());
        // エントリ2: to=0x003A, from=0x001D
        assert_eq!(&bytes[16..18], &0x003Au16.to_le_bytes());
        assert_eq!(&bytes[18..20], &0x001Du16.to_le_bytes());
        // null終端
        assert_eq!(&bytes[20..24], &0u32.to_le_bytes());
        assert_eq!(bytes.len(), 24);
    }

    #[test]
    fn build_bytes_returns_none_for_empty() {
        assert!(build_bytes(&[]).is_none());
    }

    #[test]
    fn parse_entries_handles_short_or_garbage_input_safely() {
        assert_eq!(parse_entries(&[]), Vec::new());
        assert_eq!(parse_entries(&[0u8; 11]), Vec::new());
        assert_eq!(parse_entries(&[0u8; 12]), Vec::new());
    }

    #[test]
    fn is_preset_active_true_when_both_entries_present_among_others() {
        let mut entries = vec![(0x0010, 0x0011)];
        entries.extend(preset_entries());
        assert!(is_preset_active(&entries));
    }

    #[test]
    fn is_preset_active_false_when_only_one_direction_present() {
        let entries = vec![(SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL)];
        assert!(!is_preset_active(&entries));
    }

    #[test]
    fn merge_for_enable_preserves_unrelated_entries_and_replaces_conflicting_from() {
        let existing = vec![
            (0x0010, 0x0011),             // 無関係、保持されるべき
            (SCANCODE_CAPS_EISU, 0x0099), // 衝突、置き換えられるべき
            (SCANCODE_LEFT_CTRL, 0x0088), // 衝突、置き換えられるべき
        ];
        let merged = merge_for_enable(&existing);
        assert!(merged.contains(&(0x0010, 0x0011)));
        assert!(is_preset_active(&merged));
        assert!(!merged.contains(&(SCANCODE_CAPS_EISU, 0x0099)));
        assert!(!merged.contains(&(SCANCODE_LEFT_CTRL, 0x0088)));
        assert_eq!(merged.len(), 3);
    }

    #[test]
    fn remove_preset_keeps_unrelated_entries_and_drops_preset_entries() {
        let mut existing = vec![(0x0010, 0x0011)];
        existing.extend(preset_entries());
        let removed = remove_preset(&existing);
        assert_eq!(removed, vec![(0x0010, 0x0011)]);
    }

    #[test]
    fn remove_preset_on_preset_only_yields_empty() {
        let existing = preset_entries().to_vec();
        assert!(remove_preset(&existing).is_empty());
    }
}
