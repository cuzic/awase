//! OS 観測レイヤー — Win32 API を使って IME / フォーカス状態を取得し、
//! OS 非依存の観測結果型に変換する。
//!
//! このモジュールは bin クレート（Windows 依存）に属する。
//! Engine（lib クレート）は観測結果型のみを受け取り、Win32 API を一切呼ばない。

pub mod focus_observer;
pub mod ime_observer;
