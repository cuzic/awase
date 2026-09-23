//! 実行器: `SimIme`(実機のドライバに当たる)を、異常方針・読み取り方針つきで動かし、観測を表に記録し、時間・件数を数える。

use std::collections::{HashMap, HashSet};

use crate::anomaly::{Anomaly, AnomalyPolicy, AnomalyTracker, ResetLevel};
use crate::model::{Outcome, Status};
use crate::sim::{PressReport, SimIme};
use crate::table::Table;

/// status読み取りの方針(観測経路を2つ使う頻度)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadPolicy {
    /// 1経路だけ読む。
    Single,
    /// 各statusを初めて見たときだけ2経路で読んで照合する(文献のR2)。
    DoubleFirst,
    /// 毎回2経路で読む。
    DoubleAlways,
}

/// 1回の押下の結果。
#[derive(Debug, Clone, Copy)]
pub struct PressInfo {
    pub before: Status,
    pub outcome: Outcome,
}

/// 実行の統計。
#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub presses: u32,
    pub resets: u32,
    pub reads: u32,
    pub retries: u32,
    pub sync_losses: u32,
    pub forced_resets: u32,
    pub anomalies: HashMap<Anomaly, u32>,
    /// 押下ごとの (経過ms, 1回以上測ったセル数, 2回以上測ったセル数)。
    pub timeline: Vec<(f64, usize, usize)>,
}

/// IMEへの注入・観測・待機を内包するドライバ。
pub trait ImeDriver {
    fn press(&mut self, key: usize) -> PressReport;
    fn press_setup(&mut self, key: usize);
    fn read_primary(&mut self) -> Status;
    fn read_secondary(&mut self) -> Status;
    fn reread_status(&mut self) -> Status;
    fn settle_setup(&mut self) -> Status;
    fn reset(&mut self, level: ResetLevel) -> bool;
    fn elapsed_ms(&self) -> f64;
    fn machine_initial_status(&self) -> Status;
}

/// 実行器。
#[derive(Debug)]
pub struct Executor<D: ImeDriver = SimIme> {
    pub driver: D,
    pub table: Table,
    pub stats: Stats,
    tracker: AnomalyTracker,
    read: ReadPolicy,
    recording: bool,
    cur: Option<Status>,
    last_key: Option<usize>,
    run_len: usize,
    max_run: Option<usize>,
    seen: HashSet<Status>,
    initial: Status,
}

impl<D: ImeDriver> Executor<D> {
    pub fn new(driver: D, policy: AnomalyPolicy, read: ReadPolicy) -> Self {
        let initial = driver.machine_initial_status();
        Self {
            driver,
            table: Table::new(),
            stats: Stats::default(),
            tracker: AnomalyTracker::new(policy),
            read,
            recording: true,
            cur: None,
            last_key: None,
            run_len: 0,
            max_run: None,
            seen: HashSet::new(),
            initial,
        }
    }

    pub fn set_read_policy(&mut self, r: ReadPolicy) {
        self.read = r;
    }

    pub fn set_recording(&mut self, on: bool) {
        self.recording = on;
    }

    /// 1巡の長さの上限(R7)。連続して押した数がこれに達したら強制リセットが要る。
    pub fn set_max_run(&mut self, n: Option<usize>) {
        self.max_run = n;
    }

    pub fn elapsed_ms(&self) -> f64 {
        self.driver.elapsed_ms()
    }

    pub const fn last_key(&self) -> Option<usize> {
        self.last_key
    }

    pub const fn current(&self) -> Option<Status> {
        self.cur
    }

    fn note_anomaly(&mut self, a: Anomaly) {
        *self.stats.anomalies.entry(a).or_default() += 1;
    }

    fn should_double_read(&self, primary: Status) -> bool {
        match self.read {
            ReadPolicy::Single => false,
            ReadPolicy::DoubleAlways => true,
            ReadPolicy::DoubleFirst => !self.seen.contains(&primary),
        }
    }

