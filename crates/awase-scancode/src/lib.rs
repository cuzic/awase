//! Windows Scancode Map（キー入れ替え）の判断・編集モデル・表示文言・適用の核（OS 非依存。ADR-274 段階 1）。
//!
//! `awase-windows` の `scancode_*.rs` から、分割前と同じ `src/` 内の相対パスのまま移した（`git mv`、類似度 100%）。
//! 外部の crate に依存しない。レジストリ I/O は殻の `awase-windows` の `scancode_registry.rs`（`#[cfg(windows)]`）にある。
//! `awase-windows` は `awase_windows::scancode_*` の経路を変えずに使える（再公開する）。

// 殻の `awase-windows` の `lib.rs` と同じ allow（移したコードは分割前の殻の中で、この allow のもとで lint を通っていた）。
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::type_complexity
)]

pub mod scancode_apply;
pub mod scancode_diagram;
pub mod scancode_editor;
pub mod scancode_map;
pub mod scancode_pairs;
