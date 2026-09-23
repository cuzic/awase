//! 段階2: 自己検証(独立ランダムウォークでの信頼度算出)。
//!
//! [ADR-195](../../../docs/adr/195-keymap-learn-productization.md) 決定・段階2は、学習した表([`crate::table::Table`])を、
//! 学習に使っていない独立のキー列(ランダムウォーク)で採点し、一段予測の正答率と信頼度を算出する。
//! CI実機の格子で確立した手法(`score_walk.py`相当)の移植で、ATOKの実測(一致278・不一致1・表に無い19、99.6%)と
//! 同じ形の指標([`ScoreReport`])を返す。
//!
//! 「信頼度が低い」セル(ADR本文: 段階1の「誤りに強い分類」〈[`classify_robust`]〉で弾かれる、つまり
//! [`crate::table::Class::declared_not_det`] が真になるセル)は、学習をやり直すか、当該セルを予測しない。
//! やり直しは1回までで、[`RetryTracker`]がその上限を管理する。

use std::collections::HashMap;

use crate::model::{Outcome, Status};
use crate::table::{Class, Table};

/// 誤りに強い分類の既定閾値: 文脈内の少数派の観測数がこれ未満なら観測誤りとみなし、
/// 多数派の結果を採用する(単発の観測誤りだけでは非決定と宣言しない)。
pub const DEFAULT_MIN_MINORITY: usize = 2;

/// 誤りに強い分類: [`Table::class`] と同じ規則だが、文脈内で割れていても、
/// 少数派の観測数が `min_minority` 未満なら観測誤りとみなして無視する。
///
/// `min_minority == 1` を渡すと [`Table::class`] と完全に同じ結果になる
/// (どんな1件の食い違いも非決定の証拠として扱う、旧来の厳密一致)。
pub fn classify_robust(table: &Table, status: Status, key: usize, min_minority: usize) -> Class {
    let obs = table.observations(status, key);
    match obs.len() {
        0 => return Class::Unmeasured,
        1 => return Class::Single(obs[0].outcome),
        _ => {}
    }
    let first = obs[0].outcome;
    if obs.iter().all(|o| o.outcome == first) {
        return Class::Det(first);
    }

    let mut groups: HashMap<Option<usize>, Vec<Outcome>> = HashMap::new();
    for o in obs {
        groups.entry(o.ctx).or_default().push(o.outcome);
    }

    let mut any_inhomogeneous = false;
    let mut multi = 0;
    let mut effective: Vec<Outcome> = Vec::with_capacity(groups.len());
    for g in groups.values() {
        if g.len() >= 2 {
            multi += 1;
            // 多数派を選ぶ。同数なら先に現れたものを採用する(table.rs::majority()と同じ
            // 規則)。HashMap<Outcome, usize>のiterで選ぶと反復順がRandomStateに依存し、
            // 同数タイの場合に呼び出しごとに結果が変わってしまう(非決定的)ため、
            // Vecの出現順を線形走査する。
            let mut maj_outcome = g[0];
            let mut maj_count = 0usize;
            for o in g {
                let n = g.iter().filter(|x| *x == o).count();
                if n > maj_count {
                    maj_count = n;
                    maj_outcome = *o;
                }
            }
            let minority = g.len() - maj_count;
            if minority >= min_minority {
                any_inhomogeneous = true;
            }
            effective.push(maj_outcome);
        } else {
            effective.push(g[0]);
        }
    }

    if any_inhomogeneous {
        return Class::NonDet;
    }
    if effective.iter().all(|o| *o == effective[0]) {
        Class::Det(effective[0])
    } else if multi >= 1 && groups.len() >= 2 {
        Class::HistoryDep
    } else {
        Class::Conflict
    }
}

/// セル `(status, key)` の一段予測。誤りに強い分類で決定的とみなせた場合のみ `Some`。
pub fn predict(table: &Table, status: Status, key: usize, min_minority: usize) -> Option<Outcome> {
    match classify_robust(table, status, key, min_minority) {
        Class::Det(o) | Class::Single(o) => Some(o),
        Class::Unmeasured | Class::HistoryDep | Class::NonDet | Class::Conflict => None,
    }
}

