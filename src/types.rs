/// マイクロ秒精度のタイムスタンプ（テスト容易性のため `Instant` を置換）
pub type Timestamp = u64;

/// プラットフォーム固有のキーコード（Windows VK, macOS keycode, Linux evdev keycode）
///
/// Engine はこの値を直接検査しない。再注入・ログ出力等でプラットフォーム層に返すために保持する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VkCode(pub u16);

impl From<u16> for VkCode {
    fn from(v: u16) -> Self {
        Self(v)
    }
}
impl From<VkCode> for u16 {
    fn from(v: VkCode) -> Self {
        v.0
    }
}

/// プラットフォーム固有のスキャンコード（Windows Set 1, macOS keycode, Linux evdev keycode）
///
/// Engine はこの値を直接検査しない。プラットフォーム層が `PhysicalPos` に変換済み。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScanCode(pub u32);

impl From<u32> for ScanCode {
    fn from(v: u32) -> Self {
        Self(v)
    }
}
impl From<ScanCode> for u32 {
    fn from(v: ScanCode) -> Self {
        v.0
    }
}

// ── 特殊キー ──

/// プラットフォーム非依存の特殊キー種別
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpecialKey {
    /// Backspace
    Backspace,
    /// Escape
    Escape,
    /// Enter / Return
    Enter,
    /// Space
    Space,
    /// Delete
    Delete,
}

// ── 修飾キー ──

/// プラットフォーム非依存の修飾キー種別
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModifierKey {
    Ctrl,
    Shift,
    Alt,
    /// Windows / Cmd / Super
    Meta,
}

// ── IME 関連 ──

/// IME 状態への影響（プラットフォーム非依存）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowImeAction {
    TurnOn,
    TurnOff,
    Toggle,
}

/// キーの IME 関連情報（プラットフォーム層が事前分類）
#[derive(Debug, Clone, Copy, Default)]
pub struct ImeRelevance {
    /// このキーが IME 状態を変更する可能性がある
    pub may_change_ime: bool,
    /// Shadow IME 状態への効果（None = 影響なし）
    pub shadow_action: Option<ShadowImeAction>,
    /// ユーザー設定の IME 同期キー（ガード対象）
    pub is_sync_key: bool,
    /// IME 同期キーの方向
    pub sync_direction: Option<ShadowImeAction>,
    /// IME 制御キー（半角/全角等、保留フラッシュ必要）
    pub is_ime_control: bool,
}

// ── キーイベント ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEventType {
    KeyDown,
    KeyUp,
}

/// キーの基本分類（プラットフォーム層が事前に決定）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyClassification {
    /// 文字キー（NICOLA 変換対象、PhysicalPos あり）
    Char,
    /// 左親指キー
    LeftThumb,
    /// 右親指キー
    RightThumb,
    /// パススルー（修飾キー、Fキー、ナビゲーション等）
    Passthrough,
}

/// フックから受け取る生のキーイベント
///
/// プラットフォーム層が事前分類した情報を含む。Engine は `vk_code`/`scan_code` を
/// 直接検査せず、分類済みフィールドを使用する。
#[derive(Debug, Clone, Copy)]
pub struct RawKeyEvent {
    /// プラットフォーム固有キーコード（再注入用に保持）
    pub vk_code: VkCode,
    /// プラットフォーム固有スキャンコード（再注入用に保持）
    pub scan_code: ScanCode,
    pub event_type: KeyEventType,
    pub extra_info: usize,
    pub timestamp: Timestamp,
    /// キーの基本分類（プラットフォーム層が事前に決定）
    pub key_classification: KeyClassification,
    /// 物理キー位置（Char キーの場合のみ Some）
    pub physical_pos: Option<crate::scanmap::PhysicalPos>,
    /// IME 関連の事前分類（プラットフォーム層が設定）
    pub ime_relevance: ImeRelevance,
    /// 修飾キー分類（プラットフォーム層が設定、None = 修飾キーではない）
    pub modifier_key: Option<ModifierKey>,
}

/// 出力アクション
#[derive(Debug, Clone)]
pub enum KeyAction {
    /// 特殊キーを押下（プラットフォーム非依存）
    SpecialKey(SpecialKey),
    /// プラットフォーム固有キーコードを押下（再注入・フォールバック用）
    Key(VkCode),
    /// プラットフォーム固有キーコードをリリース
    KeyUp(VkCode),
    /// Unicode 文字を直接出力
    Char(char),
    /// 何もしない（キーを握りつぶす）
    Suppress,
    /// ローマ字文字列をキーイベントとして送信（IME ローマ字入力モード用）
    Romaji(String),
    /// キーシーケンスとして出力（IME がキーストロークを変換する）
    KeySequence(String),
}

