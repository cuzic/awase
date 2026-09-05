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
//! - field 41: `session_keymap`（varint, keymap style を表す enum 値）
//! - field 42: `custom_keymap_table`（length-delimited, カスタムキーマップ
//!   全体を表す TSV 文字列。1行目ヘッダ `status\tkey\tcommand`）
//! - field 68: `overlay_keymaps`（packed repeated varint, ベースのキーマップに
//!   追加で重ね掛けするオーバーレイの enum 値集合。`custom_keymap_table`とは
//!   独立に効き、`session_keymap`がプリセットでも無条件に適用される）
//!
//! # field 22 は `session_keymap` ではない（BUG-115）
//!
//! 旧実装は `session_keymap` のフィールド番号を **22** と誤って記録していた
//! （本家 `google/mozc` の `src/protocol/config.proto` を実際に取得して確認した
//! ところ、field 22 は無関係の `check_default`（bool、既定IME確認ダイアログの
//! 表示要否）であり、`session_keymap` は field **41**）。どちらも wire type が
//! varint で一致するため、パース自体は失敗せず「もっともらしい」誤った値を
//! 返していた（実機の `config1.db` で検証: field 22 = 1、field 41 = 0(CUSTOM)。
//! 誤って field 22 を使うと「1 = ATOK」という完全な誤判定になる）。

/// `Config` メッセージの `session_keymap` フィールド番号（BUG-115: 旧実装は
/// 22 と誤っていた。本家 `config.proto` の実値は 41）。
const SESSION_KEYMAP_FIELD: u64 = 41;
/// `Config` メッセージの `custom_keymap_table` フィールド番号。
const CUSTOM_KEYMAP_TABLE_FIELD: u64 = 42;
/// `Config` メッセージの `overlay_keymaps` フィールド番号（packed repeated
/// varint、`SessionKeymap` enum値の集合）。
const OVERLAY_KEYMAPS_FIELD: u64 = 68;

/// `config1.db` から読み取った、このクレートが関心を持つフィールドだけの集合。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GjiRawConfig {
    /// keymap style（CUSTOM か、ATOK風/MSIME風等のプリセットか）を表す enum 値。
    /// 具体的な値の意味はこのクレートでは解釈しない（呼び出し側の関心事）。
    pub session_keymap: Option<i64>,
    /// カスタムキーマップの TSV 文字列（`session_keymap` がプリセットの場合は
    /// 通常 `None`）。
    pub custom_keymap_table: Option<String>,
    /// ベースのキーマップに重ね掛けされるオーバーレイの enum 値集合
    /// （`SessionKeymap` enum、例: `OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF = 100`）。
    /// `session_keymap`/`custom_keymap_table` の値に関わらず独立に効く。
    /// フィールド自体が存在しなければ空 `Vec`。
    pub overlay_keymaps: Vec<i64>,
}

