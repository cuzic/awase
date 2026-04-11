//! Windows VK コードの分類ユーティリティ
//!
//! Windows 固有の仮想キーコード判定関数群。

use awase::types::VkCode;

/// Windows 言語 ID: 日本語 (0x0411)
pub const LANGID_JAPANESE: u32 = 0x0411;

// ── IME キー種別 ──────────────────────────────────────────

/// IME の ON/OFF 状態を変更するキーの種別。
///
/// raw な VK コード (0xF2, 0x19 等) の代わりにパターンマッチで使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeKeyKind {
    /// VK_KANA (0x15) — カタカナ/ひらがなトグル。
    /// wezterm 等で IME on/off のトグルとして動作する。
    KanaToggle,
    /// VK_IME_ON (0x16)
    ImeOn,
    /// VK_JUNJA (0x17) — IME on 系
    Junja,
    /// VK_KANJI (0x19) — 半角/全角トグル
    KanjiToggle,
    /// VK_IME_OFF (0x1A)
    ImeOff,
    /// VK_DBE_ALPHANUMERIC / VK_OEM_ATTN (0xF0) — 英数モード（IME OFF 扱い）
    Alphanumeric,
    /// VK_DBE_KATAKANA (0xF1) — カタカナモード（IME ON）
    Katakana,
    /// VK_DBE_HIRAGANA (0xF2) — ひらがなモード（IME ON）
    Activate,
    /// VK_DBE_SBCSCHAR / VK_OEM_AUTO (0xF3) — 半角モード（IME OFF 扱い）
    Deactivate,
    /// VK_DBE_DBCSCHAR / VK_OEM_ENLW (0xF4) — 全角モード（IME ON）
    ActivatePair,
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
            0x15 => Some(Self::KanaToggle),
            0x16 => Some(Self::ImeOn),
            0x17 => Some(Self::Junja),
            0x19 => Some(Self::KanjiToggle),
            0x1A => Some(Self::ImeOff),
            0xF0 => Some(Self::Alphanumeric),
            0xF1 => Some(Self::Katakana),
            0xF2 => Some(Self::Activate),
            0xF3 => Some(Self::Deactivate),
            0xF4 => Some(Self::ActivatePair),
            _ => None,
        }
    }

    /// このキーが shadow IME 状態に与える効果。
    #[must_use]
    pub const fn shadow_effect(&self) -> ShadowImeEffect {
        match self {
            Self::ImeOn
            | Self::Junja
            | Self::Katakana
            | Self::Activate
            | Self::ActivatePair => ShadowImeEffect::TurnOn,
            Self::ImeOff | Self::Alphanumeric | Self::Deactivate => ShadowImeEffect::TurnOff,
            Self::KanjiToggle | Self::KanaToggle => ShadowImeEffect::Toggle,
        }
    }
}

/// VK コードが IME 状態を変更する可能性があるかどうかを判定する。
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
        0x10 | 0x11 | 0x12 |
        0xA0 | 0xA1 | 0xA2 | 0xA3 | 0xA4 | 0xA5 |
        0x5B | 0x5C |
        0x14 |
        0x1B |
        0x70..=0x87 |
        0x21..=0x28 |
        0x2D | 0x2E |
        0x90 | 0x91 |
        0x2C | 0x13 |
        0x09 |
        0x60..=0x6F |
        0xAD..=0xB7 |
        0xA6..=0xAC |
        0x5D |
        0x5E | 0x5F
    )
}

/// IME 制御キーかどうかを判定する。
#[must_use]
pub const fn is_ime_control(vk_code: VkCode) -> bool {
    matches!(vk_code.0, 0x15 | 0x16 | 0x17 | 0x19 | 0x1A | 0xE5)
}

/// IME コンテキストキーかどうかを判定する。
#[must_use]
pub const fn is_ime_context(vk_code: VkCode) -> bool {
    matches!(
        vk_code.0,
        0x15 | 0x16 | 0x17 | 0x19 | 0x1A | 0x1C | 0x1D | 0xE5
    )
}

/// 修飾キー（Ctrl/Alt）が押されていない単独文字キーかどうかを判定する。
#[must_use]
pub fn is_modifier_free_char(vk_code: VkCode, os_modifier_held: bool) -> bool {
    !is_ime_control(vk_code)
        && !is_passthrough(vk_code)
        && vk_code != VkCode(0x1C)
        && vk_code != VkCode(0x1D)
        && vk_code != VkCode(0x08)
        && !os_modifier_held
}

/// ブラウザ系・Electron 系のウィンドウクラスかどうかを判定する。
#[must_use]
pub fn is_browser_or_electron_class(class_name: &str) -> bool {
    class_name == "Chrome_WidgetWin_1" || class_name == "MozillaWindowClass"
}

// ── キー名解決（config パース用）──

