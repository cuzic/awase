//! 内蔵表と食い違ったセルの再測定([ADR-196](../../../docs/adr/196-keymap-learn-truth-priority.md)
//! 決定1b項目7〜8、[ADR196-T2](../../../docs/tasks/adr196-t2-mismatch-adjudication.md)の
//! 再測定オーケストレーション)。
//!
//! 学習本体の巡回(`strategy::run`)とは**別のセットアップ経路**で対象の`(status, key)`へ
//! 到達し直す: リセット後にランダムなキー列を押して目的のstatusに出会うまで歩き、出会えた
//! 時点で対象キーを押して結果を見る。学習時と同じ経路を再現してしまうと、経路依存の
//! 誤観測（学習時だけ起きた外部書き込み等）を同じ形で再現して「再測定で一致した」ことに
//! なり、再測定の意味が無くなるため。
//!
//! 本モジュールは[`ImeDriver`]だけに依存する純粋ロジックで、`SimIme`でLinux上でテストできる。
//! 実機の`RealImeDriver`への結線は`awase-keymap-learn-win`側が行う。

use crate::exec::{Executor, ImeDriver};
use crate::judgement::{CellReconciliation, ReconciliationSummary};
use crate::model::{Outcome, Status};
use crate::rng::Rng;

/// 再測定の対象1セル（内蔵表と食い違った学習セル）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MismatchedTarget {
    pub status: Status,
    /// `KEYS`の添字（実VKではない、`Executor::press`と同じ空間）。
    pub key: usize,
    /// 学習時に予測として持っていた結果（再測定がこれと一致すれば採用）。
    pub learned: Outcome,
}

/// 再測定のパラメータ。
#[derive(Debug, Clone, Copy)]
pub struct RemeasureParams {
    /// 1セルあたり、目的のstatusへ到達しようとして押してよいセットアップ押下数の上限。
    pub max_setup_presses: usize,
    /// セットアップ押下がこの回数続けて目的のstatusに出会えなかったらリセットして歩き直す。
    pub reset_every: usize,
    /// キー空間の大きさ（`KEYS.len()`）。
    pub key_count: usize,
}

/// 1セルの再測定結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemeasureResult {
    /// 目的のstatusから対象キーを押した結果が学習値と一致した。
    Reproduced,
    /// 到達して測ったが、学習値と一致しなかった。
    NotReproduced,
    /// 上限内に目的のstatusへ到達できない、または測定が汚染され続けた、
    /// あるいはドライバが中止を求めた。確認できなかったセルは
    /// **再現されなかったものとして扱う**（確認できない値を採用しない安全側）。
    Unconfirmed,
}

/// 対象セルを1つ再測定する（`recording=false`で呼ぶこと——表を汚さない）。
pub fn remeasure_cell<D: ImeDriver>(
    exec: &mut Executor<D>,
    target: MismatchedTarget,
    rng: &mut Rng,
    params: &RemeasureParams,
) -> RemeasureResult {
    exec.reset();
    let mut since_reset = 0usize;
    for _ in 0..params.max_setup_presses {
        if exec.driver.should_abort() {
            return RemeasureResult::Unconfirmed;
        }
        if exec.current() == Some(target.status) {
            match exec.press(target.key) {
                Some(info) if !info.contaminated && info.before == target.status => {
                    return if info.outcome == target.learned {
                        RemeasureResult::Reproduced
                    } else {
                        RemeasureResult::NotReproduced
                    };
                }
                // 押下が届かない・汚染された・押下直前の読み直しでstatusがずれていた:
                // 測れていないので歩き直す。
                _ => {}
            }
            since_reset += 1;
        } else {
            exec.press(rng.below(params.key_count));
            since_reset += 1;
        }
        if since_reset >= params.reset_every || exec.should_reset() {
            exec.reset();
            since_reset = 0;
        }
    }
    RemeasureResult::Unconfirmed
}

/// [`reconcile_with_bundled`]の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconciliation {
    pub summary: ReconciliationSummary,
    /// 再測定で再現できなかった（確認できなかったものを含む）セル。呼び出し側は
    /// これらの`prediction`を`None`（予測なし）に落とすこと。
    pub dropped: Vec<(Status, usize)>,
    /// 再測定した全セルと結果（診断ログ用。`dropped`はこのうち再現しなかったもの）。
    pub cells: Vec<(MismatchedTarget, RemeasureResult)>,
}

