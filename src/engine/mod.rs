//! NICOLA 親指シフトエンジン
//!
//! - `Engine`: 統合エントリポイント（on_input / on_timeout / on_command）
//! - `NicolaFsm`: 同時打鍵判定 FSM（timed-fsm ベース）

pub mod decision;
#[allow(clippy::module_inception)]
mod engine;
pub mod fsm_types;
pub mod input_tracker;
mod nicola_fsm;
pub mod observation;
pub mod output_history;

// Public re-exports
pub use decision::{
    Decision, Effect, EngineCommand, FocusEffect, ImeCacheEffect, ImeEffect, ImeSyncKeys,
    InputContext, InputEffect, KeyBuffer, SpecialKeyCombos, TimerEffect, UiEffect,
};
pub use engine::Engine;
pub use fsm_types::{
    ClassifiedEvent, EngineState, FinalizePlan, KeyClass, ModifierState, OutputRecord,
    OutputUpdate, PendingKey, PendingThumbData, TimerIntent,
};
pub use nicola_fsm::NicolaFsm;
pub use observation::{FocusObservation, ImeObservation};

pub use nicola_fsm::{TIMER_PENDING, TIMER_SPECULATIVE};

#[cfg(test)]
mod tests;
