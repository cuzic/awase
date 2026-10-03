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
//! 巡回が終わったか(=末尾へ入ったか)は、発見済みの全セルを測り終えたこと(`all_measured`)で
//! 判定する(5回の実測すべてでこれで決まった)。測れたセル数が一時的に動かなくなっても、巡回の
//! 途中なら巡回のままにする(「動かない打鍵数」で末尾と見なすと、巡回の途中で止まった環境で
//! 残りを約19秒少なく見積もり、その後ほぼ動かなくなる。Opus相談2の再現)。
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
    /// 押下の試行回数の上限(`VERIFICATION_WALK_MAX_STEPS`)。残りの見積りをこれで切る。
    pub max_attempts: u32,
    /// ウォークが終わった(以降は再測定だけが残る)。
    pub finished: bool,
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
    /// ウォークが終わった時点の打鍵数(再測定の消化を数える起点)。
    walk_done_at: Option<u32>,
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
            if w.finished {
                // ウォークが終わったら残りは再測定だけ。打鍵が進んだ分だけ消化する(消化を
                // 引かないと、再測定の間ずっと残りが減らず、割合が止まって見える)。
                let at = *self.walk_done_at.get_or_insert(s.presses);
                let left = self.extra_tail - f64::from(s.presses.saturating_sub(at));
                return (0.0, left.max(1.0));
            }
            let walk = if w.predicted > 0 {
                f64::from(w.target.saturating_sub(w.predicted)) * f64::from(w.attempts)
                    / f64::from(w.predicted)
            } else {
                VERIFY_WALK_PRESSES
            };
            // 予測できる割合が低い環境では、上限(試行回数)で打ち切られる。
            let cap = f64::from(w.max_attempts.saturating_sub(w.attempts));
            return (0.0, walk.min(cap).max(1.0) + self.extra_tail);
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
        let tail_total = TAIL_PRESSES + VERIFY_WALK_PRESSES + self.extra_tail;
        if all_measured {
            let done_at = *self.cells_done_at.get_or_insert(s.presses);
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

    fn walk(predicted: u32, attempts: u32, finished: bool) -> Option<WalkProgress> {
        Some(WalkProgress {
            predicted,
            target: 300,
            attempts,
            max_attempts: 1500,
            finished,
        })
    }

    /// 実機・CIの1回分の進捗(`tests/fixtures/*.csv`、列は cell,total,elapsed_ms,presses,statuses)と、
    /// 検証ウォークが始まった打鍵数。
    struct Run {
        name: &'static str,
        csv: &'static str,
        expected_statuses: u32,
        walk_start: u32,
    }

    const RUNS: [Run; 4] = [
        Run {
            name: "GJI(実機)",
            csv: include_str!("../tests/fixtures/progress_gji_local.csv"),
            expected_statuses: 6,
            walk_start: 1510,
        },
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

    /// 再生する1行: (測れたセル数, 経過ms, 打鍵数, 発見済みの状態数)。
    type Row = (u32, f64, u32, u32);

    fn parse(run: &Run) -> Vec<Row> {
        run.csv
            .lines()
            .filter(|l| !l.starts_with('#'))
            .map(|l| {
                let v: Vec<u32> = l.split(',').map(|x| x.parse().unwrap()).collect();
                (v[0], f64::from(v[2]), v[3], v[4])
            })
            .collect()
    }

    /// 行列を再生し、(経過ms, 割合, 残りms)の列(残り時間が出ている行だけ)と総所要時間を返す。
    /// 検証ウォークは`walk_start`〜`walk_end`打鍵で、予測できたステップが0→300へ線形に進む
    /// として作る。`walk_end`を過ぎた行は、ウォークが終わった(再測定だけが残る)状態にする。
    /// `extra`は学習後に判明した再測定の打鍵数(ウォークの前に足す)。
    fn replay_rows(
        rows: &[Row],
        expected_statuses: u32,
        walk_start: u32,
        walk_end: u32,
        extra: f64,
    ) -> (f64, Vec<(f64, f64, f64)>) {
        let mut lp = LinearProgress::new();
        lp.add_extra_tail(extra);
        let (mut end_ms, mut out) = (0.0, Vec::new());
        for &(cell, elapsed, presses, statuses) in rows {
            let walk = (presses >= walk_start).then(|| {
                let done = f64::from(presses.min(walk_end) - walk_start + 1)
                    / f64::from(walk_end - walk_start + 1);
                WalkProgress {
                    predicted: (300.0 * done).round() as u32,
                    target: 300,
                    attempts: presses.min(walk_end) - walk_start + 1,
                    max_attempts: 1500,
                    finished: presses >= walk_end,
                }
            });
            let d = lp.update(
                Snapshot {
                    presses,
                    covered_cells: cell,
                    observed_statuses: statuses,
                    keys: 14,
                    expected_statuses,
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

    fn replay(run: &Run) -> (f64, Vec<(f64, f64, f64)>) {
        let rows = parse(run);
        let walk_end = rows.last().unwrap().2;
        replay_rows(&rows, run.expected_statuses, run.walk_start, walk_end, 0.0)
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

    /// 末尾に再測定(`extra`打鍵を45ms/打鍵)を足した実行を再生する。
    fn replay_with_remeasure(run: &Run, extra: u32) -> (f64, Vec<(f64, f64, f64)>) {
        let mut rows = parse(run);
        let &(cell, mut el, mut p, statuses) = rows.last().unwrap();
        let walk_end = p;
        for _ in 0..extra / 10 {
            p += 10;
            el += 450.0;
            rows.push((cell, el, p, statuses));
        }
        replay_rows(
            &rows,
            run.expected_statuses,
            run.walk_start,
            walk_end,
            f64::from(extra),
        )
    }

    #[test]
    fn remeasure_after_the_walk_does_not_stall_the_progress() {
        // 再測定(18セル×24=432打鍵)が残る実行でも、最後の20%の時間に割合が十分進む
        // (単調性だけでは検出できない: 再測定の消化を引かないと、残りが減らず割合が
        // 87〜88%のまま約20秒止まり、最後に100%へ跳んだ)。
        for run in &RUNS {
            let (end, rows) = replay_with_remeasure(run, 432);
            let at = |t: f64| rows.iter().find(|r| r.0 >= t * end).unwrap().1;
            assert!(
                at(1.0) - at(0.8) > 0.12,
                "{}: {} -> {}",
                run.name,
                at(0.8),
                at(1.0)
            );
            let (e, f, eta) = *rows.last().unwrap();
            assert!(
                f > 0.99 && eta < 1_500.0,
                "{}: 終了時 {e} {f} {eta}",
                run.name
            );
        }
    }

    #[test]
    fn a_pause_in_the_cell_phase_is_not_mistaken_for_the_end() {
        // 巡回の途中で、測れたセル数が200打鍵(60ms/打鍵)動かなくなっても、末尾へ入ったと
        // 誤判定しない(動かない打鍵数で判定すると、残りを約19秒少なく見積もり、その後
        // 約22秒ほぼ動かなかった)。
        let run = &RUNS[1];
        let mut rows = parse(run);
        let at = rows.iter().position(|r| r.2 >= 600).unwrap();
        let (cell, mut el, mut p, statuses) = rows[at];
        let mut paused = rows[..=at].to_vec();
        for _ in 0..20 {
            p += 10;
            el += 600.0;
            paused.push((cell, el, p, statuses));
        }
        let shift_p = p - rows[at].2;
        let shift_e = el - rows[at].1;
        for r in &mut rows[at + 1..] {
            paused.push((r.0, r.1 + shift_e, r.2 + shift_p, r.3));
        }
        let walk_end = paused.last().unwrap().2;
        let (end, out) = replay_rows(
            &paused,
            run.expected_statuses,
            run.walk_start + shift_p,
            walk_end,
            0.0,
        );
        for &(e, _, eta) in out.iter().filter(|r| r.0 >= 0.5 * end) {
            assert!(
                (eta - (end - e)).abs() < 26_000.0,
                "{e}: eta {eta} vs {}",
                end - e
            );
        }
        // 停滞の後も残りは減り続ける(2秒以上動かない区間が無い)。
        for w in out.windows(2) {
            if w[0].0 >= 0.5 * end && w[1].0 - w[0].0 < 5_000.0 {
                assert!(w[1].2 < w[0].2 - 0.0 || w[1].0 - w[0].0 < 2_000.0, "{w:?}");
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
    fn walk_remaining_follows_predicted_steps() {
        let mut e = ProgressEstimator::new();
        let mut s = snap(1500, 168, 12);
        s.walk = walk(150, 160, false);
        // 予測できた割合が150/160なら、残りは(300-150)×160/150 = 160打鍵。
        assert_eq!(e.expected_presses(s), 1500 + 160);
    }

    #[test]
    fn walk_remaining_is_capped_by_the_attempt_limit() {
        // 予測できる割合が低い環境でも、残りは試行回数の上限(1500)を超えない。
        let mut e = ProgressEstimator::new();
        let mut s = snap(1500, 168, 12);
        s.walk = walk(10, 1400, false);
        assert_eq!(e.expected_presses(s), 1500 + 100);
    }

    #[test]
    fn remeasure_is_consumed_after_the_walk_finishes() {
        let mut e = ProgressEstimator::new();
        e.add_extra_tail(240.0);
        let mut s = snap(1500, 168, 12);
        s.walk = walk(150, 160, false);
        assert_eq!(e.expected_presses(s), 1500 + 160 + 240);
        s.presses = 1700;
        s.walk = walk(300, 300, true);
        assert_eq!(e.expected_presses(s), 1700 + 240);
        s.presses = 1800;
        assert_eq!(e.expected_presses(s), 1800 + 140);
        s.presses = 2000;
        assert_eq!(e.expected_presses(s), 2000 + 1);
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
