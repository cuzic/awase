//! 学習の進捗率・残り時間の見積り。打鍵数ベースの「残り作業量」を局面別に見積もり、局面別の
//! 1打鍵あたりの時間で時間へ換算し、表示用の割合・残り時間が線形に近づくようならす。
//!
//! 学習は次の局面を順に進む(打鍵数は5回の実機・CIの実測、GJI/ATOK/MS-IME本体):
//! 1. **セル巡回**: 発見済みの`Status`の全セルを測り終えるまで。終わる打鍵数は環境で大きく違う
//!    (ATOK 280・MS-IME本体 570・GJI 870)ため、終わるまで分からない。序盤は観測した最長
//!    ([`CELL_PHASE_PRESSES_FLOOR`])を下限に長めに見積もり、終わった時点で正確な値へ一気に補正する
//!    (長めに見積もって早く終わる方が、短めに見積もって止まるより体験が良い)。
//! 2. **やり直し・検証などの末尾**: [`TAIL_PRESSES`]。巡回後に約630〜970打鍵(実測 388〜972)。
//! 3. **検証ウォーク**: 予測できたステップが目標に届くまで押す。進み具合([`WalkProgress`])から
//!    残りを直接求める。
//!
//! 巡回が終わったか(=末尾へ入ったか)は、(a)発見済みの全セルを測り終えた、または(b)測れたセル数が
//! [`STAGNATION_PRESSES`]打鍵動かない(MS-IME本体は210セル中154セルから先へ進まない)ときとする。
//! 新しいセルが出ない間隔の最大は巡回の途中でも60打鍵(実測5回)なので、150なら誤判定しない。
//!
//! 時間への換算([`LinearProgress`])は、巡回中は経過/打鍵数、末尾は巡回後の実測(30打鍵たまるまでは
//! 巡回中の[`TAIL_RATE_RATIO`]倍)を使う。巡回後が速いのは、リセット(約90〜116ms)が巡回中に
//! 集中するため。総時間の見積りは、上げる向きをゆるやかに([`SLEW_UP`]<1なら割合は単調増加・
//! 残り時間は単調減少)、下げる向きは速く([`SLEW_DOWN`])動かす。
//!
//! 実測(GJI実機2回・CIのGJI(ATOK/MS-IMEプリセット)・MS-IME本体)の再生では、GJI以外の環境で
//! 旧版が終了時に29〜50%で終わっていた(巡回の下限を168セル分に固定し、総時間を下げる速さを
//! 制限していたため)。
//!
//! OS非依存なのでLinuxでもユニットテストできる。

/// 1セルを測るのに要する打鍵数(GJI 168セル/870打鍵から)。
pub const PRESSES_PER_CELL: f64 = 5.2;
/// セル巡回の打鍵数の長めの見積り(観測した最長: GJI 870打鍵)。巡回が終わるまでの下限に使う。
pub const CELL_PHASE_PRESSES_FLOOR: f64 = 870.0;
/// セルを測り終えた後のやり直し・検証などの打鍵数の枠。
pub const TAIL_PRESSES: f64 = 630.0;
/// 内蔵表と食い違ったセルの再測定1セルあたりの打鍵数(実測: windows-latest実GJI+ATOKで
/// 到達所要押下数の平均22〜26、`REMEASURE_RESET_EVERY`の比較〈n=240〉より)。
pub const REMEASURE_PRESSES_PER_CELL: f64 = 24.0;
/// 学習後の検証ウォークの打鍵数の見積り(実測 302〜331。ウォークが始まれば進み具合で置き換える)。
pub const VERIFY_WALK_PRESSES: f64 = 330.0;
/// 測れたセル数がこの打鍵数動かなければ、巡回は終わったとみなす。
pub const STAGNATION_PRESSES: u32 = 150;

