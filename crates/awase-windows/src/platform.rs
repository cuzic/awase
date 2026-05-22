//! Windows 実装の `PlatformRuntime`。
//!
//! `Output`, `SystemTray`, `AppKindClassifier`, `Win32Timer` を束ね、
//! `PlatformRuntime` トレイトを実装する。

use std::time::Duration;

use awase::platform::PlatformRuntime;
use awase::types::{KeyAction, RawKeyEvent};
use crate::output::Output;
use crate::runtime::AppKindClassifier;
use crate::timer::Win32Timer;
use crate::tray::SystemTray;

/// Windows 固有のプラットフォーム実装
#[allow(missing_debug_implementations)]
pub struct WindowsPlatform {
    pub output: Output,
    pub tray: SystemTray,
    pub focus: AppKindClassifier,
    pub timer: Win32Timer,
}

impl WindowsPlatform {
    /// TIMER_TSF_PROBE ハンドラ。pending_tsf フェーズを進め、完了したらタイマーを kill する。
    pub fn advance_tsf_probe(&mut self) {
        if self.output.advance_tsf_probe() {
            self.timer.kill(crate::TIMER_TSF_PROBE);
        }
    }
}

impl PlatformRuntime for WindowsPlatform {
    // ── キー出力 ──

    fn send_keys(&mut self, actions: &[KeyAction]) {
        self.output.send_keys(actions);
        // cold-start 時に pending_tsf が設定された場合は 10ms タイマーを起動してプローブを進める。
        if self.output.pending_tsf.borrow().is_some() {
            self.timer.set(crate::TIMER_TSF_PROBE, std::time::Duration::from_millis(10));
        }
    }

    fn reinject_key(&mut self, event: &RawKeyEvent) {
        unsafe { crate::reinject_key(event) };
    }

    // ── タイマー ──

    fn set_timer(&mut self, id: usize, duration: Duration) {
        self.timer.set(id, duration);
    }

    fn kill_timer(&mut self, id: usize) {
        self.timer.kill(id);
    }

    // ── IME ──

    fn set_ime_open(&mut self, open: bool) -> bool {
        unsafe { crate::ime::set_ime_open_cross_process(open) }
    }

    fn post_ime_refresh(&mut self) {
        // SetOpen 後の IME 状態反映に数十ms かかるため、即時ではなく
        // 統合タイマー経由で短い遅延後にリフレッシュする。
        // guard が active なら後続キーはバッファされるので安全。
        self.timer.set(
            crate::TIMER_IME_REFRESH,
            Duration::from_millis(20),
        );
    }

    // ── トレイ ──

    fn update_tray(&mut self, enabled: bool) {
        self.tray.set_enabled(enabled);
    }

    fn show_balloon(&mut self, title: &str, message: &str) {
        self.tray.show_balloon(title, message);
    }

    fn set_tray_layout_name(&mut self, name: &str) {
        self.tray.set_layout_name(name);
    }

    fn composition_output(&self) -> Option<&dyn awase::platform::CompositionOutput> {
        Some(&self.output)
    }
}
