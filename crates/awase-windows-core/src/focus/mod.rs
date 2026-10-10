//! フォーカス分類の純粋な部品（殻の `awase-windows::focus` が再公開する）。

pub mod class_names;
pub mod hwnd_cache;
pub mod kinds;

pub use kinds::{AppKind, FocusChangedAxes, FocusKind};
