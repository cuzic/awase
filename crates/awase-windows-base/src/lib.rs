//! `awase-windows` の OS 非依存な核の土台（ADR-274 段階 2）。
//!
//! 定数（`tuning`）・仮想キー（`vk`）・キーマップ表（`keymap`）・世代・時計・IME 種別・アプリ分類の型を持つ。
//! 他のどの core 系 crate にも依存しない。`windows` crate に依存しない（`layer_boundary_guard` の
//! `core_crates_do_not_depend_on_windows` が固定する）。
//!
//! `awase-windows-core` から、分割前と同じ `src/` 内の相対パスのまま移した。上位の crate は
//! `pub use awase_windows_base::...` で再公開するので、`crate::state::...`・`crate::tuning::...` の経路は変わらない。

// `awase-windows-core` の `lib.rs` と同じ allow（移したコードは分割前の core の中で、この allow のもとで lint を通っていた）。
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::type_complexity
)]

pub mod focus;
pub mod keymap;
pub mod state;
pub mod tsf;
pub mod tuning;
pub mod vk;