/// 独立ランダムウォークで観測した1件。学習(`Table::record`)には使っていない観測。
#[derive(Debug, Clone, Copy)]
pub struct WalkObs {
    pub status: Status,
    pub key: usize,
    pub outcome: Outcome,
}

/// 採点結果。ATOKの実測(一致278・不一致1・表に無い19)と同じ形。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScoreReport {
    pub correct: usize,
    pub incorrect: usize,
    pub not_in_table: usize,
}

impl ScoreReport {
    pub const fn total(&self) -> usize {
        self.correct + self.incorrect + self.not_in_table
    }

    /// 正答率: 予測を持っていた観測のうち一致した割合(`correct / (correct + incorrect)`)。
    /// 予測が一件も無ければ `0.0`。
    pub fn accuracy(&self) -> f64 {
        let predicted = self.correct + self.incorrect;
        if predicted == 0 {
            0.0
        } else {
            self.correct as f64 / predicted as f64
        }
    }

    /// 信頼度: 全観測のうち、予測を持っていた(表に無いに縮退しなかった)割合。
    /// `docs/tasks/adr195-t4-runtime-loading.md` が言う「縮退率」の余事象。
    pub fn confidence(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            0.0
        } else {
            (self.correct + self.incorrect) as f64 / total as f64
        }
    }
}

/// 独立ランダムウォークの観測列で表を採点する。
///
/// ウォークは同じ`(status, key)`セルを何度も踏むことが多い(表全体のセル数より
/// ウォーク長の方が長いのが通常)ため、セルごとの予測を一度計算したらキャッシュして
/// 使い回す(`classify_robust`のグルーピング計算をウォーク長ぶん繰り返さない)。
pub fn score_walk(table: &Table, min_minority: usize, walk: &[WalkObs]) -> ScoreReport {
    let mut report = ScoreReport::default();
    let mut cache: HashMap<(Status, usize), Option<Outcome>> = HashMap::new();
    for w in walk {
        let prediction = *cache
            .entry((w.status, w.key))
            .or_insert_with(|| predict(table, w.status, w.key, min_minority));
        match prediction {
            Some(o) if o == w.outcome => report.correct += 1,
            Some(_) => report.incorrect += 1,
            None => report.not_in_table += 1,
        }
    }
    report
}

/// セル単位のやり直し回数を管理する。ADR-195段階2(round3 m-3)によりやり直しは1回まで。
#[derive(Debug, Clone, Default)]
pub struct RetryTracker {
    attempts: HashMap<(Status, usize), u8>,
}

impl RetryTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// このセルについて、もう一度学習をやり直してよいか(1回まで)を判定する。
    /// 許可した場合はやり直し済みとして記録し、以降は同じセルに対して `false` を返す。
    pub fn should_retry(&mut self, status: Status, key: usize) -> bool {
        let n = self.attempts.entry((status, key)).or_insert(0);
        if *n < 1 {
            *n += 1;
            true
        } else {
            false
        }
    }

    /// このセルが既にやり直し済みか。
    pub fn has_retried(&self, status: Status, key: usize) -> bool {
        self.attempts.get(&(status, key)).is_some_and(|&n| n >= 1)
    }
}

/// セル1件について、次に取るべき行動。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellDecision {
    /// この結果を予測として採用してよい。
    Predict(Outcome),
    /// 信頼度が低い(誤りに強い分類でも決定的と言えない)。まだやり直していないので再学習する。
    RetryLearning,
    /// やり直しても信頼度が低いままだった(またはやり直し済み)。予測しない側に倒す。
    GiveUp,
}

