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
    NoEvidence,
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
        (true, TipIdentity::Other) => ConverterTarget::NoEvidence,
        (true, TipIdentity::Gji | TipIdentity::MsImeNative) | (false, _) => ConverterTarget::Gji,
    }
}

/// 証拠の切り替え（`Gji` ⇄ `NoEvidence`）のデバウンス。
///
/// 外す向き（`NoEvidence`）は安全側（安全網を外すだけで誤判定は増えない）なので 1 回の観測で即時に切り替え、
/// **同時に候補を消す**（消さないと、連続していない 2 回の `Gji` の観測で接続してしまう）。付ける向き（`Gji`）だけ
/// 2 回連続の観測を要求する（単発のフリップで監視を往復しない）。
#[derive(Debug, Default)]
pub struct TargetSwitch {
    candidate: Option<ConverterTarget>,
}

impl TargetSwitch {
    #[must_use]
    pub const fn new() -> Self {
        Self { candidate: None }
    }

    /// 新しい観測を投入する。`current` を変えるべきなら新しい値を返す。
    pub fn observe(
        &mut self,
        observed: ConverterTarget,
        current: ConverterTarget,
    ) -> Option<ConverterTarget> {
        if observed == current {
            self.candidate = None;
            return None;
        }
        if observed == ConverterTarget::NoEvidence {
            self.candidate = None;
            return Some(observed);
        }
        if self.candidate == Some(observed) {
            self.candidate = None;
            Some(observed)
        } else {
            self.candidate = Some(observed);
            None
        }
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
            ConverterTarget::NoEvidence
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
    fn no_evidence_is_applied_immediately() {
        let mut sw = TargetSwitch::new();
        assert_eq!(
            sw.observe(ConverterTarget::NoEvidence, ConverterTarget::Gji),
            Some(ConverterTarget::NoEvidence)
        );
    }

    #[test]
    fn gji_needs_two_consecutive_observations() {
        let mut sw = TargetSwitch::new();
        assert_eq!(
            sw.observe(ConverterTarget::Gji, ConverterTarget::NoEvidence),
            None
        );
        assert_eq!(
            sw.observe(ConverterTarget::Gji, ConverterTarget::NoEvidence),
            Some(ConverterTarget::Gji)
        );
    }

    /// 連続していない 2 回の `Gji` の観測（間に `NoEvidence`）で接続しない（候補を消す）。
    #[test]
    fn non_consecutive_gji_observations_do_not_attach() {
        let mut sw = TargetSwitch::new();
        // 最初は Mozc 系（current = NoEvidence）。Gji を 1 回観測 → 候補。
        assert_eq!(
            sw.observe(ConverterTarget::Gji, ConverterTarget::NoEvidence),
            None
        );
        // 間に NoEvidence（current と同じ）→ 候補が消える。
        assert_eq!(
            sw.observe(ConverterTarget::NoEvidence, ConverterTarget::NoEvidence),
            None
        );
        // 再び Gji を 1 回観測 → まだ候補（接続しない）。
        assert_eq!(
            sw.observe(ConverterTarget::Gji, ConverterTarget::NoEvidence),
            None
        );
    }

    /// current が Gji のとき、NoEvidence → Gji のフリップは、NoEvidence で即時に切り替わった後、
    /// 次の Gji 1 回では戻らない。
    #[test]
    fn single_flip_back_does_not_reattach() {
        let mut sw = TargetSwitch::new();
        assert_eq!(
            sw.observe(ConverterTarget::NoEvidence, ConverterTarget::Gji),
            Some(ConverterTarget::NoEvidence)
        );
        assert_eq!(
            sw.observe(ConverterTarget::Gji, ConverterTarget::NoEvidence),
            None
        );
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
