//! GJIの設定が、内蔵表が対応する「既知の構成」（ATOK/MS-IMEプリセット）と
//! 一致するかを判定する（ADR-196決定1c）。
//!
//! この判定結果は、学習結果の採否そのものには使わない——一致する場合に限り
//! 学習結果と内蔵表をセル単位で突き合わせ（不一致は棄却でなく再測定の引き金
//! にする、決定1b）、不一致率の計算対象を絞り込むためだけに使う。

use crate::tsv::parse_custom_keymap_table;
use crate::{SESSION_KEYMAP_ATOK, SESSION_KEYMAP_MSIME};

/// 内蔵表が対応する既知のGJI構成（Microsoft IME本体側の判定は別途
/// `awase-windows`側が持つ——本クレートはGJI固有の設定しか読めないため）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnownGjiKeymap {
    Atok,
    /// GJIのMS-IMEプリセット（Microsoft IME本体そのものではない）。
    MsIme,
}

/// `custom_keymap_table`（TSV）に、無変換/変換キーの行が1件でも存在するか。
///
/// 存在すれば、実効的なキーマップは同梱表が測定した対象（プリセットの素の
/// 挙動）と異なりうるため、MS-IMEプリセットの「既知構成」判定から除外する。
fn has_henkan_or_muhenkan_row(custom_keymap_table: &str) -> bool {
    parse_custom_keymap_table(custom_keymap_table)
        .iter()
        .any(|row| row.key == "Henkan" || row.key == "Muhenkan")
}

/// GJIの設定が、内蔵表が対応する既知の構成と一致するかを判定する
/// （ADR-196決定1c）。
///
/// - **ATOK**: `custom_keymap_table`の有無・中身を問わない（ADR-186決定(c)に
///   より、GJI自身がATOKプリセット選択時にこのテーブルを実行時に無視する
///   ため）。
/// - **MS-IMEプリセット**: `custom_keymap_table`が空、または無変換/変換の
///   行を含まないこと（実機知見が食い違うため、安全側に倒して除外する。
///   ADR-196決定1c参照）。
/// - いずれの場合も`overlay_keymaps`が空であること（overlayは既定の挙動を
///   別の意味論へ丸ごと変えるため）。
#[must_use]
pub fn classify_known_gji_keymap(
    session_keymap: Option<i64>,
    overlay_keymaps: &[i64],
    custom_keymap_table: Option<&str>,
) -> Option<KnownGjiKeymap> {
    if !overlay_keymaps.is_empty() {
        return None;
    }
    match session_keymap {
        Some(v) if v == SESSION_KEYMAP_ATOK => Some(KnownGjiKeymap::Atok),
        Some(v) if v == SESSION_KEYMAP_MSIME => {
            let overridden = custom_keymap_table
                .is_some_and(|table| !table.trim().is_empty() && has_henkan_or_muhenkan_row(table));
            if overridden {
                None
            } else {
                Some(KnownGjiKeymap::MsIme)
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atok_matches_regardless_of_custom_table() {
        assert_eq!(
            classify_known_gji_keymap(Some(SESSION_KEYMAP_ATOK), &[], None),
            Some(KnownGjiKeymap::Atok)
        );
        // ADR-186決定(c): ATOK選択時はGJIがcustom_keymap_tableを無視するので、
        // 中身がHenkan/Muhenkanを上書きしていても既知構成のまま。
        assert_eq!(
            classify_known_gji_keymap(
                Some(SESSION_KEYMAP_ATOK),
                &[],
                Some("status\tkey\tcommand\nDirectInput\tHenkan\tIMEOn\n")
            ),
            Some(KnownGjiKeymap::Atok)
        );
    }

    #[test]
    fn msime_matches_when_custom_table_absent_or_empty() {
        assert_eq!(
            classify_known_gji_keymap(Some(SESSION_KEYMAP_MSIME), &[], None),
            Some(KnownGjiKeymap::MsIme)
        );
        assert_eq!(
            classify_known_gji_keymap(Some(SESSION_KEYMAP_MSIME), &[], Some("")),
            Some(KnownGjiKeymap::MsIme)
        );
        assert_eq!(
            classify_known_gji_keymap(
                Some(SESSION_KEYMAP_MSIME),
                &[],
                Some("status\tkey\tcommand\nDirectInput\tF21\tIMEOn\n")
            ),
            Some(KnownGjiKeymap::MsIme)
        );
    }

    #[test]
    fn msime_excluded_when_custom_table_overrides_henkan_or_muhenkan() {
        assert_eq!(
            classify_known_gji_keymap(
                Some(SESSION_KEYMAP_MSIME),
                &[],
                Some("status\tkey\tcommand\nDirectInput\tHenkan\tIMEOn\n")
            ),
            None
        );
        assert_eq!(
            classify_known_gji_keymap(
                Some(SESSION_KEYMAP_MSIME),
                &[],
                Some("status\tkey\tcommand\nDirectInput\tMuhenkan\tIMEOff\n")
            ),
            None
        );
    }

    #[test]
    fn overlay_excludes_both_presets() {
        assert_eq!(
            classify_known_gji_keymap(Some(SESSION_KEYMAP_ATOK), &[100], None),
            None
        );
        assert_eq!(
            classify_known_gji_keymap(Some(SESSION_KEYMAP_MSIME), &[100], None),
            None
        );
    }

    #[test]
    fn custom_session_keymap_is_never_known() {
        assert_eq!(
            classify_known_gji_keymap(Some(crate::SESSION_KEYMAP_CUSTOM), &[], None),
            None
        );
        assert_eq!(classify_known_gji_keymap(None, &[], None), None);
    }
}