/// `bytes` を protobuf wire format として走査し、field 41/42/68 を抜き出す。
///
/// 空バイト列のみ `None` を返す。それ以外は、たとえ途中で未知の構造
/// （group 型フィールド等、対応していないもの）や壊れたバイト列に遭遇しても、
/// そこまでに読めたフィールドを保持したまま `Some` を返す（パニックしない）。
///
/// 注意: フィールドは番号昇順で直列化される（protobuf の一般的な実装の挙動）
/// 前提なので、field 68 より前の位置に未対応/破損フィールドがあると、そこで
/// 走査を打ち切り `overlay_keymaps` 自体を読み損ねる。Mozc の `Config`
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
                // enum は proto2 上 int32 相当で負値(`NONE = -1`)を取りうる。
                // varint は負値を64bit二の補数の符号拡張で運ぶため、値として
                // 範囲チェックする `i64::try_from` ではなく、ビット列をその
                // まま符号付きに再解釈する `cast_signed` を使う（BUG-115、
                // Opusレビュー指摘: `try_from`だと`NONE`が常に`None`に化ける）。
                config.session_keymap = Some(value.cast_signed());
            }
            (CUSTOM_KEYMAP_TABLE_FIELD, 2) => {
                let Some(text) = read_length_delimited_string(bytes, &mut pos) else {
                    break;
                };
                config.custom_keymap_table = Some(text);
            }
            (OVERLAY_KEYMAPS_FIELD, 2) => {
                // packed encoding。protobuf 仕様上、同じ repeated フィールドが
                // メッセージ内に複数回出現しうる（分割直列化）ため、
                // 上書きではなく連結する（BUG-115、Opusレビュー指摘）。
                let Some((start, end)) = read_len_delimited_range(bytes, &mut pos) else {
                    break;
                };
                config
                    .overlay_keymaps
                    .extend(read_packed_varints(&bytes[start..end]));
            }
            (OVERLAY_KEYMAPS_FIELD, 0) => {
                // 非packed encoding（`[packed=true]`指定でもprotobufの後方/
                // 前方互換規則により送信側は非packedで書いてよく、受信側は
                // 両対応が必須。BUG-115、Opusレビュー指摘）。1回の出現につき
                // 値1個。
                let Some(value) = read_varint(bytes, &mut pos) else {
                    break;
                };
                config.overlay_keymaps.push(value.cast_signed());
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

/// packed repeated varint フィールドのペイロード（length-delimited の中身）を
/// 連続する varint 列として読む。壊れた varint に遭遇したら、そこまでに
/// 読めた値を保持したまま打ち切る（パニックしない）。
fn read_packed_varints(payload: &[u8]) -> Vec<i64> {
    let mut pos = 0usize;
    let mut values = Vec::new();
    while pos < payload.len() {
        let Some(value) = read_varint(payload, &mut pos) else {
            break;
        };
        // enum の負値(`NONE = -1`)を捨てないよう `cast_signed` を使う
        // （BUG-115、`session_keymap`側と同じ理由。上の doc 参照）。
        values.push(value.cast_signed());
    }
    values
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
    use super::{parse_top_level, GjiRawConfig};

    /// field 41 (varint=1) + field 42 (string "hello") をエンコードした
    /// テストベクタ。Python で手計算・検算済み（BUG-115: 旧テストは誤って
    /// field 22 でエンコードしていた）。
    const FIELD_41_AND_42: &[u8] = &[200, 2, 1, 210, 2, 5, 104, 101, 108, 108, 111];

    /// field 68 (packed varint `[100, 5]`) をエンコードしたテストベクタ。
    /// `100` は `OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF`。
    const FIELD_68: &[u8] = &[162, 4, 2, 100, 5];

    /// field 22 (varint=1) 単体をエンコードしたテストベクタ。BUG-115で
    /// 誤って `session_keymap` として読んでいた無関係フィールド
    /// （実際は `check_default`）。field 41 だけを読むことの回帰ガード。
    const FIELD_22_ONLY: &[u8] = &[176, 1, 1];

    #[test]
    fn empty_bytes_returns_none() {
        assert_eq!(parse_top_level(&[]), None);
    }

    #[test]
    fn parses_session_keymap_and_custom_keymap_table() {
        let config = parse_top_level(FIELD_41_AND_42).expect("should parse");
        assert_eq!(
            config,
            GjiRawConfig {
                session_keymap: Some(1),
                custom_keymap_table: Some("hello".to_string()),
                overlay_keymaps: vec![],
            }
        );
    }

    #[test]
    fn parses_overlay_keymaps_as_packed_varints() {
        let mut bytes = FIELD_41_AND_42.to_vec();
        bytes.extend_from_slice(FIELD_68);
        let config = parse_top_level(&bytes).expect("should parse");
        assert_eq!(config.overlay_keymaps, vec![100, 5]);
    }

    /// BUG-115の回帰ガード: field 22（無関係な`check_default`）が
    /// `session_keymap`として読まれないこと。将来「互換性のため22も見る」
    /// を足すとこのテストが落ちる。
    #[test]
    fn field_22_is_not_mistaken_for_session_keymap() {
        let config = parse_top_level(FIELD_22_ONLY).expect("should parse");
        assert_eq!(config.session_keymap, None);
    }

    /// 上記の否定側（22を無視する）と `parses_session_keymap_and_custom_keymap_table`
    /// の肯定側（41を読む）を1本化した回帰ガード。field 22=1（実機の
    /// `check_default`）と field 41=0（実機の`session_keymap=CUSTOM`）を
    /// 両方積んだ、実際の `config1.db` のバイト形状そのもの。「22を無視し
    /// かつ41を読む」を同時に固定する（Opusレビュー指摘R3）。
    #[test]
    fn field_22_and_field_41_together_yield_session_keymap_from_41_only() {
        let mut bytes = FIELD_22_ONLY.to_vec(); // field 22 = 1
        bytes.extend_from_slice(&[200, 2, 0]); // field 41 = 0 (CUSTOM)
        let config = parse_top_level(&bytes).expect("should parse");
        assert_eq!(config.session_keymap, Some(0));
    }

    /// 複数回出現した packed repeated フィールドは上書きではなく連結される
    /// （protobuf 仕様: 分割直列化の許容。BUG-115、Opusレビュー指摘）。
    #[test]
    fn overlay_keymaps_from_multiple_chunks_are_concatenated_not_overwritten() {
        let mut bytes = FIELD_68.to_vec(); // [100, 5]
        bytes.extend_from_slice(FIELD_68); // 同じチャンクをもう一度
        let config = parse_top_level(&bytes).expect("should parse");
        assert_eq!(config.overlay_keymaps, vec![100, 5, 100, 5]);
    }

    /// 非packed encoding（`wire_type=0`が同じfield番号で複数回出現する形）も
    /// 受理できる（protobufは受信側にpacked/非packed両対応を要求する。
    /// BUG-115、Opusレビュー指摘）。
    #[test]
    fn overlay_keymaps_accepts_unpacked_encoding() {
        // tag(68, wire_type=0) = 68<<3|0 = 544 → varint [160, 4]。
        let mut bytes = vec![160, 4, 100]; // field 68 varint=100
        bytes.extend_from_slice(&[160, 4, 5]); // field 68 varint=5 (2個目)
        let config = parse_top_level(&bytes).expect("should parse");
        assert_eq!(config.overlay_keymaps, vec![100, 5]);
    }

    /// enum の負値（`NONE = -1`）が varint の符号拡張から正しく復元される
    /// こと。`i64::try_from` では `u64::MAX` が `Err` になり値が消えていた
    /// （BUG-115、Opusレビュー指摘: 実測で `u64::MAX` を確認済み）。
    #[test]
    fn negative_enum_value_survives_session_keymap_and_overlay_keymaps() {
        // -1 を64bit二の補数の符号拡張varint（10バイト）でエンコード。
        let neg_one_varint: [u8; 10] = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01];

        // field 41 (session_keymap), wire_type=0, 値=-1。
        let mut bytes = vec![200, 2]; // tag: field 41, wire type 0
        bytes.extend_from_slice(&neg_one_varint);
        let config = parse_top_level(&bytes).expect("should parse");
        assert_eq!(config.session_keymap, Some(-1));

        // field 68 (overlay_keymaps), packed, 値=[-1]。
        let mut bytes = vec![162, 4]; // tag: field 68, wire type 2
        bytes.push(u8::try_from(neg_one_varint.len()).expect("fits in u8"));
        bytes.extend_from_slice(&neg_one_varint);
        let config = parse_top_level(&bytes).expect("should parse");
        assert_eq!(config.overlay_keymaps, vec![-1]);
    }

    /// packed varint ペイロード内で壊れた（末尾で途切れた）varintに遭遇したら
    /// パニックせず、そこまでに読めた値を保持して打ち切る。
    #[test]
    fn overlay_keymaps_truncated_mid_payload_keeps_prior_values_without_panic() {
        // payload = [100, 0x80]。100は妥当な単バイトvarint、0x80は継続
        // ビット付きだが後続バイトが無い＝壊れたvarint。
        let payload = [100u8, 0x80];
        let mut bytes = vec![162, 4]; // tag: field 68, wire type 2
        bytes.push(u8::try_from(payload.len()).expect("fits in u8"));
        bytes.extend_from_slice(&payload);
        let config = parse_top_level(&bytes).expect("should parse");
        assert_eq!(config.overlay_keymaps, vec![100]);
    }

    #[test]
    fn unknown_field_is_skipped_without_disturbing_target_fields() {
        // field 5 (varint), 適当な値。field 41/42 の前に挟んでも無視されること。
        let mut bytes = vec![
            5 << 3, // tag: field 5, wire type 0 (varint)
            42,     // 値
        ];
        bytes.extend_from_slice(FIELD_41_AND_42);
        let config = parse_top_level(&bytes).expect("should parse");
        assert_eq!(config.session_keymap, Some(1));
        assert_eq!(config.custom_keymap_table.as_deref(), Some("hello"));
    }

    #[test]
    fn truncated_bytes_never_panics() {
        for end in 1..FIELD_41_AND_42.len() {
            let _ = parse_top_level(&FIELD_41_AND_42[..end]);
        }
    }

    /// field 68 を含む列を全長にわたって切り詰めてもパニックしないこと
    /// （BUG-115で追加した経路は旧`truncated_bytes_never_panics`では
    /// 一度も踏まれていなかった）。
    #[test]
    fn truncated_bytes_with_overlay_field_never_panics() {
        let mut bytes = FIELD_41_AND_42.to_vec();
        bytes.extend_from_slice(FIELD_68);
        for end in 1..bytes.len() {
            let _ = parse_top_level(&bytes[..end]);
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
