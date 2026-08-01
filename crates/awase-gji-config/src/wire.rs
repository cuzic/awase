//! Google 日本語入力 (GJI) の `config1.db` は SQLite ではなく protobuf
//! シリアライズされたバイナリ（`.db` という拡張子はミスリーディング）。
//!
//! ここでは Mozc（GJIのOSSベース、<https://github.com/google/mozc>）の
//! `Config` メッセージのうち、必要な2フィールドだけを protobuf wire format
//! レベルで抜き出す最小限のパーサを実装する。汎用 protobuf crate
//! （`prost` 等）は使わない — フィールド定義全体への依存を避け、field
//! number とワイヤ型さえ一致していれば動く形にすることで、Google 配布版が
//! 将来他のフィールドを増改築しても影響を受けないようにするため。
//!
//! 対象フィールド（field number は Mozc `config.proto` の `Config`
//! メッセージに基づく非公式知識。Google 配布版がこれと完全に同一である
//! 保証はない — その前提でパース失敗は常に静かなフォールバックにする）:
//! - field 22: `session_keymap`（varint, keymap style を表す enum 値）
//! - field 42: `custom_keymap_table`（length-delimited, カスタムキーマップ
//!   全体を表す TSV 文字列。1行目ヘッダ `status\tkey\tcommand`）

/// `Config` メッセージの `session_keymap` フィールド番号。
const SESSION_KEYMAP_FIELD: u64 = 22;
/// `Config` メッセージの `custom_keymap_table` フィールド番号。
const CUSTOM_KEYMAP_TABLE_FIELD: u64 = 42;

/// `config1.db` から読み取った、このクレートが関心を持つフィールドだけの集合。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GjiRawConfig {
    /// keymap style（CUSTOM か、ATOK風/MSIME風等のプリセットか）を表す enum 値。
    /// 具体的な値の意味はこのクレートでは解釈しない（呼び出し側の関心事）。
    pub session_keymap: Option<i64>,
    /// カスタムキーマップの TSV 文字列（`session_keymap` がプリセットの場合は
    /// 通常 `None`）。
    pub custom_keymap_table: Option<String>,
}

/// `bytes` を protobuf wire format として走査し、field 22/42 を抜き出す。
///
/// 空バイト列のみ `None` を返す。それ以外は、たとえ途中で未知の構造
/// （group 型フィールド等、対応していないもの）や壊れたバイト列に遭遇しても、
/// そこまでに読めたフィールドを保持したまま `Some` を返す（パニックしない）。
///
/// 注意: フィールドは番号昇順で直列化される（protobuf の一般的な実装の挙動）
/// 前提なので、field 42 より前の位置に未対応/破損フィールドがあると、そこで
/// 走査を打ち切り `custom_keymap_table` 自体を読み損ねる。Mozc の `Config`
/// に group 型フィールドは存在しないため実害リスクは低いが、限界として記す。
#[must_use]
pub fn parse_top_level(bytes: &[u8]) -> Option<GjiRawConfig> {
    if bytes.is_empty() {
        return None;
    }
    let mut pos = 0usize;
    let mut config = GjiRawConfig::default();
    while pos < bytes.len() {
        let Some(tag) = read_varint(bytes, &mut pos) else {
            break;
        };
        let field_number = tag >> 3;
        let wire_type = tag & 0x7;
        match (field_number, wire_type) {
            (SESSION_KEYMAP_FIELD, 0) => {
                let Some(value) = read_varint(bytes, &mut pos) else {
                    break;
                };
                config.session_keymap = i64::try_from(value).ok();
            }
            (CUSTOM_KEYMAP_TABLE_FIELD, 2) => {
                let Some(text) = read_length_delimited_string(bytes, &mut pos) else {
                    break;
                };
                config.custom_keymap_table = Some(text);
            }
            _ => {
                if skip_field(bytes, &mut pos, wire_type).is_none() {
                    break;
                }
            }
        }
    }
    Some(config)
}

/// protobuf の base-128 varint を1つ読み、`pos` を読み進める。
/// バッファ終端に達した、または 64bit に収まらない場合は `None`。
fn read_varint(bytes: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result: u64 = 0;
    let mut shift: u32 = 0;
    loop {
        let byte = *bytes.get(*pos)?;
        *pos += 1;
        if shift >= 64 {
            return None;
        }
        result |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
    }
}

