//! Linux evdev キーコードの分類ユーティリティ
//!
//! プラットフォーム非依存のキー名を evdev キーコード（`VkCode` として格納）に変換する。

use awase::types::VkCode;

/// プラットフォーム非依存のキー名を evdev キーコード値に変換する。
///
/// evdev キーコードは `VkCode` に格納される（プラットフォーム固有コードの共通ラッパー）。
#[must_use]
pub fn key_name_to_evdev(name: &str) -> Option<VkCode> {
    let code: u16 = match name {
        // ── 日本語 IME キー ──
        "Nonconvert" => 94, // KEY_MUHENKAN
        "Convert" => 92,    // KEY_HENKAN
        "Kanji" => 93,      // KEY_KATAKANAHIRAGANA

        // ── 特殊キー ──
        "Escape" => 1,     // KEY_ESC
        "Backspace" => 14, // KEY_BACKSPACE
        "Tab" => 15,       // KEY_TAB
        "Enter" => 28,     // KEY_ENTER
        "Space" => 57,     // KEY_SPACE
        "Delete" => 111,   // KEY_DELETE

        // ── 修飾キー ──
        "Shift" => 42,            // KEY_LEFTSHIFT
        "Ctrl" | "Control" => 29, // KEY_LEFTCTRL
        "Alt" => 56,              // KEY_LEFTALT

        // ── 数字キー 0-9 ──
        "0" => 11, // KEY_0
        "1" => 2,  // KEY_1
        "2" => 3,  // KEY_2
        "3" => 4,  // KEY_3
        "4" => 5,  // KEY_4
        "5" => 6,  // KEY_5
        "6" => 7,  // KEY_6
        "7" => 8,  // KEY_7
        "8" => 9,  // KEY_8
        "9" => 10, // KEY_9

        // ── 文字キー A-Z ──
        "A" => 30, // KEY_A
        "B" => 48, // KEY_B
        "C" => 46, // KEY_C
        "D" => 32, // KEY_D
        "E" => 18, // KEY_E
        "F" => 33, // KEY_F
        "G" => 34, // KEY_G
        "H" => 35, // KEY_H
        "I" => 23, // KEY_I
        "J" => 36, // KEY_J
        "K" => 37, // KEY_K
        "L" => 38, // KEY_L
        "M" => 50, // KEY_M
        "N" => 49, // KEY_N
        "O" => 24, // KEY_O
        "P" => 25, // KEY_P
        "Q" => 16, // KEY_Q
        "R" => 19, // KEY_R
        "S" => 31, // KEY_S
        "T" => 20, // KEY_T
        "U" => 22, // KEY_U
        "V" => 47, // KEY_V
        "W" => 17, // KEY_W
        "X" => 45, // KEY_X
        "Y" => 21, // KEY_Y
        "Z" => 44, // KEY_Z

        // ── ファンクションキー F1-F12 ──
        "F1" => 59,  // KEY_F1
        "F2" => 60,  // KEY_F2
        "F3" => 61,  // KEY_F3
        "F4" => 62,  // KEY_F4
        "F5" => 63,  // KEY_F5
        "F6" => 64,  // KEY_F6
        "F7" => 65,  // KEY_F7
        "F8" => 66,  // KEY_F8
        "F9" => 67,  // KEY_F9
        "F10" => 68, // KEY_F10
        "F11" => 87, // KEY_F11
        "F12" => 88, // KEY_F12

        _ => return None,
    };
    Some(VkCode(code))
}
