//! キー効果の学習結果の読み取りと予測の核（OS 非依存。ADR-274 段階 2）。
//!
//! 「キーを押したら IME がどうなるか」の表（内蔵表・学習済みの表）と予測、キー配置の仮説を持つ。
//! `awase-ime-core` の `state/` から、分割前と同じ相対パスのまま移した。
//! `windows` crate に依存しない（`layer_boundary_guard` の `core_crates_do_not_depend_on_windows` が固定する）。

// `awase-ime-core` の `lib.rs` と同じ allow（移したコードは分割前の core の中で、この allow のもとで lint を通っていた）。
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::type_complexity
)]

// 移したファイルの中の `crate::vk::...` が、分割前と同じ書き方で解決できるようにする。
pub use awase_windows_base::vk;

pub mod state;
