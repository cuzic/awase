//! Windows Scancode Map のエントリ列とバイト列の変換（ADR-111 / ADR-126 / ADR-230）。
//!
//! 値は `HKLM\SYSTEM\CurrentControlSet\Control\Keyboard Layout\Scancode Map`。何をどう書くかの判断は
//! `scancode_pairs`（ペア）・`scancode_apply`（昇格側の適用）が持つ。
//!
//! バイナリ形式は Windows のドキュメント化された固定フォーマット
//! （4バイトヘッダ×2 + エントリ数(null終端込み) + エントリ配列 + null終端）
//! に従う。エントリはリトルエンディアン `u16` ペア `[to_scancode,
//! from_scancode]` の順。パース・生成・マージのロジックは純粋関数として
//! 全プラットフォームでコンパイル・テストできる（`awase-settings` から
//! Linux上でも単体テストするため）。レジストリ I/O は殻の `scancode_registry.rs`（`#[cfg(windows)]`、ADR-274）。

/// JIS「英数」キー / US「CapsLock」キーの物理スキャンコード（Set 1）。
/// 両者は物理的に同一の位置・同一のスキャンコードを共有し、レイアウト
/// ドライバが Shift 状態で異なる VK に翻訳する（ADR-111 決定1）。
pub const SCANCODE_CAPS_EISU: u16 = 0x003A;
/// Left Ctrl のスキャンコード（Set 1、非拡張）。
pub const SCANCODE_LEFT_CTRL: u16 = 0x001D;

/// Scancode Map の `REG_BINARY` 値をパースし `(from, to)` のリストを返す。
/// 不正な形式（短すぎる等）は空リストを返す。**壊れた値と空の値を区別しないので、上書きの判断には使わない**
/// （`scancode_apply::parse_entries_strict` を使う）。
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

/// `REG_MULTI_SZ`（UTF-16LE、NUL 区切り、末尾は二重 NUL）を文字列の一覧に直す。空の要素は捨てる。
#[must_use]
pub fn parse_multi_sz(bytes: &[u8]) -> Vec<String> {
    // 奇数バイトの端数（`as_chunks` の余り）は無視する。
    let (pairs, _rest) = bytes.as_chunks::<2>();
    let units: Vec<u16> = pairs.iter().map(|&c| u16::from_le_bytes(c)).collect();
    units
        .split(|&u| u == 0)
        .filter(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_multi_sz_splits_on_nul_and_ignores_the_double_nul_terminator() {
        let to_bytes =
            |s: &str| -> Vec<u8> { s.encode_utf16().flat_map(u16::to_le_bytes).collect() };
        let bytes = to_bytes("HPKbfDriver\0kbdclass\0ctrl2cap\0\0");
        assert_eq!(
            parse_multi_sz(&bytes),
            vec!["HPKbfDriver", "kbdclass", "ctrl2cap"]
        );
        assert!(parse_multi_sz(&[]).is_empty());
        assert!(parse_multi_sz(&to_bytes("\0\0")).is_empty());
        // 奇数バイトの端数は無視する。
        let mut odd = to_bytes("a\0\0");
        odd.push(0x41);
        assert_eq!(parse_multi_sz(&odd), vec!["a"]);
    }

    /// Caps(英数)⇔左 Ctrl 入れ替え（2エントリ）。
    const SWAP: [(u16, u16); 2] = [
        (SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL),
        (SCANCODE_LEFT_CTRL, SCANCODE_CAPS_EISU),
    ];
    /// Caps(英数)→左 Ctrl の片方向（1エントリ）。
    const CAPS_AS_EXTRA_CTRL: [(u16, u16); 1] = [(SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL)];

    #[test]
    fn build_then_parse_roundtrips_two_entries() {
        let bytes = build_bytes(&SWAP).expect("non-empty entries");
        assert_eq!(parse_entries(&bytes), SWAP.to_vec());
    }

    #[test]
    fn build_then_parse_roundtrips_one_entry() {
        let bytes = build_bytes(&CAPS_AS_EXTRA_CTRL).expect("non-empty entries");
        assert_eq!(parse_entries(&bytes), CAPS_AS_EXTRA_CTRL.to_vec());
    }

    #[test]
    fn build_bytes_matches_documented_layout_for_two_entries() {
        let bytes = build_bytes(&SWAP).unwrap();
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
    fn build_bytes_matches_documented_layout_for_one_entry() {
        let bytes = build_bytes(&CAPS_AS_EXTRA_CTRL).unwrap();
        // ヘッダ(Version=0, Flags=0) + count=2(1エントリ+null終端)
        assert_eq!(&bytes[0..4], &0u32.to_le_bytes());
        assert_eq!(&bytes[4..8], &0u32.to_le_bytes());
        assert_eq!(&bytes[8..12], &2u32.to_le_bytes());
        // エントリ1: to=0x001D, from=0x003A
        assert_eq!(&bytes[12..14], &0x001Du16.to_le_bytes());
        assert_eq!(&bytes[14..16], &0x003Au16.to_le_bytes());
        // null終端
        assert_eq!(&bytes[16..20], &0u32.to_le_bytes());
        assert_eq!(bytes.len(), 20);
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
}
