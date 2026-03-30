//! フォーカス検出モジュール
//!
//! ウィンドウのフォーカス変更を監視し、テキスト入力コントロールかどうかを判定する。
//! Phase 1-2（同期）+ Phase 3（UIA 非同期）の多段判定を行う。

pub mod cache;
pub mod classify;
pub mod msaa;
pub mod pattern;
pub mod uia;
