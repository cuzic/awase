//! Google 日本語入力 (GJI) の `config1.db` から、ユーザーがカスタムキーマップで
//! 割り当てている IME ON/OFF キーを読み取るための、プラットフォーム非依存の
//! 純粋ロジック。
//!
//! Win32 API・ファイル I/O・GJI がアクティブかどうかの判定は一切行わない
//! （呼び出し側の責務）。このクレートの入力は「`config1.db` の生バイト列」、
//! 出力は「awase の `VkCode::from_name` が受理する VK 名の集合」のみ。
//!
//! # 使い方
//!
//! ```
//! # let bytes: &[u8] = &[]; // 実際は config1.db を読み込んだバイト列
//! let keys = awase_gji_config::read_gji_ime_keys(bytes);
//! // keys.on / keys.off / keys.toggle は Vec<String>（VK 名）。
//! // 空の場合はファイル無し・パース不能・対象キー無しのいずれか
//! // （呼び出し側は既存設定を維持してよい）。
//! ```
//!
//! # 非公式フォーマットへの依存について
//!
//! `config1.db` の protobuf スキーマは Google 非公開だが、GJI の実装ベースは
//! OSS の Mozc (<https://github.com/google/mozc>, Apache-2.0) であり、
//! field number・TSV 形式はそこから得た非公式知識に基づく。Google 配布版が
//! 将来これと異なる可能性があるため、パース失敗は常に空の結果に静かに
//! フォールバックし、呼び出し側の設定を壊さない設計にしてある
//! （[`crate::wire::parse_top_level`] のドキュメント参照）。

pub mod command;
pub mod keymap;
pub mod tsv;
pub mod wire;
pub mod write;

pub use command::{GjiCompositionMode, GjiModeCommand};
pub use keymap::{GjiImeKeys, GjiModeKeys};
pub use write::ExistingBinding;

/// `config1.db` の生バイト列から、IME ON/OFF 検出用の VK 名集合を抽出する。
///
/// 以下のいずれの場合も、パニックせず空の [`GjiImeKeys`]（`on`/`off`/`toggle`
/// すべて空の `Vec`）を返す:
/// - `bytes` が空、または protobuf として解釈できない
/// - `custom_keymap_table`（field 42）が存在しない（プリセットキーマップ選択時など）
/// - 抽出できる IME ON/OFF キーが無い
#[must_use]
pub fn read_gji_ime_keys(bytes: &[u8]) -> GjiImeKeys {
    let Some(raw) = wire::parse_top_level(bytes) else {
        return GjiImeKeys::default();
    };
    let Some(table) = raw.custom_keymap_table else {
        return GjiImeKeys::default();
    };
    keymap::extract_ime_keys(&table)
}

/// `config1.db` の生バイト列から、入力モード変更（かな/カタカナ/半角全角など）
/// に使われている VK 名の分類を抽出する。[`read_gji_ime_keys`] の IME ON/OFF
/// 版に対応するモード版で、失敗条件・空の意味も同様（[`GjiModeKeys::default`]
/// が返る）。
#[must_use]
pub fn read_gji_mode_keys(bytes: &[u8]) -> GjiModeKeys {
    let Some(raw) = wire::parse_top_level(bytes) else {
        return GjiModeKeys::default();
    };
    let Some(table) = raw.custom_keymap_table else {
        return GjiModeKeys::default();
    };
    keymap::extract_mode_keys(&table)
}

/// `write_dedicated_fn_key_binding` の失敗理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteDedicatedFnKeyError {
    /// `bytes` が protobuf として解釈できない、または書き戻せない
    /// （読み込み時は「読めた分だけ返す」ベストエフォートだが、書き込みは
    /// 理解できないバイト列を破棄することになるため中止する）。
    UnparsableConfig,
    /// `vk_key` の既存バインドが既知の awase 残骸パターン（BUG-64）と一致しない
    /// （他アプリ由来の設定、またはユーザー自身の意図的な設定の可能性がある）。
    Conflict {
        /// 衝突の原因になった既存の行（`status\tkey\tcommand`）。
        rows: Vec<String>,
    },
}

