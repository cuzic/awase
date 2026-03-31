//! Win32 タイマー管理
//!
//! `SetTimer(HWND NULL, ...)` は OS が独自の ID を割り当てるため、
//! 論理 ID（`TIMER_PENDING` 等）と OS ID のマッピングが必要。
//! この型が全てを隠蔽する。

use std::collections::HashMap;
use std::time::Duration;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

/// Win32 タイマー管理。論理 ID ⇔ OS ID のマッピングを内部に隠蔽。
pub struct Win32Timer {
    to_os: HashMap<usize, usize>,
    to_logical: HashMap<usize, usize>,
}

impl Win32Timer {
    pub fn new() -> Self {
        Self {
            to_os: HashMap::new(),
            to_logical: HashMap::new(),
        }
    }

    /// タイマーを設定する。同じ論理 ID で再度呼ぶと上書きされる。
    pub fn set(&mut self, logical_id: usize, duration: Duration) {
        let ms = u32::try_from(duration.as_millis()).unwrap_or(u32::MAX);
        let os_id = unsafe { SetTimer(HWND::default(), 0, ms, None) };
        log::debug!("Timer set: logical={logical_id}, ms={ms}, os_id={os_id}");

        // 古いマッピングがあれば OS タイマーも破棄
        if let Some(old_os) = self.to_os.insert(logical_id, os_id) {
            self.to_logical.remove(&old_os);
            unsafe {
                let _ = KillTimer(HWND::default(), old_os);
            }
        }
        self.to_logical.insert(os_id, logical_id);
    }

    /// タイマーをキャンセルする。
    pub fn kill(&mut self, logical_id: usize) {
        if let Some(os_id) = self.to_os.remove(&logical_id) {
            self.to_logical.remove(&os_id);
            unsafe {
                let _ = KillTimer(HWND::default(), os_id);
            }
            log::debug!("Timer killed: logical={logical_id}, os_id={os_id}");
        }
    }

    /// `WM_TIMER` の `wParam` から論理タイマー ID を解決する。
    pub fn resolve(&self, wparam: usize) -> Option<usize> {
        self.to_logical.get(&wparam).copied()
    }
}

impl Default for Win32Timer {
    fn default() -> Self {
        Self::new()
    }
}
