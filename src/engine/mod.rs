//! NICOLA 親指シフトエンジン
//!
//! - `Engine`: 統合エントリポイント（on_input / on_timeout / on_command）
//! - `NicolaFsm`: 同時打鍵判定 FSM（timed-fsm ベース）

mod confirm_policy;
pub mod consecutive_counter;
pub mod conv;
pub mod decision;
#[allow(clippy::module_inception)]
mod engine;
mod fsm_adapter;
pub mod fsm_types;
pub mod idle_check;
pub mod input_tracker;
pub mod key_lifecycle;
pub mod mode_state;
mod nicola_fsm;
pub mod output_history;
pub mod timing;

// Public re-exports

pub use conv::{Charset, ConvMode};
pub use decision::{
    ActivationState, Decision, Effect, EffectVec, EngineCommand, ImeEffect, InputContext,
    InputEffect, SpecialKeyCombos, TimerEffect, UiEffect,
};
pub use engine::Engine;
pub use fsm_types::{
    ClassifiedEvent, ComposingHint, EngineState, KeyClass, ModifierState, OutputUpdate,
    ParseAction, PendingKey, PendingThumbData, TimerIntent, TIMER_PENDING, TIMER_SPECULATIVE,
};
pub use idle_check::should_run_idle_conv_check;
pub use key_lifecycle::KeyLifecycle;
pub use mode_state::{AssumedReason, InputModeState};
pub use nicola_fsm::NicolaFsm;
pub use timing::{ThreeKeyResult, TimingJudge};

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod proptest_tests;
