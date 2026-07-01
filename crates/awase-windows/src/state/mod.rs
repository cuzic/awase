// pub mod が必要: lib.rs の pub use crate::state::{...} 再エクスポートチェーンを支える。
// unreachable_pub lint はこの再エクスポートパターンを認識できないため抑制する。
#![allow(unreachable_pub)]

// ── TickMs ─────────────────────────────────────────────────────────────────────

/// `GetTickCount64` 由来のミリ秒タイムスタンプを表すニュータイプ。
///
/// state/ 層が `hook::current_tick_ms()` を直接呼び出す代わりに、
/// 呼び出し元（runtime 層）からタイムスタンプを注入するために使う。
/// これにより state/ が hook 実装に依存しない純粋な型になる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct TickMs(pub u64);

impl TickMs {
    /// `self - base` を飽和演算で計算して返す。
    #[must_use]
    pub const fn saturating_sub(self, base: u64) -> u64 {
        self.0.saturating_sub(base)
    }
}

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
