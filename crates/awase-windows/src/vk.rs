//! Windows VK コードの分類ユーティリティ
//!
//! Windows 固有の仮想キーコード判定関数群。

use std::collections::HashMap;
use awase::types::{ModifierKey, VkCode};

/// Windows 言語 ID: 日本語 (0x0411)
pub const LANGID_JAPANESE: u32 = 0x0411;
/// Windows 言語 ID: 英語 US (0x0409)
pub const LANGID_ENGLISH_US: u32 = 0x0409;

// ── VK コード定数 ────────────────────────────────────────
//
// 各ファイルに散らばっていた `const VK_FOO: u16 = 0x..` を
// `VkCode` 型として集約。Windows API 境界では `.0` で剥がす。

pub const VK_BACK: VkCode = VkCode(0x08);
pub const VK_TAB: VkCode = VkCode(0x09);
pub const VK_RETURN: VkCode = VkCode(0x0D);
pub const VK_SHIFT: VkCode = VkCode(0x10);
pub const VK_CONTROL: VkCode = VkCode(0x11);
pub const VK_MENU: VkCode = VkCode(0x12);
pub const VK_KANA: VkCode = VkCode(0x15);
pub const VK_IME_ON: VkCode = VkCode(0x16);
pub const VK_JUNJA: VkCode = VkCode(0x17);
pub const VK_KANJI: VkCode = VkCode(0x19);
pub const VK_IME_OFF: VkCode = VkCode(0x1A);
pub const VK_ESCAPE: VkCode = VkCode(0x1B);
pub const VK_CONVERT: VkCode = VkCode(0x1C);
pub const VK_NONCONVERT: VkCode = VkCode(0x1D);
pub const VK_SPACE: VkCode = VkCode(0x20);
pub const VK_DELETE: VkCode = VkCode(0x2E);
pub const VK_F11: VkCode = VkCode(0x7A);
pub const VK_F12: VkCode = VkCode(0x7B);
pub const VK_F13: VkCode = VkCode(0x7C);
pub const VK_F14: VkCode = VkCode(0x7D);
pub const VK_LSHIFT: VkCode = VkCode(0xA0);
pub const VK_RSHIFT: VkCode = VkCode(0xA1);
pub const VK_LCONTROL: VkCode = VkCode(0xA2);
pub const VK_RCONTROL: VkCode = VkCode(0xA3);
pub const VK_LMENU: VkCode = VkCode(0xA4);
pub const VK_RMENU: VkCode = VkCode(0xA5);
pub const VK_OEM_MINUS: VkCode = VkCode(0xBD);
pub const VK_LWIN:   VkCode = VkCode(0x5B);
pub const VK_RWIN:   VkCode = VkCode(0x5C);
pub const VK_DBE_ALPHANUMERIC: VkCode = VkCode(0xF0);
pub const VK_DBE_KATAKANA: VkCode = VkCode(0xF1);
pub const VK_DBE_HIRAGANA: VkCode = VkCode(0xF2);
pub const VK_DBE_SBCSCHAR: VkCode = VkCode(0xF3);
pub const VK_DBE_DBCSCHAR: VkCode = VkCode(0xF4);
pub const VK_NONAME: VkCode = VkCode(0xFC);

// ── IME キー種別 ──────────────────────────────────────────

/// IME の ON/OFF 状態を変更するキーの種別。
///
/// raw な VK コード (0xF2, 0x19 等) の代わりにパターンマッチで使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeKeyKind {
    /// VK_KANA (0x15) — カタカナ/ひらがなキー
    ///
    /// Microsoft 公式: "The IME On key has the virtual key code VK_KANA (0x15)".
    /// 単独押下でひらがな入力モードに入る（IME ON）。Shift+ で カタカナモード。
    /// トグルではなく常に IME ON にする動作。
    /// wezterm 等のアプリで IME ON キーとして使われる。
    Kana,
    /// VK_IME_ON (0x16)
    ImeOn,
    /// VK_JUNJA (0x17) — IME on 系
    Junja,
    /// VK_KANJI (0x19) — 半角/全角キー
    /// 多くの JIS キーボードでは IME ON/OFF のトグルとして動作する。
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
            0x15 => Some(Self::Kana),
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
            Self::Kana
            | Self::ImeOn
            | Self::Junja
            | Self::Katakana
            | Self::Activate
            | Self::ActivatePair => ShadowImeEffect::TurnOn,
            Self::ImeOff | Self::Alphanumeric | Self::Deactivate => ShadowImeEffect::TurnOff,
            Self::KanjiToggle => ShadowImeEffect::Toggle,
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

