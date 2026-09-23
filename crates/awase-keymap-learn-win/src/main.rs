#[cfg(windows)]
mod app {
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
        run(
            strategy,
            &mut executor,
            &prior,
            &cost,
            &model.history_suspects,
            &Req::default(),
            &mut rng,
        );
        println!(
            "strategy={} elapsed_ms={:.0} presses={} cells={}",
            strategy.name(),
            executor.elapsed_ms(),
            executor.stats.presses,
            executor.table.covered1()
        );
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
