//! `awase-settings` の wgpu フォールバックが使う `wgpu` / `egui_wgpu` の再公開。
//!
//! 中身は無い。`awase-settings` がこの crate を optional に依存することで、
//! PR の CI は `--no-default-features` で wgpu 一式のコンパイルを省ける。
pub use egui_wgpu;
pub use wgpu;