    /// 2経路の読み取り結果 `(a, b)` を方針に従って1つに決める。
    fn resolve(&mut self, a: Status, b: Status) -> Status {
        let double = self.should_double_read(a);
        self.stats.reads += 1;
        let mut out = a;
        if double {
            self.stats.reads += 1;
            if a != b {
                self.note_anomaly(Anomaly::ChannelMismatch);
                let c = self.driver.reread_status();
                self.stats.reads += 1;
                out = if c == a || c == b { c } else { a };
            }
        }
        self.seen.insert(out);
        out
    }

    /// 現在のstatusを読む。
    pub fn read_status(&mut self) -> Status {
        let a = self.driver.read_primary();
        let b = if self.should_double_read(a) {
            self.driver.read_secondary()
        } else {
            a
        };
        let s = self.resolve(a, b);
        self.cur = Some(s);
        s
    }

    /// 強制リセットが要るか(異常が窓内で多い、または1巡の上限)。
    pub fn should_reset(&self) -> bool {
        self.tracker.should_force_reset() || self.max_run.is_some_and(|m| self.run_len >= m)
    }

    /// キー `key` を押し、押下前後のstatusと行方を読んで記録する。キーが届かなかったときは再試行し、それでも届かなければ `None`。
    pub fn press(&mut self, key: usize) -> Option<PressInfo> {
        let before = self.read_status();
        let mut report = None;
        let retries = self.tracker.policy().max_press_retries;
        for attempt in 0..=retries {
            let r = self.driver.press(key);
            if r.delivered {
                report = Some(r);
                break;
            }
            self.note_anomaly(Anomaly::KeyNotDelivered);
            self.tracker.note(true);
            if attempt < retries {
                self.stats.retries += 1;
            }
        }
        let r = report?;
        let flag_before = self.stats.anomalies.get(&Anomaly::ChannelMismatch).copied();
        let after = self.resolve(r.seen.status, r.seen_b);
        let flag_after = self.stats.anomalies.get(&Anomaly::ChannelMismatch).copied();
        self.tracker.note(flag_before != flag_after);
        let outcome = Outcome {
            status: after,
            disp: r.seen.disp,
        };
        if self.recording {
            self.table.record(before, key, self.last_key, outcome);
        }
        self.cur = Some(after);
        self.last_key = Some(key);
        self.run_len += 1;
        self.stats.presses += 1;
        self.stats.timeline.push((
            self.driver.elapsed_ms(),
            self.table.covered1(),
            self.table.covered2(),
        ));
        Some(PressInfo { before, outcome })
    }

    /// S0用: 状態を作るための押下(観測も記録もしない)。コストは経路のキー間隔だけ。
    pub fn press_setup(&mut self, key: usize) {
        self.driver.press_setup(key);
        self.last_key = Some(key);
    }

    /// S0用: 経路を打ち終えた後の待ちと検証(statusを読む)。
    ///
    /// `driver.settle_setup()`は単一チャネルの読み取りで、`read_status()`とは違い
    /// `ReadPolicy`(Double系)の二重読み取り・`seen`集合の更新を経由しない。
    /// 現状の呼び出し元は`strategy::s0`(常に`ReadPolicy::Single`)だけなので実害は
    /// 無いが、将来Double系ポリシーの戦略がS0を再利用すると、二重読み取りが
    /// 黙って行われなくなる(レビュー指摘)。その組み合わせを早期に検知する。
    pub fn settle_setup(&mut self) -> Status {
        debug_assert_eq!(
            self.read,
            ReadPolicy::Single,
            "settle_setup() は単一チャネル読み取りのみ対応(ReadPolicy::Double*と組み合わせない)"
        );
        let status = self.driver.settle_setup();
        self.cur = Some(status);
        status
    }

