//! macOS イベントループ (CFRunLoop)
//!
//! 将来的に CFRunLoop + CFRunLoopTimer で CGEventTap とタイマーを統合する。
//! 現在はスタブ実装。

/// macOS イベントループ（スタブ）
#[derive(Debug)]
pub struct EventLoop;

impl EventLoop {
    pub fn new() -> Self {
        Self
    }

    /// イベントループを開始する（スタブ: 即座にリターン）
    pub fn run(&mut self) -> anyhow::Result<()> {
        log::warn!("macOS event loop not yet implemented");
        Ok(())
    }
}

impl Default for EventLoop {
    fn default() -> Self {
        Self::new()
    }
}