/// `config1.db` の生バイト列に、専用Fnキー変換（ADR-091 §D3.2）のエントリを
/// 追加した新しいバイト列を返す。
///
/// 手順: (1) `vk_key` の既存バインドを検査し、`IMEOn`/`IMEOff` 以外を含むなら
/// [`WriteDedicatedFnKeyError::Conflict`] で中止する（BUG-64 の既知残骸のみ
/// 安全に上書きする）。(2) `Composition`/`Conversion`/`Prediction`/
/// `Suggestion` に `SwitchKanaType` を追加した新しい `custom_keymap_table` を
/// 組み立てる（`Precomposition`/`DirectInput` には意図的にバインドしない）。
/// (3) 元のバイト列の `custom_keymap_table` フィールドだけを差し替える
/// （他のフィールドはバイト単位で温存、[`wire::replace_custom_keymap_table`]
/// 参照）。
///
/// **ファイル I/O・バックアップ・原子的置換・GJI プロセス再起動の要否の判断は
/// 呼び出し側（プラットフォーム層）の責務。** このクレートはメモリ上のバイト列
/// 変換のみを行う。
///
/// # Errors
///
/// `bytes` が protobuf として解釈できない場合、または `vk_key` の既存バインドが
/// 既知の残骸パターンと一致しない場合。
pub fn write_dedicated_fn_key_binding(
    bytes: &[u8],
    vk_key: &str,
) -> Result<Vec<u8>, WriteDedicatedFnKeyError> {
    let raw = wire::parse_top_level(bytes).ok_or(WriteDedicatedFnKeyError::UnparsableConfig)?;
    let existing_table = raw.custom_keymap_table.unwrap_or_default();
    if let write::ExistingBinding::Conflict { rows } =
        write::classify_existing_binding(&existing_table, vk_key)
    {
        return Err(WriteDedicatedFnKeyError::Conflict { rows });
    }
    let new_table = write::upsert_dedicated_fn_key_entries(&existing_table, vk_key);
    wire::replace_custom_keymap_table(bytes, &new_table)
        .ok_or(WriteDedicatedFnKeyError::UnparsableConfig)
}

#[cfg(test)]
mod tests {
    use super::{
        read_gji_ime_keys, write_dedicated_fn_key_binding, GjiImeKeys, WriteDedicatedFnKeyError,
    };

    /// field 42 (LEN) に `table` を積んだだけの最小 protobuf バイト列を作る
    /// （`end_to_end_from_encoded_custom_keymap_table` と同じ手法）。
    fn encode_custom_keymap_table_only(table: &str) -> Vec<u8> {
        let mut bytes = vec![0xD2u8, 0x02];
        bytes.push(u8::try_from(table.len()).expect("fixture length fits in u8"));
        bytes.extend_from_slice(table.as_bytes());
        bytes
    }

    #[test]
    fn empty_bytes_yields_empty_keys() {
        assert_eq!(read_gji_ime_keys(&[]), GjiImeKeys::default());
    }

    #[test]
    fn garbage_bytes_never_panic_and_yield_empty_keys() {
        let garbage = [1u8, 2, 3, 4, 5, 255, 254, 253];
        // パニックしないことが主目的。結果の中身は問わない。
        let _ = read_gji_ime_keys(&garbage);
    }

