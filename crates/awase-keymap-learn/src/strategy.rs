//! 戦略 S0〜S9(文献調査の(c))の**定義の正**(READMEはここを参照する)。どれも同じ `Executor` と事前モデル(`Prior`)を使い、各セルを `k` 回測る(S7〜S9は矛盾したセルなどを増やす)。
//!
//! - S0 現状: 試行ごとにリセット→キー列で状態を作る→押す(作る間の押下は記録しない)。
//! - S1 ランダムウォーク: 確率 `restart` でリセット。
//! - S2 貪欲: 最寄りの未測定セルへ最短経路で移動。
//! - S3 有向CPP: 全セル `k` 回を覆う最小コストの巡回(計画が外れたら、残りで計画し直す)。
//! - S4 rural CPP: 1周目(k=1)の後、2周目は必須辺だけを別の順序で(経路を変えて)巡る。
//! - S5 status付きtour: S3 + 各statusを初めて見たときだけ2経路で読む(R2)。
//! - S6 部分1-switch: 直前キーが疑わしいキー集合のときだけ、文脈つきの拡大グラフで巡る。
//! - S7 適応: S5を別経路で2周し、矛盾したセルだけ `adaptive_n` 回まで増やす。
//! - S8 S6+適応: 文脈つきグラフでS6の巡回をした後、矛盾したセルだけ `adaptive_n` 回まで増やす。
//! - S9 全セルn回: 1周ごとに巡回の順序を変えて(別経路)、全セルの観測回数を1回ずつ増やし `adaptive_n` 回まで。

use crate::cost::CostModel;
use crate::exec::{Executor, PressInfo, ReadPolicy};
use crate::graph::{cpp_plan, EdgeKind, Graph, Prior};
use crate::rng::Rng;

/// 戦略。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Strategy {
    S0,
    S1 { restart: f64 },
    S2,
    S3,
    S4,
    S5,
    S6,
    S7,
    S8,
    S9,
}

impl Strategy {
    pub fn name(self) -> String {
        match self {
            Self::S0 => "S0 現状(毎回リセット)".into(),
            Self::S1 { restart } => format!("S1 ランダム(p={restart})"),
            Self::S2 => "S2 貪欲".into(),
            Self::S3 => "S3 有向CPP".into(),
            Self::S4 => "S4 rural CPP".into(),
            Self::S5 => "S5 status付きtour".into(),
            Self::S6 => "S6 部分1-switch".into(),
            Self::S7 => "S7 適応(S5+矛盾セル)".into(),
            Self::S8 => "S8 S6+適応".into(),
            Self::S9 => "S9 全セルn回(別経路)".into(),
        }
    }
}

/// 要求。
#[derive(Debug, Clone, Copy)]
pub struct Req {
    /// 各セルの目標観測回数。
    pub k: u32,
    /// 打ち切りの予算(ms)。
    pub budget_ms: f64,
    /// 押下数の上限(暴走防止)。
    pub max_presses: u32,
    /// S7が矛盾したセルに対して目指す観測回数。
    pub adaptive_n: u32,
}

impl Default for Req {
    fn default() -> Self {
        Self {
            k: 2,
            budget_ms: 60.0 * 60.0 * 1000.0,
            max_presses: 20_000,
            adaptive_n: 12,
        }
    }
}

fn over(exec: &Executor, req: &Req) -> bool {
    exec.elapsed_ms() > req.budget_ms || exec.stats.presses >= req.max_presses
}

