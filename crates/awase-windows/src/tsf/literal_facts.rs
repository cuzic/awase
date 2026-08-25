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
