//! macOS キーコード (CGKeyCode) ⇔ PhysicalPos マッピング
//!
//! JIS キーボード配列における macOS キーコードから物理位置への変換。
//! macOS のキーコードは Carbon HIToolbox/Events.h の kVK_* 定数に対応する。

use awase::scanmap::PhysicalPos;

/// macOS キーコード (CGKeyCode) → JIS キーボード物理位置
///
/// NICOLA 配列で使用する文字キー（数字行・Q行・A行・Z行）のみを対象とする。
/// 親指キー（英数・かな・スペース等）は含まない。
#[must_use]
pub const fn keycode_to_pos(keycode: u16) -> Option<PhysicalPos> {
    let (row, col) = match keycode {
        // Row 0: number row
        0x12 => (0, 0),  // kVK_ANSI_1
        0x13 => (0, 1),  // kVK_ANSI_2
        0x14 => (0, 2),  // kVK_ANSI_3
        0x15 => (0, 3),  // kVK_ANSI_4
        0x17 => (0, 4),  // kVK_ANSI_5
        0x16 => (0, 5),  // kVK_ANSI_6
        0x1A => (0, 6),  // kVK_ANSI_7
        0x1C => (0, 7),  // kVK_ANSI_8
        0x19 => (0, 8),  // kVK_ANSI_9
        0x1D => (0, 9),  // kVK_ANSI_0
        0x1B => (0, 10), // kVK_ANSI_Minus
        0x18 => (0, 11), // kVK_ANSI_Equal (JIS: ^)
        0x5E => (0, 12), // kVK_JIS_Yen (¥)

        // Row 1: Q row
        0x0C => (1, 0),  // kVK_ANSI_Q
        0x0D => (1, 1),  // kVK_ANSI_W
        0x0E => (1, 2),  // kVK_ANSI_E
        0x0F => (1, 3),  // kVK_ANSI_R
        0x11 => (1, 4),  // kVK_ANSI_T
        0x10 => (1, 5),  // kVK_ANSI_Y
        0x20 => (1, 6),  // kVK_ANSI_U
        0x22 => (1, 7),  // kVK_ANSI_I
        0x1F => (1, 8),  // kVK_ANSI_O
        0x23 => (1, 9),  // kVK_ANSI_P
        0x21 => (1, 10), // kVK_ANSI_LeftBracket (JIS: @)
        0x1E => (1, 11), // kVK_ANSI_RightBracket (JIS: [)

        // Row 2: A row (home row)
        0x00 => (2, 0),  // kVK_ANSI_A
        0x01 => (2, 1),  // kVK_ANSI_S
        0x02 => (2, 2),  // kVK_ANSI_D
        0x03 => (2, 3),  // kVK_ANSI_F
        0x05 => (2, 4),  // kVK_ANSI_G
        0x04 => (2, 5),  // kVK_ANSI_H
        0x26 => (2, 6),  // kVK_ANSI_J
        0x28 => (2, 7),  // kVK_ANSI_K
        0x25 => (2, 8),  // kVK_ANSI_L
        0x29 => (2, 9),  // kVK_ANSI_Semicolon (JIS: ;)
        0x27 => (2, 10), // kVK_ANSI_Quote (JIS: :)
        0x2A => (2, 11), // kVK_ANSI_Backslash (JIS: ])

        // Row 3: Z row
        0x06 => (3, 0),  // kVK_ANSI_Z
        0x07 => (3, 1),  // kVK_ANSI_X
        0x08 => (3, 2),  // kVK_ANSI_C
        0x09 => (3, 3),  // kVK_ANSI_V
        0x0B => (3, 4),  // kVK_ANSI_B
        0x2D => (3, 5),  // kVK_ANSI_N
        0x2E => (3, 6),  // kVK_ANSI_M
        0x2B => (3, 7),  // kVK_ANSI_Comma
        0x2F => (3, 8),  // kVK_ANSI_Period
        0x2C => (3, 9),  // kVK_ANSI_Slash
        0x5D => (3, 10), // kVK_JIS_Underscore (_)

        _ => return None,
    };
    Some(PhysicalPos::new(row, col))
}
