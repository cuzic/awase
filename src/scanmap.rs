/// Physical key position on the keyboard (row, column).
///
/// Row 0 = number row, Row 1 = upper letter row (Q row on QWERTY),
/// Row 2 = home row (A row), Row 3 = lower letter row (Z row).
/// Column 0 = leftmost key in that row.
///
/// この座標系はキーボードモデル（JIS, US 等）に依存しない。
/// 各モデルでの行あたりキー数は `KeyboardModel` で定義される。
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

/// キーボードの物理レイアウトモデル
///
/// 行ごとのキー数はモデルによって異なる。
/// .yab レイアウトのパース時と、プラットフォーム層のキーコード変換で使用される。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyboardModel {
    /// JIS キーボード (日本語109キー)
    /// Row 0: 13, Row 1: 12, Row 2: 12, Row 3: 11
    #[default]
    Jis,
    /// US キーボード (ANSI 104キー)
    /// Row 0: 13, Row 1: 12, Row 2: 11, Row 3: 10
    Us,
}

impl KeyboardModel {
    /// 各行のキー数上限を返す
    #[must_use]
    pub const fn row_sizes(&self) -> [usize; 4] {
        match self {
            Self::Jis => [13, 12, 12, 11],
            Self::Us => [13, 12, 11, 10],
        }
    }
}

impl std::fmt::Display for KeyboardModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Jis => write!(f, "jis"),
            Self::Us => write!(f, "us"),
        }
    }
}

impl std::str::FromStr for KeyboardModel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "jis" | "jp" | "jis109" => Ok(Self::Jis),
            "us" | "ansi" | "us104" => Ok(Self::Us),
            _ => Err(format!("Unknown keyboard model: {s}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    // ── KeyboardModel::row_sizes ──

    #[test]
    fn jis_row_sizes() {
        assert_eq!(KeyboardModel::Jis.row_sizes(), [13, 12, 12, 11]);
    }

    #[test]
    fn us_row_sizes() {
        assert_eq!(KeyboardModel::Us.row_sizes(), [13, 12, 11, 10]);
    }

    // ── KeyboardModel::Display ──

    #[test]
    fn display_jis() {
        assert_eq!(format!("{}", KeyboardModel::Jis), "jis");
    }

    #[test]
    fn display_us() {
        assert_eq!(format!("{}", KeyboardModel::Us), "us");
    }

    // ── KeyboardModel::FromStr ──

    #[test]
    fn from_str_jis_variants() {
        assert_eq!(KeyboardModel::from_str("jis").unwrap(), KeyboardModel::Jis);
        assert_eq!(KeyboardModel::from_str("jp").unwrap(), KeyboardModel::Jis);
        assert_eq!(
            KeyboardModel::from_str("jis109").unwrap(),
            KeyboardModel::Jis
        );
        assert_eq!(KeyboardModel::from_str("JIS").unwrap(), KeyboardModel::Jis);
    }

    #[test]
    fn from_str_us_variants() {
        assert_eq!(KeyboardModel::from_str("us").unwrap(), KeyboardModel::Us);
        assert_eq!(KeyboardModel::from_str("ansi").unwrap(), KeyboardModel::Us);
        assert_eq!(KeyboardModel::from_str("us104").unwrap(), KeyboardModel::Us);
        assert_eq!(KeyboardModel::from_str("US").unwrap(), KeyboardModel::Us);
    }

    #[test]
    fn from_str_invalid() {
        assert!(KeyboardModel::from_str("invalid").is_err());
        assert!(KeyboardModel::from_str("").is_err());
    }

    // ── KeyboardModel::Default ──

    #[test]
    fn default_is_jis() {
        assert_eq!(KeyboardModel::default(), KeyboardModel::Jis);
    }

    // ── PhysicalPos ──

    #[test]
    fn physical_pos_new_and_fields() {
        let pos = PhysicalPos::new(2, 5);
        assert_eq!(pos.row, 2);
        assert_eq!(pos.col, 5);
    }

    #[test]
    fn physical_pos_equality() {
        assert_eq!(PhysicalPos::new(0, 0), PhysicalPos::new(0, 0));
        assert_ne!(PhysicalPos::new(0, 0), PhysicalPos::new(0, 1));
        assert_ne!(PhysicalPos::new(0, 0), PhysicalPos::new(1, 0));
    }
}
