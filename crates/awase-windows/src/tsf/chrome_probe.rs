//! Chrome IME 向け cold-start warmup probe。
//!
//! [`TsfProbeMachine::new_chrome`] を [`TickableFsm`] トレイト経由で使えるようにラップする。

use crate::tsf::probe::{LiteralDetector, TsfReadinessProbe};
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::probe_fsm::{ProbeAction, TsfEnvSnapshot, TsfProbeMachine};
use crate::tsf::tickable_fsm::TickableFsm;
use awase::types::VkCode;

pub(crate) struct ChromeProbe(TsfProbeMachine);

impl ChromeProbe {
    pub(crate) fn new(
        romaji: &str,
        cold_seq: u32,
        probe: TsfReadinessProbe,
        total_max_ms: u64,
        guard: OutputActiveGuard,
    ) -> Self {
        Self(TsfProbeMachine::new_chrome(romaji, cold_seq, probe, total_max_ms, guard))
    }
}

impl TickableFsm for ChromeProbe {
    fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        self.0.tick(env)
    }

    fn cold_seq_hint(&self) -> u32 {
        self.0.cold_seq_hint()
    }

    // forces_prepend_f2_for_extra_f2 / apply_fresh_f2_sent は GjiWarmupCoro 専用。
    // TsfProbeMachine はデフォルト（false / no-op）を返すため委譲不要。

    fn apply_transmit_done(
        &mut self,
        romaji: String,
        ze_bs_count: usize,
        detector: Option<LiteralDetector>,
        literal_detect_ms: u64,
        expected_kana: Option<char>,
    ) -> bool {
        self.0.apply_transmit_done(romaji, ze_bs_count, detector, literal_detect_ms, expected_kana)
    }

    fn push_deferred(&mut self, vk: VkCode, needs_shift: bool) {
        self.0.push_deferred(vk, needs_shift);
    }
}