/// 戦略を実行する。`suspects` は履歴依存が疑われるキーの添字(S6・S7の部分アルファベット)。
pub fn run(
    strategy: Strategy,
    exec: &mut Executor,
    prior: &Prior,
    cost: &CostModel,
    suspects: &[usize],
    req: &Req,
    rng: &mut Rng,
) {
    match strategy {
        Strategy::S0 => {
            let g = Graph::build(prior, &[], cost);
            s0(exec, &g, req);
        }
        Strategy::S1 { restart } => {
            let g = Graph::build(prior, &[], cost);
            s1(exec, &g, req, rng, restart);
        }
        Strategy::S2 => {
            let mut g = Graph::build(prior, &[], cost);
            s2(exec, &mut g, req);
        }
        Strategy::S3 => {
            let mut g = Graph::build(prior, &[], cost);
            tour(exec, &mut g, req, rng, false, |e, g| need_full(e, g, req.k));
        }
        Strategy::S4 => {
            let mut g = Graph::build(prior, &[], cost);
            tour(exec, &mut g, req, rng, false, |e, g| need_full(e, g, 1));
            tour(exec, &mut g, req, rng, true, |e, g| need_full(e, g, req.k));
        }
        Strategy::S5 => {
            exec.set_read_policy(ReadPolicy::DoubleFirst);
            let mut g = Graph::build(prior, &[], cost);
            tour(exec, &mut g, req, rng, false, |e, g| need_full(e, g, req.k));
        }
        Strategy::S6 => {
            let mut g = Graph::build(prior, suspects, cost);
            tour(exec, &mut g, req, rng, false, |e, g| {
                need_ctx(e, g, req.k, suspects)
            });
        }
        Strategy::S7 => {
            exec.set_read_policy(ReadPolicy::DoubleFirst);
            let mut g = Graph::build(prior, &[], cost);
            tour(exec, &mut g, req, rng, false, |e, g| need_full(e, g, 1));
            tour(exec, &mut g, req, rng, true, |e, g| need_full(e, g, req.k));
            let n = req.adaptive_n;
            tour(exec, &mut g, req, rng, true, move |e, g| {
                need_adaptive(e, g, n)
            });
        }
        Strategy::S9 => {
            exec.set_read_policy(ReadPolicy::DoubleFirst);
            let mut g = Graph::build(prior, &[], cost);
            for pass in 1..=req.adaptive_n {
                tour(exec, &mut g, req, rng, pass > 1, move |e, g| {
                    need_full(e, g, pass)
                });
            }
        }
        Strategy::S8 => {
            exec.set_read_policy(ReadPolicy::DoubleFirst);
            let mut g = Graph::build(prior, suspects, cost);
            tour(exec, &mut g, req, rng, false, |e, g| {
                need_ctx(e, g, req.k, suspects)
            });
            let n = req.adaptive_n;
            tour(exec, &mut g, req, rng, true, move |e, g| {
                need_adaptive(e, g, n)
            });
        }
    }
}

/// セル(status, キー)を満たす節点。文脈つきグラフでは、初期節点から到達できる最寄りの文脈の節点。
fn pick_node(g: &Graph, si: usize, key: usize) -> Option<usize> {
    (0..g.n_ctx)
        .map(|c| si * g.n_ctx + c)
        .filter(|n| g.edge_exists(*n, key) && g.reachable_from_initial(*n))
        .min_by_key(|n| g.dist(g.initial_node, *n))
}

fn edge_cells(g: &Graph) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    for si in 0..g.statuses.len() {
        for key in 0..g.n_keys {
            if let Some(node) = pick_node(g, si, key) {
                v.push((node, key));
            }
        }
    }
    v
}

fn need_full(exec: &Executor, g: &Graph, k: u32) -> Vec<u32> {
    let mut need = vec![0u32; g.n_nodes * g.n_keys];
    for (node, key) in edge_cells(g) {
        let c = exec.table.count(g.status_of_node(node), key) as u32;
        need[node * g.n_keys + key] = k.saturating_sub(c);
    }
    need
}

fn need_adaptive(exec: &Executor, g: &Graph, n: u32) -> Vec<u32> {
    let mut need = vec![0u32; g.n_nodes * g.n_keys];
    for (node, key) in edge_cells(g) {
        let s = g.status_of_node(node);
        if exec.table.class(s, key).declared_not_det() {
            let c = exec.table.count(s, key) as u32;
            need[node * g.n_keys + key] = n.saturating_sub(c);
        }
    }
    need
}

/// S6: 疑わしいキー(`suspects`)のセルは、文脈(直前キーが疑わしいキーのどれか/どれでもない)ごとに1回。他のセルは `k` 回
/// (文脈のうち、初期節点から到達できる最寄りの節点で満たす)。
fn need_ctx(exec: &Executor, g: &Graph, k: u32, suspects: &[usize]) -> Vec<u32> {
    let mut need = vec![0u32; g.n_nodes * g.n_keys];
    for si in 0..g.statuses.len() {
        let s = g.statuses[si];
        for key in 0..g.n_keys {
            if suspects.contains(&key) {
                for c in 0..g.n_ctx {
                    let node = si * g.n_ctx + c;
                    if !g.edge_exists(node, key) || !g.reachable_from_initial(node) {
                        continue;
                    }
                    let have = exec
                        .table
                        .observations(s, key)
                        .iter()
                        .filter(|o| ctx_id(g, o.ctx) == c)
                        .count() as u32;
                    need[node * g.n_keys + key] = 1u32.saturating_sub(have);
                }
            } else {
                let best = pick_node(g, si, key);
                if let Some(node) = best {
                    let have = exec.table.count(s, key) as u32;
                    need[node * g.n_keys + key] = k.saturating_sub(have);
                }
            }
        }
    }
    need
}