/// コンテキスト無効化の理由（ログ・デバッグ用）
#[derive(Debug, Clone, Copy)]
pub enum ContextChange {
    /// IME がオフになった
    ImeOff,
    /// 入力言語が変更された
    InputLanguageChanged,
    /// エンジンが無効化された（ホットキー等）
    EngineDisabled,
    /// レイアウトが差し替えられた
    LayoutSwapped,
    /// フォーカスが別のコントロールに移動した
    FocusChanged,
}

/// 外部プロセスの IME 状態をクロスプロセス API で正確に取得できるかの信頼度
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ImeReliability {
    /// クロスプロセス IME 検出を信頼できる
    Reliable = 0,
    /// クロスプロセス IME 検出が不正確な可能性
    Unreliable = 1,
    /// 未判定
    Unknown = 2,
}

impl ImeReliability {
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Reliable,
            1 => Self::Unreliable,
            _ => Self::Unknown,
        }
    }

    pub fn load(atomic: &std::sync::atomic::AtomicU8) -> Self {
        Self::from_u8(atomic.load(std::sync::atomic::Ordering::Acquire))
    }

    pub fn store(self, atomic: &std::sync::atomic::AtomicU8) {
        atomic.store(self as u8, std::sync::atomic::Ordering::Release);
    }
}

/// IME 状態キャッシュ値
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ImeCacheState {
    /// IME OFF
    Off = 0,
    /// IME ON
    On = 1,
    /// 未判定（初期状態 or キャッシュ未更新）
    Unknown = 2,
}

impl ImeCacheState {
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Off,
            1 => Self::On,
            _ => Self::Unknown,
        }
    }

    pub fn load(atomic: &std::sync::atomic::AtomicU8) -> Self {
        Self::from_u8(atomic.load(std::sync::atomic::Ordering::Acquire))
    }

    pub fn store(self, atomic: &std::sync::atomic::AtomicU8) {
        atomic.store(self as u8, std::sync::atomic::Ordering::Release);
    }

    /// `AtomicU8` に対してアトミック swap を行い、以前の値を返す
    #[must_use]
    pub fn swap(self, atomic: &std::sync::atomic::AtomicU8) -> Self {
        Self::from_u8(atomic.swap(self as u8, std::sync::atomic::Ordering::AcqRel))
    }

    /// Unknown の場合は shadow state にフォールバック
    #[must_use]
    pub const fn resolve_with_shadow(self, shadow_ime_on: bool) -> bool {
        match self {
            Self::On => true,
            Self::Off => false,
            Self::Unknown => shadow_ime_on,
        }
    }

    /// 表示用ラベル
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::On => "ON",
            Self::Unknown => "Unknown",
        }
    }
}

impl From<bool> for ImeCacheState {
    fn from(ime_on: bool) -> Self {
        if ime_on {
            Self::On
        } else {
            Self::Off
        }
    }
}

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
    /// Chromium ベースアプリ（Chrome, Edge, Electron 等）
    Chrome = 1,
    /// UWP / XAML / DirectUI アプリ
    Uwp = 2,
}