/// 総所要時間の見積りが1秒の経過で上げてよい量(経過時間に対する比)。1未満なら、
/// 割合(`経過/総時間`)は単調増加・残り時間(`総時間-経過`)は単調減少になる。
const SLEW_UP: f64 = 0.9;
/// 下げてよい量。大きくしておけば、長めの見積りから巡回が終わった時点で素早く補正できる。
const SLEW_DOWN: f64 = 6.0;
/// 見積りが尽きた(超過した)時点の残り時間。経過時間に対する比と、絶対値の下限の大きい方。
/// 以降は実時間と同じ速さで減らし、終わりそうなら0秒・ほぼ100%へ向かって加速する。
const MIN_ETA_FRACTION: f64 = 0.02;
const MIN_ETA_MS: f64 = 300.0;
/// 超過を減らし続けても、残り時間をこれ未満にはしない(0秒・100%に見せるのは完了の結果行)。
const FLOOR_ETA_MS: f64 = 200.0;
/// 速さ(1打鍵あたりの時間)が落ち着くまで残り時間を出さない打鍵数。起動直後は
/// 1打鍵あたりが遅く(10打鍵時点で約140ms、100打鍵以降は約94ms)、外挿すると過大になる。
const MIN_PRESSES_FOR_ETA: u32 = 100;
/// 巡回後の1打鍵あたりの時間 ÷ 巡回中の1打鍵あたりの時間。巡回後はリセットが少なく速い。
/// 実測の比: GJI実機 0.62、CIのGJI(MS-IMEプリセット)0.63、MS-IME本体 0.70、ATOK 0.87。
/// 巡回後に[`TAIL_OBSERVE_PRESSES`]打鍵たまったら、この仮定でなく実測を使う。
const TAIL_RATE_RATIO: f64 = 0.62;
const TAIL_OBSERVE_PRESSES: u32 = 30;

/// 検証ウォークの進み具合。
#[derive(Debug, Clone, Copy)]
pub struct WalkProgress {
    /// これまでに予測できたステップ数。
    pub predicted: u32,
    /// 目標の予測ステップ数(`MIN_PREDICTED_STEPS`)。
    pub target: u32,
    /// これまでの押下の試行回数(予測できなかった押下を含む)。
    pub attempts: u32,
}

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
    /// モデルが見込む`Status`数(初期仮説の見積り。実際はこれより多くも少なくもなる)。
    pub expected_statuses: u32,
    /// 検証ウォーク中ならその進み具合。
    pub walk: Option<WalkProgress>,
}

/// 局面別の残り打鍵数の見積り。
#[derive(Debug, Default)]
pub struct ProgressEstimator {
    /// 巡回が終わったとみなした時点の打鍵数(まだなら`None`)。
    cells_done_at: Option<u32>,
    /// 測れたセル数の最大値と、それに初めて達した打鍵数。
    best_covered: u32,
    best_covered_at: u32,
    /// 学習後に分かった追加の打鍵数(内蔵表との不一致セルの再測定など)。
    extra_tail: f64,
}

impl ProgressEstimator {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 学習後に判明した追加の作業量(打鍵数。内蔵表との不一致セルの再測定など)を足す。
    pub fn add_extra_tail(&mut self, presses: f64) {
        self.extra_tail += presses;
    }

    /// 局面別の残り打鍵数 `(セル巡回の残り, それ以降の残り)`。
    fn remaining_parts(&mut self, s: Snapshot) -> (f64, f64) {
        if let Some(w) = s.walk {
            let walk = if w.predicted > 0 {
                f64::from(w.target.saturating_sub(w.predicted)) * f64::from(w.attempts)
                    / f64::from(w.predicted)
            } else {
                VERIFY_WALK_PRESSES
            };
            return (0.0, walk.max(1.0) + self.extra_tail);
        }

        if s.covered_cells > self.best_covered {
            self.best_covered = s.covered_cells;
            self.best_covered_at = s.presses;
        }
        let known_cells = s.observed_statuses * s.keys;
        let unmeasured_known = known_cells.saturating_sub(s.covered_cells);
        let expected = s.expected_statuses.max(s.observed_statuses);
        let unseen_weight = if known_cells == 0 {
            1.0
        } else {
            f64::from(unmeasured_known) / f64::from(known_cells)
        };
        let unseen_cells =
            f64::from(expected - s.observed_statuses) * f64::from(s.keys) * unseen_weight;

        let all_measured = unmeasured_known == 0 && unseen_cells == 0.0 && known_cells > 0;
        let stagnant = self.best_covered > 0
            && s.presses.saturating_sub(self.best_covered_at) >= STAGNATION_PRESSES;
        let tail_total = TAIL_PRESSES + VERIFY_WALK_PRESSES + self.extra_tail;
        if all_measured || stagnant {
            let done_at = *self.cells_done_at.get_or_insert(if all_measured {
                s.presses
            } else {
                self.best_covered_at
            });
            let tail = tail_total - f64::from(s.presses.saturating_sub(done_at));
            (0.0, tail.max(1.0))
        } else {
            self.cells_done_at = None;
            let from_discovery = (f64::from(unmeasured_known) + unseen_cells) * PRESSES_PER_CELL;
            let from_floor = CELL_PHASE_PRESSES_FLOOR - f64::from(s.presses);
            (from_discovery.max(from_floor).max(0.0), tail_total)
        }
    }

    /// 巡回が終わったとみなした時点の打鍵数(まだなら`None`)。
    #[must_use]
    pub fn tail_started_at(&self) -> Option<u32> {
        self.cells_done_at
    }

