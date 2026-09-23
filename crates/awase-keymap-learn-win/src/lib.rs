//! ADR-195段階1のWindows実機ドライバ。

#[cfg(windows)]
mod driver;

#[cfg(windows)]
pub use driver::RealImeDriver;
