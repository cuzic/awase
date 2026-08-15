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
pub use write::{DedicatedFnKeySpec, ExistingBinding, RECOMMENDED_DEDICATED_FN_KEYS};

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

/// Mozc `SessionKeymap` enum の `CUSTOM` 値。
///
/// `config.proto`（非公式知識だが `google/mozc` 本家ソースで確認済み:
/// `NONE=-1, CUSTOM=0, ATOK=1, MSIME=2, KOTOERI=3, MOBILE=4, CHROMEOS=5, ...`）。
/// `session_keymap` がこの値でない（ATOK/MS-IME 等のプリセットが選択されている）
/// 場合、GJI は `custom_keymap_table` を一切参照しない。
pub const SESSION_KEYMAP_CUSTOM: i64 = 0;

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
    /// `session_keymap` が [`SESSION_KEYMAP_CUSTOM`] でない（ATOK/MS-IME 等の
    /// プリセットが選択されている）。この状態で `custom_keymap_table` を
    /// 書いても GJI はそれを一切参照しないため、書き込む意味が無く中止する。
    /// ユーザーが GJI の「キー設定」を「カスタム」に切り替えてから再試行する
    /// 必要がある（`session_keymap` 自体の書き換えは、他の既存カスタム
    /// バインド全体の有効/無効を左右する影響範囲の大きい変更になるため、
    /// このクレートは行わない）。
    NotCustomKeymap,
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
/// `bytes` が protobuf として解釈できない場合、`session_keymap` が
/// [`SESSION_KEYMAP_CUSTOM`] でない場合、または `vk_key` の既存バインドが
/// 既知の残骸パターンと一致しない場合。
pub fn write_dedicated_fn_key_binding(
    bytes: &[u8],
    vk_key: &str,
) -> Result<Vec<u8>, WriteDedicatedFnKeyError> {
    let raw = wire::parse_top_level(bytes).ok_or(WriteDedicatedFnKeyError::UnparsableConfig)?;
    if raw.session_keymap != Some(SESSION_KEYMAP_CUSTOM) {
        return Err(WriteDedicatedFnKeyError::NotCustomKeymap);
    }
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

/// `config1.db` の生バイト列に、[`RECOMMENDED_DEDICATED_FN_KEYS`] 全キー
/// （F21-F24）のエントリを一度に追加した新しいバイト列を返す。
///
/// [`write_dedicated_fn_key_binding`] の単一キー版と同じ手順を4キーぶん
/// まとめて行う（`session_keymap` チェック→衝突検出→TSV組み立て→
/// フィールド差し替え）。**呼び出しは1回のファイル書き込み・1回のGJI再起動で
/// 完結する** ——ユーザーに何度もサインアウト/インを依頼しないための設計
/// （2026-08-15、ユーザー要望）。
///
/// # Errors
///
/// [`write_dedicated_fn_key_binding`] と同様。ただし衝突は4キーのうち1つでも
/// 検出されれば、書き込みは行わず全体を中止する
/// （[`write::classify_existing_binding_set`] 参照）。
pub fn write_dedicated_fn_key_set(bytes: &[u8]) -> Result<Vec<u8>, WriteDedicatedFnKeyError> {
    let raw = wire::parse_top_level(bytes).ok_or(WriteDedicatedFnKeyError::UnparsableConfig)?;
    if raw.session_keymap != Some(SESSION_KEYMAP_CUSTOM) {
        return Err(WriteDedicatedFnKeyError::NotCustomKeymap);
    }
    let existing_table = raw.custom_keymap_table.unwrap_or_default();
    if let write::ExistingBinding::Conflict { rows } =
        write::classify_existing_binding_set(&existing_table)
    {
        return Err(WriteDedicatedFnKeyError::Conflict { rows });
    }
    let new_table = write::upsert_dedicated_fn_key_set(&existing_table);
    wire::replace_custom_keymap_table(bytes, &new_table)
        .ok_or(WriteDedicatedFnKeyError::UnparsableConfig)
}

#[cfg(test)]
mod tests {
    use super::{
        read_gji_ime_keys, read_gji_mode_keys, write_dedicated_fn_key_binding,
        write_dedicated_fn_key_set, GjiImeKeys, WriteDedicatedFnKeyError,
    };

    /// `session_keymap = SESSION_KEYMAP_CUSTOM`（field 22）と
    /// `custom_keymap_table = table`（field 42）を積んだ最小 protobuf バイト列を
    /// 作る。`write_dedicated_fn_key_binding` は `session_keymap ==
    /// Some(SESSION_KEYMAP_CUSTOM)` を前提とするため、このヘルパーはその前提を
    /// 満たす「カスタムキーマップ選択中」のフィクスチャを表す。
    fn encode_custom_keymap_table_only(table: &str) -> Vec<u8> {
        let mut bytes = vec![176u8, 1, 0]; // field 22 (session_keymap) = CUSTOM(0)
        bytes.push(0xD2u8);
        bytes.push(0x02);
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
        let table = "status\tkey\tcommand\nComposition\tF21\tBackspace\n";
        let bytes = encode_custom_keymap_table_only(table);
        let err = write_dedicated_fn_key_binding(&bytes, "F21").expect_err("should conflict");
        assert!(matches!(err, WriteDedicatedFnKeyError::Conflict { .. }));
    }

    /// `write_dedicated_fn_key_binding` は冪等: 一度書き込んだ結果に対して
    /// 再度呼んでも衝突扱いにならず、同じ内容を返す（同一セッション内の
    /// ポップアップ再試行、または複数セッションでの再検出のいずれでも
    /// 失敗しないことの固定）。
    #[test]
    fn write_dedicated_fn_key_binding_is_idempotent() {
        let bytes = encode_custom_keymap_table_only("status\tkey\tcommand\n");
        let once = write_dedicated_fn_key_binding(&bytes, "F21").expect("should write");
        let twice = write_dedicated_fn_key_binding(&once, "F21").expect("should write again");
        assert_eq!(once, twice);
    }

    #[test]
    fn write_dedicated_fn_key_binding_on_unparsable_bytes_is_error() {
        // session_keymap = CUSTOM を先に置き、その後ろに group wire type (3、
        // 未対応)を続ける。session_keymap チェックは通過させた上で、
        // replace_custom_keymap_table 側の再走査が壊れたバイト列を検出することを
        // 固定する（session_keymap チェックより先に UnparsableConfig を返す
        // ケースの回帰）。
        let mut bytes = vec![176u8, 1, 0]; // field 22 (session_keymap) = CUSTOM(0)
        bytes.push((5 << 3) | 3); // field 5, group wire type (未対応)
        let err = write_dedicated_fn_key_binding(&bytes, "F21").expect_err("should be unparsable");
        assert_eq!(err, WriteDedicatedFnKeyError::UnparsableConfig);
    }

    /// `parse_top_level` 自体が空バイト列で `None` を返すケース
    /// （session_keymap チェックに到達する前に中止する）。
    #[test]
    fn write_dedicated_fn_key_binding_on_empty_bytes_is_unparsable() {
        let err = write_dedicated_fn_key_binding(&[], "F21").expect_err("should be unparsable");
        assert_eq!(err, WriteDedicatedFnKeyError::UnparsableConfig);
    }

    /// `custom_keymap_table`（field 42）自体が元々存在しなくても、
    /// `session_keymap = CUSTOM` でさえあれば新規追加できる
    /// （カスタムキーマップを選択した直後、まだ何もカスタマイズしていない状態）。
    #[test]
    fn write_dedicated_fn_key_binding_creates_table_when_absent() {
        // field 22 (session_keymap = CUSTOM) のみ、field 42 は無し。
        let bytes: &[u8] = &[176, 1, 0];
        let written = write_dedicated_fn_key_binding(bytes, "F21").expect("should write");
        let mode_keys = super::read_gji_mode_keys(&written);
        assert_eq!(mode_keys.toggle_kana_type, vec!["VK_F21".to_string()]);
    }

    /// `session_keymap` が `CUSTOM` でない（プリセット選択中、またはフィールド
    /// 自体が無い）場合、`custom_keymap_table` に何が書かれていても GJI は
    /// それを参照しないため、書き込みを中止する（Opus レビュー指摘）。
    #[test]
    fn write_dedicated_fn_key_binding_refuses_when_not_custom_keymap() {
        // field 22 (session_keymap = ATOK = 1)。
        let bytes: &[u8] = &[176, 1, 1];
        let err = write_dedicated_fn_key_binding(bytes, "F21").expect_err("should refuse");
        assert_eq!(err, WriteDedicatedFnKeyError::NotCustomKeymap);
    }

    /// `session_keymap` フィールド自体が無い場合も同様に中止する
    /// （デフォルトが `CUSTOM` である保証がない、安全側に倒す）。
    #[test]
    fn write_dedicated_fn_key_binding_refuses_when_session_keymap_absent() {
        let bytes =
            encode_custom_keymap_table_only_without_session_keymap("status\tkey\tcommand\n");
        let err = write_dedicated_fn_key_binding(&bytes, "F21").expect_err("should refuse");
        assert_eq!(err, WriteDedicatedFnKeyError::NotCustomKeymap);
    }

    /// `session_keymap` を含めない `custom_keymap_table` のみのフィクスチャ
    /// （`refuses_when_session_keymap_absent` 専用）。
    fn encode_custom_keymap_table_only_without_session_keymap(table: &str) -> Vec<u8> {
        let mut bytes = vec![0xD2u8, 0x02];
        bytes.push(u8::try_from(table.len()).expect("fixture length fits in u8"));
        bytes.extend_from_slice(table.as_bytes());
        bytes
    }

    // ── write_dedicated_fn_key_set ────────────────────────────────────────

    /// F21 は全 status で `SwitchKanaType`（単一の役割）のため
    /// `read_gji_mode_keys`（`extract_mode_keys` の「1キー1役割」前提、
    /// `keymap.rs` のモジュールdoc・関数doc参照）で正しく検出できる。
    /// F22-24 は status によって役割が変わる（Precomposition では
    /// `CompositionModeX`、Composition 系では `SwitchKanaType`）ため、
    /// `extract_mode_keys` の設計上あえて拾わない（「遷移先が一意に定まらない」
    /// として意図的にスキップされる）。F22-24 の中身は生の TSV 文字列で検証する
    /// （`classify_existing_binding_set`/`upsert_dedicated_fn_key_set` の
    /// テストと同じレイヤー）。
    #[test]
    fn write_dedicated_fn_key_set_end_to_end_on_empty_config() {
        let bytes = encode_custom_keymap_table_only("status\tkey\tcommand\n");
        let written = write_dedicated_fn_key_set(&bytes).expect("should write");

        let mode_keys = read_gji_mode_keys(&written);
        assert_eq!(mode_keys.toggle_kana_type, vec!["VK_F21".to_string()]);

        let raw = crate::wire::parse_top_level(&written).expect("should parse");
        let table = raw.custom_keymap_table.unwrap_or_default();
        assert!(table.contains("Precomposition\tF22\tCompositionModeHiragana"));
        assert!(table.contains("Precomposition\tF23\tCompositionModeFullKatakana"));
        assert!(table.contains("Precomposition\tF24\tCompositionModeHalfKatakana"));
        for key in ["F21", "F22", "F23", "F24"] {
            for status in ["Composition", "Conversion", "Prediction", "Suggestion"] {
                assert!(table.contains(&format!("{status}\t{key}\tSwitchKanaType")));
            }
        }
    }

    #[test]
    fn write_dedicated_fn_key_set_is_idempotent() {
        let bytes = encode_custom_keymap_table_only("status\tkey\tcommand\n");
        let once = write_dedicated_fn_key_set(&bytes).expect("should write");
        let twice = write_dedicated_fn_key_set(&once).expect("should write again");
        assert_eq!(once, twice);
    }

    /// 4キーのうち1つ（F23）が他アプリ/ユーザー由来と思われる未知のコマンドと
    /// 衝突していれば、他の3キーが安全でも全体を中止し、何も書き込まない。
    #[test]
    fn write_dedicated_fn_key_set_refuses_on_partial_conflict() {
        let table = "status\tkey\tcommand\nComposition\tF23\tBackspace\n";
        let bytes = encode_custom_keymap_table_only(table);
        let err = write_dedicated_fn_key_set(&bytes).expect_err("should conflict");
        assert!(matches!(err, WriteDedicatedFnKeyError::Conflict { .. }));
    }

    #[test]
    fn write_dedicated_fn_key_set_refuses_when_not_custom_keymap() {
        let bytes: &[u8] = &[176, 1, 1]; // session_keymap = ATOK
        let err = write_dedicated_fn_key_set(bytes).expect_err("should refuse");
        assert_eq!(err, WriteDedicatedFnKeyError::NotCustomKeymap);
    }
}
