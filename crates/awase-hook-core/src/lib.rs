//! フック・修飾キー・半角英数トグルの状態の核（OS 非依存。ADR-274 段階 1）。
//!
//! `awase-ime-core` の `state/` から、分割前と同じ `src/state/` 内の相対パスのまま移した。
//! `windows` crate に依存しない（`layer_boundary_guard` の `core_crates_do_not_depend_on_windows` が固定する）。
//! `awase-windows` は `crate::state::...` の経路を変えずに使う（`state/mod.rs` で再公開する）。

// `awase-ime-core` の `lib.rs` と同じ allow（移したコードは分割前の core の中で、この allow のもとで lint を通っていた）。
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::type_complexity
)]

// 移したファイルの中の `crate::vk::...` が、分割前と同じ書き方で解決できるようにする。
pub use awase_ime_core::vk;

pub mod state;
