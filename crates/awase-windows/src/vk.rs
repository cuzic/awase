//! Windows VK コードの分類ユーティリティ
//!
//! Windows 固有の仮想キーコード判定関数群。

use awase::types::{ModifierKey, VkCode};
use std::collections::HashMap;

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
/// VK_A (0x41) — 'A' キー。GJI cold-start warmup の犠牲キー (`send_unicode_cold_warmup_keys`) 用途。
pub const VK_A: VkCode = VkCode(0x41);
pub const VK_F11: VkCode = VkCode(0x7A);
pub const VK_F12: VkCode = VkCode(0x7B);
pub const VK_LSHIFT: VkCode = VkCode(0xA0);
pub const VK_RSHIFT: VkCode = VkCode(0xA1);
pub const VK_LCONTROL: VkCode = VkCode(0xA2);
pub const VK_RCONTROL: VkCode = VkCode(0xA3);
pub const VK_LMENU: VkCode = VkCode(0xA4);
pub const VK_RMENU: VkCode = VkCode(0xA5);
pub const VK_OEM_MINUS: VkCode = VkCode(0xBD);
pub const VK_LWIN: VkCode = VkCode(0x5B);
pub const VK_RWIN: VkCode = VkCode(0x5C);
pub const VK_DBE_ALPHANUMERIC: VkCode = VkCode(0xF0);
pub const VK_DBE_KATAKANA: VkCode = VkCode(0xF1);
pub const VK_DBE_HIRAGANA: VkCode = VkCode(0xF2);
pub const VK_DBE_SBCSCHAR: VkCode = VkCode(0xF3);
pub const VK_DBE_DBCSCHAR: VkCode = VkCode(0xF4);
/// VK_DBE_ROMAN (0xF5) — ローマ字入力モードへの切替（IME open 状態は変えない）。
///
/// `ImeKeyKind`（IME ON/OFF の shadow 追従用）には**含めない**: このキーは
/// ROMAN ビット（かな入力方式）のみを制御し、`ShadowImeEffect::TurnOn/TurnOff/Toggle`
/// のいずれにも該当しない。IME 自体の開閉状態を持つ shadow 追従の対象外。
pub const VK_DBE_ROMAN: VkCode = VkCode(0xF5);
/// VK_DBE_NOROMAN (0xF6) — JIS かな直接入力モードへの切替（IME open 状態は変えない）。
/// `VK_DBE_ROMAN` と同じ理由で `ImeKeyKind` には含めない。
pub const VK_DBE_NOROMAN: VkCode = VkCode(0xF6);
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
    matches!(vk_code.0, 0xF0..=0xF6)
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
        | 0x5B | 0x5C // VK_LWIN, VK_RWIN
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
    matches!(vk.0, 0x20 | 0x0D | 0x1B) // VK_SPACE, VK_RETURN, VK_ESCAPE
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
#[expect(clippy::wrong_self_convention)]
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
    fn from_name(name: &str) -> Option<Self>
    where
        Self: Sized;
}

