//! macOS キー出力 (CGEventPost)

use awase::types::{KeyAction, SpecialKey};

/// SpecialKey を macOS keycode に変換する
pub const fn special_key_to_keycode(sk: SpecialKey) -> u16 {
    match sk {
        SpecialKey::Backspace => 0x33,
        SpecialKey::Escape => 0x35,
        SpecialKey::Enter => 0x24,
        SpecialKey::Space => 0x31,
        SpecialKey::Delete => 0x75,
    }
}

/// ASCII 文字を macOS keycode に変換する
pub const fn ascii_to_keycode(ch: char) -> Option<(u16, bool)> {
    match ch {
        'a'..='z' => {
            // macOS keycodes are NOT sequential like VK codes
            // Map each letter individually
            let keycode = match ch {
                'a' => 0x00,
                'b' => 0x0B,
                'c' => 0x08,
                'd' => 0x02,
                'e' => 0x0E,
                'f' => 0x03,
                'g' => 0x05,
                'h' => 0x04,
                'i' => 0x22,
                'j' => 0x26,
                'k' => 0x28,
                'l' => 0x25,
                'm' => 0x2E,
                'n' => 0x2D,
                'o' => 0x1F,
                'p' => 0x23,
                'q' => 0x0C,
                'r' => 0x0F,
                's' => 0x01,
                't' => 0x11,
                'u' => 0x20,
                'v' => 0x09,
                'w' => 0x0D,
                'x' => 0x07,
                'y' => 0x10,
                'z' => 0x06,
                _ => return None,
            };
            Some((keycode, false))
        }
        'A'..='Z' => {
            // Same keycode as lowercase, but with shift
            let lower = (ch as u8 + 32) as char;
            if let Some((kc, _)) = ascii_to_keycode(lower) {
                Some((kc, true))
            } else {
                None
            }
        }
        '0' => Some((0x1D, false)),
        '1' => Some((0x12, false)),
        '2' => Some((0x13, false)),
        '3' => Some((0x14, false)),
        '4' => Some((0x15, false)),
        '5' => Some((0x17, false)),
        '6' => Some((0x16, false)),
        '7' => Some((0x1A, false)),
        '8' => Some((0x1C, false)),
        '9' => Some((0x19, false)),
        '-' => Some((0x1B, false)),
        '.' => Some((0x2F, false)),
        ',' => Some((0x2B, false)),
        '/' => Some((0x2C, false)),
        _ => None,
    }
}

/// macOS キー出力（スタブ実装）
///
/// 実際の CGEventPost 呼び出しは macOS ビルド時のみ有効。
#[derive(Debug)]
pub struct Output;

impl Output {
    pub fn new() -> Self {
        Self
    }

    pub fn send_keys(&mut self, actions: &[KeyAction]) {
        for action in actions {
            match action {
                KeyAction::SpecialKey(sk) => {
                    let _kc = special_key_to_keycode(*sk);
                    log::trace!("macOS output: SpecialKey({sk:?}) -> keycode 0x{_kc:02X}");
                }
                KeyAction::Key(vk) => {
                    log::trace!("macOS output: Key(0x{:02X})", vk.0);
                }
                KeyAction::KeyUp(vk) => {
                    log::trace!("macOS output: KeyUp(0x{:02X})", vk.0);
                }
                KeyAction::Char(ch) => {
                    log::trace!("macOS output: Char('{ch}') via CGEvent::set_string");
                }
                KeyAction::Romaji(s) => {
                    log::trace!("macOS output: Romaji(\"{s}\")");
                }
                KeyAction::Suppress => {}
            }
        }
    }
}

impl Default for Output {
    fn default() -> Self {
        Self::new()
    }
}
