//! NICOLA 親指シフトエンジン
//!
//! - `Engine`: 統合エントリポイント（on_input / on_timeout / on_command）
//! - `NicolaFsm`: 同時打鍵判定 FSM（timed-fsm ベース）

mod confirm_policy;
pub mod decision;
#[allow(clippy::module_inception)]
mod engine;
mod fsm_adapter;
pub mod fsm_types;
pub mod ime_coordinator;
pub mod input_tracker;
pub mod key_lifecycle;
mod nicola_fsm;
pub mod observation;
pub mod output_history;
pub mod timing;

// Public re-exports
pub use decision::{
    Decision, Effect, EffectVec, EngineCommand, FocusEffect, ImeCacheEffect, ImeEffect,
    ImeSyncKeys, InputContext, InputEffect, KeyBuffer, SpecialKeyCombos, TimerEffect, UiEffect,
};
pub use engine::Engine;
pub use fsm_types::{
    ClassifiedEvent, EngineState, KeyClass, ModifierState, OutputRecord, OutputUpdate, ParseAction,
    PendingKey, PendingThumbData, TimerIntent,
};
pub use ime_coordinator::ImeCoordinator;
pub use key_lifecycle::KeyLifecycle;
pub use nicola_fsm::NicolaFsm;
pub use observation::{FocusObservation, ImeObservation};

pub use nicola_fsm::{TIMER_PENDING, TIMER_SPECULATIVE};
pub use timing::{ThreeKeyResult, TimingJudge};

#[cfg(test)]
mod tests;
