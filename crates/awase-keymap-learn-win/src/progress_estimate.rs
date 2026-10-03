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

    /// 局面別の残り打鍵数 `(セル巡回の残り, 巡回後の残り)` を返す。
    ///
    /// セル巡回の残りは、発見済みの未測定セルからの見積り([`PRESSES_PER_CELL`])と、
    /// 「`max_statuses`×キー数のセルを測り終えるまでの総打鍵数から、これまでの打鍵数を引いた値」
    /// の大きい方にする。状態は巡回の途中で次々に見つかるため、発見数からの見積りだけでは
    /// 序盤に大きく過小になる(実機: 100打鍵時点で見積り総時間が実際の約0.8倍)。
    fn remaining_parts(&mut self, s: Snapshot) -> (f64, f64) {
        let known_cells = s.observed_statuses * s.keys;
        let unmeasured_known = known_cells.saturating_sub(s.covered_cells);
        let unseen_statuses = s.max_statuses.saturating_sub(s.observed_statuses);

        let unseen_weight = if known_cells == 0 {
            1.0
        } else {
            f64::from(unmeasured_known) / f64::from(known_cells)
        };
        let unseen_cells = f64::from(unseen_statuses * s.keys) * unseen_weight;
        let tail_total = TAIL_PRESSES + VERIFY_WALK_PRESSES + self.extra_tail;

        if unmeasured_known == 0 && unseen_cells == 0.0 && known_cells > 0 {
            let done_at = *self.cells_done_at.get_or_insert(s.presses);
            let tail = tail_total - f64::from(s.presses.saturating_sub(done_at));
            (0.0, tail.max(1.0))
        } else {
            self.cells_done_at = None;
            let from_discovery = (f64::from(unmeasured_known) + unseen_cells) * PRESSES_PER_CELL;
            let from_total =
                f64::from(s.max_statuses * s.keys) * PRESSES_PER_CELL - f64::from(s.presses);
            (from_discovery.max(from_total).max(0.0), tail_total)
        }
    }

    /// 巡回後の局面に入った時点の打鍵数(まだなら`None`)。
    #[must_use]
    pub fn tail_started_at(&self) -> Option<u32> {
        self.cells_done_at
    }

    /// 分母(想定の総打鍵数)を返す。`presses`より必ず大きい。見積りが変わるたびに動くので、
    /// 表示用の割合・残り時間は[`LinearProgress`]がなめらかにする。
    pub fn expected_presses(&mut self, s: Snapshot) -> u32 {
        let (cells, tail) = self.remaining_parts(s);
        let expected = (f64::from(s.presses) + cells + tail).max(f64::from(s.presses) + 1.0);
        expected.ceil() as u32
    }
}

/// 総所要時間の見積りが1秒の経過で動いてよい量(経過時間に対する比)。1未満なら、
/// 割合(`経過/総時間`)は単調増加・残り時間(`総時間-経過`)は単調減少になる。
const SLEW: f64 = 0.5;
/// 見積りが尽きた(超過した)時点の残り時間。経過時間に対する比と、絶対値の下限の大きい方。
/// 以降は実時間と同じ速さで減らし、終わりそうなら0秒・ほぼ100%へ向かって加速する
/// (固定したままだと、実機で終了の約2秒前から98%・残り2.7秒で止まって見えた)。
const MIN_ETA_FRACTION: f64 = 0.02;
const MIN_ETA_MS: f64 = 300.0;
/// 超過を減らし続けても、残り時間をこれ未満にはしない(0秒・100%に見せるのは完了の結果行)。
const FLOOR_ETA_MS: f64 = 200.0;
/// 速さ(1打鍵あたりの時間)が落ち着くまで残り時間を出さない打鍵数。起動直後は
/// 1打鍵あたりが遅く(実機: 10打鍵時点で約140ms、100打鍵以降は約94ms)、外挿すると
/// 総時間が大きく過大になる(実機: 初期見積り239秒、実際138.5秒)。
const MIN_PRESSES_FOR_ETA: u32 = 100;
/// 巡回後の局面の1打鍵あたりの時間 ÷ 巡回中の1打鍵あたりの時間。巡回後はリセット
/// (1回約116ms)がほぼ無いため速い。実機1回分の実測: 巡回870打鍵に81.8秒(94.0ms/打鍵)、
/// 巡回後972打鍵に56.8秒(58.5ms/打鍵)で比0.62。巡回後に入って
/// [`TAIL_OBSERVE_PRESSES`]打鍵たまったら、この仮定でなく実測を使う。
const TAIL_RATE_RATIO: f64 = 0.62;
const TAIL_OBSERVE_PRESSES: u32 = 30;

