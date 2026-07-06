//! フォーカス／アプリ種別の分類 enum（Windows 固有）。
//!
//! `AppKind`（UI フレームワーク種別）と `FocusKind`（コントロール種別）は
//! フォーカス中のアプリ／コントロールに応じて出力方式を適応的に切り替えるために使う。
//! いずれも `#[repr(u8)]` で `AtomicU8` に load/store できる。

/// アプリケーションの UI フレームワーク種別
///
/// フォーカス中のアプリに応じて出力方式を適応的に切り替えるために使用する。
/// - Win32: ローマ字送信（デフォルト）
/// - Chrome: VK キーストローク送信（KEYEVENTF_UNICODE だと全角→半角変換される問題の回避）
/// - Uwp: Unicode 直接送信（VK キーストロークが正しく処理されない場合がある）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AppKind {
    /// クラシック Win32 / WinForms アプリ
    Win32 = 0,
    /// TSF ネイティブアプリ（Chrome, Edge, VS Code, Electron, WezTerm 等）
    TsfNative = 1,
    /// UWP / XAML / DirectUI アプリ
    Uwp = 2,
}

impl AppKind {
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Win32,
            1 => Self::TsfNative,
            _ => Self::Uwp,
        }
    }

    pub fn load(atomic: &std::sync::atomic::AtomicU8) -> Self {
        Self::from_u8(atomic.load(std::sync::atomic::Ordering::Acquire))
    }

    pub fn store(self, atomic: &std::sync::atomic::AtomicU8) {
        atomic.store(self as u8, std::sync::atomic::Ordering::Release);
    }
}

/// フォーカス中コントロールの種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FocusKind {
    /// テキスト入力コントロール（エンジン処理を許可）
    TextInput = 0,
    /// 非テキストコントロール（エンジンをバイパス）
    NonText = 1,
    /// 判定不能
    Undetermined = 2,
}

impl FocusKind {
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::TextInput,
            1 => Self::NonText,
            _ => Self::Undetermined,
        }
    }

    pub fn load(atomic: &std::sync::atomic::AtomicU8) -> Self {
        Self::from_u8(atomic.load(std::sync::atomic::Ordering::Acquire))
    }

    pub fn store(self, atomic: &std::sync::atomic::AtomicU8) {
        atomic.store(self as u8, std::sync::atomic::Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── AppKind ──

    #[test]
    fn app_kind_from_u8_known_values() {
        assert_eq!(AppKind::from_u8(0), AppKind::Win32);
        assert_eq!(AppKind::from_u8(1), AppKind::TsfNative);
        assert_eq!(AppKind::from_u8(2), AppKind::Uwp);
    }

    #[test]
    fn app_kind_from_u8_fallback() {
        assert_eq!(AppKind::from_u8(255), AppKind::Uwp);
    }

    #[test]
    fn app_kind_load_store_roundtrip() {
        let atomic = std::sync::atomic::AtomicU8::new(0);
        AppKind::TsfNative.store(&atomic);
        assert_eq!(AppKind::load(&atomic), AppKind::TsfNative);
    }

    // ── FocusKind ──

    #[test]
    fn focus_kind_from_u8_known_values() {
        assert_eq!(FocusKind::from_u8(0), FocusKind::TextInput);
        assert_eq!(FocusKind::from_u8(1), FocusKind::NonText);
        assert_eq!(FocusKind::from_u8(2), FocusKind::Undetermined);
    }

    #[test]
    fn focus_kind_from_u8_unknown_fallback() {
        assert_eq!(FocusKind::from_u8(255), FocusKind::Undetermined);
    }

    #[test]
    fn test_bypass_state_repr_values() {
        // repr(u8) の値が AtomicU8 との変換で正しいことを確認
        assert_eq!(FocusKind::TextInput as u8, 0);
        assert_eq!(FocusKind::NonText as u8, 1);
        assert_eq!(FocusKind::Undetermined as u8, 2);
    }

    #[test]
    fn test_bypass_state_equality() {
        assert_eq!(FocusKind::TextInput, FocusKind::TextInput);
        assert_ne!(FocusKind::TextInput, FocusKind::NonText);
        assert_ne!(FocusKind::NonText, FocusKind::Undetermined);
    }

    #[test]
    fn test_bypass_state_copy_clone() {
        let state = FocusKind::NonText;
        let copied = state; // Copy
        let cloned = state.clone(); // Clone
        assert_eq!(copied, FocusKind::NonText);
        assert_eq!(cloned, FocusKind::NonText);
    }

    #[test]
    fn test_bypass_state_debug_format() {
        // Debug trait が実装されていることを確認
        let s = format!("{:?}", FocusKind::TextInput);
        assert_eq!(s, "TextInput");
        let s = format!("{:?}", FocusKind::NonText);
        assert_eq!(s, "NonText");
        let s = format!("{:?}", FocusKind::Undetermined);
        assert_eq!(s, "Undetermined");
    }
}
