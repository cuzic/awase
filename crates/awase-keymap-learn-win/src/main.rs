#[cfg(windows)]
mod app {
    use std::io::Write;

    use awase_keymap_learn::anomaly::AnomalyPolicy;
    use awase_keymap_learn::cost::CostModel;
    use awase_keymap_learn::exec::{Executor, ReadPolicy};
    use awase_keymap_learn::graph::Prior;
    use awase_keymap_learn::rng::Rng;
    use awase_keymap_learn::sample_models::atok_like;
    use awase_keymap_learn::strategy::{run, Req, Strategy};
    use awase_keymap_learn_win::RealImeDriver;

    const KEYS: [u32; 14] = [
        0x1D, 0x1C, 0xF2, 0xF1, 0xF0, 0xF3, 0x19, 0x16, 0x1A, 0x1B, 0x0D, 0x20, 0x08, 0x41,
    ];

    /// ADR-195段階6: 何押下ごとに標準出力へ進捗行を書き出すか。毎回書くと
    /// 子プロセス側(awase-settings)のパース負荷・パイプI/Oが無駄に増えるため間引く。
    const PROGRESS_EVERY_N_PRESSES: u32 = 10;

    pub fn run_main() -> windows::core::Result<()> {
        let strategy = if std::env::args().any(|arg| arg == "--strategy=s0") {
            Strategy::S0
        } else {
            Strategy::S6
        };
        let driver = RealImeDriver::new(KEYS.to_vec())?;
        let initial = driver.initial_status();
        let mut model = atok_like();
        for state in &mut model.states {
            // ヒューリスティックな初期仮説として、抽象mode 0/1を実機のConv値0x09/0x00へ対応づける。
            state.status.mode = if state.status.mode == 0 { 0x09 } else { 0x00 };
        }
        if let Some(index) = model
            .states
            .iter()
            .position(|state| state.status == initial)
        {
            model.initial = index;
        }

        let mut rng = Rng::new(195);
        let prior = Prior::from_machine(&model, 0.0, &mut rng);
        let cost = CostModel::event();
        let mut executor = Executor::new(driver, AnomalyPolicy::default(), ReadPolicy::Single);

        // ADR-195段階6: 進捗(現在何セル目/推定残り時間)を標準出力へ運ぶ。
        // awase-settings(較正ウィザード)はこの行をパースしてUI表示する。IPCは
        // 使わない(ペイロードが1ワード固定で表本体を運べないため、詳細はADR
        // 本文「段階6」節参照)。表本体はここでは一切標準出力へ出さない。
        let total_cells = model.states.len() as u32 * KEYS.len() as u32;
        executor.set_progress_sink(move |stats, table| {
            if stats.presses % PROGRESS_EVERY_N_PRESSES != 0 {
                return;
            }
            let cell = table.covered1() as u32;
            let elapsed_ms = stats.timeline.last().map_or(0.0, |&(ms, _, _)| ms);
            // 経過時間からの単純な線形外挿。0除算・未進捗時はeta不明(-1)を返す。
            let eta_ms = if cell == 0 || cell >= total_cells {
                -1.0
            } else {
                elapsed_ms / f64::from(cell) * f64::from(total_cells - cell)
            };
            println!(
                "progress cell={cell} total={total_cells} elapsed_ms={elapsed_ms:.0} eta_ms={eta_ms:.0}"
            );
            let _ = std::io::stdout().flush();
        });

        run(
            strategy,
            &mut executor,
            &prior,
            &cost,
            &model.history_suspects,
            &Req::default(),
            &mut rng,
        );
        let decode_errors = executor.driver.decode_error_count();
        println!(
            "result status={} strategy={} elapsed_ms={:.0} presses={} cells={} total={} decode_errors={}",
            if decode_errors == 0 { "success" } else { "success_with_warnings" },
            strategy.name(),
            executor.elapsed_ms(),
            executor.stats.presses,
            executor.table.covered1(),
            total_cells,
            decode_errors
        );
        let _ = std::io::stdout().flush();
        if decode_errors > 0 {
            eprintln!(
                "警告: observe_imm失敗によるフォールバックが{decode_errors}回発生。学習表に信頼できない観測が混じっている可能性がある。"
            );
        }
        Ok(())
    }
}

#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    app::run_main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("awase-keymap-learn-win is Windows-only");
}
