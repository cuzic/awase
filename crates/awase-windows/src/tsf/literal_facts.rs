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
