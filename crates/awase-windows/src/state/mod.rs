// pub mod が必要: lib.rs の pub use crate::state::{...} 再エクスポートチェーンを支える。
// unreachable_pub lint はこの再エクスポートパターンを認識できないため抑制する。
#![allow(unreachable_pub)]

// ── 純粋サブモジュール（全プラットフォーム）──────────────────────────────────────
pub mod belief;
pub use belief::*;

pub mod hook_state;
pub use hook_state::*;

pub(crate) mod conv_mode;
#[cfg(windows)]
pub(crate) use conv_mode::{Charset, ConvModeMgr};

pub mod app_ime_policy;
pub mod force_guard;
pub mod ime_event;
pub mod ime_model;
#[cfg(windows)]
pub(crate) use ime_model::AppliedImeState;
pub mod input_barrier;
pub mod observation_store;
pub mod transition;

// ── Windows 専用サブモジュール ───────────────────────────────────────────────────
#[cfg(windows)]
pub mod platform_state;
#[cfg(windows)]
pub use platform_state::PlatformState;

#[cfg(windows)]
pub(crate) mod ime_decision_view;
#[cfg(windows)]
pub(crate) use ime_decision_view::{ControlLog, FocusFacts, ImeControlView, ObservedState};

#[cfg(windows)]
pub mod ime_event_log;
