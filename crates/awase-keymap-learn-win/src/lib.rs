//! ADR-195段階1のWindows実機ドライバ。
//!
//! `hook_monitor`/`ime_notify`はADR-196決定1bが定める「学習窓への外部からの
//! 書き込みを直接観測する」基盤（ADR196-T1）で、`driver::RealImeDriver`が
//! 所有する。

#[cfg(windows)]
mod driver;
#[cfg(windows)]
mod hook_monitor;
#[cfg(windows)]
mod ime_notify;

#[cfg(windows)]
pub use driver::RealImeDriver;
#[cfg(windows)]
pub use hook_monitor::HookMonitor;
#[cfg(windows)]
pub use ime_notify::ImeNotifyMonitor;
