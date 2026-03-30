use crate::types::ScanCode;

/// Physical key position on the keyboard (row, column).
/// Row 0 = number row, Row 1 = Q row, Row 2 = A row, Row 3 = Z row.
/// Column 0 = leftmost key in that row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PhysicalPos {
    pub row: u8,
    pub col: u8,
}

impl PhysicalPos {
    #[must_use]
    pub const fn new(row: u8, col: u8) -> Self {
        Self { row, col }
    }
}

/// Maps a `ScanCode` to a physical key position on a JIS keyboard.
#[must_use]
pub const fn scan_to_pos(scan_code: ScanCode) -> Option<PhysicalPos> {
    let (row, col) = match scan_code.0 {
        // Row 0: number row (13 keys)
        0x02 => (0, 0),  // 1
        0x03 => (0, 1),  // 2
        0x04 => (0, 2),  // 3
        0x05 => (0, 3),  // 4
        0x06 => (0, 4),  // 5
        0x07 => (0, 5),  // 6
        0x08 => (0, 6),  // 7
        0x09 => (0, 7),  // 8
        0x0A => (0, 8),  // 9
        0x0B => (0, 9),  // 0
        0x0C => (0, 10), // -
        0x0D => (0, 11), // ^
        0x7D => (0, 12), // ¥

        // Row 1: Q row (12 keys)
        0x10 => (1, 0),  // Q
        0x11 => (1, 1),  // W
        0x12 => (1, 2),  // E
        0x13 => (1, 3),  // R
        0x14 => (1, 4),  // T
        0x15 => (1, 5),  // Y
        0x16 => (1, 6),  // U
        0x17 => (1, 7),  // I
        0x18 => (1, 8),  // O
        0x19 => (1, 9),  // P
        0x1A => (1, 10), // @
        0x1B => (1, 11), // [

        // Row 2: A row (12 keys)
        0x1E => (2, 0),  // A
        0x1F => (2, 1),  // S
        0x20 => (2, 2),  // D
        0x21 => (2, 3),  // F
        0x22 => (2, 4),  // G
        0x23 => (2, 5),  // H
        0x24 => (2, 6),  // J
        0x25 => (2, 7),  // K
        0x26 => (2, 8),  // L
        0x27 => (2, 9),  // ;
        0x28 => (2, 10), // :
        0x2B => (2, 11), // ]

        // Row 3: Z row (11 keys)
        0x2C => (3, 0),  // Z
        0x2D => (3, 1),  // X
        0x2E => (3, 2),  // C
        0x2F => (3, 3),  // V
        0x30 => (3, 4),  // B
        0x31 => (3, 5),  // N
        0x32 => (3, 6),  // M
        0x33 => (3, 7),  // ,
        0x34 => (3, 8),  // .
        0x35 => (3, 9),  // /
        0x73 => (3, 10), // _

        _ => return None,
    };
    Some(PhysicalPos::new(row, col))
}

