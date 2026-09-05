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

pub use command::{GjiCompositionMode, GjiModeCommand};
pub use keymap::{GjiImeKeys, GjiModeKeys};

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
/// `config.proto`（`google/mozc` 本家ソース `src/protocol/config.proto` で
/// 実値を確認済み: `NONE=-1, CUSTOM=0, ATOK=1, MSIME=2, KOTOERI=3, MOBILE=4,
/// CHROMEOS=5, OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF=100, ...`）。
/// `session_keymap` がこの値でない（ATOK/MS-IME 等のプリセットが選択されている）
/// 場合、GJI は `custom_keymap_table` を一切参照しない。
pub const SESSION_KEYMAP_CUSTOM: i64 = 0;

/// Mozc `SessionKeymap` enum の `ATOK` 値（BUG-115）。
///
/// `google/mozc` の `src/data/keymap/atok.tsv`（2026-09-05取得）は
/// `DirectInput`状態でHenkan/Muhenkan双方を`IMEOn`に、`Precomposition`
/// 状態で双方を`CancelAndIMEOff`に割り当てている。他のプリセット
/// （MSIME/MOBILE/KOTOERI/CHROMEOS）にはこの割当てが無い（MSIME/MOBILEの
/// Henkanは`Reconvert`でIME開閉と無関係、Muhenkanは同状態への割当て自体が
/// 無い。KOTOERI/CHROMEOSは該当行が無い）。
pub const SESSION_KEYMAP_ATOK: i64 = 1;

/// Mozc `SessionKeymap` enum の `MSIME` 値（BUG-115）。Windows版GJIの実質
/// 既定（`session_keymap`不在/`NONE`時のフォールバック、
/// `config_handler.cc::GetDefaultKeyMap()`で確認済み）。
///
/// `src/data/keymap/ms-ime.tsv`（2026-09-05取得）は`DirectInput`状態で
/// Hiragana/Katakanaを`IMEOn`に割り当てている（Henkan/Muhenkanは
/// `Reconvert`のみでIME開閉と無関係、`SESSION_KEYMAP_ATOK`のdoc参照）。
pub const SESSION_KEYMAP_MSIME: i64 = 2;

/// Mozc `SessionKeymap` enum の `MOBILE` 値（BUG-115）。
///
/// `src/data/keymap/mobile.tsv`（2026-09-05取得）は`ms-ime.tsv`と同一の
/// Henkan/Muhenkan/Hiragana/Katakana割当て。
pub const SESSION_KEYMAP_MOBILE: i64 = 4;

/// Mozc `SessionKeymap` enum の `OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF` 値
/// （BUG-115）。
///
/// `overlay_keymaps`（`config.proto` field 68、`session_keymap`/
/// `custom_keymap_table` とは独立の repeated フィールド）にこの値が含まれると、
/// GJI は `session_keymap` の値（CUSTOM かプリセットか）に関わらず、変換
/// （Henkan）→IMEOn を Composition/Conversion/DirectInput/Precomposition の
/// 全4状態に、無変換（Muhenkan）→IMEOff を Composition/Conversion/
/// Precomposition の3状態（IMEが既にOFFの`DirectInput`除く）に無条件で
/// 重ね掛けする（本家ソース
/// `src/data/keymap/overlay_henkan_muhenkan_to_ime_on_off.tsv` で確認済み）。
/// このクレートは現時点でこの値を検出する手段
/// （[`crate::wire::GjiRawConfig::overlay_keymaps`]）を提供するのみで、
/// `read_gji_ime_keys`/`read_gji_mode_keys` の戻り値には反映していない
/// （無変換/変換キーは `mozc_key_to_vk_name` の出力範囲に含まれないため、
/// `custom_keymap_table` 経由の通常の抽出ロジックでは表現できない。呼び出し
/// 側でこの定数を直接チェックする必要がある）。
pub const SESSION_KEYMAP_OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF: i64 = 100;

#[cfg(test)]
mod tests {
    use super::{
        read_gji_ime_keys, GjiImeKeys, SESSION_KEYMAP_OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF,
    };

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

    /// BUG-115: field 68 (`overlay_keymaps`) に
    /// `OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF` (100) が含まれることを、
    /// `wire::parse_top_level` 経由で検出できることを確認する。
    #[test]
    fn detects_henkan_muhenkan_overlay_via_wire_parse() {
        // field 68, wire type 2 (length-delimited packed varint) に値 100 のみ。
        // tag = (68 << 3) | 2 = 546 → varint [162, 4]。ペイロード長 1、値 100。
        let bytes = [162, 4, 1, 100];
        let raw = crate::wire::parse_top_level(&bytes).expect("should parse");
        assert!(raw
            .overlay_keymaps
            .contains(&SESSION_KEYMAP_OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF));
    }
}