/// VK コードから修飾キー種別を返す（汎用 + 左右別）。
///
/// VK_SHIFT / VK_LSHIFT / VK_RSHIFT 等の左右別バリアントを全て吸収する。
#[must_use]
pub const fn classify_modifier(vk: VkCode) -> Option<ModifierKey> {
    match vk.0 {
        0x10 | 0xA0 | 0xA1 => Some(ModifierKey::Shift),
        0x11 | 0xA2 | 0xA3 => Some(ModifierKey::Ctrl),
        0x12 | 0xA4 | 0xA5 => Some(ModifierKey::Alt),
        0x5B | 0x5C => Some(ModifierKey::Meta),
        _ => None,
    }
}

/// Shift 以外の修飾キー（Ctrl/Alt/Win）かどうかを判定する。
///
/// これらのキーは NICOLA 処理に関与しないため、Engine をバイパスして
/// 常に OS に直接渡す。KeyDown/KeyUp ペアの保証により Ctrl スタックを防止する。
#[must_use]
pub const fn is_non_shift_modifier(vk: VkCode) -> bool {
    matches!(
        vk.0,
        0x11 | 0xA2 | 0xA3  // VK_CONTROL, VK_LCONTROL, VK_RCONTROL
        | 0x12 | 0xA4 | 0xA5  // VK_MENU, VK_LMENU, VK_RMENU
        | 0x5B | 0x5C          // VK_LWIN, VK_RWIN
    )
}

/// Ctrl 系のいずれか（VK_CONTROL / VK_LCONTROL / VK_RCONTROL）かどうかを判定する。
#[must_use]
pub const fn is_ctrl_variant(vk: VkCode) -> bool {
    matches!(vk.0, 0x11 | 0xA2 | 0xA3)
}

/// composition を確定／キャンセルするキー（Space / Enter / Escape）かどうかを判定する。
///
/// これらの KeyDown は IME composition を消費し終わらせるため、TSF
/// warm/cold 状態管理上の特別扱いが必要（mark_cold + eager warmup）。
#[must_use]
pub const fn is_composition_confirm_key(vk: VkCode) -> bool {
    matches!(vk.0, 0x20 | 0x0D | 0x1B)  // VK_SPACE, VK_RETURN, VK_ESCAPE
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

/// Windows VK 分類メソッドを `VkCode` にメソッドとして追加する拡張トレイト。
pub trait VkCodeExt {
    fn is_passthrough(self) -> bool;
    fn is_ime_control(self) -> bool;
    fn is_ime_context(self) -> bool;
    fn is_non_shift_modifier(self) -> bool;
    fn is_ctrl_variant(self) -> bool;
    fn is_composition_confirm_key(self) -> bool;
    fn is_modifier_free_char(self, os_modifier_held: bool) -> bool;
    fn may_change_ime(self) -> bool;
    fn classify_modifier(self) -> Option<ModifierKey>;
    fn ime_kind(self) -> Option<ImeKeyKind>;
    fn to_pos(self) -> Option<awase::scanmap::PhysicalPos>;
    /// キー名（"VK_A" 等）から VkCode を解決する。
    fn from_name(name: &str) -> Option<Self> where Self: Sized;
}

impl VkCodeExt for VkCode {
    fn is_passthrough(self) -> bool          { is_passthrough(self) }
    fn is_ime_control(self) -> bool          { is_ime_control(self) }
    fn is_ime_context(self) -> bool          { is_ime_context(self) }
    fn is_non_shift_modifier(self) -> bool   { is_non_shift_modifier(self) }
    fn is_ctrl_variant(self) -> bool         { is_ctrl_variant(self) }
    fn is_composition_confirm_key(self) -> bool { is_composition_confirm_key(self) }
    fn is_modifier_free_char(self, held: bool) -> bool { is_modifier_free_char(self, held) }
    fn may_change_ime(self) -> bool          { may_change_ime(self) }
    fn classify_modifier(self) -> Option<ModifierKey> { classify_modifier(self) }
    fn ime_kind(self) -> Option<ImeKeyKind>  { ImeKeyKind::from_vk(self) }
    fn to_pos(self) -> Option<awase::scanmap::PhysicalPos> { vk_to_pos(self) }
    #[allow(deprecated)]
    fn from_name(name: &str) -> Option<Self> { vk_name_to_code(name) }
}

/// ブラウザ系・Electron 系のウィンドウクラスかどうかを判定する。
#[must_use]
pub fn is_browser_or_electron_class(class_name: &str) -> bool {
    class_name == "Chrome_WidgetWin_1" || class_name == "MozillaWindowClass"
}

// ── キー名解決（config パース用）──

/// 仮想キーコード名（"VK_A" 等）を実際の VkCode 値に変換する
#[must_use]
#[deprecated(note = "VkCode::from_name(name) を使ってください（VkCodeExt トレイト）")]
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
        "VK_HOME" => Some(VkCode(0x24)),
        "VK_END" => Some(VkCode(0x23)),
        "VK_PRIOR" => Some(VkCode(0x21)),
        "VK_NEXT" => Some(VkCode(0x22)),
        "VK_INSERT" => Some(VkCode(0x2D)),
        "VK_SNAPSHOT" => Some(VkCode(0x2C)),
        _ => None,
    }
}

