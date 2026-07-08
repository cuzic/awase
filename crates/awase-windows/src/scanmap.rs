//! Windows Set 1 スキャンコード ⇔ PhysicalPos マッピング
//!
//! JIS/US とも、行・列内の主要キー（数字行・QWERTY 行・ASDF 行・ZXCV 行の
//! アルファベット部分）は物理的に同じ Set 1 スキャンコードを共有する。差分は
//! JIS 固有キー（半角/全角・row2 の `]`・`ろ`・¥）の有無のみ。
//! US 側にだけあるグレイブキー（`` ` ``、scan 0x29）は JIS の半角/全角と
//! 同様グリッド外パススルーとして扱う（NICOLA ローマ字面に割り当てる実用上の
//! 必要性が薄いため）。

use awase::scanmap::{KeyboardModel, PhysicalPos};
use awase::types::ScanCode;

/// Windows Set 1 スキャンコード → 物理位置（`model` に応じたテーブルを使う）
#[must_use]
pub const fn scan_to_pos(model: KeyboardModel, scan_code: ScanCode) -> Option<PhysicalPos> {
    match model {
        KeyboardModel::Jis => scan_to_pos_jis(scan_code),
        KeyboardModel::Us => scan_to_pos_us(scan_code),
    }
}

/// 物理位置 → Windows Set 1 スキャンコード（`model` に応じたテーブルを使う）
#[must_use]
pub const fn pos_to_scan(model: KeyboardModel, pos: PhysicalPos) -> Option<ScanCode> {
    match model {
        KeyboardModel::Jis => pos_to_scan_jis(pos),
        KeyboardModel::Us => pos_to_scan_us(pos),
    }
}

/// Windows Set 1 スキャンコード → JIS キーボード物理位置
#[must_use]
const fn scan_to_pos_jis(scan_code: ScanCode) -> Option<PhysicalPos> {
    let (row, col) = match scan_code.0 {
        0x02 => (0, 0),
        0x03 => (0, 1),
        0x04 => (0, 2),
        0x05 => (0, 3),
        0x06 => (0, 4),
        0x07 => (0, 5),
        0x08 => (0, 6),
        0x09 => (0, 7),
        0x0A => (0, 8),
        0x0B => (0, 9),
        0x0C => (0, 10),
        0x0D => (0, 11),
        0x7D => (0, 12),
        0x10 => (1, 0),
        0x11 => (1, 1),
        0x12 => (1, 2),
        0x13 => (1, 3),
        0x14 => (1, 4),
        0x15 => (1, 5),
        0x16 => (1, 6),
        0x17 => (1, 7),
        0x18 => (1, 8),
        0x19 => (1, 9),
        0x1A => (1, 10),
        0x1B => (1, 11),
        0x1E => (2, 0),
        0x1F => (2, 1),
        0x20 => (2, 2),
        0x21 => (2, 3),
        0x22 => (2, 4),
        0x23 => (2, 5),
        0x24 => (2, 6),
        0x25 => (2, 7),
        0x26 => (2, 8),
        0x27 => (2, 9),
        0x28 => (2, 10),
        0x2B => (2, 11),
        0x2C => (3, 0),
        0x2D => (3, 1),
        0x2E => (3, 2),
        0x2F => (3, 3),
        0x30 => (3, 4),
        0x31 => (3, 5),
        0x32 => (3, 6),
        0x33 => (3, 7),
        0x34 => (3, 8),
        0x35 => (3, 9),
        0x73 => (3, 10),
        _ => return None,
    };
    Some(PhysicalPos::new(row, col))
}

/// JIS キーボード物理位置 → Windows Set 1 スキャンコード
#[must_use]
const fn pos_to_scan_jis(pos: PhysicalPos) -> Option<ScanCode> {
    let raw = match (pos.row, pos.col) {
        (0, 0) => 0x02,
        (0, 1) => 0x03,
        (0, 2) => 0x04,
        (0, 3) => 0x05,
        (0, 4) => 0x06,
        (0, 5) => 0x07,
        (0, 6) => 0x08,
        (0, 7) => 0x09,
        (0, 8) => 0x0A,
        (0, 9) => 0x0B,
        (0, 10) => 0x0C,
        (0, 11) => 0x0D,
        (0, 12) => 0x7D,
        (1, 0) => 0x10,
        (1, 1) => 0x11,
        (1, 2) => 0x12,
        (1, 3) => 0x13,
        (1, 4) => 0x14,
        (1, 5) => 0x15,
        (1, 6) => 0x16,
        (1, 7) => 0x17,
        (1, 8) => 0x18,
        (1, 9) => 0x19,
        (1, 10) => 0x1A,
        (1, 11) => 0x1B,
        (2, 0) => 0x1E,
        (2, 1) => 0x1F,
        (2, 2) => 0x20,
        (2, 3) => 0x21,
        (2, 4) => 0x22,
        (2, 5) => 0x23,
        (2, 6) => 0x24,
        (2, 7) => 0x25,
        (2, 8) => 0x26,
        (2, 9) => 0x27,
        (2, 10) => 0x28,
        (2, 11) => 0x2B,
        (3, 0) => 0x2C,
        (3, 1) => 0x2D,
        (3, 2) => 0x2E,
        (3, 3) => 0x2F,
        (3, 4) => 0x30,
        (3, 5) => 0x31,
        (3, 6) => 0x32,
        (3, 7) => 0x33,
        (3, 8) => 0x34,
        (3, 9) => 0x35,
        (3, 10) => 0x73,
        _ => return None,
    };
    Some(ScanCode(raw))
}

