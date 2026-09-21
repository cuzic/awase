//! 巡回戦略 S0〜S9 のオフライン比較。`cargo run --release -p awase-keymap-learn --example simulate`
//! 各モデル×条件×待ちの方式で、戦略ごとに指標(M1〜M9)を出す(乱数シード5本の平均)。

use awase_keymap_learn::anomaly::AnomalyPolicy;
use awase_keymap_learn::cost::CostModel;
use awase_keymap_learn::exec::{Executor, ReadPolicy};
use awase_keymap_learn::graph::Prior;
use awase_keymap_learn::metrics::{evaluate, mean, Metrics};
use awase_keymap_learn::model::Machine;
use awase_keymap_learn::rng::Rng;
use awase_keymap_learn::sample_models::{atok_like, synthetic};
use awase_keymap_learn::sim::{SimConfig, SimIme};
use awase_keymap_learn::strategy::{run, Req, Strategy};

const SEEDS: u64 = 5;

fn one(
    m: &Machine,
    cfg: SimConfig,
    cost: CostModel,
    prior_err: f64,
    s: Strategy,
    seed: u64,
) -> Metrics {
    let mut rng = Rng::new(seed * 1000 + 7);
    // S0 は「探索で経路を検証済み」の現状の実装を模すので、事前モデルの誤りは与えない。
    let err = if s == Strategy::S0 { 0.0 } else { prior_err };
    let prior = Prior::from_machine(m, err, &mut rng);
    let mut cfg = cfg;
    cfg.seed = seed + 1;
    let sim = SimIme::new(m.clone(), cfg, cost);
    let mut exec = Executor::new(sim, AnomalyPolicy::default(), ReadPolicy::Single);
    run(
        s,
        &mut exec,
        &prior,
        &cost,
        &m.history_suspects,
        &Req::default(),
        &mut rng,
    );
    evaluate(&exec, m)
}

fn pct(x: f64) -> String {
    if x.is_nan() {
        "  -  ".into()
    } else {
        format!("{:>4.0}%", x * 100.0)
    }
}

fn min(x: f64) -> String {
    if x.is_nan() {
        "  -  ".into()
    } else {
        format!("{:>5.1}", x / 60000.0)
    }
}

fn row(name: &str, m: &Metrics) {
    println!(
        "| {name:<20} | {} | {:>6.0} | {:>5.0} | {} | {} | {} | {:>4.1} | {:>4.1} | {:>4.1} | {} | {} | {} | {} | {} |",
        min(m.time_ms),
        m.presses,
        m.resets,
        pct(m.cov1),
        pct(m.cov2),
        min(m.t_cov2),
        m.ctx_diversity,
        m.sync_losses,
        m.anomalies,
        pct(m.nondet_detect),
        pct(m.hist_detect),
        pct(m.false_alarm),
        pct(m.accuracy),
        pct(m.false_det),
    );
}

fn header(title: &str) {
    println!("\n### {title}\n");
    println!("| 戦略 | 時間(分) | 押下 | リセット | 網羅1 | 網羅2 | 全2回まで(分) | 文脈数 | 同期喪失 | 異常 | 非決定検出 | 履歴検出 | 誤検出 | 精度 | 誤断定 |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|");
}

fn main() {
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
    let models: Vec<(String, Machine)> = vec![
        ("ATOK風(12状態・14キー・隠れ状態あり)".into(), atok_like()),
        (
            "合成A(隠れ状態3・非決定10%)".into(),
            synthetic(11, 13, 3, 0.10),
        ),
        (
            "合成B(隠れ状態1=履歴依存なし・非決定0%)".into(),
            synthetic(12, 13, 1, 0.0),
        ),
    ];
    let clean = SimConfig::default();
    let noisy = SimConfig {
        key_drop_prob: 0.02,
        obs_noise: 0.03,
        drift_hazard: 0.005,
        reset_fail_prob: 0.05,
        ..SimConfig::default()
    };
    for (name, m) in &models {
        for (cond, cfg) in [
            ("理想(異常なし)", clean),
            (
                "異常あり(キー欠落2%・観測誤り3%・ドリフト0.5%・リセット失敗5%)",
                noisy,
            ),
        ] {
            for (prof, cost) in [
                ("現状の固定待ち", CostModel::current()),
                ("イベント待ち", CostModel::event()),
            ] {
                header(&format!("{name} / {cond} / {prof} / 事前モデルの誤り5%"));
                for s in strategies {
                    let v: Vec<Metrics> = (0..SEEDS)
                        .map(|seed| one(m, cfg, cost, 0.05, s, seed))
                        .collect();
                    row(&s.name(), &mean(&v));
                }
            }
        }
    }
}
