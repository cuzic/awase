//! フォーカスの計画のうち、HWND ごとの IME 状態のキャッシュ（ADR-274 段階 1）。
//!
//! `focus/` は複数の crate にまたがる。下位の crate（`awase-windows-core`）の `focus` を glob で再公開し、
//! `hwnd_cache` だけをこの crate が持つ。

pub use awase_windows_core::focus::*;

pub mod hwnd_cache;
