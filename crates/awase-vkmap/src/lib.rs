//! Windows VKコード⇔物理キー位置(JIS配列)のマッピング表。
//!
//! `crates/awase-windows`（GJI/MS-IME制御の本体、windows 0.62等の重い依存を
//! 持つ）から、この純粋なデータマッピングだけを切り出したもの。
//! `awaza`（別リポジトリ、Windows専用のTSF TIP実装）が、`awase-windows`
//! 一式を引き込まずにこの表だけを再利用できるようにするための独立crate。
//!
//! 経緯: `awaza`側で当初この表を直接コピーしていたが、二重化を避けるため
//! 2026-08-24にここへ切り出した（design doc §7.1、ユーザー承認済み）。

use awase::scanmap::PhysicalPos;
use awase::types::VkCode;

/// Windows VK コードから物理キー位置（JIS キーボード）へのマッピング。
///
/// NICOLA 配列で使用する文字キー（数字行・Q行・A行・Z行）のみを対象とする。
/// 親指キー（変換・無変換・スペース等）は含まない。
#[must_use]
pub const fn vk_to_pos(vk: VkCode) -> Option<PhysicalPos> {
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
    Some(PhysicalPos::new(row, col))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_row_resolves() {
        assert_eq!(vk_to_pos(VkCode(0x31)), Some(PhysicalPos::new(0, 0)));
    }

    #[test]
    fn thumb_keys_are_not_mapped() {
        // 変換(0x1C)・無変換(0x1D)は対象外(呼び出し元が別途分類する)。
        assert_eq!(vk_to_pos(VkCode(0x1C)), None);
        assert_eq!(vk_to_pos(VkCode(0x1D)), None);
    }
}