    /// 想定の総打鍵数(`presses`より必ず大きい)。
    pub fn expected_presses(&mut self, s: Snapshot) -> u32 {
        let (cells, tail) = self.remaining_parts(s);
        let expected = (f64::from(s.presses) + cells + tail).max(f64::from(s.presses) + 1.0);
        expected.ceil() as u32
    }
}

/// [`LinearProgress::update`]の出力。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Display {
    /// 残り時間(ms)。まだ速さが分からないうちは`None`。
    pub eta_ms: Option<f64>,
    /// 想定の総打鍵数。残り時間があるときは`presses / expected_presses`が経過/総時間に等しくなるよう置く。
    pub expected_presses: u32,
}

/// [`ProgressEstimator`]の局面別の残り打鍵数を、局面別の1打鍵あたりの時間で時間へ換算し、
/// 総所要時間の見積りを上げる向きはゆるやかに・下げる向きは速く動かす。
#[derive(Debug, Default)]
pub struct LinearProgress {
    estimator: ProgressEstimator,
    total_ms: Option<f64>,
    last_elapsed_ms: f64,
    /// 見積りが尽きた最初の時点の(残り時間, 経過ms)。以降は実時間と同じ速さで減らす下限にする。
    eta_floor: Option<(f64, f64)>,
    /// 巡回後の局面に入った時点の(打鍵数, 経過ms)。
    tail_start: Option<(u32, f64)>,
    /// 検証ウォークに入った時点の(打鍵数, 経過ms)。
    walk_start: Option<(u32, f64)>,
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
            expected_presses: (f64::from(s.presses) + cells + tail)
                .max(f64::from(s.presses) + 1.0)
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
        if s.walk.is_some() && self.walk_start.is_none() {
            self.walk_start = Some((s.presses, elapsed_ms));
        }
        let (rate_cells, mut rate_tail) = if let Some((p0, e0)) = self.tail_start {
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
        if let Some((p0, e0)) = self.walk_start {
            let n = s.presses - p0;
            if n >= TAIL_OBSERVE_PRESSES {
                rate_tail = (elapsed_ms - e0) / f64::from(n);
            }
        }
        let raw_total = elapsed_ms + cells * rate_cells + tail * rate_tail;

        let dt = (elapsed_ms - self.last_elapsed_ms).max(0.0);
        self.last_elapsed_ms = elapsed_ms;
        let slewed = match self.total_ms {
            None => raw_total,
            Some(t) => t + (raw_total - t).clamp(-SLEW_DOWN * dt, SLEW_UP * dt),
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
            expected_statuses: 6,
            walk: None,
        }
    }

    /// 実機・CIの1回分の進捗(`tests/fixtures/*.csv`、列は cell,total,elapsed_ms,presses,statuses)と、
    /// 検証ウォークが始まった打鍵数。
    struct Run {
        name: &'static str,
        csv: &'static str,
        expected_statuses: u32,
        walk_start: u32,
    }

    const RUNS: [Run; 3] = [
        Run {
            name: "GJI(MS-IMEプリセット)",
            csv: include_str!("../tests/fixtures/progress_gji_msimepreset.csv"),
            expected_statuses: 6,
            walk_start: 1500,
        },
        Run {
            name: "GJI(ATOKプリセット)",
            csv: include_str!("../tests/fixtures/progress_gji_atok.csv"),
            expected_statuses: 6,
            walk_start: 668,
        },
        Run {
            name: "MS-IME本体",
            csv: include_str!("../tests/fixtures/progress_msime_native.csv"),
            expected_statuses: 15,
            walk_start: 1237,
        },
    ];

    /// 1回分を再生し、(経過ms, 割合, 残りms)の列(残り時間が出ている行だけ)と総所要時間を返す。
    /// 検証ウォークの進み具合は、始まりから終わりまで予測ステップが0→300へ線形に進むとして作る。
    fn replay(run: &Run) -> (f64, Vec<(f64, f64, f64)>) {
        let rows: Vec<Vec<u32>> = run
            .csv
            .lines()
            .filter(|l| !l.starts_with('#'))
            .map(|l| l.split(',').map(|x| x.parse().unwrap()).collect())
            .collect();
        let end_presses = rows.last().unwrap()[3];
        let mut lp = LinearProgress::new();
        let (mut end_ms, mut out) = (0.0, Vec::new());
        for v in rows {
            let (cell, elapsed, presses, statuses) = (v[0], f64::from(v[2]), v[3], v[4]);
            let walk = (presses >= run.walk_start).then(|| {
                let done = f64::from(presses - run.walk_start + 1)
                    / f64::from(end_presses - run.walk_start + 1);
                WalkProgress {
                    predicted: (300.0 * done).round() as u32,
                    target: 300,
                    attempts: presses - run.walk_start + 1,
                }
            });
            let d = lp.update(
                Snapshot {
                    presses,
                    covered_cells: cell,
                    observed_statuses: statuses,
                    keys: 14,
                    expected_statuses: run.expected_statuses,
                    walk,
                },
                elapsed,
            );
            end_ms = elapsed;
            if let Some(eta) = d.eta_ms {
                out.push((
                    elapsed,
                    f64::from(presses) / f64::from(d.expected_presses),
                    eta,
                ));
            }
        }
        (end_ms, out)
    }

