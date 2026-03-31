//! Windows 実装の `PlatformRuntime`。
//!
//! `Output`, `SystemTray`, `FocusDetector` および Win32 グローバル状態を束ね、
//! `PlatformRuntime` トレイトを実装する。

use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, PostMessageW, SetTimer};

use awase::platform::PlatformRuntime;
use awase::types::{FocusKind, ImeCacheState, ImeReliability, KeyAction, RawKeyEvent};

use crate::focus::cache::DetectionSource;
use crate::focus::uia::SendableHwnd;
use crate::output::Output;
use crate::runtime::FocusDetector;
use crate::tray::SystemTray;
use crate::{FOCUS_KIND, IME_RELIABILITY, IME_STATE_CACHE};

/// Windows 固有のプラットフォーム実装
pub struct WindowsPlatform {
    pub output: Output,
    pub tray: SystemTray,
    pub focus: FocusDetector,
}

impl PlatformRuntime for WindowsPlatform {
    // ── キー出力 ──

    fn send_keys(&mut self, actions: &[KeyAction]) {
        self.output.send_keys(actions);
    }

    fn reinject_key(&mut self, event: &RawKeyEvent) {
        // SAFETY: reinject_key は Win32 API (SendInput)。メインスレッドから呼ぶ。
        unsafe { crate::reinject_key(event) };
    }

    // ── タイマー ──

    fn set_timer(&mut self, id: usize, duration: Duration) {
        let ms = u32::try_from(duration.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: SetTimer は Win32 API。メインスレッドから呼ぶ。
        // HWND NULL + SetTimer は OS が独自の ID を割り当てる。
        // 戻り値をグローバルマップに保存し、WM_TIMER で逆引きする。
        unsafe {
            let os_id = SetTimer(HWND::default(), 0, ms, None);
            log::debug!("SetTimer(logical={id}, ms={ms}) → os_id={os_id}");
            crate::timer_map_set(id, os_id);
        }
    }

    fn kill_timer(&mut self, id: usize) {
        // SAFETY: KillTimer は Win32 API。メインスレッドから呼ぶ。
        unsafe {
            if let Some(os_id) = crate::timer_map_remove(id) {
                let _ = KillTimer(HWND::default(), os_id);
                log::debug!("KillTimer(logical={id}, os_id={os_id})");
            }
        }
    }

    // ── IME 制御 ──

    fn set_ime_open(&mut self, open: bool) -> bool {
        // SAFETY: set_ime_open_cross_process は Win32 API。メインスレッドから呼ぶ。
        unsafe { crate::ime::set_ime_open_cross_process(open) }
    }

    fn post_ime_refresh(&mut self) {
        // SAFETY: PostMessageW は Win32 API。メインスレッドから呼ぶ。
        unsafe {
            let _ = PostMessageW(
                HWND::default(),
                crate::WM_IME_KEY_DETECTED,
                WPARAM(0),
                LPARAM(0),
            );
        }
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
        kind.store(&FOCUS_KIND);
    }

    fn reset_ime_reliability(&mut self) {
        ImeReliability::Unknown.store(&IME_RELIABILITY);
    }

    fn insert_focus_cache(&mut self, process_id: u32, class_name: String, kind: FocusKind) {
        self.focus
            .cache
            .insert(process_id, class_name, kind, DetectionSource::Automatic);
    }

    fn request_uia_classification(&mut self) {
        if let Some(tx) = self.focus.uia_sender.as_ref() {
            use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
            let fg = unsafe { GetForegroundWindow() };
            let _ = tx.send(SendableHwnd(fg));
        }
    }

    fn update_last_focus_info(&mut self, process_id: u32, class_name: String) {
        self.focus.last_focus_info = Some((process_id, class_name));
    }

    fn save_engine_state(&mut self, process_id: u32, class_name: String, enabled: bool) {
        self.focus
            .cache
            .set_engine_state(process_id, class_name, enabled);
    }

    // ── IME キャッシュ ──

    fn update_ime_cache(&mut self, ime_on: bool) {
        let new_state = ImeCacheState::from(ime_on);
        let old_state = new_state.swap(&IME_STATE_CACHE);
        if old_state != new_state {
            log::debug!(
                "IME state cache updated: {} → {}",
                old_state.as_str(),
                new_state.as_str(),
            );
        }
    }

    fn invalidate_ime_cache(&mut self) {
        ImeCacheState::Unknown.store(&IME_STATE_CACHE);
        log::trace!("IME state cache invalidated → Unknown");
    }
}
