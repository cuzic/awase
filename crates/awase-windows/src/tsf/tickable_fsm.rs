//! tick 駆動型 FSM の共通インターフェース。
//!
//! 10ms タイマー (`TIMER_TSF_PROBE`) から `tick()` が呼ばれるパターンを型として表現する。
//! [`crate::tsf::probe_fsm::TsfProbeMachine`] が現在この役割を担っており、
//! 将来の `GjiWarmupFsm` / `LiteralDetectFsm` / `ChromeProbe` もこのトレイトを実装する予定。

use crate::tsf::probe_fsm::{ProbeAction, TsfEnvSnapshot};

/// tick 駆動型 FSM の共通インターフェース。
pub(crate) trait TickableFsm {
    /// 1 ステップ進める。[`ProbeAction::Done`] が含まれたら完了。
    fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction>;
    /// ログ相関用の cold_seq を返す。
    fn cold_seq_hint(&self) -> u32;
}
