//! VK コードの分類ユーティリティ
//!
//! エンジンおよびフック側で共通利用する、仮想キーコードの判定関数群。

use crate::types::VkCode;

/// Windows 言語 ID: 日本語 (0x0411)
pub const LANGID_JAPANESE: u32 = 0x0411;

// ── IME キー種別 ──────────────────────────────────────────

/// IME の ON/OFF 状態を変更するキーの種別。
///
/// raw な VK コード (0xF2, 0x19 等) の代わりにパターンマッチで使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeKeyKind {
    /// 半角/全角トグル (VK_KANJI, 0x19)
    KanjiToggle,
    /// IME ON — VK_DBE_HIRAGANA (0xF2)
    Activate,
    /// IME OFF — VK_DBE_SBCSCHAR / VK_OEM_AUTO (0xF3)
    Deactivate,
    /// IME ON ペア — VK_DBE_DBCSCHAR / VK_OEM_ENLW (0xF4)
    ActivatePair,
    /// VK_IME_ON (0x16)
    ImeOn,
    /// VK_IME_OFF (0x1A)
    ImeOff,
}

/// `ImeKeyKind` が IME 状態に与える効果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowImeEffect {
    TurnOn,
    TurnOff,
    Toggle,
}

impl ImeKeyKind {
    /// VK コードから `ImeKeyKind` への変換。該当しなければ `None`。
    #[must_use]
    pub const fn from_vk(vk: VkCode) -> Option<Self> {
        match vk.0 {
            0x19 => Some(Self::KanjiToggle),
            0xF2 => Some(Self::Activate),
            0xF3 => Some(Self::Deactivate),
            0xF4 => Some(Self::ActivatePair),
            0x16 => Some(Self::ImeOn),
            0x1A => Some(Self::ImeOff),
            _ => None,
        }
    }

    /// このキーが shadow IME 状態に与える効果。
    #[must_use]
    pub const fn shadow_effect(&self) -> ShadowImeEffect {
        match self {
            Self::Activate | Self::ActivatePair | Self::ImeOn => ShadowImeEffect::TurnOn,
            Self::Deactivate | Self::ImeOff => ShadowImeEffect::TurnOff,
            Self::KanjiToggle => ShadowImeEffect::Toggle,
        }
    }

    /// IME を ON にするキーか。
    #[must_use]
    pub const fn is_on(&self) -> bool {
        matches!(self, Self::Activate | Self::ActivatePair | Self::ImeOn)
    }

    /// IME を OFF にするキーか。
    #[must_use]
    pub const fn is_off(&self) -> bool {
        matches!(self, Self::Deactivate | Self::ImeOff)
    }

    /// IME をトグルするキーか。
    #[must_use]
    pub const fn is_toggle(&self) -> bool {
        matches!(self, Self::KanjiToggle)
    }
}

/// VK コードが IME 状態を変更する可能性があるかどうかを判定する。
///
/// `is_ime_control()` に加え、DBE 系キー (0xF0..=0xF5) も含む。
#[must_use]
pub const fn may_change_ime(vk_code: VkCode) -> bool {
    if is_ime_control(vk_code) {
        return true;
    }
    matches!(vk_code.0, 0xF0..=0xF5)
}

/// 変換対象外のキー（修飾キー、ファンクションキー等）を判定する
#[must_use]
pub const fn is_passthrough(vk_code: VkCode) -> bool {
    matches!(
        vk_code.0,
        // 修飾キー
        0x10 | 0x11 | 0x12 |  // Shift, Ctrl, Alt
        0xA0 | 0xA1 | 0xA2 | 0xA3 | 0xA4 | 0xA5 |  // L/R Shift, Ctrl, Alt
        // Windows キー
        0x5B | 0x5C |
        // Caps Lock
        0x14 |
        // Esc
        0x1B |
        // ファンクションキー (F1-F24)
        0x70..=0x87 |
        // ナビゲーション
        0x21..=0x28 |  // PageUp, PageDown, End, Home, Arrow keys
        // Insert, Delete
        0x2D | 0x2E |
        // Num Lock, Scroll Lock
        0x90 | 0x91 |
        // Print Screen, Pause
        0x2C | 0x13
    )
}

/// IME 制御キーかどうかを判定する。
///
/// これらのキーはエンジンの変換対象外だが、`is_passthrough` とは異なり
/// 保留状態で到着した場合はフラッシュが必要。
#[must_use]
pub const fn is_ime_control(vk_code: VkCode) -> bool {
    matches!(
        vk_code.0,
        0x15 |  // VK_KANA (カタカナ/ひらがな)
        0x16 |  // VK_IME_ON
        0x17 |  // VK_JUNJA
        0x19 |  // VK_KANJI / VK_HANJA (半角/全角)
        0x1A |  // VK_IME_OFF
        0xE5 // VK_PROCESSKEY (IME PROCESS)
    )
}

/// IME コンテキストキーかどうかを判定する。
///
/// `is_ime_control()` のスーパーセットに親指キー（変換/無変換）を追加。
/// これらのキーが押された場合、ユーザーがテキスト入力コンテキストにいる強いシグナルとなる。
#[must_use]
pub const fn is_ime_context(vk_code: VkCode) -> bool {
    matches!(
        vk_code.0,
        0x15 | 0x16 | 0x17 | 0x19 | 0x1A | 0x1C | 0x1D | 0xE5
    )
}

/// 修飾キー（Ctrl/Alt）が押されていない単独文字キーかどうかを判定する。
///
/// パターン検出およびハイブリッドバッファリングで使用。
/// `os_modifier_held` は呼び出し側で OS の修飾キー状態を取得して渡す。
#[must_use]
pub fn is_modifier_free_char(vk_code: VkCode, os_modifier_held: bool) -> bool {
    !is_ime_control(vk_code)
        && !is_passthrough(vk_code)
        && vk_code != VkCode(0x1C)  // VK_CONVERT (右親指)
        && vk_code != VkCode(0x1D)  // VK_NONCONVERT (左親指)
        && vk_code != VkCode(0x08)  // VK_BACK（BS は別途追跡）
        && !os_modifier_held
}

/// ブラウザ系・Electron 系のウィンドウクラスかどうかを判定する。
///
/// これらのアプリは UIA Phase 3 でテキスト入力を正確に判定できるため、
/// Undetermined 時の自動 IME OFF を適用しない。
#[must_use]
pub fn is_browser_or_electron_class(class_name: &str) -> bool {
    // Chromium 系（Chrome, Edge, Brave, Opera, Vivaldi, 全 Electron アプリ）
    // Firefox 系（Firefox, Waterfox, Tor Browser）
    class_name == "Chrome_WidgetWin_1" || class_name == "MozillaWindowClass"
}
