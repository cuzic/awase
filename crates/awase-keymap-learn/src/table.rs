//! 観測表: セル `(status, キー)` ごとの観測の列と、その分類。

use std::collections::{HashMap, HashSet};

use crate::model::{Outcome, Status};

/// 1回の観測。`ctx` は直前に押したキーの添字(リセット直後などは `None`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Obs {
    pub ctx: Option<usize>,
    pub outcome: Outcome,
}

/// 観測から見たセルの分類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Unmeasured,
    /// 1回しか測っていない(決定的かどうかは言えない)。
    Single(Outcome),
    /// 複数回、全て同じ結果。
    Det(Outcome),
    /// 直前のキー(文脈)ごとには一貫するが、文脈によって結果が違う。
    HistoryDep,
    /// 同じ文脈で測っても結果が割れる。
    NonDet,
    /// 結果が割れたが、文脈ごとの観測が足りず、履歴依存か非決定か決められない。
    Conflict,
}

impl Class {
    /// 「決定的とは言えない」と宣言したか。
    pub const fn declared_not_det(self) -> bool {
        matches!(self, Self::HistoryDep | Self::NonDet | Self::Conflict)
    }
}

/// 観測表。
#[derive(Debug, Clone, Default)]
pub struct Table {
    cells: HashMap<(Status, usize), Vec<Obs>>,
    covered1: usize,
    covered2: usize,
}

impl Table {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, before: Status, key: usize, ctx: Option<usize>, outcome: Outcome) {
        let v = self.cells.entry((before, key)).or_default();
        v.push(Obs { ctx, outcome });
        match v.len() {
            1 => self.covered1 += 1,
            2 => self.covered2 += 1,
            _ => {}
        }
    }

    pub fn count(&self, status: Status, key: usize) -> usize {
        self.cells.get(&(status, key)).map_or(0, Vec::len)
    }

    /// 文脈 `ctx` での観測回数。
    pub fn count_ctx(&self, status: Status, key: usize, ctx: Option<usize>) -> usize {
        self.cells
            .get(&(status, key))
            .map_or(0, |v| v.iter().filter(|o| o.ctx == ctx).count())
    }

    pub fn distinct_ctx(&self, status: Status, key: usize) -> usize {
        self.cells
            .get(&(status, key))
            .map_or(0, |v| v.iter().map(|o| o.ctx).collect::<HashSet<_>>().len())
    }

    pub const fn covered1(&self) -> usize {
        self.covered1
    }

    pub const fn covered2(&self) -> usize {
        self.covered2
    }

    pub fn observations(&self, status: Status, key: usize) -> &[Obs] {
        self.cells.get(&(status, key)).map_or(&[], Vec::as_slice)
    }

    pub fn cells(&self) -> impl Iterator<Item = (&(Status, usize), &Vec<Obs>)> {
        self.cells.iter()
    }

    /// 多数派の結果(同数なら先に現れたもの)。
    pub fn majority(&self, status: Status, key: usize) -> Option<Outcome> {
        let v = self.cells.get(&(status, key))?;
        let mut best: Option<(Outcome, usize)> = None;
        for o in v {
            let n = v.iter().filter(|x| x.outcome == o.outcome).count();
            if best.is_none_or(|(_, bn)| n > bn) {
                best = Some((o.outcome, n));
            }
        }
        best.map(|(o, _)| o)
    }

    pub fn class(&self, status: Status, key: usize) -> Class {
        let obs = self.observations(status, key);
        match obs.len() {
            0 => return Class::Unmeasured,
            1 => return Class::Single(obs[0].outcome),
            _ => {}
        }
        let first = obs[0].outcome;
        if obs.iter().all(|o| o.outcome == first) {
            return Class::Det(first);
        }
        // 文脈ごとに分ける。
        let mut groups: HashMap<Option<usize>, Vec<Outcome>> = HashMap::new();
        for o in obs {
            groups.entry(o.ctx).or_default().push(o.outcome);
        }
        let mut any_inhomogeneous = false;
        let mut multi = 0;
        for g in groups.values() {
            if g.len() >= 2 {
                multi += 1;
                if g.iter().any(|x| *x != g[0]) {
                    any_inhomogeneous = true;
                }
            }
        }
        if any_inhomogeneous {
            Class::NonDet
        } else if multi >= 1 && groups.len() >= 2 {
            // 各文脈は一貫し、文脈間で違う。
            Class::HistoryDep
        } else {
            Class::Conflict
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Disposition;

    fn st(o: bool) -> Status {
        Status {
            open: o,
            mode: 0,
            composing: false,
        }
    }

    fn out(o: bool) -> Outcome {
        Outcome {
            status: st(o),
            disp: Disposition::None,
        }
    }

    #[test]
    fn classifies_det_history_and_nondet() {
        let mut t = Table::new();
        let s = st(true);
        assert_eq!(t.class(s, 0), Class::Unmeasured);
        t.record(s, 0, Some(1), out(true));
        assert_eq!(t.class(s, 0), Class::Single(out(true)));
        t.record(s, 0, Some(1), out(true));
        assert_eq!(t.class(s, 0), Class::Det(out(true)));
        // 文脈2では別の結果が一貫して出る → 履歴依存。
        t.record(s, 0, Some(2), out(false));
        t.record(s, 0, Some(2), out(false));
        assert_eq!(t.class(s, 0), Class::HistoryDep);
        // 同じ文脈で割れる → 非決定。
        t.record(s, 1, Some(1), out(true));
        t.record(s, 1, Some(1), out(false));
        assert_eq!(t.class(s, 1), Class::NonDet);
    }

    #[test]
    fn conflict_when_only_singletons_disagree() {
        let mut t = Table::new();
        let s = st(true);
        t.record(s, 0, Some(1), out(true));
        t.record(s, 0, Some(2), out(false));
        assert_eq!(t.class(s, 0), Class::Conflict);
        assert!(t.class(s, 0).declared_not_det());
    }

    #[test]
    fn coverage_counters_track_first_and_second_observation() {
        let mut t = Table::new();
        let s = st(true);
        t.record(s, 0, None, out(true));
        assert_eq!((t.covered1(), t.covered2()), (1, 0));
        t.record(s, 0, None, out(true));
        assert_eq!((t.covered1(), t.covered2()), (1, 1));
    }
}
