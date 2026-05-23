/// 共有テストフィクスチャ: 複数のエンジンテストモジュールで使い回す定数・ヘルパー。
///
/// このモジュールは `#[cfg(test)]` 専用。
/// 各テストモジュールは `use super::test_support::*;`（または `super::super::test_support::*;`）で使う。
use crate::scanmap::PhysicalPos;
use crate::types::{ScanCode, VkCode};
use crate::yab::{YabFace, YabLayout, YabValue};

// ── VK コード ──

pub const VK_A: VkCode = VkCode::new(0x41);
pub const VK_S: VkCode = VkCode::new(0x53);
pub const VK_NONCONVERT: VkCode = VkCode::new(0x1D);
pub const VK_CONVERT: VkCode = VkCode::new(0x1C);

// ── スキャンコード ──

pub const SCAN_A: ScanCode = ScanCode::new(0x1E);
pub const SCAN_S: ScanCode = ScanCode::new(0x1F);
pub const SCAN_NONCONVERT: ScanCode = ScanCode::new(0x7B);
pub const SCAN_CONVERT: ScanCode = ScanCode::new(0x79);

// ── 物理位置 ──

pub const POS_A: PhysicalPos = PhysicalPos::new(2, 0);
pub const POS_S: PhysicalPos = PhysicalPos::new(2, 1);

// ── ヘルパー ──

pub fn lit(ch: char) -> YabValue {
    YabValue::Literal(ch.to_string())
}

/// テスト用標準レイアウト（A=う、S=し、左親指A=を、左親指S=あ、右親指A=ゔ、右親指S=じ）
pub fn make_layout() -> YabLayout {
    let mut normal = YabFace::new();
    normal.insert(POS_A, lit('う'));
    normal.insert(POS_S, lit('し'));

    let mut left_thumb = YabFace::new();
    left_thumb.insert(POS_A, lit('を'));
    left_thumb.insert(POS_S, lit('あ'));

    let mut right_thumb = YabFace::new();
    right_thumb.insert(POS_A, lit('ゔ'));
    right_thumb.insert(POS_S, lit('じ'));

    YabLayout {
        name: String::from("test"),
        normal,
        left_thumb,
        right_thumb,
        shift: YabFace::new(),
    }
}
