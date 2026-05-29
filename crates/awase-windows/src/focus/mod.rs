//! フォーカス検出モジュール
//!
//! ウィンドウのフォーカス変更を監視し、テキスト入力コントロールかどうかを判定する。
//! Phase 1-2（同期）+ Phase 3（UIA 非同期）の多段判定を行う。

pub mod cache;
pub mod class_names;
pub mod classifier;
pub mod classify;
pub mod current;
pub mod hwnd_cache;
pub mod imm_learning;
pub mod kind_classifier;
pub mod msaa;
pub mod probe;
pub mod uia;