/// [`classify_robust`]の結果と[`RetryTracker`]を突き合わせ、そのセルへの次の行動を決める。
/// `Unmeasured`(未測定)はやり直し対象ではない(巡回が担保する範囲外)ため、常に `GiveUp` を返す。
pub fn decide_cell(
    table: &Table,
    status: Status,
    key: usize,
    min_minority: usize,
    retry: &mut RetryTracker,
) -> CellDecision {
    match classify_robust(table, status, key, min_minority) {
        Class::Det(o) | Class::Single(o) => CellDecision::Predict(o),
        Class::Unmeasured => CellDecision::GiveUp,
        Class::HistoryDep | Class::NonDet | Class::Conflict => {
            if retry.should_retry(status, key) {
                CellDecision::RetryLearning
            } else {
                CellDecision::GiveUp
            }
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
    fn classify_robust_with_min_minority_1_matches_strict_class() {
        let mut t = Table::new();
        let s = st(true);
        t.record(s, 1, Some(1), out(true));
        t.record(s, 1, Some(1), out(false));
        assert_eq!(t.class(s, 1), Class::NonDet);
        assert_eq!(classify_robust(&t, s, 1, 1), Class::NonDet);
    }

    #[test]
    fn classify_robust_ignores_a_single_stray_disagreement() {
        let mut t = Table::new();
        let s = st(true);
        // 同じ文脈で4回中3回はtrue、1回だけ観測誤りでfalse。
        t.record(s, 0, Some(1), out(true));
        t.record(s, 0, Some(1), out(true));
        t.record(s, 0, Some(1), out(true));
        t.record(s, 0, Some(1), out(false));

        // 厳密一致(旧来のclass()相当)は非決定と誤判定する。
        assert_eq!(t.class(s, 0), Class::NonDet);
        assert_eq!(classify_robust(&t, s, 0, 1), Class::NonDet);

        // 誤りに強い分類(既定閾値2)は、少数派1件を観測誤りとみなし多数派で決定的とする。
        assert_eq!(
            classify_robust(&t, s, 0, DEFAULT_MIN_MINORITY),
            Class::Det(out(true))
        );
    }

    #[test]
    fn classify_robust_tie_break_is_deterministic_across_many_calls() {
        let mut t = Table::new();
        let s = st(true);
        // 同じ文脈で1対1のタイ(多数派が一意に決まらない)。minority=1はDEFAULT_MIN_MINORITY(2)
        // 未満なので観測誤り扱いとなり決定的に倒れるが、その決定先(先に現れたtrue)が
        // 呼び出しごとにぶれてはならない(HashMap反復順に依存する実装だと再現しなかった回帰)。
        t.record(s, 0, Some(1), out(true));
        t.record(s, 0, Some(1), out(false));

        let first = classify_robust(&t, s, 0, DEFAULT_MIN_MINORITY);
        for _ in 0..500 {
            assert_eq!(classify_robust(&t, s, 0, DEFAULT_MIN_MINORITY), first);
        }
        assert_eq!(first, Class::Det(out(true)));
    }

    #[test]
    fn classify_robust_declares_nondet_when_minority_reaches_threshold() {
        let mut t = Table::new();
        let s = st(true);
        // 同じ文脈で3回true・2回false: 少数派2件は閾値2に達するので、本物の非決定として扱う。
        t.record(s, 0, Some(1), out(true));
        t.record(s, 0, Some(1), out(true));
        t.record(s, 0, Some(1), out(true));
        t.record(s, 0, Some(1), out(false));
        t.record(s, 0, Some(1), out(false));

        assert_eq!(
            classify_robust(&t, s, 0, DEFAULT_MIN_MINORITY),
            Class::NonDet
        );
    }

    #[test]
    fn score_walk_counts_correct_incorrect_and_not_in_table() {
        let mut t = Table::new();
        let open = st(true);
        let closed = st(false);
        // open×key0は決定的にtrueへ。closed×key1は未測定のまま(表に無い)。
        t.record(open, 0, None, out(true));
        t.record(open, 0, None, out(true));

        let walk = vec![
            WalkObs {
                status: open,
                key: 0,
                outcome: out(true),
            }, // 一致
            WalkObs {
                status: open,
                key: 0,
                outcome: out(true),
            }, // 一致
            WalkObs {
                status: open,
                key: 0,
                outcome: out(false),
            }, // 不一致
            WalkObs {
                status: closed,
                key: 1,
                outcome: out(false),
            }, // 表に無い
        ];

        let report = score_walk(&t, DEFAULT_MIN_MINORITY, &walk);
        assert_eq!(
            report,
            ScoreReport {
                correct: 2,
                incorrect: 1,
                not_in_table: 1,
            }
        );
        assert_eq!(report.total(), 4);
        assert!((report.accuracy() - (2.0 / 3.0)).abs() < 1e-9);
        assert!((report.confidence() - 0.75).abs() < 1e-9);
    }

    #[test]
    fn score_walk_matches_atok_measurement_shape() {
        // ADR-195本文が引用するATOK実測(一致278・不一致1・表に無い19、99.6%)と
        // 同じ形の指標になることを、縮小版の数値で固定する。
        let mut t = Table::new();
        let s = st(true);
        t.record(s, 0, None, out(true));
        t.record(s, 0, None, out(true));

        let mut walk = Vec::new();
        for _ in 0..278 {
            walk.push(WalkObs {
                status: s,
                key: 0,
                outcome: out(true),
            });
        }
        walk.push(WalkObs {
            status: s,
            key: 0,
            outcome: out(false),
        });
        for i in 0..19u16 {
            walk.push(WalkObs {
                status: st(false),
                key: usize::from(i) + 1,
                outcome: out(false),
            });
        }

        let report = score_walk(&t, DEFAULT_MIN_MINORITY, &walk);
        assert_eq!(report.correct, 278);
        assert_eq!(report.incorrect, 1);
        assert_eq!(report.not_in_table, 19);
        assert!((report.accuracy() - 0.996_415_77).abs() < 1e-6);
    }

    #[test]
    fn retry_tracker_allows_exactly_one_retry_per_cell() {
        let mut retry = RetryTracker::new();
        let s = st(true);
        assert!(!retry.has_retried(s, 0));
        assert!(retry.should_retry(s, 0));
        assert!(retry.has_retried(s, 0));
        // 2回目以降は拒否する(やり直しは1回まで)。
        assert!(!retry.should_retry(s, 0));
        assert!(!retry.should_retry(s, 0));

        // 別のセルは独立に1回まで許可される。
        assert!(retry.should_retry(s, 1));
    }

    #[test]
    fn decide_cell_retries_once_then_gives_up() {
        let mut t = Table::new();
        let s = st(true);
        // 少数派が閾値に達する本物の非決定セル。
        t.record(s, 0, Some(1), out(true));
        t.record(s, 0, Some(1), out(true));
        t.record(s, 0, Some(1), out(false));
        t.record(s, 0, Some(1), out(false));

        let mut retry = RetryTracker::new();
        assert_eq!(
            decide_cell(&t, s, 0, DEFAULT_MIN_MINORITY, &mut retry),
            CellDecision::RetryLearning
        );
        // やり直し後も信頼度が低いままなら、2回目はGiveUpに倒す。
        assert_eq!(
            decide_cell(&t, s, 0, DEFAULT_MIN_MINORITY, &mut retry),
            CellDecision::GiveUp
        );
    }

    #[test]
    fn decide_cell_predicts_deterministic_cells_without_retry() {
        let mut t = Table::new();
        let s = st(true);
        t.record(s, 0, None, out(true));
        t.record(s, 0, None, out(true));

        let mut retry = RetryTracker::new();
        assert_eq!(
            decide_cell(&t, s, 0, DEFAULT_MIN_MINORITY, &mut retry),
            CellDecision::Predict(out(true))
        );
        assert!(!retry.has_retried(s, 0));
    }

    #[test]
    fn decide_cell_gives_up_on_unmeasured_without_consuming_a_retry() {
        let t = Table::new();
        let s = st(true);
        let mut retry = RetryTracker::new();
        assert_eq!(
            decide_cell(&t, s, 5, DEFAULT_MIN_MINORITY, &mut retry),
            CellDecision::GiveUp
        );
        assert!(!retry.has_retried(s, 5));
    }
}