/// 仮想キーコード名（"VK_A" 等）を実際の VkCode 値に変換する
#[must_use]
pub fn vk_name_to_code(name: &str) -> Option<VkCode> {
    match name {
        "VK_A" => Some(VkCode(0x41)),
        "VK_B" => Some(VkCode(0x42)),
        "VK_C" => Some(VkCode(0x43)),
        "VK_D" => Some(VkCode(0x44)),
        "VK_E" => Some(VkCode(0x45)),
        "VK_F" => Some(VkCode(0x46)),
        "VK_G" => Some(VkCode(0x47)),
        "VK_H" => Some(VkCode(0x48)),
        "VK_I" => Some(VkCode(0x49)),
        "VK_J" => Some(VkCode(0x4A)),
        "VK_K" => Some(VkCode(0x4B)),
        "VK_L" => Some(VkCode(0x4C)),
        "VK_M" => Some(VkCode(0x4D)),
        "VK_N" => Some(VkCode(0x4E)),
        "VK_O" => Some(VkCode(0x4F)),
        "VK_P" => Some(VkCode(0x50)),
        "VK_Q" => Some(VkCode(0x51)),
        "VK_R" => Some(VkCode(0x52)),
        "VK_S" => Some(VkCode(0x53)),
        "VK_T" => Some(VkCode(0x54)),
        "VK_U" => Some(VkCode(0x55)),
        "VK_V" => Some(VkCode(0x56)),
        "VK_W" => Some(VkCode(0x57)),
        "VK_X" => Some(VkCode(0x58)),
        "VK_Y" => Some(VkCode(0x59)),
        "VK_Z" => Some(VkCode(0x5A)),
        "VK_0" => Some(VkCode(0x30)),
        "VK_1" => Some(VkCode(0x31)),
        "VK_2" => Some(VkCode(0x32)),
        "VK_3" => Some(VkCode(0x33)),
        "VK_4" => Some(VkCode(0x34)),
        "VK_5" => Some(VkCode(0x35)),
        "VK_6" => Some(VkCode(0x36)),
        "VK_7" => Some(VkCode(0x37)),
        "VK_8" => Some(VkCode(0x38)),
        "VK_9" => Some(VkCode(0x39)),
        "VK_OEM_PLUS" => Some(VkCode(0xBB)),
        "VK_OEM_COMMA" => Some(VkCode(0xBC)),
        "VK_OEM_MINUS" => Some(VkCode(0xBD)),
        "VK_OEM_PERIOD" => Some(VkCode(0xBE)),
        "VK_OEM_2" => Some(VkCode(0xBF)),
        "VK_OEM_1" => Some(VkCode(0xBA)),
        "VK_OEM_3" => Some(VkCode(0xC0)),
        "VK_OEM_4" => Some(VkCode(0xDB)),
        "VK_OEM_5" => Some(VkCode(0xDC)),
        "VK_OEM_6" => Some(VkCode(0xDD)),
        "VK_OEM_7" => Some(VkCode(0xDE)),
        "VK_OEM_102" => Some(VkCode(0xE2)),
        "VK_SPACE" => Some(VkCode(0x20)),
        "VK_RETURN" => Some(VkCode(0x0D)),
        "VK_TAB" => Some(VkCode(0x09)),
        "VK_BACK" => Some(VkCode(0x08)),
        "VK_ESCAPE" => Some(VkCode(0x1B)),
        "VK_DELETE" => Some(VkCode(0x2E)),
        "VK_CONVERT" | "Convert" | "変換" => Some(VkCode(0x1C)),
        #[allow(clippy::match_same_arms)]
        "VK_NONCONVERT" | "VK_MUHENKAN" | "Nonconvert" | "無変換" => Some(VkCode(0x1D)),
        "VK_KANA" | "Kana" | "かな" | "カナ" => Some(VkCode(0x15)),
        "VK_KANJI" | "Kanji" | "漢字" => Some(VkCode(0x19)),
        "VK_IME_ON" | "ImeOn" | "IMEオン" => Some(VkCode(0x16)),
        "VK_IME_OFF" | "ImeOff" | "IMEオフ" => Some(VkCode(0x1A)),
        "VK_DBE_ALPHANUMERIC" => Some(VkCode(0xF0)),
        "VK_DBE_KATAKANA" => Some(VkCode(0xF1)),
        "VK_DBE_HIRAGANA" => Some(VkCode(0xF2)),
        "VK_DBE_SBCSCHAR" | "VK_OEM_AUTO" => Some(VkCode(0xF3)),
        "VK_DBE_DBCSCHAR" | "VK_OEM_ENLW" => Some(VkCode(0xF4)),
        "VK_SHIFT" => Some(VkCode(0x10)),
        "VK_CONTROL" => Some(VkCode(0x11)),
        "VK_MENU" => Some(VkCode(0x12)),
        "VK_LSHIFT" => Some(VkCode(0xA0)),
        "VK_RSHIFT" => Some(VkCode(0xA1)),
        "VK_LCONTROL" => Some(VkCode(0xA2)),
        "VK_RCONTROL" => Some(VkCode(0xA3)),
        "VK_LMENU" => Some(VkCode(0xA4)),
        "VK_RMENU" => Some(VkCode(0xA5)),
        "VK_F1" => Some(VkCode(0x70)),
        "VK_F2" => Some(VkCode(0x71)),
        "VK_F3" => Some(VkCode(0x72)),
        "VK_F4" => Some(VkCode(0x73)),
        "VK_F5" => Some(VkCode(0x74)),
        "VK_F6" => Some(VkCode(0x75)),
        "VK_F7" => Some(VkCode(0x76)),
        "VK_F8" => Some(VkCode(0x77)),
        "VK_F9" => Some(VkCode(0x78)),
        "VK_F10" => Some(VkCode(0x79)),
        "VK_F11" => Some(VkCode(0x7A)),
        "VK_F12" => Some(VkCode(0x7B)),
        _ => None,
    }
}