/// length-delimited (wire type 2) フィールドの長さ prefix を読み、ペイロードの
/// `[start, end)` 範囲を返す。`pos` はこの範囲の直後（`end`）まで読み進める。
/// [`read_length_delimited_string`] と `skip_field` の wire type 2 分岐の
/// 両方から使う共通ロジック。
fn read_len_delimited_range(bytes: &[u8], pos: &mut usize) -> Option<(usize, usize)> {
    let len = read_varint(bytes, pos)?;
    let len = usize::try_from(len).ok()?;
    let start = *pos;
    let end = start.checked_add(len)?;
    if end > bytes.len() {
        return None;
    }
    *pos = end;
    Some((start, end))
}

/// length-delimited (wire type 2) フィールドを UTF-8 文字列として読む。
/// 不正な UTF-8 はロス付きで置換する（GJI 由来データは通常 UTF-8 のはず）。
fn read_length_delimited_string(bytes: &[u8], pos: &mut usize) -> Option<String> {
    let (start, end) = read_len_delimited_range(bytes, pos)?;
    Some(String::from_utf8_lossy(&bytes[start..end]).into_owned())
}

/// 関心の無いフィールドを、ワイヤ型に応じて読み飛ばす。
/// group 型（wire type 3/4、protobuf では非推奨）は対応しないため `None`。
fn skip_field(bytes: &[u8], pos: &mut usize, wire_type: u64) -> Option<()> {
    match wire_type {
        0 => {
            read_varint(bytes, pos)?;
        }
        1 => {
            let end = pos.checked_add(8)?;
            if end > bytes.len() {
                return None;
            }
            *pos = end;
        }
        2 => {
            read_len_delimited_range(bytes, pos)?;
        }
        5 => {
            let end = pos.checked_add(4)?;
            if end > bytes.len() {
                return None;
            }
            *pos = end;
        }
        _ => return None,
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::{GjiRawConfig, parse_top_level};

    /// field 22 (varint=1) + field 42 (string "hello") をエンコードした
    /// テストベクタ。Python で手計算・検算済み。
    const FIELD_22_AND_42: &[u8] = &[176, 1, 1, 210, 2, 5, 104, 101, 108, 108, 111];

    #[test]
    fn empty_bytes_returns_none() {
        assert_eq!(parse_top_level(&[]), None);
    }

    #[test]
    fn parses_session_keymap_and_custom_keymap_table() {
        let config = parse_top_level(FIELD_22_AND_42).expect("should parse");
        assert_eq!(
            config,
            GjiRawConfig {
                session_keymap: Some(1),
                custom_keymap_table: Some("hello".to_string()),
            }
        );
    }

    #[test]
    fn unknown_field_is_skipped_without_disturbing_target_fields() {
        // field 5 (varint), 適当な値。field 22/42 の前に挟んでも無視されること。
        let mut bytes = vec![
            5 << 3, // tag: field 5, wire type 0 (varint)
            42,     // 値
        ];
        bytes.extend_from_slice(FIELD_22_AND_42);
        let config = parse_top_level(&bytes).expect("should parse");
        assert_eq!(config.session_keymap, Some(1));
        assert_eq!(config.custom_keymap_table.as_deref(), Some("hello"));
    }

    #[test]
    fn truncated_bytes_never_panics() {
        for end in 1..FIELD_22_AND_42.len() {
            let _ = parse_top_level(&FIELD_22_AND_42[..end]);
        }
    }

    #[test]
    fn garbage_bytes_never_panic() {
        let garbage = [0xFFu8; 32];
        let _ = parse_top_level(&garbage);
        let all_zero = [0u8; 32];
        let _ = parse_top_level(&all_zero);
    }

    #[test]
    fn group_wire_type_is_unsupported_and_stops_gracefully() {
        // field 5, wire type 3 (start group, deprecated) — 未対応なのでそこで打ち切る。
        let bytes = [(5 << 3) | 3];
        let config = parse_top_level(&bytes).expect("should parse (best-effort)");
        assert_eq!(config, GjiRawConfig::default());
    }
}
