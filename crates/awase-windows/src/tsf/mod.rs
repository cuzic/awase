//! TSF (Text Services Framework) 状態推測システム。
//!
//! ## 3層アーキテクチャ
//!
//! - [`observer`]     — observation 層: OS から生データを収集（GJI I/O, WinEvent）
//! - [`probe`]        — judgement 層: 観測データから「ready か？」「warm か？」を判定
//! - [`output`]       — action 層: 判定結果を元に SendInput を組み立て実行
//! - [`probe_bridge`] — メッセージループ統合: OUTPUT_GATE / WM_DRAIN_OUTPUT_QUEUE
//! - [`cold_warmup`]  — cold-start ウォームアップシーケンス（Preamble/Eager/Non-eager 分解）

pub(crate) mod cold_warmup;
pub(crate) mod gji_fsm;
pub mod observer;
pub mod output;
pub mod probe;
pub mod probe_bridge;
pub(crate) mod probe_fsm;
pub mod send;

pub use awase::gate::GateAction;
pub use awase::tsf::{
    GateEvent, GateTimer, TsfGate, TsfGateMachine, TsfGateState, WARMUP_TIMEOUT_MS,
};
