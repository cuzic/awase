//! 学習の進捗率の見積り。「残り作業量」を途中経過から見積もり直し、分母
//! (`これまでの打鍵数 + 残りの見積り`)を最悪ケースから実際の値へ縮めていく。
//!
//! セル数は序盤で頭打ちになり終盤に動かなく見える(実測: 約65秒で168セルに到達後、約27秒は不変)
//! ため、進捗の分子は打鍵数にする。残りは次の3つの和:
//! - 発見済みの`Status`の未測定セル: セルあたり[`PRESSES_PER_CELL`]打鍵。
//! - まだ見つかっていない`Status`の分: 上限(モデルの状態数)まで見込み、発見済みのセルを
//!   測り終えるにつれて0へ縮める(測るべき既知セルが無ければ新しい状態へも進めない)。
//! - やり直し・検証などの末尾の枠: [`TAIL_PRESSES`]。セルを測り終えてから消化していく。
//!
//! 割合は一度出した値より下げない(分母の見積りが伸びても逆戻りさせない)。
//!
//! 定数の根拠(GJI、12状態×14キー=168セル、同じ構成の訓練打鍵数を4回実測:
//! 1482/1512/1518/1501。セルが168に達したのが全体の約7割の時点):
//! (1500 - 450) / 168 = 6.25打鍵/セル、末尾は約450打鍵。5モードの30状態は未測定の外挿。
//!
//! OS非依存なのでLinuxでもユニットテストできる。

/// 1セルを測るのに要する打鍵数(上記の実測から)。
pub const PRESSES_PER_CELL: f64 = 6.25;
/// セルを測り終えた後のやり直し・検証などの打鍵数の枠。
pub const TAIL_PRESSES: f64 = 450.0;

/// ある時点の学習の状況。
#[derive(Debug, Clone, Copy)]
pub struct Snapshot {
    /// これまでの打鍵数。
    pub presses: u32,
    /// 1回以上測ったセル数。
    pub covered_cells: u32,
    /// 発見済みの`Status`の種類数。
    pub observed_statuses: u32,
    /// 全キー数(1`Status`あたりのセル数)。
    pub keys: u32,
    /// 到達しうる`Status`数の上限(モデルの状態数)。
    pub max_statuses: u32,
}

/// 進捗の見積り。割合を単調にするための状態を持つ。
#[derive(Debug, Default)]
pub struct ProgressEstimator {
    /// 発見済みのセルを測り終えた時点の打鍵数(まだなら`None`)。
    cells_done_at: Option<u32>,
    /// これまでに出した最大の割合。
    max_fraction: f64,
}

impl ProgressEstimator {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 分母(想定の総打鍵数)を返す。`presses`より必ず大きい。割合(`presses / 戻り値`)は
    /// 呼び出しのたびに単調非減少になる。
    pub fn expected_presses(&mut self, s: Snapshot) -> u32 {
        let known_cells = s.observed_statuses * s.keys;
        let unmeasured_known = known_cells.saturating_sub(s.covered_cells);
        let unseen_statuses = s.max_statuses.saturating_sub(s.observed_statuses);

        let unseen_weight = if known_cells == 0 {
            1.0
        } else {
            f64::from(unmeasured_known) / f64::from(known_cells)
        };
        let unseen_cells = f64::from(unseen_statuses * s.keys) * unseen_weight;

        let cells_remaining = f64::from(unmeasured_known) + unseen_cells;
        let tail_remaining = if unmeasured_known == 0 && unseen_cells == 0.0 && known_cells > 0 {
            let done_at = *self.cells_done_at.get_or_insert(s.presses);
            (TAIL_PRESSES - f64::from(s.presses.saturating_sub(done_at))).max(1.0)
        } else {
            self.cells_done_at = None;
            TAIL_PRESSES
        };

        let remaining = cells_remaining * PRESSES_PER_CELL + tail_remaining;
        let mut expected = f64::from(s.presses) + remaining;
        // 割合を下げない: これまでの最大割合を保てる範囲まで分母を伸ばして済ませる。
        if self.max_fraction > 0.0 {
            expected = expected.min(f64::from(s.presses) / self.max_fraction);
        }
        expected = expected.max(f64::from(s.presses) + 1.0);
        self.max_fraction = self.max_fraction.max(f64::from(s.presses) / expected);
        expected.ceil() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(presses: u32, covered: u32, observed: u32) -> Snapshot {
        Snapshot {
            presses,
            covered_cells: covered,
            observed_statuses: observed,
            keys: 14,
            max_statuses: 12,
        }
    }

    #[test]
    fn starts_from_worst_case_near_measured_total() {
        let mut e = ProgressEstimator::new();
        // 開始直後: 168セル×6.25 + 450 = 1500打鍵(実測の訓練打鍵数と同程度)。
        let total = e.expected_presses(snap(0, 0, 0));
        assert_eq!(total, 1500);
    }

    #[test]
    fn fraction_never_decreases_even_when_new_statuses_appear() {
        let mut e = ProgressEstimator::new();
        let mut prev = 0.0;
        // 見積りが急に増える(状態を新たに発見して未測定セルが増える)シナリオ。
        let steps = [
            snap(10, 5, 1),
            snap(200, 14, 1),
            snap(300, 20, 3),
            snap(700, 100, 8),
            snap(1000, 168, 12),
            snap(1300, 168, 12),
            snap(1480, 168, 12),
            snap(1600, 168, 12),
        ];
        for s in steps {
            let total = e.expected_presses(s);
            assert!(total > s.presses);
            let f = f64::from(s.presses) / f64::from(total);
            assert!(f + 1e-9 >= prev, "{f} < {prev} at presses={}", s.presses);
            prev = f;
        }
        assert!(prev > 0.95, "終盤はほぼ満了に近づく: {prev}");
    }

    #[test]
    fn denominator_shrinks_toward_actual_when_fewer_statuses_exist() {
        // 上限30状態を見込んで始めても、12状態で測り終えれば分母は約半分に縮む。
        let mut e = ProgressEstimator::new();
        let worst = e.expected_presses(Snapshot {
            presses: 0,
            covered_cells: 0,
            observed_statuses: 0,
            keys: 14,
            max_statuses: 30,
        });
        let done = e.expected_presses(Snapshot {
            presses: 1050,
            covered_cells: 168,
            observed_statuses: 12,
            keys: 14,
            max_statuses: 30,
        });
        assert!(worst > 3000, "{worst}");
        assert!(done < worst, "{done} < {worst}");
    }

    #[test]
    fn never_reaches_one_hundred_percent_before_finish() {
        let mut e = ProgressEstimator::new();
        let s = snap(100_000, 168, 12);
        let total = e.expected_presses(s);
        assert!(total > s.presses);
    }
}
