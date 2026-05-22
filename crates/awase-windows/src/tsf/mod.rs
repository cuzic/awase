//! TSF (Text Services Framework) 状態推測システム。
//!
//! ## 3層アーキテクチャ
//!
//! - [`observer`]     — observation 層: OS から生データを収集（GJI I/O, WinEvent）
//! - [`probe`]        — judgement 層: 観測データから「ready か？」「warm か？」を判定
//! - [`output`]       — action 層: 判定結果を元に SendInput を組み立て実行
//! - [`probe_bridge`] — メッセージループ統合: OUTPUT_ACTIVE / WM_DRAIN_OUTPUT_QUEUE
//! - [`cold_warmup`]  — cold-start ウォームアップシーケンス（Preamble/Eager/Non-eager 分解）

pub(crate) mod belief;
pub(crate) mod cold_warmup;
pub mod gate;
pub mod observer;
pub mod output;
pub mod probe;
pub mod probe_bridge;
pub(crate) mod send;
