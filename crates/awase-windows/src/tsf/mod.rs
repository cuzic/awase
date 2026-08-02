//! TSF (Text Services Framework) 状態推測システム。
//!
//! ## 4層アーキテクチャ（ADR-030）
//!
//! - `observer`     — Layer 1 observation: OS から生データを収集（GJI I/O, WinEvent）
//! - `probe`        — Layer 2 judgement: 観測データから「ready か？」「warm か？」を判定
//!   （`gji_fsm` / `composition_fsm` の判断寄り FSM もここに属する）
//! - `output`       — Layer 3 action: 判定結果を元に SendInput を組み立て実行
//! - `warmup`       — Layer 4 warmup オーケストレーション: 多段 warmup シーケンスを
//!   タイマー駆動で進め `ProbeAction` を emit（TickableFsm family / strategy）
//! - `probe_bridge` — メッセージループ統合: OUTPUT_GATE / WM_DRAIN_OUTPUT_QUEUE
//!
//! `gji_fsm` 以外の全サブモジュールは windows crate に依存するため `#[cfg(windows)]`。
//! `gji_fsm`（TSF composition の warm/cold 状態機械）だけは windows crate 依存が
//! ゼロのため ungated にし、`cargo test -p awase-windows --lib` から Linux でも
//! 常時実行できるようにしている（ADR-082 決定1実施記録の次の一歩、BUG-33 追補3・4）。
//! 上記リストの相互参照は intra-doc link にすると非 Windows ビルドで解決できなく
//! なるためプレーンテキストにしている。

#[cfg(windows)]
pub(crate) mod composition_fsm;
// 唯一の ungated モジュール。呼び出し元（composition_fsm.rs / warmup_strategy.rs）
// は windows-gated のため非 Windows では未使用になる。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) mod gji_fsm;
#[cfg(windows)]
mod gji_monitor;
#[cfg(windows)]
pub(crate) mod ime_mode_fsm;
#[cfg(windows)]
pub mod observer;
#[cfg(windows)]
pub mod output;
#[cfg(windows)]
pub mod probe;
#[cfg(windows)]
pub mod probe_bridge;
#[cfg(windows)]
pub mod send;
#[cfg(windows)]
pub(super) mod tip_detector;
#[cfg(windows)]
pub(crate) mod tsf_gate;
#[cfg(windows)]
pub(crate) mod warmup;
#[cfg(windows)]
mod win_event_obs;

#[cfg(windows)]
pub use awase::gate::GateAction;
#[cfg(windows)]
pub use tsf_gate::{
    GateEvent, GateTimer, TsfGate, TsfGateMachine, TsfGateState, TsfReadiness, WARMUP_TIMEOUT_MS,
};
