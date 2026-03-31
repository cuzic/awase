//! macOS メニューバーアイコン (NSStatusBar)
//!
//! 将来的に NSStatusBar + NSStatusItem でメニューバーにアイコンを表示する。

/// macOS システムトレイ（スタブ実装）
#[derive(Debug)]
pub struct SystemTray {
    enabled: bool,
}

impl SystemTray {
    pub fn new() -> Self {
        log::info!("Menu bar icon: stub (NSStatusBar not yet implemented)");
        Self { enabled: true }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        log::info!("Tray: engine {}", if enabled { "ON" } else { "OFF" });
    }

    pub fn show_balloon(&self, title: &str, message: &str) {
        log::info!("Notification: {title}: {message}");
    }

    pub fn set_layout_name(&self, name: &str) {
        log::info!("Tray: layout = {name}");
    }
}

impl Default for SystemTray {
    fn default() -> Self {
        Self::new()
    }
}
