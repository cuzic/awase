//! Pure policy helpers for journal lane classification, byte-budget selection,
//! and diagnostic suppression decisions.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LaneKind {
    State,
    Timing,
    Actuation,
    KeyInput,
}

#[must_use]
pub const fn lane_capacity(lane: LaneKind) -> usize {
    match lane {
        LaneKind::State => 1024,
        LaneKind::Timing | LaneKind::Actuation | LaneKind::KeyInput => 512,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetItem {
    pub seq: u64,
    pub lane: LaneKind,
    pub bytes: usize,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProbeTickFacts {
    pub state_changed: bool,
    pub needs_composition_reset: bool,
    pub has_gji_response: bool,
    pub learned_tsf: bool,
    pub completed: bool,
    pub terminal_timer: bool,
    pub is_first_tick: bool,
}

#[must_use]
pub const fn probe_tick_is_notable(f: ProbeTickFacts) -> bool {
    f.state_changed
        || f.needs_composition_reset
        || f.has_gji_response
        || f.learned_tsf
        || f.completed
        || f.terminal_timer
        || f.is_first_tick
}

#[must_use]
pub fn literal_detect_is_notable(record: &crate::tsf::literal_facts::LiteralDetectRecord) -> bool {
    use crate::tsf::literal_facts::LiteralVerdict;
    match record.facts.verdict {
        LiteralVerdict::SuspectedLiteral
        | LiteralVerdict::StaleConfirm
        | LiteralVerdict::VetoExpired
        | LiteralVerdict::PlanSkippedLiteral
        | LiteralVerdict::AbortedNoVerdict => true,
        LiteralVerdict::CompositionConfirmed if record.session_marked => true,
        LiteralVerdict::CompositionConfirmed => record.consecutive_before > 0 || record.gave_up,
        LiteralVerdict::SessionSkip => record.consecutive_before > 0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferredRecoveryFlushFacts {
    Flushed { vk_count: usize },
    DiscardedStale,
    SkippedWhilePolling,
}

#[must_use]
pub const fn deferred_recovery_flush_is_notable(f: DeferredRecoveryFlushFacts) -> bool {
    match f {
        DeferredRecoveryFlushFacts::Flushed { vk_count } => vk_count > 0,
        DeferredRecoveryFlushFacts::DiscardedStale
        | DeferredRecoveryFlushFacts::SkippedWhilePolling => true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderViolation {
    pub index: usize,
    pub previous: u64,
    pub current: u64,
}

/// `tokens` を先頭から走査し、単調増加が破れた最初の箇所を返す。
///
/// `&[u64]` ではなく `impl IntoIterator<Item = u64>` を取る（2026-09-03
/// code review指摘で変更）: 呼び出し元（`output/mod.rs::
/// flush_pending_deferred_vks`）は元々 `DeferredVk` の列から `order_token`
/// だけを `Vec<u64>` へ collect してから渡していたが、これは
/// flush のたびに（違反が無い共通ケースでも）ヒープ確保が発生していた。
/// イテレータを直接受けることで、呼び出し元は `vks.iter().map(|vk|
/// vk.order_token)` をそのまま渡せ、中間 `Vec` を経由しない。
#[must_use]
pub fn order_violation(tokens: impl IntoIterator<Item = u64>) -> Option<OrderViolation> {
    let mut iter = tokens.into_iter().enumerate();
    let (_, mut previous) = iter.next()?;
    for (index, current) in iter {
        if current <= previous {
            return Some(OrderViolation {
                index,
                previous,
                current,
            });
        }
        previous = current;
    }
    None
}

const RESERVED_PERCENT: [(LaneKind, usize); 4] = [
    (LaneKind::Timing, 35),
    (LaneKind::State, 30),
    (LaneKind::Actuation, 20),
    (LaneKind::KeyInput, 15),
];

#[must_use]
pub fn select_tail_within_budget(items: &[BudgetItem], max_bytes: usize) -> Vec<usize> {
    if max_bytes < 2 {
        return Vec::new();
    }
    let payload_budget = max_bytes - 2;
    let mut selected = vec![false; items.len()];
    let mut used = 0usize;

    for (lane, percent) in RESERVED_PERCENT {
        let lane_budget = payload_budget.saturating_mul(percent) / 100;
        let mut lane_used = 0usize;
        let mut indexes: Vec<usize> = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| (item.lane == lane).then_some(index))
            .collect();
        indexes.sort_by_key(|&index| std::cmp::Reverse(items[index].seq));
        for index in indexes {
            let cost = item_cost(items[index].bytes, used + lane_used > 0);
            if lane_used + cost <= lane_budget && used + cost <= payload_budget {
                selected[index] = true;
                lane_used += cost;
            }
        }
        used += lane_used;
    }

    let mut remaining: Vec<usize> = items
        .iter()
        .enumerate()
        .filter_map(|(index, _)| (!selected[index]).then_some(index))
        .collect();
    remaining.sort_by_key(|&index| {
        (
            lane_priority(items[index].lane),
            std::cmp::Reverse(items[index].seq),
        )
    });
    for index in remaining {
        let cost = item_cost(items[index].bytes, used > 0);
        if used + cost <= payload_budget {
            selected[index] = true;
            used += cost;
        }
    }

    let mut indexes: Vec<usize> = selected
        .into_iter()
        .enumerate()
        .filter_map(|(index, is_selected)| is_selected.then_some(index))
        .collect();
    indexes.sort_by_key(|&index| items[index].seq);
    indexes
}

const fn lane_priority(lane: LaneKind) -> usize {
    match lane {
        LaneKind::Timing => 0,
        LaneKind::State => 1,
        LaneKind::Actuation => 2,
        LaneKind::KeyInput => 3,
    }
}

const fn item_cost(bytes: usize, needs_comma: bool) -> usize {
    if needs_comma {
        bytes + 1
    } else {
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_tail_prefers_newer_entries_with_valid_array_overhead() {
        let items = [
            BudgetItem {
                seq: 0,
                lane: LaneKind::State,
                bytes: 10,
            },
            BudgetItem {
                seq: 1,
                lane: LaneKind::State,
                bytes: 10,
            },
            BudgetItem {
                seq: 2,
                lane: LaneKind::State,
                bytes: 10,
            },
        ];
        assert_eq!(select_tail_within_budget(&items, 24), vec![1, 2]);
    }

    #[test]
    fn select_tail_uses_reserved_lanes_before_leftover() {
        let items = [
            BudgetItem {
                seq: 10,
                lane: LaneKind::KeyInput,
                bytes: 10,
            },
            BudgetItem {
                seq: 11,
                lane: LaneKind::Timing,
                bytes: 10,
            },
            BudgetItem {
                seq: 12,
                lane: LaneKind::State,
                bytes: 10,
            },
        ];
        let selected = select_tail_within_budget(&items, 24);
        assert!(selected.contains(&1));
        assert!(selected.contains(&2));
    }

    #[test]
    fn probe_tick_is_notable_for_each_fact() {
        assert!(!probe_tick_is_notable(ProbeTickFacts::default()));
        assert!(probe_tick_is_notable(ProbeTickFacts {
            state_changed: true,
            ..ProbeTickFacts::default()
        }));
        assert!(probe_tick_is_notable(ProbeTickFacts {
            needs_composition_reset: true,
            ..ProbeTickFacts::default()
        }));
        assert!(probe_tick_is_notable(ProbeTickFacts {
            has_gji_response: true,
            ..ProbeTickFacts::default()
        }));
        assert!(probe_tick_is_notable(ProbeTickFacts {
            learned_tsf: true,
            ..ProbeTickFacts::default()
        }));
        assert!(probe_tick_is_notable(ProbeTickFacts {
            completed: true,
            ..ProbeTickFacts::default()
        }));
        assert!(probe_tick_is_notable(ProbeTickFacts {
            terminal_timer: true,
            ..ProbeTickFacts::default()
        }));
        assert!(probe_tick_is_notable(ProbeTickFacts {
            is_first_tick: true,
            ..ProbeTickFacts::default()
        }));
    }

    #[test]
    fn deferred_recovery_flush_is_notable_for_informative_outcomes() {
        use DeferredRecoveryFlushFacts::{DiscardedStale, Flushed, SkippedWhilePolling};

        let cases = [
            (Flushed { vk_count: 0 }, false),
            (Flushed { vk_count: 1 }, true),
            (Flushed { vk_count: 3 }, true),
            (DiscardedStale, true),
            (SkippedWhilePolling, true),
        ];

        for (facts, expected) in cases {
            assert_eq!(
                deferred_recovery_flush_is_notable(facts),
                expected,
                "{facts:?}"
            );
        }
    }

    fn literal_record(
        verdict: crate::tsf::literal_facts::LiteralVerdict,
    ) -> crate::tsf::literal_facts::LiteralDetectRecord {
        use crate::tsf::literal_facts::{
            DetectEvidence, DetectPath, DetectRoute, DetectTarget, LiteralDetectFacts,
        };
        crate::tsf::literal_facts::LiteralDetectRecord {
            cold_seq: crate::state::event_origin::Generation::INITIAL,
            facts: LiteralDetectFacts {
                verdict,
                route: DetectRoute::CheckNow,
                path: DetectPath::Word,
                target: DetectTarget::Tsf,
                vk: None,
                idx: 0,
                last_idx: 0,
                evidence: DetectEvidence::default(),
            },
            consecutive_before: 0,
            gave_up: false,
            backs: 0,
            escape_composition: false,
            session_marked: false,
            romaji: None,
        }
    }

    #[test]
    fn literal_detect_is_notable_for_failure_and_absence_verdicts() {
        use crate::tsf::literal_facts::LiteralVerdict::{
            AbortedNoVerdict, PlanSkippedLiteral, StaleConfirm, SuspectedLiteral, VetoExpired,
        };
        for verdict in [
            SuspectedLiteral,
            StaleConfirm,
            VetoExpired,
            PlanSkippedLiteral,
            AbortedNoVerdict,
        ] {
            assert!(literal_detect_is_notable(&literal_record(verdict)));
        }
    }

    #[test]
    fn literal_detect_suppresses_healthy_confirm_repetition() {
        use crate::tsf::literal_facts::LiteralVerdict;
        assert!(!literal_detect_is_notable(&literal_record(
            LiteralVerdict::CompositionConfirmed
        )));

        let mut session_marked = literal_record(LiteralVerdict::CompositionConfirmed);
        session_marked.session_marked = true;
        assert!(literal_detect_is_notable(&session_marked));

        let mut recovering = literal_record(LiteralVerdict::CompositionConfirmed);
        recovering.consecutive_before = 1;
        assert!(literal_detect_is_notable(&recovering));

        let mut gave_up = literal_record(LiteralVerdict::CompositionConfirmed);
        gave_up.gave_up = true;
        assert!(literal_detect_is_notable(&gave_up));
    }

    #[test]
    fn literal_detect_session_skip_only_records_during_failure_chain() {
        use crate::tsf::literal_facts::LiteralVerdict;
        assert!(!literal_detect_is_notable(&literal_record(
            LiteralVerdict::SessionSkip
        )));

        let mut recovering = literal_record(LiteralVerdict::SessionSkip);
        recovering.consecutive_before = 1;
        assert!(literal_detect_is_notable(&recovering));
    }

    #[test]
    fn order_violation_accepts_empty_singleton_and_increasing_sequences() {
        for tokens in [vec![], vec![1], vec![1, 2, 3, 4]] {
            assert_eq!(order_violation(tokens), None);
        }
    }

    #[test]
    fn order_violation_detects_non_increasing_sequences() {
        let cases = [
            (
                vec![1, 2, 4, 3],
                OrderViolation {
                    index: 3,
                    previous: 4,
                    current: 3,
                },
            ),
            (
                vec![1, 3, 2],
                OrderViolation {
                    index: 2,
                    previous: 3,
                    current: 2,
                },
            ),
            (
                vec![1, 2, 2],
                OrderViolation {
                    index: 2,
                    previous: 2,
                    current: 2,
                },
            ),
        ];

        for (tokens, expected) in cases {
            assert_eq!(order_violation(tokens), Some(expected));
        }
    }
}