    /// リセット(段階的に昇格しながら、初期のstatusに戻ったことを読んで確かめる)。
    ///
    /// `driver.reset(level)`のbool(実機では`settle()`込みの自己申告)だけでなく、
    /// 毎回`self.read_status()`を無条件で呼ぶ(旧`(f64, bool)`版の不変条件を維持する
    /// ためのレビュー指摘対応)。`ok`だけを見て読み取りを省略すると、`SimIme`側の
    /// `ok`は観測に基づかない確率的な値なので、`stats.reads`/コストが
    /// シナリオ(特に`reset_fail_prob`>0)ごとに黙って変わってしまう
    /// (ADR-195が要求する「シミュレータ比較実験に影響しない」という不変条件に反する)。
    pub fn reset(&mut self) {
        let mut level = self.tracker.policy().first_reset;
        for _ in 0..6 {
            let ok = self.driver.reset(level);
            self.stats.resets += 1;
            self.last_key = None;
            let s = self.read_status();
            if ok && s == self.initial {
                break;
            }
            self.note_anomaly(Anomaly::ResetFailed);
            level = level.next().unwrap_or(ResetLevel::Hard);
        }
        self.cur = Some(self.initial);
        self.run_len = 0;
        self.tracker.clear();
    }

    /// 期待と違う状態に出たときに数える(同期喪失)。
    pub fn note_sync_loss(&mut self) {
        self.stats.sync_losses += 1;
        self.note_anomaly(Anomaly::UnexpectedStatus);
        self.tracker.note(true);
    }

    pub fn note_forced_reset(&mut self) {
        self.stats.forced_resets += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost::CostModel;
    use crate::sample_models::{atok_keys, atok_like};
    use crate::sim::SimConfig;

    fn exec(cfg: SimConfig) -> Executor<SimIme> {
        Executor::new(
            SimIme::new(atok_like(), cfg, CostModel::event()),
            AnomalyPolicy::default(),
            ReadPolicy::Single,
        )
    }

    #[test]
    fn press_records_before_and_after() {
        let mut e = exec(SimConfig::default());
        e.reset();
        let info = e.press(atok_keys::HANKAKU).expect("届く");
        assert!(info.before.open && !info.outcome.status.open);
        assert_eq!(e.table.covered1(), 1);
        assert!(e.elapsed_ms() > 0.0);
    }

    #[test]
    fn undelivered_key_is_retried_then_reported() {
        let mut e = exec(SimConfig {
            key_drop_prob: 1.0,
            ..SimConfig::default()
        });
        e.reset();
        assert!(e.press(0).is_none());
        assert_eq!(e.stats.retries, 1);
        assert_eq!(e.table.covered1(), 0);
    }

    #[test]
    fn reset_recovers_even_when_the_first_level_fails() {
        let mut e = exec(SimConfig {
            reset_fail_prob: 0.9,
            seed: 3,
            ..SimConfig::default()
        });
        e.press(atok_keys::HANKAKU);
        e.reset();
        assert_eq!(e.current(), Some(e.driver.machine().initial_status()));
        assert!(e.stats.resets >= 1);
    }

    #[test]
    fn double_read_detects_channel_mismatch_and_resolves() {
        let mut e = exec(SimConfig {
            obs_noise: 0.3,
            seed: 9,
            ..SimConfig::default()
        });
        e.set_read_policy(ReadPolicy::DoubleAlways);
        let truth = e.driver.machine().initial_status();
        let mut wrong = 0;
        for _ in 0..200 {
            if e.read_status() != truth {
                wrong += 1;
            }
        }
        // 単一読みの誤り率(約0.3)より大きく減る。
        assert!(wrong < 40, "wrong={wrong}");
        assert!(
            e.stats
                .anomalies
                .get(&Anomaly::ChannelMismatch)
                .copied()
                .unwrap_or(0)
                > 0
        );
    }

    #[test]
    fn double_always_read_charges_for_both_channels_when_no_mismatch() {
        let mut e = exec(SimConfig::default());
        e.set_read_policy(ReadPolicy::DoubleAlways);
        e.reset();
        let before = e.elapsed_ms();
        for _ in 0..10 {
            e.read_status();
        }
        let read_ms = CostModel::event().read_ms;
        assert!((e.elapsed_ms() - before - 10.0 * 2.0 * read_ms).abs() < 1e-6);
    }

    #[test]
    fn max_run_forces_a_reset_request() {
        let mut e = exec(SimConfig::default());
        e.set_max_run(Some(2));
        e.reset();
        e.press(atok_keys::HIRAGANA);
        assert!(!e.should_reset());
        e.press(atok_keys::HIRAGANA);
        assert!(e.should_reset());
    }
}
