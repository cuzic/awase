//! macOS キーコードの分類・名前解決ユーティリティ
//!
//! macOS 固有の仮想キーコード (CGKeyCode) を扱う。
//! Carbon HIToolbox/Events.h の kVK_* 定数に対応する。

use awase::types::VkCode;

/// プラットフォーム非依存のキー名を macOS キーコード (CGKeyCode) に変換する。
///
/// 設定ファイル (.toml) のキー名解決に使用する。
/// macOS に直接対応しないキー（"Kanji", "ImeOn", "ImeOff" 等）は `None` を返す。
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn key_name_to_keycode(name: &str) -> Option<VkCode> {
    match name {
        // ── 日本語入力制御キー ──
        // macOS では英数/かなキーが IME 切替を担う
        "Nonconvert" | "VK_NONCONVERT" | "VK_MUHENKAN" => Some(VkCode(0x66)), // kVK_JIS_Eisu (英数)
        "Convert" | "VK_CONVERT" => Some(VkCode(0x68)),                       // kVK_JIS_Kana (かな)
        "Kanji" | "VK_KANJI" => None,    // macOS に直接対応なし
        "ImeOn" | "VK_IME_ON" => None,   // macOS では かな キーで代替
        "ImeOff" | "VK_IME_OFF" => None, // macOS では 英数 キーで代替
        "Kana" | "VK_KANA" => Some(VkCode(0x68)), // kVK_JIS_Kana

        // ── 文字キー (A-Z) ──
        "A" | "VK_A" => Some(VkCode(0x00)), // kVK_ANSI_A
        "S" | "VK_S" => Some(VkCode(0x01)), // kVK_ANSI_S
        "D" | "VK_D" => Some(VkCode(0x02)), // kVK_ANSI_D
        "F" | "VK_F" => Some(VkCode(0x03)), // kVK_ANSI_F
        "H" | "VK_H" => Some(VkCode(0x04)), // kVK_ANSI_H
        "G" | "VK_G" => Some(VkCode(0x05)), // kVK_ANSI_G
        "Z" | "VK_Z" => Some(VkCode(0x06)), // kVK_ANSI_Z
        "X" | "VK_X" => Some(VkCode(0x07)), // kVK_ANSI_X
        "C" | "VK_C" => Some(VkCode(0x08)), // kVK_ANSI_C
        "V" | "VK_V" => Some(VkCode(0x09)), // kVK_ANSI_V
        "B" | "VK_B" => Some(VkCode(0x0B)), // kVK_ANSI_B
        "Q" | "VK_Q" => Some(VkCode(0x0C)), // kVK_ANSI_Q
        "W" | "VK_W" => Some(VkCode(0x0D)), // kVK_ANSI_W
        "E" | "VK_E" => Some(VkCode(0x0E)), // kVK_ANSI_E
        "R" | "VK_R" => Some(VkCode(0x0F)), // kVK_ANSI_R
        "Y" | "VK_Y" => Some(VkCode(0x10)), // kVK_ANSI_Y
        "T" | "VK_T" => Some(VkCode(0x11)), // kVK_ANSI_T
        "O" | "VK_O" => Some(VkCode(0x1F)), // kVK_ANSI_O
        "U" | "VK_U" => Some(VkCode(0x20)), // kVK_ANSI_U
        "I" | "VK_I" => Some(VkCode(0x22)), // kVK_ANSI_I
        "P" | "VK_P" => Some(VkCode(0x23)), // kVK_ANSI_P
        "L" | "VK_L" => Some(VkCode(0x25)), // kVK_ANSI_L
        "J" | "VK_J" => Some(VkCode(0x26)), // kVK_ANSI_J
        "K" | "VK_K" => Some(VkCode(0x28)), // kVK_ANSI_K
        "N" | "VK_N" => Some(VkCode(0x2D)), // kVK_ANSI_N
        "M" | "VK_M" => Some(VkCode(0x2E)), // kVK_ANSI_M

        // ── 数字キー (0-9) ──
        "1" | "VK_1" => Some(VkCode(0x12)), // kVK_ANSI_1
        "2" | "VK_2" => Some(VkCode(0x13)), // kVK_ANSI_2
        "3" | "VK_3" => Some(VkCode(0x14)), // kVK_ANSI_3
        "4" | "VK_4" => Some(VkCode(0x15)), // kVK_ANSI_4
        "5" | "VK_5" => Some(VkCode(0x17)), // kVK_ANSI_5
        "6" | "VK_6" => Some(VkCode(0x16)), // kVK_ANSI_6
        "7" | "VK_7" => Some(VkCode(0x1A)), // kVK_ANSI_7
        "8" | "VK_8" => Some(VkCode(0x1C)), // kVK_ANSI_8
        "9" | "VK_9" => Some(VkCode(0x19)), // kVK_ANSI_9
        "0" | "VK_0" => Some(VkCode(0x1D)), // kVK_ANSI_0

        // ── 特殊キー ──
        "Backspace" | "VK_BACK" => Some(VkCode(0x33)), // kVK_Delete
        "Enter" | "VK_RETURN" => Some(VkCode(0x24)),   // kVK_Return
        "Space" | "VK_SPACE" => Some(VkCode(0x31)),    // kVK_Space
        "Escape" | "VK_ESCAPE" => Some(VkCode(0x35)),  // kVK_Escape
        "Delete" | "VK_DELETE" => Some(VkCode(0x75)),  // kVK_ForwardDelete
        "Tab" | "VK_TAB" => Some(VkCode(0x30)),        // kVK_Tab

        // ── ファンクションキー ──
        "F1" | "VK_F1" => Some(VkCode(0x7A)),   // kVK_F1
        "F2" | "VK_F2" => Some(VkCode(0x78)),   // kVK_F2
        "F3" | "VK_F3" => Some(VkCode(0x63)),   // kVK_F3
        "F4" | "VK_F4" => Some(VkCode(0x76)),   // kVK_F4
        "F5" | "VK_F5" => Some(VkCode(0x60)),   // kVK_F5
        "F6" | "VK_F6" => Some(VkCode(0x61)),   // kVK_F6
        "F7" | "VK_F7" => Some(VkCode(0x62)),   // kVK_F7
        "F8" | "VK_F8" => Some(VkCode(0x64)),   // kVK_F8
        "F9" | "VK_F9" => Some(VkCode(0x65)),   // kVK_F9
        "F10" | "VK_F10" => Some(VkCode(0x6D)), // kVK_F10
        "F11" | "VK_F11" => Some(VkCode(0x67)), // kVK_F11
        "F12" | "VK_F12" => Some(VkCode(0x6F)), // kVK_F12

        // ── 修飾キー ──
        "Shift" | "VK_SHIFT" | "VK_LSHIFT" => Some(VkCode(0x38)), // kVK_Shift
        "Ctrl" | "Control" | "VK_CONTROL" | "VK_LCONTROL" => Some(VkCode(0x3B)), // kVK_Control
        "Alt" | "VK_MENU" | "VK_LMENU" => Some(VkCode(0x3A)),     // kVK_Option

        _ => None,
    }
}