impl VkCodeExt for VkCode {
    fn is_passthrough(self) -> bool {
        is_passthrough(self)
    }
    fn is_ime_control(self) -> bool {
        is_ime_control(self)
    }
    fn is_ime_context(self) -> bool {
        is_ime_context(self)
    }
    fn is_non_shift_modifier(self) -> bool {
        is_non_shift_modifier(self)
    }
    fn is_ctrl_variant(self) -> bool {
        is_ctrl_variant(self)
    }
    fn is_composition_confirm_key(self) -> bool {
        is_composition_confirm_key(self)
    }
    fn is_modifier_free_char(self, held: bool) -> bool {
        is_modifier_free_char(self, held)
    }
    fn may_change_ime(self) -> bool {
        may_change_ime(self)
    }
    fn classify_modifier(self) -> Option<ModifierKey> {
        classify_modifier(self)
    }
    fn ime_kind(self) -> Option<ImeKeyKind> {
        ImeKeyKind::from_vk(self)
    }
    fn to_pos(self) -> Option<awase::scanmap::PhysicalPos> {
        vk_to_pos(self)
    }
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "VK_A" => Some(Self(0x41)),
            "VK_B" => Some(Self(0x42)),
            "VK_C" => Some(Self(0x43)),
            "VK_D" => Some(Self(0x44)),
            "VK_E" => Some(Self(0x45)),
            "VK_F" => Some(Self(0x46)),
            "VK_G" => Some(Self(0x47)),
            "VK_H" => Some(Self(0x48)),
            "VK_I" => Some(Self(0x49)),
            "VK_J" => Some(Self(0x4A)),
            "VK_K" => Some(Self(0x4B)),
            "VK_L" => Some(Self(0x4C)),
            "VK_M" => Some(Self(0x4D)),
            "VK_N" => Some(Self(0x4E)),
            "VK_O" => Some(Self(0x4F)),
            "VK_P" => Some(Self(0x50)),
            "VK_Q" => Some(Self(0x51)),
            "VK_R" => Some(Self(0x52)),
            "VK_S" => Some(Self(0x53)),
            "VK_T" => Some(Self(0x54)),
            "VK_U" => Some(Self(0x55)),
            "VK_V" => Some(Self(0x56)),
            "VK_W" => Some(Self(0x57)),
            "VK_X" => Some(Self(0x58)),
            "VK_Y" => Some(Self(0x59)),
            "VK_Z" => Some(Self(0x5A)),
            "VK_0" => Some(Self(0x30)),
            "VK_1" => Some(Self(0x31)),
            "VK_2" => Some(Self(0x32)),
            "VK_3" => Some(Self(0x33)),
            "VK_4" => Some(Self(0x34)),
            "VK_5" => Some(Self(0x35)),
            "VK_6" => Some(Self(0x36)),
            "VK_7" => Some(Self(0x37)),
            "VK_8" => Some(Self(0x38)),
            "VK_9" => Some(Self(0x39)),
            "VK_OEM_PLUS" => Some(Self(0xBB)),
            "VK_OEM_COMMA" => Some(Self(0xBC)),
            "VK_OEM_MINUS" => Some(Self(0xBD)),
            "VK_OEM_PERIOD" => Some(Self(0xBE)),
            "VK_OEM_2" => Some(Self(0xBF)),
            "VK_OEM_1" => Some(Self(0xBA)),
            "VK_OEM_3" => Some(Self(0xC0)),
            "VK_OEM_4" => Some(Self(0xDB)),
            "VK_OEM_5" => Some(Self(0xDC)),
            "VK_OEM_6" => Some(Self(0xDD)),
            "VK_OEM_7" => Some(Self(0xDE)),
            "VK_OEM_102" => Some(Self(0xE2)),
            "VK_SPACE" => Some(Self(0x20)),
            "VK_RETURN" => Some(Self(0x0D)),
            "VK_TAB" => Some(Self(0x09)),
            "VK_BACK" => Some(Self(0x08)),
            "VK_ESCAPE" => Some(Self(0x1B)),
            "VK_DELETE" => Some(Self(0x2E)),
            "VK_CONVERT" | "Convert" | "変換" => Some(Self(0x1C)),
            "VK_NONCONVERT" | "VK_MUHENKAN" | "Nonconvert" | "無変換" => Some(Self(0x1D)),
            "VK_KANA" | "Kana" | "かな" | "カナ" => Some(Self(0x15)),
            "VK_KANJI" | "Kanji" | "漢字" => Some(Self(0x19)),
            "VK_IME_ON" | "ImeOn" | "IMEオン" => Some(Self(0x16)),
            "VK_IME_OFF" | "ImeOff" | "IMEオフ" => Some(Self(0x1A)),
            "VK_DBE_ALPHANUMERIC" => Some(Self(0xF0)),
            "VK_DBE_KATAKANA" => Some(Self(0xF1)),
            "VK_DBE_HIRAGANA" => Some(Self(0xF2)),
            "VK_DBE_SBCSCHAR" | "VK_OEM_AUTO" => Some(Self(0xF3)),
            "VK_DBE_DBCSCHAR" | "VK_OEM_ENLW" => Some(Self(0xF4)),
            "VK_DBE_ROMAN" => Some(Self(0xF5)),
            "VK_DBE_NOROMAN" => Some(Self(0xF6)),
            "VK_SHIFT" => Some(Self(0x10)),
            "VK_CONTROL" => Some(Self(0x11)),
            "VK_MENU" => Some(Self(0x12)),
            "VK_LSHIFT" => Some(Self(0xA0)),
            "VK_RSHIFT" => Some(Self(0xA1)),
            "VK_LCONTROL" => Some(Self(0xA2)),
            "VK_RCONTROL" => Some(Self(0xA3)),
            "VK_LMENU" => Some(Self(0xA4)),
            "VK_RMENU" => Some(Self(0xA5)),
            "VK_F1" => Some(Self(0x70)),
            "VK_F2" => Some(Self(0x71)),
            "VK_F3" => Some(Self(0x72)),
            "VK_F4" => Some(Self(0x73)),
            "VK_F5" => Some(Self(0x74)),
            "VK_F6" => Some(Self(0x75)),
            "VK_F7" => Some(Self(0x76)),
            "VK_F8" => Some(Self(0x77)),
            "VK_F9" => Some(Self(0x78)),
            "VK_F10" => Some(Self(0x79)),
            "VK_F11" => Some(Self(0x7A)),
            "VK_F12" => Some(Self(0x7B)),
            "VK_F13" => Some(Self(0x7C)),
            "VK_F14" => Some(Self(0x7D)),
            "VK_F15" => Some(Self(0x7E)),
            "VK_F16" => Some(Self(0x7F)),
            "VK_F17" => Some(Self(0x80)),
            "VK_F18" => Some(Self(0x81)),
            "VK_F19" => Some(Self(0x82)),
            "VK_F20" => Some(Self(0x83)),
            "VK_F21" => Some(Self(0x84)),
            "VK_F22" => Some(Self(0x85)),
            "VK_F23" => Some(Self(0x86)),
            "VK_F24" => Some(Self(0x87)),
            "VK_HOME" => Some(Self(0x24)),
            "VK_END" => Some(Self(0x23)),
            "VK_PRIOR" => Some(Self(0x21)),
            "VK_NEXT" => Some(Self(0x22)),
            "VK_INSERT" => Some(Self(0x2D)),
            "VK_SNAPSHOT" => Some(Self(0x2C)),
            _ => None,
        }
    }
}

