//! 真のモデル(Mealy機械)。実機のIMEを、観測できる `Status` と観測できない隠れ状態を持つ機械として表す。
//!
//! - 状態(`TrueState`)は `Status`(開閉・変換モード・入力中の有無)を持つ。**同じ `Status` を持つ複数の状態**があれば、
//!   それらが隠れ状態(例: 入力中でも「変換中」か否かで Esc の結果が違う)。
//! - 遷移は分岐(`Branch`)の列で、確率 1 の1本なら決定的、複数なら非決定。
//! - 入力キーは抽象ID(`KeyId`)。実機のVKとの対応は持たない。

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// 抽象的なキーID(修飾付きのキーも、呼び出し側が別のIDを割り当てて表す)。
///
/// `Serialize`/`Deserialize` は段階3([ADR-195](../../../docs/adr/195-keymap-learn-productization.md)
/// 「段階3: 学習結果の永続化」)の永続化フォーマット(`persist`モジュール)が、この型をそのまま
/// 使い回すために付与している(パラレルな永続化専用型を新設しない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct KeyId(pub u16);

/// 観測できる状態(status message)。変換モードは抽象的な番号で持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Status {
    pub open: bool,
    pub mode: u8,
    pub composing: bool,
}

/// 入力中の文字列の行方(入力欄の変化から観測する)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Disposition {
    None,
    Kept,
    Discarded,
    Committed,
}

/// 1回の押下の観測結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Outcome {
    pub status: Status,
    pub disp: Disposition,
}

/// 遷移の1分岐。
#[derive(Debug, Clone, Copy)]
pub struct Branch {
    pub p: f64,
    pub next: usize,
    pub disp: Disposition,
}

/// 真の状態。`trans[key_idx]` がそのキーを押したときの分岐。
#[derive(Debug, Clone)]
pub struct TrueState {
    pub status: Status,
    pub trans: Vec<Vec<Branch>>,
}

/// セルの真の分類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellTruth {
    /// どの隠れ状態から押しても同じ1つの結果。
    Deterministic(Outcome),
    /// 隠れ状態ごとには決定的だが、隠れ状態によって結果が違う(履歴依存)。
    HistoryDependent,
    /// 同じ状態から押しても結果が確率的に変わる。
    NonDeterministic,
}

/// Mealy機械。
#[derive(Debug, Clone)]
pub struct Machine {
    pub states: Vec<TrueState>,
    pub keys: Vec<KeyId>,
    pub initial: usize,
    /// 履歴依存が疑われるキー(の添字)。S6/S7が使う「疑いのある部分アルファベット」。
    pub history_suspects: Vec<usize>,
}

impl Machine {
    /// 観測表(`Table`)が区別できる`Status`の数。`Table`は`Status`単位でセルを持つため、
    /// 同じ`Status`にまとまる隠れ状態(入力中の段階など)は1つに数える。進捗の分母
    /// (`distinct_status_count() × キー数`)に使う（`states.len()`で数えると約2倍になる）。
    pub fn distinct_status_count(&self) -> usize {
        self.states
            .iter()
            .map(|s| s.status)
            .collect::<std::collections::HashSet<_>>()
            .len()
    }

    /// 初期状態から到達できる状態。
    pub fn reachable(&self) -> Vec<bool> {
        let mut seen = vec![false; self.states.len()];
        let mut q = VecDeque::new();
        seen[self.initial] = true;
        q.push_back(self.initial);
        while let Some(s) = q.pop_front() {
            for branches in &self.states[s].trans {
                for b in branches {
                    if !seen[b.next] {
                        seen[b.next] = true;
                        q.push_back(b.next);
                    }
                }
            }
        }
        seen
    }

    /// 到達できる状態が持つ `Status`(重複なし、初出順)。
    pub fn statuses(&self) -> Vec<Status> {
        let reach = self.reachable();
        let mut out: Vec<Status> = Vec::new();
        for (i, s) in self.states.iter().enumerate() {
            if reach[i] && !out.contains(&s.status) {
                out.push(s.status);
            }
        }
        out
    }

    /// 状態 `state` からキー `key_idx` を押したときの結果の分布。
    pub fn outcomes(&self, state: usize, key_idx: usize) -> Vec<(f64, Outcome)> {
        self.states[state].trans[key_idx]
            .iter()
            .map(|b| {
                (
                    b.p,
                    Outcome {
                        status: self.states[b.next].status,
                        disp: b.disp,
                    },
                )
            })
            .collect()
    }

    /// セル `(status, key_idx)` の真の分類。そのstatusを持つ到達可能な状態が無ければ `None`。
    pub fn truth(&self, status: Status, key_idx: usize) -> Option<CellTruth> {
        let reach = self.reachable();
        let mut outcomes: Vec<Outcome> = Vec::new();
        let mut any = false;
        for (i, s) in self.states.iter().enumerate() {
            if !reach[i] || s.status != status {
                continue;
            }
            any = true;
            let branches = self.outcomes(i, key_idx);
            if branches.iter().any(|(p, _)| *p < 1.0 - 1e-9) {
                let mut distinct: Vec<Outcome> = Vec::new();
                for (_, o) in &branches {
                    if !distinct.contains(o) {
                        distinct.push(*o);
                    }
                }
                if distinct.len() > 1 {
                    return Some(CellTruth::NonDeterministic);
                }
            }
            for (_, o) in branches {
                if !outcomes.contains(&o) {
                    outcomes.push(o);
                }
            }
        }
        if !any {
            return None;
        }
        Some(if outcomes.len() == 1 {
            CellTruth::Deterministic(outcomes[0])
        } else {
            CellTruth::HistoryDependent
        })
    }

    /// 初期状態の `Status`。
    pub fn initial_status(&self) -> Status {
        self.states[self.initial].status
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(open: bool, mode: u8, composing: bool) -> Status {
        Status {
            open,
            mode,
            composing,
        }
    }

    /// 2状態が同じstatusを持ち(隠れ状態)、キー0の結果が違う小さな機械。
    fn tiny() -> Machine {
        let s_idle = st(true, 0, false);
        let s_comp = st(true, 0, true);
        let b = |next, disp| vec![Branch { p: 1.0, next, disp }];
        Machine {
            // 0:idle, 1:typing(composing), 2:converting(composing、statusは1と同じ)
            states: vec![
                TrueState {
                    status: s_idle,
                    trans: vec![b(1, Disposition::None), b(0, Disposition::None)],
                },
                TrueState {
                    status: s_comp,
                    trans: vec![b(2, Disposition::None), b(0, Disposition::Discarded)],
                },
                TrueState {
                    status: s_comp,
                    trans: vec![b(2, Disposition::None), b(1, Disposition::Kept)],
                },
            ],
            keys: vec![KeyId(0), KeyId(1)],
            initial: 0,
            history_suspects: vec![1],
        }
    }

    #[test]
    fn truth_detects_history_dependence_from_hidden_states() {
        let m = tiny();
        // キー1(Esc相当): typing→idle(破棄)、converting→typing(保持)。同じstatusで結果が違う。
        assert_eq!(
            m.truth(st(true, 0, true), 1),
            Some(CellTruth::HistoryDependent)
        );
        // キー0: idleからは typing。決定的。
        assert!(matches!(
            m.truth(st(true, 0, false), 0),
            Some(CellTruth::Deterministic(_))
        ));
    }

    #[test]
    fn statuses_are_distinct_and_reachable_only() {
        let m = tiny();
        assert_eq!(m.statuses().len(), 2);
    }
}