/// Maps a physical key position back to a `ScanCode` on a JIS keyboard.
#[must_use]
pub const fn pos_to_scan(pos: PhysicalPos) -> Option<ScanCode> {
    let raw = match (pos.row, pos.col) {
        // Row 0: number row
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

        // Row 1: Q row
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

        // Row 2: A row
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

        // Row 3: Z row
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

#[cfg(test)]
mod tests {
    use super::*;

    /// All known scan codes mapped in scan_to_pos.
    const ALL_SCAN_CODES: &[ScanCode] = &[
        // Row 0
        ScanCode(0x02),
        ScanCode(0x03),
        ScanCode(0x04),
        ScanCode(0x05),
        ScanCode(0x06),
        ScanCode(0x07),
        ScanCode(0x08),
        ScanCode(0x09),
        ScanCode(0x0A),
        ScanCode(0x0B),
        ScanCode(0x0C),
        ScanCode(0x0D),
        ScanCode(0x7D),
        // Row 1
        ScanCode(0x10),
        ScanCode(0x11),
        ScanCode(0x12),
        ScanCode(0x13),
        ScanCode(0x14),
        ScanCode(0x15),
        ScanCode(0x16),
        ScanCode(0x17),
        ScanCode(0x18),
        ScanCode(0x19),
        ScanCode(0x1A),
        ScanCode(0x1B),
        // Row 2
        ScanCode(0x1E),
        ScanCode(0x1F),
        ScanCode(0x20),
        ScanCode(0x21),
        ScanCode(0x22),
        ScanCode(0x23),
        ScanCode(0x24),
        ScanCode(0x25),
        ScanCode(0x26),
        ScanCode(0x27),
        ScanCode(0x28),
        ScanCode(0x2B),
        // Row 3
        ScanCode(0x2C),
        ScanCode(0x2D),
        ScanCode(0x2E),
        ScanCode(0x2F),
        ScanCode(0x30),
        ScanCode(0x31),
        ScanCode(0x32),
        ScanCode(0x33),
        ScanCode(0x34),
        ScanCode(0x35),
        ScanCode(0x73),
    ];

    #[test]
    fn all_scan_codes_map_to_valid_positions() {
        for &sc in ALL_SCAN_CODES {
            assert!(
                scan_to_pos(sc).is_some(),
                "scan code {:#04X} should map to a position",
                sc.0
            );
        }
    }

    #[test]
    fn round_trip_scan_to_pos_to_scan() {
        for &sc in ALL_SCAN_CODES {
            let pos = scan_to_pos(sc).unwrap();
            let back = pos_to_scan(pos).unwrap();
            assert_eq!(
                sc, back,
                "round-trip failed for scan code {:#04X} -> pos({},{}) -> {:#04X}",
                sc.0, pos.row, pos.col, back.0
            );
        }
    }

    #[test]
    fn round_trip_pos_to_scan_to_pos() {
        for &sc in ALL_SCAN_CODES {
            let pos = scan_to_pos(sc).unwrap();
            let sc2 = pos_to_scan(pos).unwrap();
            let pos2 = scan_to_pos(sc2).unwrap();
            assert_eq!(pos, pos2);
        }
    }

    #[test]
    fn known_specific_mappings() {
        assert_eq!(scan_to_pos(ScanCode(0x1E)), Some(PhysicalPos::new(2, 0))); // A
        assert_eq!(scan_to_pos(ScanCode(0x10)), Some(PhysicalPos::new(1, 0))); // Q
        assert_eq!(scan_to_pos(ScanCode(0x2C)), Some(PhysicalPos::new(3, 0))); // Z
        assert_eq!(scan_to_pos(ScanCode(0x02)), Some(PhysicalPos::new(0, 0))); // 1
        assert_eq!(scan_to_pos(ScanCode(0x7D)), Some(PhysicalPos::new(0, 12))); // ¥
        assert_eq!(scan_to_pos(ScanCode(0x73)), Some(PhysicalPos::new(3, 10))); // _
    }

    #[test]
    fn unknown_scan_codes_return_none() {
        assert_eq!(scan_to_pos(ScanCode(0x00)), None);
        assert_eq!(scan_to_pos(ScanCode(0x01)), None);
        assert_eq!(scan_to_pos(ScanCode(0xFF)), None);
        assert_eq!(scan_to_pos(ScanCode(0x100)), None);
    }

    #[test]
    fn invalid_positions_return_none() {
        assert_eq!(pos_to_scan(PhysicalPos::new(0, 13)), None);
        assert_eq!(pos_to_scan(PhysicalPos::new(1, 12)), None);
        assert_eq!(pos_to_scan(PhysicalPos::new(3, 11)), None);
        assert_eq!(pos_to_scan(PhysicalPos::new(4, 0)), None);
    }

    #[test]
    fn row_key_counts() {
        let count = |row: u8| {
            ALL_SCAN_CODES
                .iter()
                .filter(|&&sc| scan_to_pos(sc).unwrap().row == row)
                .count()
        };
        assert_eq!(count(0), 13, "Row 0 should have 13 keys");
        assert_eq!(count(1), 12, "Row 1 should have 12 keys");
        assert_eq!(count(2), 12, "Row 2 should have 12 keys");
        assert_eq!(count(3), 11, "Row 3 should have 11 keys");
    }

    #[test]
    fn scan_code_from_conversions() {
        let code: ScanCode = 0x1E_u32.into();
        assert_eq!(code, ScanCode(0x1E));
        let raw: u32 = code.into();
        assert_eq!(raw, 0x1E);
    }
}
