//! tick 駆動型 FSM の共通インターフェース。
//!
//! 10ms タイマー (`TIMER_TSF_PROBE`) から `tick()` が呼ばれるパターンを型として表現する。
//! [`crate::tsf::probe_fsm::TsfProbeMachine`] が現在この役割を担っており、
//! 将来の `GjiWarmupFsm` / `LiteralDetectFsm` / `ChromeProbe` もこのトレイトを実装する予定。

use crate::tsf::observer::NamechangeBaseline;
use crate::tsf::probe::LiteralDetector;
use crate::tsf::probe_fsm::{ProbeAction, TsfEnvSnapshot};
use awase::types::VkCode;

/// tick 駆動型 FSM の共通インターフェース。
pub(crate) trait TickableFsm {
    /// 1 ステップ進める。[`ProbeAction::Done`] が含まれたら完了。
    fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction>;
    /// ログ相関用の cold_seq を返す。
    fn cold_seq_hint(&self) -> u32;

    /// `dispatch_probe_actions` が呼ぶコールバック（デフォルト: no-op）
    fn forces_prepend_f2_for_extra_f2(&self) -> bool {
        false
    }
    fn apply_fresh_f2_sent(&mut self, _nc_baseline: NamechangeBaseline, _fresh_f2_ms: u64) {}
    fn apply_transmit_done(
        &mut self,
        _romaji: String,
        _ze_bs_count: usize,
        _detector: Option<LiteralDetector>,
        _literal_detect_ms: u64,
        _expected_kana: Option<char>,
    ) -> bool {
        true
    }
    fn push_deferred(&mut self, _vk: VkCode, _needs_shift: bool) {}
}