impl AppKind {
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Win32,
            1 => Self::Chrome,
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
        assert_eq!(AppKind::from_u8(1), AppKind::Chrome);
        assert_eq!(AppKind::from_u8(2), AppKind::Uwp);
    }

    #[test]
    fn app_kind_from_u8_fallback() {
        assert_eq!(AppKind::from_u8(255), AppKind::Uwp);
    }

    #[test]
    fn app_kind_load_store_roundtrip() {
        let atomic = std::sync::atomic::AtomicU8::new(0);
        AppKind::Chrome.store(&atomic);
        assert_eq!(AppKind::load(&atomic), AppKind::Chrome);
    }

    // ── ImeReliability ──

    #[test]
    fn ime_reliability_from_u8_known_values() {
        assert_eq!(ImeReliability::from_u8(0), ImeReliability::Reliable);
        assert_eq!(ImeReliability::from_u8(1), ImeReliability::Unreliable);
        assert_eq!(ImeReliability::from_u8(2), ImeReliability::Unknown);
    }

    #[test]
    fn ime_reliability_from_u8_unknown_fallback() {
        assert_eq!(ImeReliability::from_u8(255), ImeReliability::Unknown);
    }

    // ── ImeCacheState ──

    #[test]
    fn ime_cache_state_from_u8_known_values() {
        assert_eq!(ImeCacheState::from_u8(0), ImeCacheState::Off);
        assert_eq!(ImeCacheState::from_u8(1), ImeCacheState::On);
        assert_eq!(ImeCacheState::from_u8(2), ImeCacheState::Unknown);
    }

    #[test]
    fn ime_cache_state_from_u8_unknown_fallback() {
        assert_eq!(ImeCacheState::from_u8(255), ImeCacheState::Unknown);
    }

    #[test]
    fn ime_cache_state_resolve_with_shadow() {
        assert!(ImeCacheState::On.resolve_with_shadow(false));
        assert!(ImeCacheState::On.resolve_with_shadow(true));
        assert!(!ImeCacheState::Off.resolve_with_shadow(false));
        assert!(!ImeCacheState::Off.resolve_with_shadow(true));
        assert!(!ImeCacheState::Unknown.resolve_with_shadow(false));
        assert!(ImeCacheState::Unknown.resolve_with_shadow(true));
    }

    #[test]
    fn ime_cache_state_as_str() {
        assert_eq!(ImeCacheState::Off.as_str(), "OFF");
        assert_eq!(ImeCacheState::On.as_str(), "ON");
        assert_eq!(ImeCacheState::Unknown.as_str(), "Unknown");
    }

    #[test]
    fn ime_cache_state_from_bool() {
        assert_eq!(ImeCacheState::from(true), ImeCacheState::On);
        assert_eq!(ImeCacheState::from(false), ImeCacheState::Off);
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

    // ── KeyClassification ──

    #[test]
    fn key_classification_variants_exist() {
        let variants = [
            KeyClassification::Char,
            KeyClassification::LeftThumb,
            KeyClassification::RightThumb,
            KeyClassification::Passthrough,
        ];
        for (i, a) in variants.iter().enumerate() {
            for (j, b) in variants.iter().enumerate() {
                assert_eq!(i == j, a == b);
            }
        }
    }

    // ── ImeRelevance ──

    #[test]
    fn ime_relevance_default() {
        let d = ImeRelevance::default();
        assert!(!d.may_change_ime);
        assert!(d.shadow_action.is_none());
        assert!(!d.is_sync_key);
        assert!(d.sync_direction.is_none());
        assert!(!d.is_ime_control);
    }

    // ── KeyEventType ──

    #[test]
    fn key_event_type_equality() {
        assert_eq!(KeyEventType::KeyDown, KeyEventType::KeyDown);
        assert_eq!(KeyEventType::KeyUp, KeyEventType::KeyUp);
        assert_ne!(KeyEventType::KeyDown, KeyEventType::KeyUp);
    }

    // ── SpecialKey ──

    #[test]
    fn special_key_all_variants() {
        let variants = [
            SpecialKey::Backspace,
            SpecialKey::Escape,
            SpecialKey::Enter,
            SpecialKey::Space,
            SpecialKey::Delete,
        ];
        assert_eq!(variants.len(), 5);
        for (i, a) in variants.iter().enumerate() {
            for (j, b) in variants.iter().enumerate() {
                assert_eq!(i == j, a == b);
            }
        }
    }

    // ── ModifierKey ──

    #[test]
    fn modifier_key_all_variants() {
        let variants = [
            ModifierKey::Ctrl,
            ModifierKey::Shift,
            ModifierKey::Alt,
            ModifierKey::Meta,
        ];
        assert_eq!(variants.len(), 4);
        for (i, a) in variants.iter().enumerate() {
            for (j, b) in variants.iter().enumerate() {
                assert_eq!(i == j, a == b);
            }
        }
    }

    // ── ShadowImeAction ──

    #[test]
    fn shadow_ime_action_all_variants() {
        let variants = [
            ShadowImeAction::TurnOn,
            ShadowImeAction::TurnOff,
            ShadowImeAction::Toggle,
        ];
        assert_eq!(variants.len(), 3);
        for (i, a) in variants.iter().enumerate() {
            for (j, b) in variants.iter().enumerate() {
                assert_eq!(i == j, a == b);
            }
        }
    }
}