    #[test]
    fn real_runs_end_near_zero_seconds_and_full_progress() {
        // 旧版は、GJI以外の環境で終了時に29〜50%・残り48〜218秒のまま終わった。
        for run in &RUNS {
            let (_, rows) = replay(run);
            let (e, f, eta) = *rows.last().unwrap();
            assert!(
                f > 0.99 && eta < 1_500.0,
                "{}: 終了時 {e} {f} {eta}",
                run.name
            );
        }
    }

    #[test]
    fn real_runs_late_half_tracks_the_truth() {
        // 後半(実時間の50%以降)は、残り時間の誤差を抑える(序盤は巡回の長さが環境で
        // 280〜870打鍵と違い、終わるまで分からないので対象外)。ATOKは巡回後のやり直しが
        // 388打鍵と短く(事前値は630)、ウォークが始まるまで約24秒多く見積もる。
        for run in &RUNS {
            let (end, rows) = replay(run);
            for &(e, _, eta) in rows.iter().filter(|r| r.0 >= 0.5 * end) {
                assert!(
                    (eta - (end - e)).abs() < 26_000.0,
                    "{}: {e}: eta {eta} vs {}",
                    run.name,
                    end - e
                );
            }
        }
    }

    #[test]
    fn real_runs_progress_and_eta_are_monotonic() {
        for run in &RUNS {
            let (_, rows) = replay(run);
            for w in rows.windows(2) {
                // 想定打鍵数は整数へ切り上げるため、割合には最大0.2%の丸め誤差がありうる。
                assert!(
                    w[1].1 + 2e-3 >= w[0].1,
                    "{}: 割合が下がった: {w:?}",
                    run.name
                );
                assert!(w[1].2 <= w[0].2 + 1e-6, "{}: 残りが増えた: {w:?}", run.name);
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
    fn stagnant_coverage_starts_the_tail() {
        // 測れたセル数が動かなくなったら(MS-IME本体: 210セル中154セルで停滞)、未測定セルが
        // 残っていても巡回は終わったとみなし、残りは末尾の枠だけになる。
        let mut e = ProgressEstimator::new();
        let mut s = snap(570, 154, 15);
        s.expected_statuses = 15;
        let before = e.expected_presses(s);
        s.presses = 570 + STAGNATION_PRESSES;
        let after = e.expected_presses(s);
        assert!(before > 570 + 1000 && after < before, "{before} -> {after}");
        assert_eq!(e.tail_started_at(), Some(570));
    }

    #[test]
    fn walk_remaining_follows_predicted_steps() {
        let mut e = ProgressEstimator::new();
        let mut s = snap(1500, 168, 12);
        s.walk = Some(WalkProgress {
            predicted: 150,
            target: 300,
            attempts: 160,
        });
        // 予測できた割合が150/160なら、残りは150×160/150... (300-150)×160/150 = 160打鍵。
        assert_eq!(e.expected_presses(s), 1500 + 160);
    }

    #[test]
    fn extra_tail_lengthens_the_remaining_walk() {
        let mut e = ProgressEstimator::new();
        e.add_extra_tail(240.0);
        let mut s = snap(1500, 168, 12);
        s.walk = Some(WalkProgress {
            predicted: 150,
            target: 300,
            attempts: 160,
        });
        assert_eq!(e.expected_presses(s), 1500 + 160 + 240);
    }

    #[test]
    fn overrunning_the_estimate_counts_down_to_a_small_floor() {
        // 実際が見積りの1.4倍かかる場合: 見積りが尽きたら残りは実時間どおり減って小さな下限
        // (FLOOR_ETA_MS)へ向かい、増えない。
        let mut lp = LinearProgress::new();
        let mut prev_eta = f64::MAX;
        for presses in (100..=3000).step_by(10) {
            let covered = (presses * 168 / 870).min(168);
            let mut s = snap(presses, covered, 12);
            s.expected_statuses = 12;
            let d = lp.update(s, f64::from(presses) * 75.0);
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
        assert!(e.expected_presses(s) > s.presses);
    }
}
