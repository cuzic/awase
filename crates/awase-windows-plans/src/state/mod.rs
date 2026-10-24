// 新しい crate でも、core の `state/mod.rs` と同じ `#![allow(unreachable_pub)]`（再エクスポートの連鎖を支える `pub mod`）。
#![allow(unreachable_pub)]

//! TSF とフォーカスの「計画」（Facts → Plan の純粋な判断）。ADR-274 段階 1。
//!
//! `awase-ime-core` の `state/` から、分割前と同じ `src/state/` 内の相対パスのまま移した（`git mv`、類似度 100%）。
//! 依存は `awase` と `awase-ime-core`（`focus`・`state::{TickMs, focus_gen, foreground_scope, observation_store,
//! physical_disposition}`）だけで、core のどこからも使われない葉。
//!
//! 下位の crate（`awase-ime-core`）の `state` を glob で再公開するので、移したファイルの中の
//! `super::observation_store::...` などは書き換えなくてよい。

pub use awase_ime_core::state::*;

// FCIS F6b: `output/` の TSF 送信パイプライン（warm/cold・LiteralDetect 設置）の判断の核（純粋）。
// 呼び出し元（`output/`）は `#[cfg(windows)]` のため非 Windows では未使用。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod warm_send_plan;
// FCIS F6: `output/mod.rs`・`output/vk_send.rs` の「probe/recovery 進行中は退避する」判断の核（純粋）。
// 呼び出し元（`output/`）は `#[cfg(windows)]` のため非 Windows では未使用。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod deferred_gate_plan;
pub mod raw_recovery_plan;
// FCIS F3: `runtime/executor.rs` の execute_relay/drain_deferred/defer 側の「即時/キュー/ガード」判断の核（純粋）。
// 呼び出し元（executor.rs）は `#[cfg(windows)]` のため非 Windows では未使用。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod relay_plan;
// 呼び出し元（`runtime/message_handlers.rs`）は `#[cfg(windows)]` のため、非 Windows では未使用。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod post_bypass;
// 同期分類（ウィンドウスタイル・クラス名）の判断表（純粋）。本番の呼び出し元 focus/classify.rs は #[cfg(windows)]、Linux ではテストだけが使う。
pub mod focus_classify_plan;
// 純粋関数モジュール（conv_classify と同じ ungated パターン）。唯一の呼び出し元
// runtime/key_pipeline.rs の apply_focus_probe は #[cfg(windows)] のため非 Windows
// では未使用になる。PR 109 コードレビュー指摘3: apply_focus_probe/apply_effective_ime
// に埋め込まれていた match status {...} の決定ロジックを純粋関数として抽出し、
// Linux で全数テストできるようにした。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod focus_probe_plan;
// resolve_focus_kind の前段の優先順位（純粋）。本番の呼び出し元 focus/kind_classifier.rs は #[cfg(windows)]、Linux ではテストだけが使う。
pub mod focus_resolve_plan;
pub mod focus_resync_policy;
// MSAA ロール値の表引き（純粋）。本番の呼び出し元 focus/msaa.rs は #[cfg(windows)]、Linux ではテストだけが使う。
pub mod msaa_role_plan;
// BUG-217: ペース付き送信のキュー(メッセージループを止めずに、送信の間に待ちを置く)。純粋。
// 呼び出し元(`key_pacer.rs`)は `#[cfg(windows)]` のため非 Windows では未使用。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod paced_queue;
