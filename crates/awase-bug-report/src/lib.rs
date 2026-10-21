//! 不具合報告のペイロードの組み立て・サイズの縮め・再送計画・gzip の核（OS 非依存。ADR-274 段階 1）。
//!
//! `awase-windows` の `bug_report.rs` から、分割前と同じ `src/` 内の相対パスのまま移した（`git mv`、類似度 100%）。
//! `windows` crate に依存しない（`layer_boundary_guard` の `core_crates_do_not_depend_on_windows` が固定する）。
//! 学習表のファイルを読む部分は殻の `awase-windows` の `key_effect_io::bug_report_keymap_learn_summary` にある（先行 PR #726）。
//! `awase-windows` は `awase_windows::bug_report` の経路を変えずに使える（再公開する）。

// 殻の `awase-windows` の `lib.rs` と同じ allow（移したコードは分割前の殻の中で、この allow のもとで lint を通っていた）。
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::type_complexity
)]

// 移したファイルの中の `crate::state::...`・`crate::indicator_evidence::...` が、分割前と同じ書き方で解決できるようにする。
pub use awase_windows_core::{indicator_evidence, state};

pub mod bug_report;