// ── キー名解決（config パース用）──

/// ホットキー文字列をパースして修飾キーフラグと仮想キーコードに変換する。
///
/// `windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_SHIFT}` に
/// 依存する唯一の関数のため `#[cfg(windows)]`。`vk` モジュール自体は
/// この関数以外 windows crate に依存しないため ungated（ADR-082「決定1実施記録」の
/// 次の一歩、`decide_alt_impersonation` の Linux 化のための下準備）。
#[cfg(windows)]
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
///
/// 英数字に加え、`build_symbol_to_vk` の「半角 ASCII 記号」節にある記号を
/// すべて含む（2026-08-05 ユーザー報告: Shift 付き記号 `！` の cold-start
/// 半角化修正で拡張。`docs/known-bugs.md` BUG-47 参照）。
///
/// 呼び出し元は `output/`（windows-gated）のみのため、非 Windows では未使用になる。
#[cfg_attr(not(windows), allow(dead_code))]
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
        '[' => Some((VkCode(0xDB), false)),
        ']' => Some((VkCode(0xDD), false)),
        ';' => Some((VkCode(0xBB), false)),
        ':' => Some((VkCode(0xBA), false)),
        '@' => Some((VkCode(0xC0), false)),
        '^' => Some((VkCode(0xDE), false)),
        '\\' => Some((VkCode(0xE2), false)),
        '!' => Some((VkCode(0x31), true)),
        '"' => Some((VkCode(0x32), true)),
        '#' => Some((VkCode(0x33), true)),
        '$' => Some((VkCode(0x34), true)),
        '%' => Some((VkCode(0x35), true)),
        '&' => Some((VkCode(0x36), true)),
        '\'' => Some((VkCode(0x37), true)),
        '(' => Some((VkCode(0x38), true)),
        ')' => Some((VkCode(0x39), true)),
        '?' => Some((VkCode(0xBF), true)),
        '=' => Some((VkCode(0xBD), true)),
        '+' => Some((VkCode(0xBB), true)),
        '*' => Some((VkCode(0xBA), true)),
        '<' => Some((VkCode(0xBC), true)),
        '>' => Some((VkCode(0xBE), true)),
        '_' => Some((VkCode(0xE2), true)),
        '{' => Some((VkCode(0xDB), true)),
        '}' => Some((VkCode(0xDD), true)),
        '|' => Some((VkCode(0xDC), true)),
        '~' => Some((VkCode(0xDE), true)),
        '`' => Some((VkCode(0xC0), true)),
        _ => None,
    }
}

