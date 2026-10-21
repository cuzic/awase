//! TSF とフォーカスの「計画」の核（OS 非依存。ADR-274 段階 1）。
//!
//! `awase-ime-core` の `state/`・`focus/` から、分割前と同じ相対パスのまま移した。
//! `windows` crate に依存しない（`layer_boundary_guard` の `core_crates_do_not_depend_on_windows` が固定する）。
//! `awase-windows` は `crate::state::...`・`crate::focus::...` の経路を変えずに使う。

// `awase-ime-core` の `lib.rs` と同じ allow（移したコードは分割前の core の中で、この allow のもとで lint を通っていた）。
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::type_complexity
)]

// 移したファイルの中の `crate::tuning::...` が、分割前と同じ書き方で解決できるようにする。
pub use awase_ime_core::tuning;

pub mod focus;
pub mod state;
