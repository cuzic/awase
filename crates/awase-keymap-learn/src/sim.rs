//! `SimIme`: 実機のIMEの代わり。statusは読めるが隠れ状態は返さない。遅延・キー欠落・観測不一致・非決定・ドリフトを注入できる。

use crate::anomaly::ResetLevel;
use crate::cost::{CostModel, WaitModel};
use crate::model::{Disposition, Machine, Outcome, Status};
use crate::rng::Rng;

/// 注入する異常・ゆらぎ。
#[derive(Debug, Clone, Copy)]
pub struct SimConfig {
    pub seed: u64,
    /// 注入したキーがアプリに届かない確率。
    pub key_drop_prob: f64,
    /// status読み取り(各観測経路)が嘘を返す確率。
    pub obs_noise: f64,
    /// 1押下あたり、隠れ状態が勝手に別の状態へ飛ぶ確率(同期喪失。idleや長時間経過の代理)。
    pub drift_hazard: f64,
    /// 基準のリセット失敗確率。
    pub reset_fail_prob: f64,
    /// 変化の遅延(中央値ms、対数の標準偏差)。
    pub latency_median_ms: f64,
    pub latency_sigma: f64,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            seed: 1,
            key_drop_prob: 0.0,
            obs_noise: 0.0,
            drift_hazard: 0.0,
            reset_fail_prob: 0.0,
            latency_median_ms: 60.0,
            latency_sigma: 0.5,
        }
    }
}

/// 1回の押下の報告。
#[derive(Debug, Clone, Copy)]
pub struct PressReport {
    /// アプリ側の打鍵イベントで、キーが届いたと確認できたか。
    pub delivered: bool,
    pub cost_ms: f64,
    /// 経路Aで読んだ結果(確定前に読んだ場合は押下前のstatus・行方なし)。
    pub seen: Outcome,
    /// 経路Bで読んだstatus。
    pub seen_b: Status,
}

/// シミュレートしたIME。
#[derive(Debug, Clone)]
pub struct SimIme {
    machine: Machine,
    reach: Vec<usize>,
    statuses: Vec<Status>,
    cur: usize,
    rng: Rng,
    cfg: SimConfig,
    cost: CostModel,
}

impl SimIme {
    pub fn new(machine: Machine, cfg: SimConfig, cost: CostModel) -> Self {
        let reach_flags = machine.reachable();
        let reach: Vec<usize> = reach_flags
            .iter()
            .enumerate()
            .filter_map(|(i, r)| r.then_some(i))
            .collect();
        let statuses = machine.statuses();
        let cur = machine.initial;
        Self {
            machine,
            reach,
            statuses,
            cur,
            rng: Rng::new(cfg.seed),
            cfg,
            cost,
        }
    }

    pub const fn machine(&self) -> &Machine {
        &self.machine
    }

    pub const fn cost(&self) -> &CostModel {
        &self.cost
    }

    /// テスト用: 真の現在状態。
    pub const fn true_state(&self) -> usize {
        self.cur
    }

    fn noisy_status(&mut self, truth: Status) -> Status {
        if self.rng.chance(self.cfg.obs_noise) && self.statuses.len() > 1 {
            loop {
                let s = self.statuses[self.rng.below(self.statuses.len())];
                if s != truth {
                    return s;
                }
            }
        }
        truth
    }

    /// 現在のstatusを2つの観測経路(A,B)で読む。
    pub fn read_status(&mut self) -> (Status, Status) {
        let truth = self.machine.states[self.cur].status;
        (self.noisy_status(truth), self.noisy_status(truth))
    }

    /// 経路Aだけで再読み取りする。
    pub fn reread_status(&mut self) -> Status {
        let truth = self.machine.states[self.cur].status;
        self.noisy_status(truth)
    }