/// `ascii_to_vk` の逆写像。`(vk, needs_shift)` が単一 ASCII キーストロークで
/// 表現できる場合のみ `Some` を返す。
///
/// 不変条件: `vk_pair_to_ascii(v, s) == Some(c)` ⇒ `ascii_to_vk(c) == Some((v, s))`
/// （`vk_pair_to_ascii_roundtrips_with_ascii_to_vk` テストで固定）。
///
/// `symbol_to_vk`（`build_symbol_to_vk`）が生成する記号 VK は、Shift 付き記号
/// （`？`/`！`/`～` 等）も含めすべてこの関数でカバーする（2026-08-05 修正、
/// `docs/known-bugs.md` BUG-47 参照。修正前は Shift 付きが `needs_shift` の
/// 一律ガードで弾かれ、対応する半角 ASCII 記号が無い扱いになっていた）。
/// 英大文字（`A`..`Z`, Shift 付き）は `ascii_to_vk` 側には存在するが、
/// `build_symbol_to_vk` に該当エントリが無く本バグの対象外のためこの関数では
/// 未対応のまま（`vk` は `(0x41..=0x5A, false)` の非 Shift 判定のみ持つ）。
///
/// 呼び出し元は `output/`（windows-gated）のみのため、非 Windows では未使用になる。
#[cfg_attr(not(windows), allow(dead_code))]
#[must_use]
pub(crate) const fn vk_pair_to_ascii(vk: VkCode, needs_shift: bool) -> Option<char> {
    match (vk.0, needs_shift) {
        (0x41..=0x5A, false) => Some((b'a' + (vk.0 - 0x41) as u8) as char),
        (0x30..=0x39, false) => Some((b'0' + (vk.0 - 0x30) as u8) as char),
        (0xBD, false) => Some('-'),
        (0xBE, false) => Some('.'),
        (0xBC, false) => Some(','),
        (0xBF, false) => Some('/'),
        (0xDB, false) => Some('['),
        (0xDD, false) => Some(']'),
        (0xBB, false) => Some(';'),
        (0xBA, false) => Some(':'),
        (0xC0, false) => Some('@'),
        (0xDE, false) => Some('^'),
        (0xE2, false) => Some('\\'),
        (0x31, true) => Some('!'),
        (0x32, true) => Some('"'),
        (0x33, true) => Some('#'),
        (0x34, true) => Some('$'),
        (0x35, true) => Some('%'),
        (0x36, true) => Some('&'),
        (0x37, true) => Some('\''),
        (0x38, true) => Some('('),
        (0x39, true) => Some(')'),
        (0xBF, true) => Some('?'),
        (0xBD, true) => Some('='),
        (0xBB, true) => Some('+'),
        (0xBA, true) => Some('*'),
        (0xBC, true) => Some('<'),
        (0xBE, true) => Some('>'),
        (0xE2, true) => Some('_'),
        (0xDB, true) => Some('{'),
        (0xDD, true) => Some('}'),
        (0xDC, true) => Some('|'),
        (0xDE, true) => Some('~'),
        (0xC0, true) => Some('`'),
        _ => None,
    }
}

