// core の `state/mod.rs` と同じ `#![allow(unreachable_pub)]`（再エクスポートの連鎖を支える `pub mod`）。
#![allow(unreachable_pub)]

//! キー効果の学習結果の読み取りと予測（ADR-274 段階 2）。
//!
//! `awase-windows-core` の `state/` から、分割前と同じ `src/state/` 内の相対パスのまま移した（`git mv`、類似度 100%）。
//! 依存は `awase`・`awase-windows-base`・`awase-gji-config`・`awase-keymap-learn` だけ。
//!
//! 下位の crate（`awase-windows-base`）の `state` を glob で再公開するので、移したファイルの中の
//! `crate::state::ime_kind::...` などは書き換えなくてよい。

pub use awase_windows_base::state::*;

pub mod key_effect_predictor;
pub mod key_effect_rules;
pub mod key_effect_runtime;
pub mod key_effect_table;
// ADR-195 段階0。key_effect_predictor（経路1）・awase-gji-config::keymap（経路2）・
// msime_key_assignment（経路3、windows専用だが呼び出しはruntime層が担う）の出力を
// 統合する薄い集約層。ungated（純粋関数のみで、レジストリ等のI/Oはしない）。
pub mod keymap_initial_hypothesis;
// 純粋関数モジュール（conv_classify と同じ ungated パターン）。唯一の呼び出し元
// runtime/message_handlers.rs::deliver_key_event は #[cfg(windows)] のため
// 非 Windows では未使用になる。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod keymap_latch;
pub mod late_keymap_plan;
pub mod state_dependent_key_warning;
