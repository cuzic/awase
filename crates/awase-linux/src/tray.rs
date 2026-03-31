//! Linux トレイアイコン (StatusNotifierItem)
//!
//! 将来的に ksni クレートまたは D-Bus 直接実装で StatusNotifierItem プロトコルを
//! サポートする。現在はログ出力のみのスタブ実装。

/// Linux システムトレイ（スタブ実装）
#[derive(Debug)]
pub struct SystemTray {
    enabled: bool,
}

impl SystemTray {
    /// トレイアイコンを作成する（スタブ: ログ出力のみ）
    pub fn new() -> Self {
        log::info!("System tray: stub implementation (StatusNotifierItem not yet implemented)");
        Self { enabled: true }
    }

    /// エンジン状態を更新する
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        log::info!("Tray: engine {}", if enabled { "ON" } else { "OFF" });
    }

    /// バルーン通知を表示する（スタブ）
    pub fn show_balloon(&self, title: &str, message: &str) {
        log::info!("Tray balloon: {title}: {message}");
    }

    /// レイアウト名を設定する（スタブ）
    pub fn set_layout_name(&self, name: &str) {
        log::info!("Tray: layout = {name}");
    }
}

impl Default for SystemTray {
    fn default() -> Self {
        Self::new()
    }
}