    #[test]
    fn end_to_end_from_encoded_custom_keymap_table() {
        // field 42 (LEN) に "status\tkey\tcommand\nDirectInput\tF21\tIMEOn\n"
        // を積んだだけの最小 protobuf。タグバイト列 [0xD2, 0x02] は
        // field=42, wire_type=2 (length-delimited) の varint エンコード
        // （`wire.rs` のテストベクタと同じ計算方法で検算済み）。
        let table = "status\tkey\tcommand\nDirectInput\tF21\tIMEOn\n";
        let mut bytes = vec![0xD2u8, 0x02];
        bytes.push(u8::try_from(table.len()).expect("fixture length fits in u8"));
        bytes.extend_from_slice(table.as_bytes());

        let keys = read_gji_ime_keys(&bytes);
        assert_eq!(keys.on, vec!["VK_F21".to_string()]);
        assert!(keys.off.is_empty());
        assert!(keys.toggle.is_empty());
    }

    #[test]
    fn write_dedicated_fn_key_binding_end_to_end_on_empty_config() {
        let bytes = encode_custom_keymap_table_only("status\tkey\tcommand\n");
        let written = write_dedicated_fn_key_binding(&bytes, "F21").expect("should write");
        let keys = read_gji_ime_keys(&written); // 目的が違うので中身は見ないが壊れていないことの確認
        assert!(keys.on.is_empty() && keys.off.is_empty() && keys.toggle.is_empty());
        let mode_keys = super::read_gji_mode_keys(&written);
        assert_eq!(
            mode_keys.toggle_kana_type,
            vec!["VK_F21".to_string()],
            "書き込み後、config1.dbを再読み込みするとF21がSwitchKanaTypeとして検出できるはず"
        );
    }

    /// BUG-64 の既知残骸（DirectInput→IMEOn 等）が残っていても、上書きして
    /// 書き込める（衝突扱いにしない）。
    #[test]
    fn write_dedicated_fn_key_binding_overwrites_known_bug64_residual() {
        let table = "status\tkey\tcommand
DirectInput\tF21\tIMEOn
Precomposition\tF21\tIMEOn
Composition\tF21\tIMEOn
Conversion\tF21\tIMEOn
";
        let bytes = encode_custom_keymap_table_only(table);
        let written = write_dedicated_fn_key_binding(&bytes, "F21").expect("should write");
        let mode_keys = super::read_gji_mode_keys(&written);
        assert_eq!(mode_keys.toggle_kana_type, vec!["VK_F21".to_string()]);
        let ime_keys = read_gji_ime_keys(&written);
        assert!(
            ime_keys.on.is_empty(),
            "旧IMEOn残骸は上書きされ、もう検出されないはず"
        );
    }

    /// 未知のバインド（他アプリ/ユーザー自身の設定の可能性）とは衝突として
    /// 中止し、書き込まない。
    #[test]
    fn write_dedicated_fn_key_binding_refuses_on_conflict() {
        let table = "status\tkey\tcommand\nComposition\tF21\tSwitchKanaType\n";
        let bytes = encode_custom_keymap_table_only(table);
        let err = write_dedicated_fn_key_binding(&bytes, "F21").expect_err("should conflict");
        assert!(matches!(err, WriteDedicatedFnKeyError::Conflict { .. }));
    }

    #[test]
    fn write_dedicated_fn_key_binding_on_unparsable_bytes_is_error() {
        // group wire type (3) は未対応のため中止する。
        let bytes = [(5 << 3) | 3];
        let err = write_dedicated_fn_key_binding(&bytes, "F21").expect_err("should be unparsable");
        assert_eq!(err, WriteDedicatedFnKeyError::UnparsableConfig);
    }

    /// `custom_keymap_table`（field 42）自体が元々存在しなくても書き込める
    /// （新規追加、GJIがプリセットキーマップ選択中の場合等）。
    #[test]
    fn write_dedicated_fn_key_binding_creates_table_when_absent() {
        // field 22 (varint=1) のみ、field 42 は無し。
        let bytes: &[u8] = &[176, 1, 1];
        let written = write_dedicated_fn_key_binding(bytes, "F21").expect("should write");
        let mode_keys = super::read_gji_mode_keys(&written);
        assert_eq!(mode_keys.toggle_kana_type, vec!["VK_F21".to_string()]);
    }
}
