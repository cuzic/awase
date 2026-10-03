//! 学習の進捗率の見積り。「残り作業量」を途中経過から見積もり直し、分母
//! (`これまでの打鍵数 + 残りの見積り`)を最悪ケースから実際の値へ縮めていく。
//!
//! セル数は序盤で頭打ちになり終盤に動かなく見える(実測: 約65秒で168セルに到達後、約27秒は不変)
//! ため、進捗の分子は打鍵数にする。残りは次の3つの和:
//! - 発見済みの`Status`の未測定セル: セルあたり[`PRESSES_PER_CELL`]打鍵。
//! - まだ見つかっていない`Status`の分: 上限(モデルの状態数)まで見込み、発見済みのセルを
//!   測り終えるにつれて0へ縮める(測るべき既知セルが無ければ新しい状態へも進めない)。
//! - やり直し・検証などの末尾の枠: [`TAIL_PRESSES`]と、学習後の検証ウォーク
//!   [`VERIFY_WALK_PRESSES`]。セルを測り終えてから消化していく。
//!
//! 割合は一度出した値より下げない(分母の見積りが伸びても逆戻りさせない)。
//!
//! 定数の根拠(GJI、12状態×14キー=168セル): 訓練の総打鍵数は5回実測で
//! 1482/1512/1518/1501/1527。直近の実機ランで168セルに達したのは870打鍵時点
//! (62.8秒、総1527の約57%): 870/168 = 5.2打鍵/セル、末尾は 1500-870 = 約630打鍵。
//! (時間では全体の約7割に見えたが、打鍵数では約57%。打鍵は時間に対して一様でない。)
//! セル到達時点を測ったのは1回分のみ。5モードの30状態は未測定の外挿。
//!
//! 検証ウォークは学習(やり直し込み)の後に走り、予測できたステップが300に達するまで押下を
//! 続ける(`MIN_PREDICTED_STEPS`)。これを見積りに入れないと、学習が終わった時点で進捗が
//! 「ほぼ100%・残り約1秒」に張り付いたまま、ウォーク分の押下が進捗なしで続いてしまう。
//! ウォークの実打鍵数は未測定(予測できなかった押下の分だけ300より多い)ため、余裕を
//! 見て[`VERIFY_WALK_PRESSES`]を置いた。実機ログで`presses`の最終値が分かれば合わせ直す。
//!
//! OS非依存なのでLinuxでもユニットテストできる。

/// 1セルを測るのに要する打鍵数(上記の実測から)。
pub const PRESSES_PER_CELL: f64 = 5.2;
/// セルを測り終えた後のやり直し・検証などの打鍵数の枠。
pub const TAIL_PRESSES: f64 = 630.0;
/// 内蔵表と食い違ったセルの再測定1セルあたりの打鍵数(実測: windows-latest実GJI+ATOKで
/// 到達所要押下数の平均22〜26、`REMEASURE_RESET_EVERY`の比較〈n=240〉より)。
pub const REMEASURE_PRESSES_PER_CELL: f64 = 24.0;
/// 学習後の検証ウォークの打鍵数の見積り(未測定、下限は予測ステップ数300)。
pub const VERIFY_WALK_PRESSES: f64 = 330.0;

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
    /// 直近に返した(打鍵数, 分母)。[`Self::add_extra_tail`]が割合の下限を引き直すのに使う。
    last: Option<(u32, u32)>,
    /// 学習後に分かった追加の打鍵数(内蔵表との不一致セルの再測定など)。
    extra_tail: f64,
}

impl ProgressEstimator {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 学習後に判明した追加の作業量(打鍵数)を末尾の枠へ足す。分母は伸びるので、割合は
    /// この時点に限って下がりうる(下げないままだと、追加分の間ずっと「残り0秒・99%」に
    /// 張り付く)。以降は再び単調。
    pub fn add_extra_tail(&mut self, presses: f64) {
        self.extra_tail += presses;
        if let Some((p, e)) = self.last {
            self.max_fraction = f64::from(p) / (f64::from(e) + presses);
        }
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
            (TAIL_PRESSES + VERIFY_WALK_PRESSES + self.extra_tail
                - f64::from(s.presses.saturating_sub(done_at)))
            .max(1.0)
        } else {
            self.cells_done_at = None;
            TAIL_PRESSES + VERIFY_WALK_PRESSES + self.extra_tail
        };

        let remaining = cells_remaining * PRESSES_PER_CELL + tail_remaining;
        let mut expected = f64::from(s.presses) + remaining;
        // 割合を下げない: これまでの最大割合を保てる範囲まで分母を伸ばして済ませる。
        if self.max_fraction > 0.0 {
            expected = expected.min(f64::from(s.presses) / self.max_fraction);
        }
        expected = expected.max(f64::from(s.presses) + 1.0);
        self.max_fraction = self.max_fraction.max(f64::from(s.presses) / expected);
        let result = expected.ceil() as u32;
        self.last = Some((s.presses, result));
        result
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
        // 開始直後: 168セル×5.2 + 630 = 約1504打鍵(実測の訓練打鍵数1482〜1527と同程度)に、
        // 検証ウォークの330打鍵を足した約1834打鍵。
        let total = e.expected_presses(snap(0, 0, 0));
        assert!((1810..=1860).contains(&total), "{total}");
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
            snap(870, 168, 12),
            snap(1200, 168, 12),
            snap(1480, 168, 12),
            snap(1800, 168, 12),
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
            presses: 870,
            covered_cells: 168,
            observed_statuses: 12,
            keys: 14,
            max_statuses: 30,
        });
        assert!(worst > 2500, "{worst}");
        assert!(done < worst, "{done} < {worst}");
    }

    #[test]
    fn extra_tail_keeps_eta_from_sticking_at_the_end() {
        let mut e = ProgressEstimator::new();
        // 学習本体の終了直後(検証ウォークの前)に、再測定25セル分が判明した。
        let before = e.expected_presses(snap(1500, 168, 12));
        e.add_extra_tail(25.0 * REMEASURE_PRESSES_PER_CELL);
        let after = e.expected_presses(snap(1510, 168, 12));
        assert!(after >= before + 500, "{before} -> {after}");
        // 再測定の途中でも、残りがあるうちは分母が押下数へ張り付かない。
        let mid = e.expected_presses(snap(2000, 168, 12));
        assert!(mid > 2000 + 100, "{mid}");
    }

    #[test]
    fn never_reaches_one_hundred_percent_before_finish() {
        let mut e = ProgressEstimator::new();
        let s = snap(100_000, 168, 12);
        let total = e.expected_presses(s);
        assert!(total > s.presses);
    }
}
