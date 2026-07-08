//! フォーカス検出モジュール
//!
//! ウィンドウのフォーカス変更を監視し、テキスト入力コントロールかどうかを判定する。
//! Phase 1-2（同期）+ Phase 3（UIA 非同期）の多段判定を行う。

// ── 純粋サブモジュール（全プラットフォーム）──────────────────────────────────────
pub mod cache;
pub mod class_names;
pub mod kinds;

pub use kinds::{AppKind, FocusKind};

// ── Windows 専用サブモジュール ───────────────────────────────────────────────────
#[cfg(windows)]
pub mod classifier;
#[cfg(windows)]
pub mod classify;
#[cfg(windows)]
pub mod current;
#[cfg(windows)]
pub mod hwnd_cache;
#[cfg(windows)]
pub mod imm_learning;
#[cfg(windows)]
pub mod kind_classifier;
#[cfg(windows)]
pub mod msaa;
#[cfg(windows)]
pub mod probe;
#[cfg(windows)]
pub(crate) mod tracker;
#[cfg(windows)]
pub mod uia;
