//! TSF (Text Services Framework) 状態推測システム。
//!
//! ## 3層アーキテクチャ
//!
//! - [`observer`]  — observation 層: OS から生データを収集（GJI I/O, WinEvent）
//! - [`probe`]     — judgement 層: 観測データから「ready か？」「warm か？」を判定
//! - [`output`]    — action 層: 判定結果を元に SendInput を組み立て実行
//! - [`probe_bridge`] — メッセージループ統合: PROBE_ACTIVE / PROBE_KEY_QUEUE

pub mod observer;
pub mod output;
pub mod probe;
pub mod probe_bridge;
