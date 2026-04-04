//! Windows 実装の `PlatformRuntime`。
//!
//! `Output`, `SystemTray`, `FocusDetector`, `Win32Timer` を束ね、
//! `PlatformRuntime` トレイトを実装する。

use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use awase::platform::PlatformRuntime;
use awase::types::{FocusKind, KeyAction, RawKeyEvent};

use crate::focus::cache::DetectionSource;
use crate::focus::uia::SendableHwnd;
use crate::output::Output;
use crate::runtime::FocusDetector;
use crate::timer::Win32Timer;
use crate::tray::SystemTray;

/// Windows 固有のプラットフォーム実装
pub struct WindowsPlatform {
    pub output: Output,
    pub tray: SystemTray,
    pub focus: FocusDetector,
    pub timer: Win32Timer,
}

impl PlatformRuntime for WindowsPlatform {
    // ── キー出力 ──

    fn send_keys(&mut self, actions: &[KeyAction]) {
        self.output.send_keys(actions);
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
            std::time::Duration::from_millis(20),
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

    // ── フォーカス ──

    fn update_focus_kind(&mut self, kind: FocusKind) {
        unsafe {
            if let Some(app) = crate::APP.get_mut() {
                app.platform_state.focus_kind = kind;
            }
        }
    }

    fn insert_focus_cache(&mut self, process_id: u32, class_name: String, kind: FocusKind) {
        self.focus
            .cache
            .insert(process_id, class_name, kind, DetectionSource::Automatic);
    }

    fn request_uia_classification(&mut self) {
        if let Some(ref sender) = self.focus.uia_sender {
            use windows::Win32::UI::WindowsAndMessaging::{GetGUIThreadInfo, GUITHREADINFO};
            let mut info = GUITHREADINFO {
                cbSize: size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            let hwnd = if unsafe { GetGUIThreadInfo(0, &raw mut info) }.is_ok() {
                info.hwndFocus
            } else {
                HWND::default()
            };
            let _ = sender.send(SendableHwnd(hwnd));
        }
    }

    fn update_last_focus_info(&mut self, process_id: u32, class_name: String) {
        self.focus.last_focus_info = Some((process_id, class_name));
    }

}
