//! IBus / Fcitx5 の IME バックエンド検出と状態取得。
//!
//! 初期実装では `dbus-send` コマンドを使い、同期的に D-Bus サービスの
//! 存在確認を行う。実際の IME 状態取得はスタブとし、将来 zbus による
//! 非同期実装に置き換える想定。

use std::process::Command;

/// 検出された IME バックエンドの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeBackend {
    /// IBus が動作中。
    IBus,
    /// Fcitx5 が動作中。
    Fcitx5,
    /// IME バックエンドが見つからない。
    None,
}

/// セッションバス上の D-Bus サービスが存在するか `dbus-send` で確認する。
fn dbus_service_exists(service_name: &str) -> bool {
    Command::new("dbus-send")
        .args([
            "--session",
            "--dest=org.freedesktop.DBus",
            "--type=method_call",
            "--print-reply",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus.NameHasOwner",
            &format!("string:{service_name}"),
        ])
        .output()
        .map(|out| {
            // 応答に "boolean true" が含まれればサービスが存在する
            let stdout = String::from_utf8_lossy(&out.stdout);
            out.status.success() && stdout.contains("boolean true")
        })
        .unwrap_or(false)
}

/// セッションバス上の IBus / Fcitx5 サービスを探し、見つかった
/// バックエンドを返す。両方存在する場合は IBus を優先する。
pub fn detect_ime_backend() -> ImeBackend {
    if dbus_service_exists("org.freedesktop.IBus") {
        return ImeBackend::IBus;
    }
    if dbus_service_exists("org.fcitx.Fcitx5") {
        return ImeBackend::Fcitx5;
    }
    ImeBackend::None
}

/// IME が ON かどうかを問い合わせる。
///
/// 現時点ではスタブ実装であり、常に `None` を返す。
/// 将来的には `zbus` を使った非同期版に置き換える。
pub fn query_ime_state(backend: &ImeBackend) -> Option<bool> {
    match backend {
        ImeBackend::IBus => {
            log::debug!("IBus IME state query: not yet implemented");
            None
        }
        ImeBackend::Fcitx5 => {
            log::debug!("Fcitx5 IME state query: not yet implemented");
            None
        }
        ImeBackend::None => None,
    }
}

/// IME バックエンドの検出と状態問い合わせをまとめた構造体。
#[derive(Debug)]
pub struct ImeDetector {
    backend: ImeBackend,
}

impl ImeDetector {
    /// IME バックエンドを自動検出して `ImeDetector` を生成する。
    #[must_use]
    pub fn new() -> Self {
        let backend = detect_ime_backend();
        log::info!("IME backend: {backend:?}");
        Self { backend }
    }

    /// 検出済みのバックエンドを返す。
    #[must_use]
    pub const fn backend(&self) -> ImeBackend {
        self.backend
    }

    /// IME が ON かどうかを返す。判定できない場合は `None`。
    #[must_use]
    pub fn is_ime_on(&self) -> Option<bool> {
        query_ime_state(&self.backend)
    }
}

impl Default for ImeDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_backend_returns_none() {
        assert_eq!(query_ime_state(&ImeBackend::None), None);
    }

    #[test]
    fn detector_default_works() {
        // CI 環境では D-Bus が無いことが多いが、パニックしないことを確認
        let det = ImeDetector::default();
        let _ = det.is_ime_on();
        let _ = det.backend();
    }
}
