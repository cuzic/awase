//! literal 判定の証拠にする変換サーバーの選び方（BUG-207、ADR-269）。OS 非依存の純関数。
//!
//! `raw-tsf-literal` の判定（`probe_io`）と warmup の確認は、GJI の変換サーバー
//! （`GoogleIMEJaConverter`）の累積書き込み量（`TSF_OBS.gji_write_bytes`、閾値 350B は Chrome + GJI で実測）
//! を根拠にする。しかも `gji_is_active_ime()`（= 監視が健全 かつ GJI 系がアクティブ）が偽なら、
//! 判定そのものを走らせない（証拠が取れない IME では安全網を外す、という既存の規律）。
//!
//! アクティブな TIP が Mozc 系（Mozc Readable など）のとき、GJI の変換サーバーが同時に動いていると、
//! 静止した GJI を見て「書き込みなし = リテラル」と誤判定し、入力済みの文字を消していた。
//! Mozc 系の変換サーバー（`mozc_server`）は 1 VK あたり約 0.2〜0.3KB しか書かず、閾値 350B に届かない
//! （CI 実測、ADR-269）ので、`mozc_server` を見ても同じ誤判定になる。Mozc 系では GJI の証拠を使わない。

use crate::state::ime_kind::TipIdentity;

/// literal 判定の証拠にする変換サーバー。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConverterTarget {
    /// Google 日本語入力の変換サーバー（GJI 系でない TIP がアクティブでも、従来どおりこれを見る）。
    Gji,
    /// 使える証拠がない（Mozc 系）。監視に接続せず、`gji_is_active_ime()` を偽にして判定を走らせない。
    None,
}

/// Google 日本語入力の変換サーバーのプロセス名の接頭辞（大文字小文字無視）。
/// バージョンによりプロセス名が異なるため複数候補を持つ。Renderer/CacheService は対象外。
const GJI_PREFIXES: &[&str] = &["GoogleIMEJaConverter", "GoogleJapaneseInputConverter"];

/// アクティブな TIP から、literal 判定の証拠にする変換サーバーを選ぶ。
///
/// `gji_family` = `ActiveImeKind::GoogleJapaneseInput`（Google 版と Mozc 系の両方）。
/// `identity` = `TipIdentity::Gji` は Google 版のみ。GJI 系なのに `Gji` でないものは Mozc 系。
#[must_use]
pub const fn converter_target(gji_family: bool, identity: TipIdentity) -> ConverterTarget {
    match (gji_family, identity) {
        (true, TipIdentity::Other) => ConverterTarget::None,
        (true, TipIdentity::Gji | TipIdentity::MsImeNative) | (false, _) => ConverterTarget::Gji,
    }
}

/// プロセス名（`szExeFile`）が GJI の変換サーバーかどうか。
#[must_use]
pub fn is_gji_converter_process(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    GJI_PREFIXES
        .iter()
        .any(|p| lower.starts_with(&p.to_ascii_lowercase()))
}

/// 見つからなかったときのログ用に、探した接頭辞を返す。
#[must_use]
pub const fn searched_prefixes() -> &'static [&'static str] {
    GJI_PREFIXES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_tip_uses_gji_converter() {
        assert_eq!(
            converter_target(true, TipIdentity::Gji),
            ConverterTarget::Gji
        );
    }

    /// BUG-207: Mozc Readable は GJI 系（kind = GoogleJapaneseInput）だが identity は Gji でない。
    /// GJI の書き込み量を証拠にしない。
    #[test]
    fn mozc_family_tip_has_no_gji_evidence() {
        assert_eq!(
            converter_target(true, TipIdentity::Other),
            ConverterTarget::None
        );
    }

    /// GJI 系でない TIP（MS-IME・ATOK 等）では従来どおり GJI を見る（挙動を変えない）。
    #[test]
    fn non_gji_family_keeps_gji() {
        for id in [
            TipIdentity::Gji,
            TipIdentity::MsImeNative,
            TipIdentity::Other,
        ] {
            assert_eq!(converter_target(false, id), ConverterTarget::Gji);
        }
    }

    #[test]
    fn only_gji_converter_processes_match() {
        assert!(is_gji_converter_process("GoogleIMEJaConverter.exe"));
        assert!(is_gji_converter_process("googlejapaneseinputconverter.exe"));
        // Mozc 系・Renderer・別プロセスは対象外（GJI と Mozc が両方動いていても Mozc を拾わない）。
        assert!(!is_gji_converter_process("mozc_server.exe"));
        assert!(!is_gji_converter_process("GoogleIMEJaRenderer.exe"));
        assert!(!is_gji_converter_process("notepad.exe"));
    }
}