fn ctx_id(g: &Graph, last_key: Option<usize>) -> usize {
    last_key
        .and_then(|k| g.ctx_keys.iter().position(|c| *c == k))
        .map_or(0, |p| p + 1)
}

/// 観測の多数派の結果でグラフの辺を直す。
fn learn(exec: &Executor, g: &mut Graph, info: PressInfo, key: usize) {
    if let Some(maj) = exec.table.majority(info.before, key) {
        g.learn_edge(info.before, key, maj);
    }
}

fn cur_node(exec: &Executor, g: &Graph) -> Option<usize> {
    g.node_of(exec.current()?, exec.last_key())
}

enum Step {
    Done,
    Mismatch,
}

/// 計画を実行する。期待と違う状態に出た・キーが届かない・強制リセットが要る、のいずれかで `Mismatch`(計画し直す)。
fn execute(exec: &mut Executor, g: &mut Graph, plan: &[EdgeKind], req: &Req) -> Step {
    for k in plan {
        if over(exec, req) {
            return Step::Done;
        }
        if exec.should_reset() && matches!(k, EdgeKind::Press { .. }) {
            exec.note_forced_reset();
            exec.reset();
            return Step::Mismatch;
        }
        match *k {
            EdgeKind::Reset { .. } => exec.reset(),
            EdgeKind::Press { node, key } => {
                if cur_node(exec, g) != Some(node) {
                    exec.note_sync_loss();
                    return Step::Mismatch;
                }
                let Some(info) = exec.press(key) else {
                    return Step::Mismatch;
                };
                learn(exec, g, info, key);
                let after = g.node_of(info.outcome.status, exec.last_key());
                if after != Some(g.kind_to(*k)) {
                    exec.note_sync_loss();
                    return Step::Mismatch;
                }
            }
        }
    }
    Step::Done
}

fn tour(
    exec: &mut Executor,
    g: &mut Graph,
    req: &Req,
    rng: &mut Rng,
    shuffle: bool,
    need_fn: impl Fn(&Executor, &Graph) -> Vec<u32>,
) {
    if exec.current().is_none() {
        exec.reset();
    }
    for _ in 0..3000 {
        if over(exec, req) {
            return;
        }
        let need = need_fn(exec, g);
        if need.iter().all(|n| *n == 0) {
            return;
        }
        let mut start = cur_node(exec, g).unwrap_or(g.initial_node);
        if !g.reachable_from_initial(start) {
            // 事前モデルに無い節点(履歴依存で実際にだけ現れる状態)に居る。リセットして初期から計画する。
            exec.reset();
            start = g.initial_node;
        }
        let Some(plan) = cpp_plan(g, &need, start, rng, shuffle) else {
            return;
        };
        if matches!(execute(exec, g, &plan, req), Step::Done) && need_fn(exec, g) == need {
            return; // 進まなかった(必須辺に到達できない等)
        }
    }
}

fn s0(exec: &mut Executor, g: &Graph, req: &Req) {
    for (node, key) in edge_cells(g) {
        let s = g.status_of_node(node);
        let mut attempts = 0;
        while (exec.table.count(s, key) as u32) < req.k && attempts < req.k + 3 {
            if over(exec, req) {
                return;
            }
            attempts += 1;
            exec.reset();
            for kind in g.path(g.initial_node, node) {
                if let EdgeKind::Press { key, .. } = kind {
                    exec.press_setup(key);
                }
            }
            let st = exec.settle_setup();
            if st != s {
                exec.note_sync_loss();
                continue;
            }
            exec.set_recording(true);
            let _ = exec.press(key);
        }
    }
}

