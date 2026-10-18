//! `awase-windows` の OS 非依存な核（ADR-229 D4: crate の物理分割）。
//!
//! IME 状態の判断・型・表を持ち、`windows` crate に依存しない。ファイル・OS 時計・Win32 に触る部分は
//! 殻の `awase-windows` にある。`src/` 以下の配置は分割前の `awase-windows` と同じ（ガードの相対パスの文字列が
//! 変わらないように）。`awase-windows` は `pub use` でこれらを再公開するので、`crate::state::...` などの
//! パスは分割前と同じに使える。
//!
//! `ImeStateHub`/`PlatformState`（`state/platform_state.rs`）と `state/sync_actuation.rs` は殻に残してある:
//! 記録系の `pub(crate)`（INV-A97-1）をコンパイラが強制し続けるため（所有者の決定）。

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::type_complexity
)]

pub mod focus;
pub mod journal;
pub mod journal_policy;
pub mod keymap;
pub mod state;
pub mod tsf;
pub mod tuning;
pub mod vk;
