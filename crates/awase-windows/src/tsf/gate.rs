//! TsfGate — `awase::tsf` の re-export。
//!
//! 実装本体は `awase::tsf` に移動済み（Linux でもテストが走る）。
//! このモジュールは既存の `crate::tsf::gate::*` 参照を維持するための薄い再エクスポート。

pub use awase::tsf::{
    GateAction, GateEvent, GateTimer, TsfGate, TsfGateMachine, TsfGateState, WARMUP_TIMEOUT_MS,
};