fn s1(exec: &mut Executor, g: &Graph, req: &Req, rng: &mut Rng, restart: f64) {
    exec.reset();
    for _ in 0..req.max_presses {
        if over(exec, req) {
            return;
        }
        let done = edge_cells(g)
            .iter()
            .all(|(n, k)| exec.table.count(g.status_of_node(*n), *k) as u32 >= req.k);
        if done {
            return;
        }
        if rng.chance(restart) || exec.should_reset() {
            exec.reset();
            continue;
        }
        let key = rng.below(g.n_keys);
        let _ = exec.press(key);
    }
}

fn s2(exec: &mut Executor, g: &mut Graph, req: &Req) {
    exec.reset();
    for _ in 0..req.max_presses {
        if over(exec, req) {
            return;
        }
        let need = need_full(exec, g, req.k);
        let Some(cur) = cur_node(exec, g) else {
            exec.reset();
            continue;
        };
        // 未測定セルを持つ最寄りの節点。
        let mut best: Option<(i64, usize)> = None;
        for node in 0..g.n_nodes {
            if (0..g.n_keys).any(|k| need[node * g.n_keys + k] > 0) {
                let d = g.dist(cur, node);
                if best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, node));
                }
            }
        }
        let Some((_, target)) = best else { return };
        for kind in g.path(cur, target) {
            match kind {
                EdgeKind::Reset { .. } => exec.reset(),
                EdgeKind::Press { key, .. } => {
                    let Some(info) = exec.press(key) else {
                        break;
                    };
                    learn(exec, g, info, key);
                }
            }
            if over(exec, req) {
                return;
            }
        }
        if cur_node(exec, g) == Some(target) {
            if let Some(key) = (0..g.n_keys).find(|k| need[target * g.n_keys + k] > 0) {
                if let Some(info) = exec.press(key) {
                    learn(exec, g, info, key);
                }
            }
        } else {
            exec.note_sync_loss();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anomaly::AnomalyPolicy;
    use crate::metrics::evaluate;
    use crate::sample_models::atok_like;
    use crate::sim::{SimConfig, SimIme};

    fn run_strategy(
        s: Strategy,
        cfg: SimConfig,
        cost: CostModel,
    ) -> (Executor, crate::metrics::Metrics) {
        let m = atok_like();
        let mut rng = Rng::new(11);
        let prior = Prior::from_machine(&m, 0.0, &mut rng);
        let suspects = m.history_suspects.clone();
        let sim = SimIme::new(m.clone(), cfg, cost);
        let mut exec = Executor::new(sim, AnomalyPolicy::default(), ReadPolicy::Single);
        run(
            s,
            &mut exec,
            &prior,
            &cost,
            &suspects,
            &Req::default(),
            &mut rng,
        );
        let met = evaluate(&exec, &m);
        (exec, met)
    }

    #[test]
    fn every_strategy_covers_all_cells_without_noise() {
        let strategies = [
            Strategy::S0,
            Strategy::S1 { restart: 0.05 },
            Strategy::S2,
            Strategy::S3,
            Strategy::S4,
            Strategy::S5,
            Strategy::S6,
            Strategy::S7,
            Strategy::S8,
            Strategy::S9,
        ];
        for s in strategies {
            let (_e, m) = run_strategy(s, SimConfig::default(), CostModel::event());
            assert!(
                m.cov1 >= 0.999,
                "{}: cov1={} presses={}",
                s.name(),
                m.cov1,
                m.presses
            );
        }
    }

    #[test]
    fn tour_uses_far_fewer_presses_than_the_current_method() {
        let (_, s0) = run_strategy(Strategy::S0, SimConfig::default(), CostModel::event());
        let (_, s3) = run_strategy(Strategy::S3, SimConfig::default(), CostModel::event());
        assert!(
            s3.time_ms < s0.time_ms,
            "s3={} s0={}",
            s3.time_ms,
            s0.time_ms
        );
    }

    #[test]
    fn tour_survives_anomalies_and_still_covers() {
        let cfg = SimConfig {
            key_drop_prob: 0.05,
            obs_noise: 0.02,
            drift_hazard: 0.01,
            reset_fail_prob: 0.1,
            seed: 4,
            ..SimConfig::default()
        };
        let (e, m) = run_strategy(Strategy::S7, cfg, CostModel::event());
        assert!(m.cov1 > 0.95, "cov1={}", m.cov1);
        assert!(e.stats.sync_losses > 0 || !e.stats.anomalies.is_empty());
    }
}
