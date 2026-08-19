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
}