/// [`ProgressEstimator`]の局面別の残り打鍵数を、局面別の1打鍵あたりの時間で**時間**へ換算し、
/// 総所要時間の見積りを経過時間に対して[`SLEW`]以下の速さでしか動かさない。見積りが
/// 途中で変わっても、割合は一定の傾きで増え、残り時間は1秒に1秒ずつ減る。
#[derive(Debug, Default)]
pub struct LinearProgress {
    estimator: ProgressEstimator,
    total_ms: Option<f64>,
    last_elapsed_ms: f64,
    /// 見積りが尽きた最初の時点の(残り時間, 経過ms)。以降は実時間と同じ速さで減らす下限にする。
    eta_floor: Option<(f64, f64)>,
    /// 巡回後の局面に入った時点の(打鍵数, 経過ms)。
    tail_start: Option<(u32, f64)>,
}

/// [`LinearProgress::update`]の出力。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Display {
    /// 残り時間(ms)。まだ速さが分からないうちは`None`。
    pub eta_ms: Option<f64>,
    /// 想定の総打鍵数。残り時間があるときは`presses / expected_presses`が経過/総時間に等しくなるよう置く。
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

    /// `elapsed_ms`は開始からの経過時間。
    pub fn update(&mut self, s: Snapshot, elapsed_ms: f64) -> Display {
        let (cells, tail) = self.estimator.remaining_parts(s);
        let by_presses = Display {
            eta_ms: None,
            expected_presses: ((f64::from(s.presses) + cells + tail)
                .max(f64::from(s.presses) + 1.0))
            .ceil() as u32,
        };
        if s.presses < MIN_PRESSES_FOR_ETA || elapsed_ms <= 0.0 {
            return by_presses;
        }

        match (self.estimator.tail_started_at(), self.tail_start) {
            (Some(_), None) => self.tail_start = Some((s.presses, elapsed_ms)),
            (None, _) => self.tail_start = None,
            _ => {}
        }
        let (rate_cells, rate_tail) = if let Some((p0, e0)) = self.tail_start {
            let rate_cells = e0 / f64::from(p0.max(1));
            let n = s.presses - p0;
            let rate_tail = if n >= TAIL_OBSERVE_PRESSES {
                (elapsed_ms - e0) / f64::from(n)
            } else {
                rate_cells * TAIL_RATE_RATIO
            };
            (rate_cells, rate_tail)
        } else {
            let rate = elapsed_ms / f64::from(s.presses);
            (rate, rate * TAIL_RATE_RATIO)
        };
        let raw_total = elapsed_ms + cells * rate_cells + tail * rate_tail;

        let dt = (elapsed_ms - self.last_elapsed_ms).max(0.0);
        self.last_elapsed_ms = elapsed_ms;
        let slewed = match self.total_ms {
            None => raw_total,
            Some(t) => t + (raw_total - t).clamp(-SLEW * dt, SLEW * dt),
        };
        let floor = match self.eta_floor {
            Some((eta0, at)) => (eta0 - (elapsed_ms - at)).max(FLOOR_ETA_MS),
            None => MIN_ETA_MS.max(MIN_ETA_FRACTION * elapsed_ms),
        };
        if slewed < elapsed_ms + floor {
            self.eta_floor.get_or_insert((floor, elapsed_ms));
        }
        let total = slewed.max(elapsed_ms + floor);
        self.total_ms = Some(total);

        let expected_presses = (f64::from(s.presses) * total / elapsed_ms).ceil() as u32;
        Display {
            eta_ms: Some(total - elapsed_ms),
            expected_presses: expected_presses.max(s.presses + 1),
        }
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

    /// 実機の1回分(`tests/fixtures/progress_run_2026_10_03.csv`、GJI、総1842打鍵・138.5秒)を
    /// 再生し、(経過ms, 割合, 残りms, 打鍵数)の列を返す。残り時間が出ている行だけ。
    fn replay_real_run(extra_tail: f64) -> (f64, Vec<(f64, f64, f64)>) {
        let mut lp = LinearProgress::new();
        lp.add_extra_tail(extra_tail);
        let mut rows = Vec::new();
        let mut total_ms = 0.0;
        for line in include_str!("../tests/fixtures/progress_run_2026_10_03.csv")
            .lines()
            .filter(|l| !l.starts_with('#'))
        {
            let v: Vec<u32> = line.split(',').map(|x| x.parse().unwrap()).collect();
            let (cell, total, elapsed, presses) = (v[0], v[1], f64::from(v[2]), v[3]);
            let d = lp.update(
                Snapshot {
                    presses,
                    covered_cells: cell,
                    observed_statuses: (total / 14).max(cell.div_ceil(14)).max(1),
                    keys: 14,
                    max_statuses: 12,
                },
                elapsed,
            );
            total_ms = elapsed;
            if let Some(eta) = d.eta_ms {
                rows.push((
                    elapsed,
                    f64::from(presses) / f64::from(d.expected_presses),
                    eta,
                ));
            }
        }
        (total_ms, rows)
    }

    #[test]
    fn real_run_eta_and_fraction_track_the_truth() {
        // 修正前は、初期見積り239秒(実際138.5秒)が制限付きの補正で最後まで残り、
        // 残り33秒・81%で終わった。残り時間の誤差と、割合の直線からのずれを抑える。
        let (end, rows) = replay_real_run(0.0);
        assert!(rows.len() > 150, "{}", rows.len());
        for &(e, f, eta) in &rows {
            assert!(
                (eta - (end - e)).abs() < 12_000.0,
                "{e}: eta {eta} vs {}",
                end - e
            );
            assert!((f - e / end).abs() < 0.08, "{e}: {f} vs {}", e / end);
        }
        // 終わりそうなタイミングで0秒・ほぼ100%へ向かう(修正前は98%・残り2.7秒で止まった)。
        let (e, f, eta) = *rows.last().unwrap();
        assert!(f > 0.99 && eta < 1_000.0, "終了時: {e} {f} {eta}");
    }

    #[test]
    fn real_run_progress_and_eta_are_monotonic() {
        // 再測定が見積りに追加されても(実際は走らなかった場合)、割合は減らず残りは増えない。
        for extra in [0.0, 24.0 * 18.0] {
            let (_, rows) = replay_real_run(extra);
            for w in rows.windows(2) {
                // 想定打鍵数は整数へ切り上げるため、割合には最大0.2%の丸め誤差がありうる。
                assert!(w[1].1 + 2e-3 >= w[0].1, "割合が下がった: {w:?}");
                assert!(w[1].2 <= w[0].2 + 1e-6, "残りが増えた: {w:?}");
            }
        }
    }

    #[test]
    fn eta_is_withheld_until_speed_settles() {
        let mut lp = LinearProgress::new();
        let d = lp.update(snap(10, 5, 1), 1400.0);
        assert_eq!(d.eta_ms, None);
        assert!(d.expected_presses > 10);
    }

    #[test]
    fn overrunning_the_estimate_counts_down_to_a_small_floor() {
        // 実際が見積りの1.4倍かかる場合: 見積りが尽きたら残りは実時間どおり減って小さな下限
        // (FLOOR_ETA_MS)へ向かい、増えない。下限には達しうる(超過分は残りが分からない。
        // 終わりそうなら0秒へ加速することを優先した、見積りが外れたときの代償)。
        let mut lp = LinearProgress::new();
        let mut prev_eta = f64::MAX;
        for presses in (100..=2600).step_by(10) {
            let covered = (presses * 168 / 870).min(168);
            let d = lp.update(snap(presses, covered, 12), f64::from(presses) * 75.0);
            let eta = d.eta_ms.unwrap();
            assert!(eta <= prev_eta + 1e-6, "{presses}: {eta} > {prev_eta}");
            assert!(eta >= FLOOR_ETA_MS - 1e-6, "{presses}: {eta}");
            prev_eta = eta;
        }
    }

    #[test]
    fn never_reaches_one_hundred_percent_before_finish() {
        let mut e = ProgressEstimator::new();
        let s = snap(100_000, 168, 12);
        let total = e.expected_presses(s);
        assert!(total > s.presses);
    }
}
