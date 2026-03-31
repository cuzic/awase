//! macOS キーボードフック (CGEventTap)

use awase::scanmap::PhysicalPos;
use awase::types::{ImeRelevance, KeyClassification, ModifierKey, VkCode};
use std::sync::atomic::{AtomicU16, Ordering};

use crate::scanmap::keycode_to_pos;

/// 左親指キーの macOS keycode（デフォルト: 英数 0x66）
static LEFT_THUMB_KEYCODE: AtomicU16 = AtomicU16::new(0x66);
/// 右親指キーの macOS keycode（デフォルト: かな 0x68）
static RIGHT_THUMB_KEYCODE: AtomicU16 = AtomicU16::new(0x68);

/// 親指キーの keycode を設定する
pub fn set_thumb_keycodes(left: VkCode, right: VkCode) {
    LEFT_THUMB_KEYCODE.store(left.0, Ordering::Relaxed);
    RIGHT_THUMB_KEYCODE.store(right.0, Ordering::Relaxed);
}

/// macOS keycode からキー分類と物理位置を生成する
#[must_use]
pub fn classify_key(keycode: u16) -> (KeyClassification, Option<PhysicalPos>) {
    let left = LEFT_THUMB_KEYCODE.load(Ordering::Relaxed);
    let right = RIGHT_THUMB_KEYCODE.load(Ordering::Relaxed);

    if keycode == left {
        (KeyClassification::LeftThumb, None)
    } else if keycode == right {
        (KeyClassification::RightThumb, None)
    } else if is_passthrough(keycode) {
        (KeyClassification::Passthrough, None)
    } else if let Some(pos) = keycode_to_pos(keycode) {
        (KeyClassification::Char, Some(pos))
    } else {
        (KeyClassification::Passthrough, None)
    }
}

/// macOS keycode から修飾キー分類を生成する
#[must_use]
pub const fn classify_modifier(keycode: u16) -> Option<ModifierKey> {
    match keycode {
        0x38 | 0x3C => Some(ModifierKey::Shift), // LShift / RShift
        0x3B | 0x3E => Some(ModifierKey::Ctrl),  // LControl / RControl
        0x3A | 0x3D => Some(ModifierKey::Alt),   // LOption / ROption
        0x37 | 0x36 => Some(ModifierKey::Meta),  // LCommand / RCommand
        _ => None,
    }
}

/// パススルーキー判定
const fn is_passthrough(keycode: u16) -> bool {
    matches!(
        keycode,
        // Modifiers
        0x38 | 0x3C | 0x3B | 0x3E | 0x3A | 0x3D | 0x37 | 0x36 |
        // Caps Lock
        0x39 |
        // Function keys F1-F12
        0x7A | 0x78 | 0x63 | 0x76 | 0x60 | 0x61 | 0x62 | 0x64 |
        0x65 | 0x6D | 0x67 | 0x6F |
        // Navigation
        0x7B | 0x7C | 0x7D | 0x7E | // Arrow keys
        0x73 | 0x77 | 0x74 | 0x79 | // Home End PageUp PageDown
        // Escape
        0x35 |
        // Tab
        0x30
    )
}

/// IME 関連の事前分類（macOS 版スタブ）
#[must_use]
pub fn classify_ime_relevance(_keycode: u16) -> ImeRelevance {
    // macOS では英数/かな キーで IME を切り替える
    // 将来的に TISCopyCurrentKeyboardInputSource と連携
    ImeRelevance::default()
}

/// アクセシビリティ権限チェック
#[cfg(target_os = "macos")]
pub fn check_accessibility_permission() -> bool {
    // AXIsProcessTrusted() を呼ぶ
    // 未許可なら AXIsProcessTrustedWithOptions() でダイアログ表示
    todo!("macOS accessibility permission check")
}

/// アクセシビリティ権限チェック（非 macOS スタブ）
#[cfg(not(target_os = "macos"))]
pub fn check_accessibility_permission() -> bool {
    log::warn!("Accessibility permission check is macOS only");
    false
}
