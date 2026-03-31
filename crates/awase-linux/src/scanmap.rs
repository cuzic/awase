//! evdev キーコード ⇔ PhysicalPos マッピング（JIS キーボード）
//!
//! evdev キーコードは Linux カーネルの input サブシステムで定義される。
//! 大部分は Windows Set 1 スキャンコードと同一値を持つ。

use awase::scanmap::PhysicalPos;

/// evdev キーコード → JIS キーボード物理位置
///
/// NICOLA 配列で使用する文字キー（数字行・Q行・A行・Z行）のみを対象とする。
/// 親指キー（変換・無変換・スペース等）は含まない。
#[must_use]
pub const fn evdev_to_pos(keycode: u32) -> Option<PhysicalPos> {
    let (row, col) = match keycode {
        // Row 0: number row — KEY_1(2)..KEY_0(11), KEY_MINUS(12), KEY_EQUAL(13), KEY_YEN(124)
        2 => (0, 0),    // KEY_1
        3 => (0, 1),    // KEY_2
        4 => (0, 2),    // KEY_3
        5 => (0, 3),    // KEY_4
        6 => (0, 4),    // KEY_5
        7 => (0, 5),    // KEY_6
        8 => (0, 6),    // KEY_7
        9 => (0, 7),    // KEY_8
        10 => (0, 8),   // KEY_9
        11 => (0, 9),   // KEY_0
        12 => (0, 10),  // KEY_MINUS
        13 => (0, 11),  // KEY_EQUAL (JIS: ^)
        124 => (0, 12), // KEY_YEN (JIS: ¥)

        // Row 1: Q row — KEY_Q(16)..KEY_RIGHTBRACE(27)
        16 => (1, 0),  // KEY_Q
        17 => (1, 1),  // KEY_W
        18 => (1, 2),  // KEY_E
        19 => (1, 3),  // KEY_R
        20 => (1, 4),  // KEY_T
        21 => (1, 5),  // KEY_Y
        22 => (1, 6),  // KEY_U
        23 => (1, 7),  // KEY_I
        24 => (1, 8),  // KEY_O
        25 => (1, 9),  // KEY_P
        26 => (1, 10), // KEY_LEFTBRACE (JIS: @)
        27 => (1, 11), // KEY_RIGHTBRACE (JIS: [)

        // Row 2: A row — KEY_A(30)..KEY_APOSTROPHE(40), KEY_BACKSLASH(43)
        30 => (2, 0),  // KEY_A
        31 => (2, 1),  // KEY_S
        32 => (2, 2),  // KEY_D
        33 => (2, 3),  // KEY_F
        34 => (2, 4),  // KEY_G
        35 => (2, 5),  // KEY_H
        36 => (2, 6),  // KEY_J
        37 => (2, 7),  // KEY_K
        38 => (2, 8),  // KEY_L
        39 => (2, 9),  // KEY_SEMICOLON
        40 => (2, 10), // KEY_APOSTROPHE (JIS: :)
        43 => (2, 11), // KEY_BACKSLASH (JIS: ])

        // Row 3: Z row — KEY_Z(44)..KEY_SLASH(53), KEY_RO(89)
        44 => (3, 0),  // KEY_Z
        45 => (3, 1),  // KEY_X
        46 => (3, 2),  // KEY_C
        47 => (3, 3),  // KEY_V
        48 => (3, 4),  // KEY_B
        49 => (3, 5),  // KEY_N
        50 => (3, 6),  // KEY_M
        51 => (3, 7),  // KEY_COMMA
        52 => (3, 8),  // KEY_DOT
        53 => (3, 9),  // KEY_SLASH
        89 => (3, 10), // KEY_RO (JIS: _)

        _ => return None,
    };
    Some(PhysicalPos::new(row, col))
}
