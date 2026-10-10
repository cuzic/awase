//! `state/` の殻側(ADR-229 D4)。純粋な部品は `awase-windows-core` にあり、ここで再公開する
//! （`crate::state::foo::Bar` のパスは分割前と同じ）。`ImeStateHub`/`PlatformState`（`platform_state`）と
//! `sync_actuation`、`ime_decision_view` は殻の crate に残す: 記録系の `pub(crate)`（INV-A97-1）を
//! コンパイラが強制し続けるため（所有者の決定、Opus 案 (d)）。

pub use awase_windows_core::state::*;

// 実機（Windows）以外では呼び出し元（`runtime/`・`app/`）が無く、未使用警告が出る。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod platform_state;
pub use platform_state::PlatformState;

// ADR-241 決定2: 同期経路の actuation の判断（`ImeController::apply` の本体・`dispatch_ime_set_open` の判断）の核。
// 本番の呼び出し元（`ime_controller.rs`・`runtime/executor.rs`）は `#[cfg(windows)]`、Linux ではテストだけが使う。
#[cfg(any(windows, test))]
pub(crate) mod sync_actuation;

#[cfg(windows)]
pub(crate) mod ime_decision_view;
#[cfg(windows)]
pub(crate) use ime_decision_view::{ControlLog, FocusFacts, ImeControlView, ObservedState};