    /// キー `key_idx` を押す。
    pub fn press(&mut self, key_idx: usize) -> PressReport {
        if self.rng.chance(self.cfg.drift_hazard) && !self.reach.is_empty() {
            self.cur = self.reach[self.rng.below(self.reach.len())];
        }
        let before = self.machine.states[self.cur].status;
        let wait_ms = match self.cost.wait {
            WaitModel::Fixed { ms } => ms,
            WaitModel::Event { nochange_ms, .. } => nochange_ms,
        };
        if self.rng.chance(self.cfg.key_drop_prob) {
            let s = self.noisy_status(before);
            let sb = self.noisy_status(before);
            return PressReport {
                delivered: false,
                cost_ms: self.cost.press_ms + wait_ms,
                seen: Outcome {
                    status: s,
                    disp: Disposition::None,
                },
                seen_b: sb,
            };
        }
        // 分岐を確率で選ぶ。
        let branches = &self.machine.states[self.cur].trans[key_idx];
        let mut x = self.rng.f64();
        let mut chosen = branches.last().copied();
        for b in branches {
            if x < b.p {
                chosen = Some(*b);
                break;
            }
            x -= b.p;
        }
        let b = chosen.expect("遷移の分岐が空");
        let after = self.machine.states[b.next].status;
        let changed = after != before || b.disp != Disposition::None;
        let latency = self
            .rng
            .lognormal(self.cfg.latency_median_ms, self.cfg.latency_sigma);
        let (cost_ms, settled) = match self.cost.wait {
            WaitModel::Fixed { ms } => (self.cost.press_ms + ms, !(changed && latency > ms)),
            WaitModel::Event {
                quiet_ms,
                nochange_ms,
                timeout_ms,
            } => {
                if changed {
                    (
                        self.cost.press_ms + latency.min(timeout_ms) + quiet_ms,
                        latency <= timeout_ms,
                    )
                } else {
                    (self.cost.press_ms + nochange_ms, true)
                }
            }
        };
        self.cur = b.next;
        let (vis_status, vis_disp) = if settled {
            (after, b.disp)
        } else {
            (before, Disposition::None) // 確定前に読んだ古い状態
        };
        let s = self.noisy_status(vis_status);
        let sb = self.noisy_status(vis_status);
        PressReport {
            delivered: true,
            cost_ms,
            seen: Outcome {
                status: s,
                disp: vis_disp,
            },
            seen_b: sb,
        }
    }

    /// リセットを試みる(段階 `level`)。戻り値は (かかった時間ms, 効いたか)。
    pub fn reset(&mut self, level: ResetLevel) -> (f64, bool) {
        let cost = self.cost.reset_ms * level.cost_factor();
        let ok = self
            .rng
            .chance(level.success_prob(self.cfg.reset_fail_prob));
        if ok {
            self.cur = self.machine.initial;
        }
        (cost, ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample_models::atok_like;

    fn sim(cfg: SimConfig) -> SimIme {
        SimIme::new(atok_like(), cfg, CostModel::event())
    }

    #[test]
    fn same_seed_gives_same_results() {
        let run = |seed| {
            let mut s = sim(SimConfig {
                seed,
                obs_noise: 0.1,
                ..SimConfig::default()
            });
            (0..30)
                .map(|i| {
                    let r = s.press(i % 5);
                    (r.delivered, r.seen.status.open, s.true_state())
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(run(5), run(5));
    }

    #[test]
    fn reads_hide_the_hidden_state_but_report_status() {
        let mut s = sim(SimConfig::default());
        let (a, b) = s.read_status();
        assert_eq!(a, b);
        assert_eq!(a, s.machine().initial_status());
    }

    #[test]
    fn key_drop_reports_undelivered_and_keeps_state() {
        let mut s = sim(SimConfig {
            key_drop_prob: 1.0,
            ..SimConfig::default()
        });
        let before = s.true_state();
        let r = s.press(0);
        assert!(!r.delivered);
        assert_eq!(s.true_state(), before);
    }

    #[test]
    fn short_fixed_wait_misses_slow_changes() {
        let mut cost = CostModel::current();
        cost.wait = WaitModel::Fixed { ms: 1.0 };
        let mut s = SimIme::new(atok_like(), SimConfig::default(), cost);
        // 変化を起こすキーで、確定前の古い状態を読む(statusが変わらない)。
        let mut stale = 0;
        for _ in 0..30 {
            s.reset(ResetLevel::Hard);
            let r = s.press(0);
            let after = s.machine().states[s.true_state()].status;
            if r.seen.status != after {
                stale += 1;
            }
        }
        assert!(stale > 0);
    }

    #[test]
    fn reset_returns_to_initial_when_it_succeeds() {
        let mut s = sim(SimConfig::default());
        s.press(0);
        let (_, ok) = s.reset(ResetLevel::Mode);
        assert!(ok);
        assert_eq!(s.true_state(), s.machine().initial);
    }
}