/// ホットキー文字列をパースして修飾キーフラグと仮想キーコードに変換する。
#[must_use]
pub fn parse_hotkey(s: &str) -> Option<(u32, VkCode)> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_SHIFT};

    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    if parts.is_empty() {
        return None;
    }

    let mut modifiers = 0u32;
    for &part in &parts[..parts.len() - 1] {
        match part {
            "Ctrl" | "Control" => modifiers |= MOD_CONTROL.0,
            "Shift" => modifiers |= MOD_SHIFT.0,
            "Alt" => modifiers |= MOD_ALT.0,
            _ => return None,
        }
    }

    let key_name = format!("VK_{}", parts.last()?);
    let vk = VkCode::from_name(&key_name)?;

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
    let vk = VkCode::from_name(key_name)?;

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

// ── 文字→VK 変換テーブル（output/resolve.rs から移動）───────────────────────

/// ASCII 文字を対応する VK コードに変換する。
#[must_use]
pub(crate) const fn ascii_to_vk(ch: char) -> Option<(VkCode, bool)> {
    match ch {
        'a'..='z' => Some((VkCode(0x41 + (ch as u16 - 'a' as u16)), false)),
        'A'..='Z' => Some((VkCode(0x41 + (ch as u16 - 'A' as u16)), true)),
        '0'..='9' => Some((VkCode(0x30 + (ch as u16 - '0' as u16)), false)),
        '-' => Some((VkCode(0xBD), false)),
        '.' => Some((VkCode(0xBE), false)),
        ',' => Some((VkCode(0xBC), false)),
        '/' => Some((VkCode(0xBF), false)),
        _ => None,
    }
}

