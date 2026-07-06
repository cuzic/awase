//! TSF (Text Services Framework) 状態推測システム。
//!
//! ## 4層アーキテクチャ（ADR-030）
//!
//! - [`observer`]     — Layer 1 observation: OS から生データを収集（GJI I/O, WinEvent）
//! - [`probe`]        — Layer 2 judgement: 観測データから「ready か？」「warm か？」を判定
//!   （`gji_fsm` / `composition_fsm` の判断寄り FSM もここに属する）
//! - [`output`]       — Layer 3 action: 判定結果を元に SendInput を組み立て実行
//! - [`warmup`]       — Layer 4 warmup オーケストレーション: 多段 warmup シーケンスを
//!   タイマー駆動で進め `ProbeAction` を emit（TickableFsm family / strategy）
//! - [`probe_bridge`] — メッセージループ統合: OUTPUT_GATE / WM_DRAIN_OUTPUT_QUEUE

pub(super) mod tip_detector;
mod gji_monitor;
mod win_event_obs;
pub(crate) mod composition_fsm;
pub(crate) mod gji_fsm;
pub(crate) mod ime_mode_fsm;
pub mod observer;
pub(crate) mod tsf_gate;
pub(crate) mod warmup;
pub mod output;
pub mod probe;
pub mod probe_bridge;
pub mod send;

pub use awase::gate::GateAction;
pub use tsf_gate::{
    GateEvent, GateTimer, TsfGate, TsfGateMachine, TsfGateState, TsfReadiness, WARMUP_TIMEOUT_MS,
};
