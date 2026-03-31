//! macOS 固有のプラットフォーム実装クレート。
//!
//! CGEventTap によるキーボードフック、CGEventPost による出力、
//! TIS による IME 制御など、macOS API 依存コードを集約する。

pub mod event_loop;
pub mod hook;
pub mod ime;
pub mod output;
pub mod scanmap;
pub mod tray;
pub mod vk;

// Future modules:
// (none remaining)