/// 記号の VK マッピング（文字 → (VK コード, Shift 必要)）
///
/// JIS キーボード + IME ひらがなモード前提。
/// IME が有効な状態でこれらのキーストロークを送ると、
/// 対応する全角記号が入力される。
pub(crate) fn build_symbol_to_vk() -> HashMap<char, (VkCode, bool)> {
    let entries: &[(char, u16, bool)] = &[
        // 句読点・括弧
        ('、', 0xBC, false),  // , (VK_OEM_COMMA)
        ('。', 0xBE, false),  // . (VK_OEM_PERIOD)
        ('・', 0xBF, false),  // / (VK_OEM_2)
        ('「', 0xDB, false),  // [ (VK_OEM_4)
        ('」', 0xDD, false),  // ] (VK_OEM_6)
        // 長音・記号
        ('ー', 0xBD, false),  // - (VK_OEM_MINUS)
        ('～', 0xDE, true),   // Shift+^ (VK_OEM_7, JIS)
        // 全角 ASCII 記号
        ('？', 0xBF, true),   // Shift+/
        ('！', 0x31, true),   // Shift+1
        ('＃', 0x33, true),   // Shift+3
        ('＄', 0x34, true),   // Shift+4
        ('％', 0x35, true),   // Shift+5
        ('＆', 0x36, true),   // Shift+6
        ('（', 0x38, true),   // Shift+8
        ('）', 0x39, true),   // Shift+9
        ('＝', 0xBD, true),   // Shift+- (JIS: =)
        ('＋', 0xBB, true),   // Shift+; (VK_OEM_PLUS, JIS: +)
        ('＊', 0xBA, true),   // Shift+: (VK_OEM_1, JIS: *)
        ('＜', 0xBC, true),   // Shift+,
        ('＞', 0xBE, true),   // Shift+.
        ('＠', 0xC0, false),  // @ (VK_OEM_3, JIS)
        ('｛', 0xDB, true),   // Shift+[
        ('｝', 0xDD, true),   // Shift+]
        ('＿', 0xE2, true),   // Shift+＼ (JIS: _)
        ('｜', 0xDC, true),   // Shift+¥ (JIS: |)
        ('"', 0x32, true),    // Shift+2 (JIS: ")
        ('＂', 0x32, true),   // 全角" → Shift+2
        ('；', 0xBB, false),  // ; (VK_OEM_PLUS, JIS: ;)
        ('：', 0xBA, false),  // : (VK_OEM_1, JIS: :)
        ('－', 0xBD, false),  // - (VK_OEM_MINUS) 全角ハイフンマイナス
        ('／', 0xBF, false),  // / (VK_OEM_2)
        ('＾', 0xDE, false),  // ^ (VK_OEM_7, JIS)
        ('｀', 0xC0, true),   // Shift+@ (JIS: `)
        ('＇', 0x37, true),   // Shift+7 (JIS: ')
        ('＼', 0xE2, false),  // ＼ (VK_OEM_102, JIS)
        // 全角数字
        ('０', 0x30, false),
        ('１', 0x31, false),
        ('２', 0x32, false),
        ('３', 0x33, false),
        ('４', 0x34, false),
        ('５', 0x35, false),
        ('６', 0x36, false),
        ('７', 0x37, false),
        ('８', 0x38, false),
        ('９', 0x39, false),
        // 半角数字
        ('0', 0x30, false),
        ('1', 0x31, false),
        ('2', 0x32, false),
        ('3', 0x33, false),
        ('4', 0x34, false),
        ('5', 0x35, false),
        ('6', 0x36, false),
        ('7', 0x37, false),
        ('8', 0x38, false),
        ('9', 0x39, false),
        // 半角 ASCII 記号
        ('!', 0x31, true),   // Shift+1
        ('"', 0x32, true),   // Shift+2 (JIS)
        ('#', 0x33, true),   // Shift+3
        ('$', 0x34, true),   // Shift+4
        ('%', 0x35, true),   // Shift+5
        ('&', 0x36, true),   // Shift+6
        ('\'', 0x37, true),  // Shift+7 (JIS)
        ('(', 0x38, true),   // Shift+8
        (')', 0x39, true),   // Shift+9
        ('?', 0xBF, true),   // Shift+/
        ('-', 0xBD, false),
        ('=', 0xBD, true),   // Shift+- (JIS)
        ('.', 0xBE, false),
        (',', 0xBC, false),
        ('/', 0xBF, false),
        ('[', 0xDB, false),
        (']', 0xDD, false),
        (';', 0xBB, false),  // JIS: ;
        (':', 0xBA, false),  // JIS: :
        ('+', 0xBB, true),   // Shift+; (JIS)
        ('*', 0xBA, true),   // Shift+: (JIS)
        ('<', 0xBC, true),   // Shift+,
        ('>', 0xBE, true),   // Shift+.
        ('@', 0xC0, false),  // JIS: @
        ('^', 0xDE, false),  // JIS: ^
        ('_', 0xE2, true),   // Shift+＼ (JIS)
        ('{', 0xDB, true),   // Shift+[
        ('}', 0xDD, true),   // Shift+]
        ('|', 0xDC, true),   // Shift+¥ (JIS)
        ('~', 0xDE, true),   // Shift+^ (JIS)
        ('`', 0xC0, true),   // Shift+@ (JIS)
        ('\\', 0xE2, false), // JIS: ＼
    ];
    entries.iter().map(|&(ch, vk, shift)| (ch, (VkCode(vk), shift))).collect()
}