/// Windows Set 1 スキャンコード → US (ANSI 104) キーボード物理位置
///
/// JIS と同じスキャンコードを共有する主要キーは値をそのまま流用する。
/// JIS 固有キー（半角/全角 0x29・row2 の `]` 0x2B・`ろ` 0x73・¥ 0x7D）は
/// US 配列に物理キーが存在しないため、グリッド外（`None` → パススルー）とする。
#[must_use]
const fn scan_to_pos_us(scan_code: ScanCode) -> Option<PhysicalPos> {
    let (row, col) = match scan_code.0 {
        0x02 => (0, 0),
        0x03 => (0, 1),
        0x04 => (0, 2),
        0x05 => (0, 3),
        0x06 => (0, 4),
        0x07 => (0, 5),
        0x08 => (0, 6),
        0x09 => (0, 7),
        0x0A => (0, 8),
        0x0B => (0, 9),
        0x0C => (0, 10),
        0x0D => (0, 11),
        0x10 => (1, 0),
        0x11 => (1, 1),
        0x12 => (1, 2),
        0x13 => (1, 3),
        0x14 => (1, 4),
        0x15 => (1, 5),
        0x16 => (1, 6),
        0x17 => (1, 7),
        0x18 => (1, 8),
        0x19 => (1, 9),
        0x1A => (1, 10),
        0x1B => (1, 11),
        0x1E => (2, 0),
        0x1F => (2, 1),
        0x20 => (2, 2),
        0x21 => (2, 3),
        0x22 => (2, 4),
        0x23 => (2, 5),
        0x24 => (2, 6),
        0x25 => (2, 7),
        0x26 => (2, 8),
        0x27 => (2, 9),
        0x28 => (2, 10),
        0x2C => (3, 0),
        0x2D => (3, 1),
        0x2E => (3, 2),
        0x2F => (3, 3),
        0x30 => (3, 4),
        0x31 => (3, 5),
        0x32 => (3, 6),
        0x33 => (3, 7),
        0x34 => (3, 8),
        0x35 => (3, 9),
        _ => return None,
    };
    Some(PhysicalPos::new(row, col))
}

/// US (ANSI 104) キーボード物理位置 → Windows Set 1 スキャンコード
#[must_use]
const fn pos_to_scan_us(pos: PhysicalPos) -> Option<ScanCode> {
    let raw = match (pos.row, pos.col) {
        (0, 0) => 0x02,
        (0, 1) => 0x03,
        (0, 2) => 0x04,
        (0, 3) => 0x05,
        (0, 4) => 0x06,
        (0, 5) => 0x07,
        (0, 6) => 0x08,
        (0, 7) => 0x09,
        (0, 8) => 0x0A,
        (0, 9) => 0x0B,
        (0, 10) => 0x0C,
        (0, 11) => 0x0D,
        (1, 0) => 0x10,
        (1, 1) => 0x11,
        (1, 2) => 0x12,
        (1, 3) => 0x13,
        (1, 4) => 0x14,
        (1, 5) => 0x15,
        (1, 6) => 0x16,
        (1, 7) => 0x17,
        (1, 8) => 0x18,
        (1, 9) => 0x19,
        (1, 10) => 0x1A,
        (1, 11) => 0x1B,
        (2, 0) => 0x1E,
        (2, 1) => 0x1F,
        (2, 2) => 0x20,
        (2, 3) => 0x21,
        (2, 4) => 0x22,
        (2, 5) => 0x23,
        (2, 6) => 0x24,
        (2, 7) => 0x25,
        (2, 8) => 0x26,
        (2, 9) => 0x27,
        (2, 10) => 0x28,
        (3, 0) => 0x2C,
        (3, 1) => 0x2D,
        (3, 2) => 0x2E,
        (3, 3) => 0x2F,
        (3, 4) => 0x30,
        (3, 5) => 0x31,
        (3, 6) => 0x32,
        (3, 7) => 0x33,
        (3, 8) => 0x34,
        (3, 9) => 0x35,
        _ => return None,
    };
    Some(ScanCode(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jis_and_us_round_trip_all_grid_positions() {
        for model in [KeyboardModel::Jis, KeyboardModel::Us] {
            let row_sizes = model.row_sizes();
            for (row, &max_col) in row_sizes.iter().enumerate() {
                for col in 0..max_col {
                    let row_u8 = u8::try_from(row).unwrap();
                    let col_u8 = u8::try_from(col).unwrap();
                    let pos = PhysicalPos::new(row_u8, col_u8);
                    let scan = pos_to_scan(model, pos)
                        .unwrap_or_else(|| panic!("{model} {pos:?} has no scan code"));
                    assert_eq!(
                        scan_to_pos(model, scan),
                        Some(pos),
                        "{model} {pos:?} round trip via scan {scan:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn us_excludes_jis_only_keys() {
        // 半角/全角(0x29)・row2 の JIS 拡張キー`]`(0x2B)・ろ(0x73)・¥(0x7D)
        for jis_only_scan in [0x29, 0x2B, 0x73, 0x7D] {
            assert_eq!(scan_to_pos_us(ScanCode(jis_only_scan)), None);
        }
    }

    #[test]
    fn jis_and_us_share_scan_codes_for_common_letters() {
        // Q キー位置 (row1, col0) は JIS/US とも scan 0x10 を共有する
        let pos = PhysicalPos::new(1, 0);
        assert_eq!(
            pos_to_scan(KeyboardModel::Jis, pos),
            pos_to_scan(KeyboardModel::Us, pos)
        );
    }
}
