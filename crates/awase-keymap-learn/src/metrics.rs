//! 指標(M1〜M9)。実行後の `Executor` と真のモデルから計算する。

use crate::exec::Executor;
use crate::model::{CellTruth, Machine};
use crate::table::Class;

/// 指標。割合は 0〜1。到達しなかった時刻は NaN。
#[derive(Debug, Clone, Copy, Default)]
pub struct Metrics {
    /// M1: 総壁時計時間(ms)。
    pub time_ms: f64,
    /// M2: 総押下数。
    pub presses: f64,
    /// M3: リセット回数。
    pub resets: f64,
    /// M4: 全セルを1回以上/2回以上測るまでの時間(ms)。
    pub t_cov1: f64,
    pub t_cov2: f64,
    /// 最終のセル網羅率(1回以上/2回以上)。
    pub cov1: f64,
    pub cov2: f64,
    /// M5: 観測のあるセルの、直前キー(文脈)の種類数の平均。
    pub ctx_diversity: f64,
    /// M6: 同期喪失の回数と、異常の総数。
    pub sync_losses: f64,
    pub anomalies: f64,
    /// M7: 真の非決定セルのうち、決定的でないと宣言できた割合 / 決定的なセルを誤って非決定と宣言した割合。
    pub nondet_detect: f64,
    pub false_alarm: f64,
    /// M8: 真の履歴依存セルのうち、決定的でないと宣言できた割合。
    pub hist_detect: f64,
    /// M9: 真の決定的なセルのうち、多数派が真と一致した割合(未測定は誤り扱い)/ 非決定・履歴依存のセルを「決定的」と断定した割合。
    pub accuracy: f64,
    pub false_det: f64,
}

fn ratio(n: usize, d: usize) -> f64 {
    if d == 0 {
        f64::NAN
    } else {
        n as f64 / d as f64
    }
}

/// 指標を計算する。
pub fn evaluate(exec: &Executor, m: &Machine) -> Metrics {
    let statuses = m.statuses();
    let mut total = 0usize;
    let (mut c1, mut c2) = (0usize, 0usize);
    let (mut det_total, mut det_correct, mut det_alarm) = (0usize, 0usize, 0usize);
    let (mut nd_total, mut nd_found) = (0usize, 0usize);
    let (mut hd_total, mut hd_found) = (0usize, 0usize);
    let (mut bad_total, mut bad_declared_det) = (0usize, 0usize);
    let (mut ctx_sum, mut ctx_n) = (0usize, 0usize);
    for st in &statuses {
        for k in 0..m.keys.len() {
            let Some(truth) = m.truth(*st, k) else {
                continue;
            };
            total += 1;
            let n = exec.table.count(*st, k);
            if n >= 1 {
                c1 += 1;
                ctx_sum += exec.table.distinct_ctx(*st, k);
                ctx_n += 1;
            }
            if n >= 2 {
                c2 += 1;
            }
            let class = exec.table.class(*st, k);
            match truth {
                CellTruth::Deterministic(o) => {
                    det_total += 1;
                    if exec.table.majority(*st, k) == Some(o) {
                        det_correct += 1;
                    }
                    if class.declared_not_det() {
                        det_alarm += 1;
                    }
                }
                CellTruth::NonDeterministic => {
                    nd_total += 1;
                    bad_total += 1;
                    if class.declared_not_det() {
                        nd_found += 1;
                    }
                    if matches!(class, Class::Det(_)) {
                        bad_declared_det += 1;
                    }
                }
                CellTruth::HistoryDependent => {
                    hd_total += 1;
                    bad_total += 1;
                    if class.declared_not_det() {
                        hd_found += 1;
                    }
                    if matches!(class, Class::Det(_)) {
                        bad_declared_det += 1;
                    }
                }
            }
        }
    }
    // M4: 時系列(注: 観測ノイズで存在しないセルが混じると、covered は過大になる)。
    let first_reach = |sel: fn(&(f64, usize, usize)) -> usize| {
        exec.stats
            .timeline
            .iter()
            .find(|t| sel(t) >= total)
            .map_or(f64::NAN, |t| t.0)
    };
    Metrics {
        time_ms: exec.elapsed_ms(),
        presses: f64::from(exec.stats.presses),
        resets: f64::from(exec.stats.resets),
        t_cov1: first_reach(|t| t.1),
        t_cov2: first_reach(|t| t.2),
        cov1: ratio(c1, total),
        cov2: ratio(c2, total),
        ctx_diversity: ratio(ctx_sum, ctx_n),
        sync_losses: f64::from(exec.stats.sync_losses),
        anomalies: f64::from(exec.stats.anomalies.values().sum::<u32>()),
        nondet_detect: ratio(nd_found, nd_total),
        false_alarm: ratio(det_alarm, det_total),
        hist_detect: ratio(hd_found, hd_total),
        accuracy: ratio(det_correct, det_total),
        false_det: ratio(bad_declared_det, bad_total),
    }
}

/// 複数の実行の平均(NaNは除いて平均。全部NaNなら NaN)。
pub fn mean(v: &[Metrics]) -> Metrics {
    let avg = |f: fn(&Metrics) -> f64| {
        let xs: Vec<f64> = v.iter().map(f).filter(|x| !x.is_nan()).collect();
        if xs.is_empty() {
            f64::NAN
        } else {
            xs.iter().sum::<f64>() / xs.len() as f64
        }
    };
    Metrics {
        time_ms: avg(|m| m.time_ms),
        presses: avg(|m| m.presses),
        resets: avg(|m| m.resets),
        t_cov1: avg(|m| m.t_cov1),
        t_cov2: avg(|m| m.t_cov2),
        cov1: avg(|m| m.cov1),
        cov2: avg(|m| m.cov2),
        ctx_diversity: avg(|m| m.ctx_diversity),
        sync_losses: avg(|m| m.sync_losses),
        anomalies: avg(|m| m.anomalies),
        nondet_detect: avg(|m| m.nondet_detect),
        false_alarm: avg(|m| m.false_alarm),
        hist_detect: avg(|m| m.hist_detect),
        accuracy: avg(|m| m.accuracy),
        false_det: avg(|m| m.false_det),
    }
}
