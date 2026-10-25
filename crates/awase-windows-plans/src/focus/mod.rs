//! フォーカスの計画のうち、HWND ごとの IME 状態のキャッシュ（ADR-274 段階 1）。
//!
//! `focus/` は複数の crate にまたがる。下位の crate（`awase-ime-core`）の `focus` を glob で再公開し、
//! `hwnd_cache` だけをこの crate が持つ。

pub use awase_ime_core::focus::*;

pub mod hwnd_cache;

// BUG-218: TsfNative 窓へ入るときの、HWND キャッシュの ON 復元の判断（純粋）。
pub mod tsf_cache_restore;