/// ホットキー文字列をパースして修飾キーフラグと仮想キーコードに変換する。
#[must_use]
pub fn parse_hotkey(s: &str) -> Option<(u32, VkCode)> {
    const MOD_ALT: u32 = 0x0001;
    const MOD_CONTROL: u32 = 0x0002;
    const MOD_SHIFT: u32 = 0x0004;

    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    if parts.is_empty() {
        return None;
    }

    let mut modifiers: u32 = 0;
    for &part in &parts[..parts.len() - 1] {
        match part {
            "Ctrl" | "Control" => modifiers |= MOD_CONTROL,
            "Shift" => modifiers |= MOD_SHIFT,
            "Alt" => modifiers |= MOD_ALT,
            _ => return None,
        }
    }

    let key_name = format!("VK_{}", parts.last()?);
    let vk = vk_name_to_code(&key_name)?;

    Some((modifiers, vk))
}

/// キーコンボ文字列をパースする
#[must_use]
pub fn parse_key_combo(s: &str) -> Option<awase::config::ParsedKeyCombo> {
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    if parts.is_empty() {
        return None;
    }

    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    for &part in &parts[..parts.len() - 1] {
        match part {
            "Ctrl" | "Control" => ctrl = true,
            "Shift" => shift = true,
            "Alt" => alt = true,
            _ => return None,
        }
    }

    let key_name = *parts.last()?;
    let vk = vk_name_to_code(key_name)?;

    Some(awase::config::ParsedKeyCombo {
        ctrl,
        shift,
        alt,
        vk,
    })
}

/// Windows VK コードから物理キー位置（JIS キーボード）へのマッピング。
///
/// NICOLA 配列で使用する文字キー（数字行・Q行・A行・Z行）のみを対象とする。
/// 親指キー（変換・無変換・スペース等）は含まない。
#[must_use]
pub const fn vk_to_pos(vk: VkCode) -> Option<awase::scanmap::PhysicalPos> {
    let (row, col) = match vk.0 {
        // Row 0: number row (0x30..=0x39 → '0'..'9')
        0x31 => (0, 0),  // 1
        0x32 => (0, 1),  // 2
        0x33 => (0, 2),  // 3
        0x34 => (0, 3),  // 4
        0x35 => (0, 4),  // 5
        0x36 => (0, 5),  // 6
        0x37 => (0, 6),  // 7
        0x38 => (0, 7),  // 8
        0x39 => (0, 8),  // 9
        0x30 => (0, 9),  // 0
        0xBD => (0, 10), // VK_OEM_MINUS (-)
        0xDE => (0, 11), // VK_OEM_7 (^) — JIS layout
        0xDC => (0, 12), // VK_OEM_5 (¥)

        // Row 1: Q row
        0x51 => (1, 0),  // Q
        0x57 => (1, 1),  // W
        0x45 => (1, 2),  // E
        0x52 => (1, 3),  // R
        0x54 => (1, 4),  // T
        0x59 => (1, 5),  // Y
        0x55 => (1, 6),  // U
        0x49 => (1, 7),  // I
        0x4F => (1, 8),  // O
        0x50 => (1, 9),  // P
        0xC0 => (1, 10), // VK_OEM_3 (@) — JIS layout
        0xDB => (1, 11), // VK_OEM_4 ([)

        // Row 2: A row
        0x41 => (2, 0),  // A
        0x53 => (2, 1),  // S
        0x44 => (2, 2),  // D
        0x46 => (2, 3),  // F
        0x47 => (2, 4),  // G
        0x48 => (2, 5),  // H
        0x4A => (2, 6),  // J
        0x4B => (2, 7),  // K
        0x4C => (2, 8),  // L
        0xBB => (2, 9),  // VK_OEM_PLUS (;) — JIS layout
        0xBA => (2, 10), // VK_OEM_1 (:)
        0xDD => (2, 11), // VK_OEM_6 (])

        // Row 3: Z row
        0x5A => (3, 0),  // Z
        0x58 => (3, 1),  // X
        0x43 => (3, 2),  // C
        0x56 => (3, 3),  // V
        0x42 => (3, 4),  // B
        0x4E => (3, 5),  // N
        0x4D => (3, 6),  // M
        0xBC => (3, 7),  // VK_OEM_COMMA (,)
        0xBE => (3, 8),  // VK_OEM_PERIOD (.)
        0xBF => (3, 9),  // VK_OEM_2 (/)
        0xE2 => (3, 10), // VK_OEM_102 (_) — JIS layout

        _ => return None,
    };
    Some(awase::scanmap::PhysicalPos::new(row, col))
}