/// 内蔵表との突き合わせ全体（一致数・片側のみ・不一致セルの再測定）を集計する。
///
/// 不一致セルは（起動閾値を置かず）全件再測定する（決定1b項目7）。
pub fn reconcile_with_bundled<D: ImeDriver>(
    exec: &mut Executor<D>,
    matched: u32,
    only_in_one_table: u32,
    mismatched: &[MismatchedTarget],
    rng: &mut Rng,
    params: &RemeasureParams,
) -> Reconciliation {
    let mut summary = ReconciliationSummary::new();
    for _ in 0..matched {
        summary.record(CellReconciliation::Matched);
    }
    for _ in 0..only_in_one_table {
        summary.record(CellReconciliation::OnlyInOneTable);
    }
    let mut dropped = Vec::new();
    let mut cells = Vec::new();
    for &target in mismatched {
        let result = remeasure_cell(exec, target, rng, params);
        cells.push((target, result));
        match result {
            RemeasureResult::Reproduced => {
                summary.record(CellReconciliation::ReconfirmedByRemeasurement);
            }
            RemeasureResult::NotReproduced | RemeasureResult::Unconfirmed => {
                summary.record(CellReconciliation::NotReproduced);
                dropped.push((target.status, target.key));
            }
        }
    }
    Reconciliation {
        summary,
        dropped,
        cells,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anomaly::AnomalyPolicy;
    use crate::cost::CostModel;
    use crate::exec::ReadPolicy;
    use crate::sample_models::{atok_keys, atok_like};
    use crate::sim::{SimConfig, SimIme};

    fn exec() -> Executor<SimIme> {
        let mut e = Executor::new(
            SimIme::new(atok_like(), SimConfig::default(), CostModel::event()),
            AnomalyPolicy::default(),
            ReadPolicy::Single,
        );
        e.set_recording(false);
        e
    }

    const PARAMS: RemeasureParams = RemeasureParams {
        max_setup_presses: 200,
        reset_every: 12,
        key_count: atok_keys::COUNT,
    };

    /// 初期statusで`key`を押した真の結果（学習値として使う）。
    fn truth(key: usize) -> (Status, Outcome) {
        let mut e = exec();
        e.reset();
        let before = e.read_status();
        let info = e.press(key).expect("届く");
        (before, info.outcome)
    }

    #[test]
    fn reproduced_when_learned_value_matches() {
        let (status, outcome) = truth(atok_keys::HIRAGANA);
        let mut e = exec();
        let mut rng = Rng::new(7);
        let target = MismatchedTarget {
            status,
            key: atok_keys::HIRAGANA,
            learned: outcome,
        };
        assert_eq!(
            remeasure_cell(&mut e, target, &mut rng, &PARAMS),
            RemeasureResult::Reproduced
        );
    }

    #[test]
    fn not_reproduced_when_learned_value_differs() {
        let (status, outcome) = truth(atok_keys::HIRAGANA);
        let mut wrong = outcome;
        wrong.status.open = !wrong.status.open;
        let mut e = exec();
        let mut rng = Rng::new(7);
        let target = MismatchedTarget {
            status,
            key: atok_keys::HIRAGANA,
            learned: wrong,
        };
        assert_eq!(
            remeasure_cell(&mut e, target, &mut rng, &PARAMS),
            RemeasureResult::NotReproduced
        );
    }

    #[test]
    fn unconfirmed_when_status_unreachable() {
        let (status, outcome) = truth(atok_keys::HIRAGANA);
        let mut unreachable = status;
        unreachable.mode = 0x7F; // 機械に存在しないmode
        let mut e = exec();
        let mut rng = Rng::new(7);
        let target = MismatchedTarget {
            status: unreachable,
            key: atok_keys::HIRAGANA,
            learned: outcome,
        };
        assert_eq!(
            remeasure_cell(&mut e, target, &mut rng, &PARAMS),
            RemeasureResult::Unconfirmed
        );
    }

    #[test]
    fn reconcile_counts_and_drops_unreproduced_cells() {
        let (status, outcome) = truth(atok_keys::HIRAGANA);
        let mut wrong = outcome;
        wrong.status.open = !wrong.status.open;
        let ok = MismatchedTarget {
            status,
            key: atok_keys::HIRAGANA,
            learned: outcome,
        };
        let bad = MismatchedTarget {
            status,
            key: atok_keys::HIRAGANA,
            learned: wrong,
        };
        let mut e = exec();
        let mut rng = Rng::new(3);
        let r = reconcile_with_bundled(&mut e, 5, 2, &[ok, bad], &mut rng, &PARAMS);
        assert_eq!(r.summary.matched, 5);
        assert_eq!(r.summary.only_in_one_table, 2);
        assert_eq!(r.summary.reconfirmed, 1);
        assert_eq!(r.summary.not_reproduced, 1);
        assert_eq!(r.dropped, vec![(status, atok_keys::HIRAGANA)]);
        assert_eq!(r.cells.len(), 2);
        assert_eq!(r.cells[0].1, RemeasureResult::Reproduced);
        assert_eq!(r.cells[1].1, RemeasureResult::NotReproduced);
        // 分母は matched+reconfirmed+not_reproduced = 7 (only_in_one_tableは含めない)。
        assert_eq!(r.summary.common_cells(), 7);
    }
}
