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
//! 表示用の割合・残り時間は、見積りの揺れをならして線形に近づける[`LinearProgress`]が担う。
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
    /// 学習後に分かった追加の打鍵数(内蔵表との不一致セルの再測定など)。
    extra_tail: f64,
}

impl ProgressEstimator {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 学習後に判明した追加の作業量(打鍵数。内蔵表との不一致セルの再測定など)を
    /// 末尾の枠へ足す。
    pub fn add_extra_tail(&mut self, presses: f64) {
        self.extra_tail += presses;
    }

    /// 分母(想定の総打鍵数)を返す。`presses`より必ず大きい。見積りが変わるたびに動くので、
    /// 表示用の割合・残り時間は[`LinearProgress`]がなめらかにする。
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
        let expected = (f64::from(s.presses) + remaining).max(f64::from(s.presses) + 1.0);
        expected.ceil() as u32
    }
}

/// 総所要時間の見積りが1秒の経過で動いてよい量(経過時間に対する比)。1未満なら、
/// 割合(`経過/総時間`)は単調増加・残り時間(`総時間-経過`)は単調減少になる。
const SLEW: f64 = 0.5;
/// 見積りを超過しても残り時間をここまでは下回らせない(0秒・100%に張り付かない)。
/// 経過時間に対する比と、絶対値の下限の大きい方。超過しているときは残りが分からないので、
/// 「まだ少しかかる」程度(経過の5%)を示し続ける。
const MIN_ETA_FRACTION: f64 = 0.05;
const MIN_ETA_MS: f64 = 300.0;
/// 直近の1打鍵あたりの所要時間を求める窓(打鍵数)。
const RATE_WINDOW_PRESSES: u32 = 100;

/// [`ProgressEstimator`]の打鍵数の見積りを**時間**へ換算し、総所要時間の見積りを
/// 経過時間に対して[`SLEW`]以下の速さでしか動かさない。見積りが途中で変わっても、
/// 割合は一定の傾きで増え、残り時間は1秒に1秒ずつ減る(見積りが変わった分だけ傾きが
/// なだらかに変わる)。打鍵の所要時間は局面で違う(学習・検証・再測定)ため、
/// 1打鍵あたりは全体平均でなく直近[`RATE_WINDOW_PRESSES`]打鍵の実績を使う。
#[derive(Debug, Default)]
pub struct LinearProgress {
    estimator: ProgressEstimator,
    total_ms: Option<f64>,
    last_elapsed_ms: f64,
    /// 見積りを超過した最初の時点で固めた残り時間の下限(以降は伸ばさず、残りが増えないようにする)。
    eta_floor_ms: Option<f64>,
    window: std::collections::VecDeque<(u32, f64)>,
}

/// [`LinearProgress::update`]の出力。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Display {
    /// 残り時間(ms)。
    pub eta_ms: f64,
    /// 想定の総打鍵数。`presses / expected_presses`が経過/総時間に等しくなるよう置く。
    pub expected_presses: u32,
}

impl LinearProgress {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// [`ProgressEstimator::add_extra_tail`]と同じ。
    pub fn add_extra_tail(&mut self, presses: f64) {
        self.estimator.add_extra_tail(presses);
    }

    /// `elapsed_ms`は開始からの経過時間。`s.presses`が0なら`None`(まだ速さが分からない)。
    pub fn update(&mut self, s: Snapshot, elapsed_ms: f64) -> Option<Display> {
        let expected = self.estimator.expected_presses(s);
        if s.presses == 0 || elapsed_ms <= 0.0 {
            return None;
        }
        self.window.push_back((s.presses, elapsed_ms));
        while self
            .window
            .front()
            .is_some_and(|&(p, _)| s.presses - p > RATE_WINDOW_PRESSES)
        {
            self.window.pop_front();
        }
        let (p0, e0) = self.window.front().copied().unwrap_or((0, 0.0));
        let rate = if s.presses > p0 {
            (elapsed_ms - e0) / f64::from(s.presses - p0)
        } else {
            elapsed_ms / f64::from(s.presses)
        };
        let raw_total = elapsed_ms + f64::from(expected - s.presses) * rate;

        let dt = (elapsed_ms - self.last_elapsed_ms).max(0.0);
        self.last_elapsed_ms = elapsed_ms;
        let slewed = match self.total_ms {
            None => raw_total,
            Some(t) => t + (raw_total - t).clamp(-SLEW * dt, SLEW * dt),
        };
        let floor = self
            .eta_floor_ms
            .unwrap_or_else(|| MIN_ETA_MS.max(MIN_ETA_FRACTION * elapsed_ms));
        if slewed < elapsed_ms + floor {
            self.eta_floor_ms.get_or_insert(floor);
        }
        let total = slewed.max(elapsed_ms + floor);
        self.total_ms = Some(total);

        let expected_presses = (f64::from(s.presses) * total / elapsed_ms).ceil() as u32;
        Some(Display {
            eta_ms: total - elapsed_ms,
            expected_presses: expected_presses.max(s.presses + 1),
        })
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

    /// 1打鍵`ms_per_press`で`actual_total`打鍵まで進む実行を再生し、
    /// (経過ms, 割合, 残りms)の列を返す。見積りの前提(168セルは870打鍵で到達)に従う。
    fn run(actual_total: u32, ms_per_press: f64, extra: f64) -> Vec<(f64, f64, f64)> {
        let mut lp = LinearProgress::new();
        lp.add_extra_tail(extra);
        let mut out = Vec::new();
        for presses in (10..=actual_total).step_by(10) {
            let covered = (presses * 168 / 870).min(168);
            let elapsed = f64::from(presses) * ms_per_press;
            let d = lp.update(snap(presses, covered, 12), elapsed).unwrap();
            out.push((
                elapsed,
                f64::from(presses) / f64::from(d.expected_presses),
                d.eta_ms,
            ));
        }
        out
    }

    #[test]
    fn progress_and_eta_are_monotonic_even_when_estimate_is_off() {
        // 見積り(約1834)より実際が長い(2600)・短い(1200)場合でも、割合は減らず残りは増えない。
        for actual in [2600, 1200, 1830] {
            let rows = run(actual, 75.0, 0.0);
            for w in rows.windows(2) {
                assert!(w[1].1 + 1e-9 >= w[0].1, "割合が下がった {actual}: {w:?}");
                assert!(w[1].2 <= w[0].2 + 1e-6, "残り時間が増えた {actual}: {w:?}");
            }
        }
    }

    #[test]
    fn progress_is_nearly_linear_when_estimate_is_right() {
        let rows = run(1830, 75.0, 0.0);
        let total = rows.last().unwrap().0;
        for &(e, f, _) in &rows {
            assert!((f - e / total).abs() < 0.08, "{e}: {f} vs {}", e / total);
        }
    }

    #[test]
    fn eta_does_not_stick_near_zero_for_long() {
        // 実際が見積りの1.4倍かかっても、残り1秒未満のまま続く区間は全体の5%以内。
        let rows = run(2600, 75.0, 0.0);
        let stuck = rows.iter().filter(|r| r.2 < 1000.0).count();
        assert!(stuck * 20 <= rows.len(), "{stuck}/{}", rows.len());
    }

    #[test]
    fn never_reaches_one_hundred_percent_before_finish() {
        let mut e = ProgressEstimator::new();
        let s = snap(100_000, 168, 12);
        let total = e.expected_presses(s);
        assert!(total > s.presses);
    }
}
