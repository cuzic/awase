//! I/O を監視する変換サーバーの選び方（BUG-207）。OS 非依存の純関数。
//!
//! `raw-tsf-literal` の判定（`probe_io`）と warmup の確認は、変換サーバーの書き込み量
//! （`TSF_OBS.gji_write_bytes`）を根拠にする。アクティブな TIP が Mozc 系（Mozc Readable など）
//! のとき、変換サーバーは `GoogleIMEJaConverter` ではなく `mozc_server` なので、監視の対象を
//! Google 日本語入力に固定すると、Mozc が正しく変換していても「書き込みなし = リテラル」と
//! 誤判定して入力済みの文字を消す。GJI と Mozc の両方が動いている環境では、プロセス名で
//! 最初に見つかったものではなく、アクティブな TIP で選ぶ。

use crate::state::ime_kind::TipIdentity;

/// 書き込み量を見る変換サーバーの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConverterTarget {
    /// Google 日本語入力（既定。GJI 以外の TIP がアクティブでも従来どおりこれを見る）。
    Gji,
    /// Mozc とそのフォーク（Mozc Readable など）の `mozc_server`。
    MozcServer,
}

/// Google 日本語入力の変換サーバーのプロセス名の接頭辞（大文字小文字無視）。
/// バージョンによりプロセス名が異なるため複数候補を持つ。Renderer/CacheService は対象外。
const GJI_PREFIXES: &[&str] = &["GoogleIMEJaConverter", "GoogleJapaneseInputConverter"];

/// Mozc 系の変換サーバーのプロセス名の接頭辞（大文字小文字無視）。
const MOZC_PREFIXES: &[&str] = &["mozc_server"];

/// アクティブな TIP から、監視する変換サーバーを選ぶ。
///
/// `gji_family` = `ActiveImeKind::GoogleJapaneseInput`（Google 版と Mozc 系の両方）。
/// `identity` = `TipIdentity::Gji` は Google 版のみ。GJI 系なのに `Gji` でないものは Mozc 系。
#[must_use]
pub const fn converter_target(gji_family: bool, identity: TipIdentity) -> ConverterTarget {
    match (gji_family, identity) {
        (true, TipIdentity::Gji | TipIdentity::MsImeNative) | (false, _) => ConverterTarget::Gji,
        (true, TipIdentity::Other) => ConverterTarget::MozcServer,
    }
}

/// プロセス名（`szExeFile`）が、`target` の変換サーバーかどうか。
#[must_use]
pub fn is_converter_process(target: ConverterTarget, name: &str) -> bool {
    let prefixes = match target {
        ConverterTarget::Gji => GJI_PREFIXES,
        ConverterTarget::MozcServer => MOZC_PREFIXES,
    };
    let lower = name.to_ascii_lowercase();
    prefixes
        .iter()
        .any(|p| lower.starts_with(&p.to_ascii_lowercase()))
}

/// 見つからなかったときのログ用に、探した接頭辞を返す。
#[must_use]
pub const fn searched_prefixes(target: ConverterTarget) -> &'static [&'static str] {
    match target {
        ConverterTarget::Gji => GJI_PREFIXES,
        ConverterTarget::MozcServer => MOZC_PREFIXES,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_tip_watches_gji_converter() {
        assert_eq!(
            converter_target(true, TipIdentity::Gji),
            ConverterTarget::Gji
        );
    }

    /// BUG-207: Mozc Readable は GJI 系（kind = GoogleJapaneseInput）だが identity は Gji でない。
    #[test]
    fn mozc_family_tip_watches_mozc_server() {
        assert_eq!(
            converter_target(true, TipIdentity::Other),
            ConverterTarget::MozcServer
        );
    }

    /// GJI 系でない TIP（MS-IME・ATOK 等）では従来どおり GJI を見る（挙動を変えない）。
    #[test]
    fn non_gji_family_keeps_watching_gji() {
        for id in [
            TipIdentity::Gji,
            TipIdentity::MsImeNative,
            TipIdentity::Other,
        ] {
            assert_eq!(converter_target(false, id), ConverterTarget::Gji);
        }
    }

    #[test]
    fn process_names_match_only_their_own_target() {
        assert!(is_converter_process(
            ConverterTarget::Gji,
            "GoogleIMEJaConverter.exe"
        ));
        assert!(is_converter_process(
            ConverterTarget::Gji,
            "googlejapaneseinputconverter.exe"
        ));
        assert!(is_converter_process(
            ConverterTarget::MozcServer,
            "mozc_server.exe"
        ));
        assert!(is_converter_process(
            ConverterTarget::MozcServer,
            "Mozc_Server.EXE"
        ));
        // 両方が動いていても、互いを拾わない。
        assert!(!is_converter_process(
            ConverterTarget::Gji,
            "mozc_server.exe"
        ));
        assert!(!is_converter_process(
            ConverterTarget::MozcServer,
            "GoogleIMEJaConverter.exe"
        ));
        // Renderer / CacheService / 別プロセスは対象外。
        assert!(!is_converter_process(
            ConverterTarget::Gji,
            "GoogleIMEJaRenderer.exe"
        ));
        assert!(!is_converter_process(
            ConverterTarget::MozcServer,
            "mozc_renderer.exe"
        ));
    }
}
