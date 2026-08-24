//! literal-detect の判定結果を journal へ持ち上げるための純粋データ型。

use crate::state::event_origin::Generation;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum LiteralVerdict {
    CompositionConfirmed,
    SuspectedLiteral,
    StaleConfirm,
    VetoExpired,
    SessionSkip,
    PlanSkippedLiteral,
    AbortedNoVerdict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum DetectRoute {
    CheckNow,
    VisibleFencing,
    SessionFlag,
    PlanDecision,
    ProbeEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum DetectPath {
    PerVk,
    Word,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum DetectTarget {
    Chrome,
    Tsf,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct DetectEvidence {
    pub show_changed: bool,
    pub candidate_visible: bool,
    pub write_delta: u64,
    pub evidence_fresh: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct LiteralDetectFacts {
    pub verdict: LiteralVerdict,
    pub route: DetectRoute,
    pub path: DetectPath,
    pub target: DetectTarget,
    pub vk: Option<u16>,
    pub idx: u16,
    pub last_idx: u16,
    pub evidence: DetectEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LiteralDetectRecord {
    pub cold_seq: Generation,
    pub facts: LiteralDetectFacts,
    pub consecutive_before: u32,
    pub gave_up: bool,
    pub backs: usize,
    pub escape_composition: bool,
    pub session_marked: bool,
    /// BUG-74/ADR-100 決定3 案L: `RawTsfLiteralRecovery`（初回疑い・give-up 双方）で
    /// 送信対象だった romaji。`None` はこの verdict が romaji を持たない（`Composition
    /// Confirmed`/`LiteralDetectNote`/`PlanSkippedLiteral`/`AbortedNoVerdict`）ことを表す
    /// — 空文字列との混同（「記録し忘れ」なのか「そもそも romaji を持たない verdict」
    /// なのか区別できなくなる）を避けるため、`String::new()` ではなく `Option` にする。
    ///
    /// give-up（`gave_up=true`）で romaji が失われる（backspace のみ、再送なし）場合
    /// でも、この記録には**送信予定だった元の romaji**を残す。ADR-100 決定3 が
    /// 「give-up 分岐に reinit 完了確認後の retry を追加する」提案2 を却下した代わりに
    /// 採用した対策（完了通知経路が存在しない・focus 世代照合が未整備 (F6) 等、
    /// 却下理由の詳細は ADR-100 参照）。次に同種の文字消失が報告されたとき、
    /// journal からどの romaji が失われたかを機械可読に復元できるようにする。
    pub romaji: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum LiteralDetectTraceItem {
    VkSent {
        cold_seq: u64,
        vk: u16,
        idx: u16,
        last_idx: u16,
        target: DetectTarget,
    },
    Verdict(LiteralDetectRecord),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct LiteralDetectTrace(pub(crate) Vec<LiteralDetectTraceItem>);

/// `StaleConfirm`（per-VK confirm、先頭 VK＝`failed_idx==0`）の回収で再送する
/// romaji を決める純関数（BUG-75）。
///
/// `StaleConfirm` は「confirm 根拠が古い」ことの検出であって「literal である」
/// 証拠ではない（BUG-33 追補4）。先頭 VK（`failed_idx==0`）の `StaleConfirm` は
/// `per_vk_recovery_params(true, 0)` が `backs=0, escape_composition=false` を
/// 返す唯一のケースで、composition を ESC せず romaji 全体を再送していた。しかし
/// このケースは「候補ウィンドウが既に可視」（`VisibleFencing`）等、GJI が実際に
/// 受理し先頭 VK が着弾している場合を含み、全体再送は着弾済みの先頭文字を
/// 重複させる（実機報告 `report_id=01M0S4S6R4C1YJ581YJ9ZGAXXD`、「つかって」→
/// 「っつかって」）。先頭 VK を除いた suffix だけを再送することで重複を避ける。
///
/// **単一 VK のローマ字（`last_idx==0`）は対象外**とし、全体（＝そのまま）を返す。
/// suffix を取ると必ず空文字列になり、本当に literal 化していた場合（GJI が
/// 実は受理していなかった場合）に文字が痕跡なく完全に失われる—BUG-74 が
/// 「痕跡なく完全に失われる」ことを理由に ADR-101 まで作って直した症状と
/// 同型の退行になるため。
///
/// `failed_idx > 0` の `StaleConfirm` は `per_vk_recovery_params` が
/// `escape_composition=true` を返し、呼び出し元が composition を ESC で破棄
/// するため、先行 VK は残らない。この場合は変更前と同じ全体再送のままでよい
/// （ESC 後は「着弾済み prefix」という概念自体が存在しない）。
#[must_use]
pub(crate) fn stale_confirm_resend_romaji(
    romaji: &str,
    failed_idx: usize,
    last_idx: usize,
) -> String {
    if failed_idx == 0 && last_idx > 0 {
        romaji.chars().skip(1).collect()
    } else {
        romaji.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::stale_confirm_resend_romaji;

    // crux: BUG-75 の報告シナリオそのもの。"tu" の先頭 'T' が既に着弾済みで
    // StaleConfirm になったケースは、suffix "u" だけを再送すべき。
    #[test]
    fn stale_confirm_resend_romaji_first_vk_multi_char_returns_suffix() {
        assert_eq!(stale_confirm_resend_romaji("tu", 0, 1), "u");
        assert_eq!(stale_confirm_resend_romaji("ka", 0, 1), "a");
        assert_eq!(stale_confirm_resend_romaji("ltu", 0, 2), "tu");
    }

    // BUG-74 退行防止: 単一 VK のローマ字は suffix を取ると空文字列になり
    // 文字が痕跡なく失われるため、対象外として全体（そのまま）を返す。
    #[test]
    fn stale_confirm_resend_romaji_single_vk_word_returns_whole_unchanged() {
        assert_eq!(stale_confirm_resend_romaji("a", 0, 0), "a");
        assert_eq!(stale_confirm_resend_romaji("n", 0, 0), "n");
    }

    // failed_idx > 0 は escape_composition=true で composition が ESC される
    // ため「着弾済み prefix」という概念が無い。変更前と同じ全体再送のまま。
    #[test]
    fn stale_confirm_resend_romaji_later_vk_returns_whole_unchanged() {
        assert_eq!(stale_confirm_resend_romaji("ltu", 1, 2), "ltu");
        assert_eq!(stale_confirm_resend_romaji("ka", 1, 1), "ka");
    }
}