/// 記号の VK マッピング（文字 → (VK コード, Shift 必要)）
///
/// JIS キーボード + IME ひらがなモード前提。
/// IME が有効な状態でこれらのキーストロークを送ると、
/// 対応する全角記号が入力される。
///
/// 呼び出し元は `output/`（windows-gated）のみのため、非 Windows では未使用になる。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn build_symbol_to_vk() -> HashMap<char, (VkCode, bool)> {
    let entries: &[(char, u16, bool)] = &[
        // 句読点・括弧
        ('、', 0xBC, false), // , (VK_OEM_COMMA)
        ('。', 0xBE, false), // . (VK_OEM_PERIOD)
        ('・', 0xBF, false), // / (VK_OEM_2)
        ('「', 0xDB, false), // [ (VK_OEM_4)
        ('」', 0xDD, false), // ] (VK_OEM_6)
        // 長音・記号
        ('ー', 0xBD, false), // - (VK_OEM_MINUS)
        ('～', 0xDE, true),  // Shift+^ (VK_OEM_7, JIS)
        // 全角 ASCII 記号
        ('？', 0xBF, true),  // Shift+/
        ('！', 0x31, true),  // Shift+1
        ('＃', 0x33, true),  // Shift+3
        ('＄', 0x34, true),  // Shift+4
        ('％', 0x35, true),  // Shift+5
        ('＆', 0x36, true),  // Shift+6
        ('（', 0x38, true),  // Shift+8
        ('）', 0x39, true),  // Shift+9
        ('＝', 0xBD, true),  // Shift+- (JIS: =)
        ('＋', 0xBB, true),  // Shift+; (VK_OEM_PLUS, JIS: +)
        ('＊', 0xBA, true),  // Shift+: (VK_OEM_1, JIS: *)
        ('＜', 0xBC, true),  // Shift+,
        ('＞', 0xBE, true),  // Shift+.
        ('＠', 0xC0, false), // @ (VK_OEM_3, JIS)
        ('｛', 0xDB, true),  // Shift+[
        ('｝', 0xDD, true),  // Shift+]
        ('＿', 0xE2, true),  // Shift+＼ (JIS: _)
        ('｜', 0xDC, true),  // Shift+¥ (JIS: |)
        ('"', 0x32, true),   // Shift+2 (JIS: ")
        ('＂', 0x32, true),  // 全角" → Shift+2
        ('；', 0xBB, false), // ; (VK_OEM_PLUS, JIS: ;)
        ('：', 0xBA, false), // : (VK_OEM_1, JIS: :)
        ('－', 0xBD, false), // - (VK_OEM_MINUS) 全角ハイフンマイナス
        ('／', 0xBF, false), // / (VK_OEM_2)
        ('＾', 0xDE, false), // ^ (VK_OEM_7, JIS)
        ('｀', 0xC0, true),  // Shift+@ (JIS: `)
        ('＇', 0x37, true),  // Shift+7 (JIS: ')
        ('＼', 0xE2, false), // ＼ (VK_OEM_102, JIS)
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
        ('!', 0x31, true),  // Shift+1
        ('"', 0x32, true),  // Shift+2 (JIS)
        ('#', 0x33, true),  // Shift+3
        ('$', 0x34, true),  // Shift+4
        ('%', 0x35, true),  // Shift+5
        ('&', 0x36, true),  // Shift+6
        ('\'', 0x37, true), // Shift+7 (JIS)
        ('(', 0x38, true),  // Shift+8
        (')', 0x39, true),  // Shift+9
        ('?', 0xBF, true),  // Shift+/
        ('-', 0xBD, false),
        ('=', 0xBD, true), // Shift+- (JIS)
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
    entries
        .iter()
        .map(|&(ch, vk, shift)| (ch, (VkCode(vk), shift)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{ascii_to_vk, build_symbol_to_vk, vk_pair_to_ascii, VkCode};

    /// `vk_pair_to_ascii` は `ascii_to_vk` の厳密な逆写像である
    /// （2026-08-03 ユーザー報告 BUG-47: 句読点「。」「、」・長音「ー」が
    /// 半角化する修正の前提となる不変条件。VK 0x00-0xFF × shift 2値を全網羅）。
    #[test]
    fn vk_pair_to_ascii_roundtrips_with_ascii_to_vk() {
        for raw in 0x00u16..=0xFF {
            for needs_shift in [false, true] {
                let vk = VkCode(raw);
                if let Some(ch) = vk_pair_to_ascii(vk, needs_shift) {
                    assert_eq!(
                        ascii_to_vk(ch),
                        Some((vk, needs_shift)),
                        "vk_pair_to_ascii(0x{raw:02X}, {needs_shift}) = Some({ch:?}) だが \
                         ascii_to_vk({ch:?}) が往復しない"
                    );
                }
            }
        }
    }

    /// 本当に未対応な VK（英大文字の Shift 付き、F1、Backspace）は引き続き None。
    /// 2026-08-05 修正前は shift 付きを一律 None にしていたが、その前提は
    /// もう成り立たない（下記 `vk_pair_to_ascii_covers_shift_symbols` 参照）ため、
    /// 「shift は常に None」ではなく「未対応 VK は None」の形に修正した。
    #[test]
    fn vk_pair_to_ascii_rejects_unmapped_vks() {
        assert_eq!(vk_pair_to_ascii(VkCode(0x41), true), None); // 'A' (Shift+A、対象外)
        assert_eq!(vk_pair_to_ascii(VkCode(0x70), false), None); // VK_F1
        assert_eq!(vk_pair_to_ascii(VkCode(0x70), true), None); // VK_F1 + Shift
        assert_eq!(vk_pair_to_ascii(VkCode(0x08), false), None); // VK_BACK
    }

    /// 今回のユーザー報告3文字（。→VK_OEM_PERIOD、、→VK_OEM_COMMA、ー→VK_OEM_MINUS）
    /// が正しく ASCII へ解決できることを明示的に固定する。
    #[test]
    fn vk_pair_to_ascii_covers_reported_symbols() {
        assert_eq!(vk_pair_to_ascii(VkCode(0xBE), false), Some('.')); // 。
        assert_eq!(vk_pair_to_ascii(VkCode(0xBC), false), Some(',')); // 、
        assert_eq!(vk_pair_to_ascii(VkCode(0xBD), false), Some('-')); // ー
    }

    /// 2026-08-05 ユーザー報告（「！」が半角化する）で追加した Shift 付き記号の
    /// 代表例。`docs/known-bugs.md` BUG-47 の「未対応」節で名指しされていた
    /// `？`/`！`/`～` を明示的に固定する。
    #[test]
    fn vk_pair_to_ascii_covers_shift_symbols() {
        assert_eq!(vk_pair_to_ascii(VkCode(0x31), true), Some('!')); // ！
        assert_eq!(vk_pair_to_ascii(VkCode(0xBF), true), Some('?')); // ？
        assert_eq!(vk_pair_to_ascii(VkCode(0xDE), true), Some('~')); // ～
    }

    /// ドリフト防止: `build_symbol_to_vk` に載っている `(VkCode, needs_shift)` は
    /// すべて `vk_pair_to_ascii` が `Some` を返す（＝cold-start保護つきの romaji
    /// 経路に合流できる）ことを固定する。「記号が cold-start 保護の外に取り残され
    /// ていないか」を直接検証する（値の重複は許容: `！`/`!` 等は同じペアを共有する
    /// ため、キーの文字ではなく値のペアを走査する）。
    #[test]
    fn vk_pair_to_ascii_covers_every_build_symbol_to_vk_pair() {
        for (vk, needs_shift) in build_symbol_to_vk().into_values() {
            assert!(
                vk_pair_to_ascii(vk, needs_shift).is_some(),
                "build_symbol_to_vk に (VK 0x{:02X}, shift={needs_shift}) があるのに \
                 vk_pair_to_ascii が None を返す → cold-start 保護経路に合流できない",
                vk.0
            );
        }
    }
}
