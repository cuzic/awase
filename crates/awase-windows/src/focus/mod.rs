//! フォーカス検出モジュール
//!
//! ウィンドウのフォーカス変更を監視し、テキスト入力コントロールかどうかを判定する。
//! Phase 1-2（同期）+ Phase 3（UIA 非同期）の多段判定を行う。

// ── 純粋サブモジュール（全プラットフォーム）──────────────────────────────────────
pub mod cache;
pub mod current;
// ADR-272 / ECRS F3: 学習ストアは Win32 を呼ばない。`classifier.rs`(Windows 専用)から分けて Linux でもテストする。
// 利用側(tracker・bootstrap)が Windows 専用なので、Linux では未使用の項目が出る。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod learning_store;
pub mod thread_scope;

pub use awase_windows_core::focus::{class_names, kinds, AppKind, FocusChangedAxes, FocusKind};
// `hwnd_cache` は `awase-windows-plans`（`focus/` は複数の crate にまたがる。ADR-274 段階 1）。
pub use awase_windows_plans::focus::hwnd_cache;

// ── Windows 専用サブモジュール ───────────────────────────────────────────────────
#[cfg(windows)]
pub mod classifier;
#[cfg(windows)]
pub mod classify;
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
