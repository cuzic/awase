use awase::types::KeyAction;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, VIRTUAL_KEY,
};

/// 自己注入マーカー（"KEYM" = 0x4B45_594D）
pub const INJECTED_MARKER: usize = 0x4B45_594D;

/// ASCII 文字を対応する VK コードに変換する。
/// a-z, A-Z, 0-9, および一般的な句読点のみを扱う。
/// 戻り値は `(vk_code, needs_shift)`。
const fn ascii_to_vk(ch: char) -> Option<(u16, bool)> {
    match ch {
        'a'..='z' => Some((0x41 + (ch as u16 - 'a' as u16), false)),
        'A'..='Z' => Some((0x41 + (ch as u16 - 'A' as u16), true)),
        '0'..='9' => Some((0x30 + (ch as u16 - '0' as u16), false)),
        '-' => Some((0xBD, false)), // VK_OEM_MINUS
        '.' => Some((0xBE, false)), // VK_OEM_PERIOD
        ',' => Some((0xBC, false)), // VK_OEM_COMMA
        '/' => Some((0xBF, false)), // VK_OEM_2
        _ => None,
    }
}

/// SendInput によるキー注入を行うモジュール
pub struct Output;

impl Output {
    pub const fn new() -> Self {
        Self
    }

    /// アクション列を順に実行する
    pub fn send_keys(&self, actions: &[KeyAction]) {
        for action in actions {
            match action {
                KeyAction::Key(vk) => {
                    self.send_key(*vk, false);
                }
                KeyAction::KeyUp(vk) => {
                    self.send_key(*vk, true);
                }
                KeyAction::Char(ch) => {
                    self.send_unicode_char(*ch);
                }
                KeyAction::Suppress => {
                    // 何もしない
                }
                KeyAction::Romaji(s) => {
                    self.send_romaji(s);
                }
            }
        }
    }

    /// 仮想キーコードを使って KeyDown または KeyUp を送信する
    #[allow(clippy::unused_self)] // メソッドチェーンの一貫性のため &self を保持
    fn send_key(&self, vk: u16, is_keyup: bool) {
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: 0,
                    dwFlags: if is_keyup {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS::default()
                    },
                    time: 0,
                    dwExtraInfo: INJECTED_MARKER,
                },
            },
        };
        unsafe {
            SendInput(&[input], i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"));
        }
    }

    /// Unicode 文字を直接送信する（`KEYEVENTF_UNICODE`）
    /// BMP 外の文字（U+10000 以上）はサロゲートペアとして 2 回に分けて送信する
    #[allow(clippy::unused_self)]
    fn send_unicode_char(&self, ch: char) {
        let mut utf16_buf = [0u16; 2];
        let utf16 = ch.encode_utf16(&mut utf16_buf);

        let mut inputs = Vec::with_capacity(utf16.len() * 2);
        for &code_unit in utf16.iter() {
            // KeyDown
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: code_unit,
                        dwFlags: KEYEVENTF_UNICODE,
                        time: 0,
                        dwExtraInfo: INJECTED_MARKER,
                    },
                },
            });
            // KeyUp
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: code_unit,
                        dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: INJECTED_MARKER,
                    },
                },
            });
        }

        unsafe {
            SendInput(&inputs, i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"));
        }
    }

    /// ローマ字文字列を VK コードのキーイベントとして送信する。
    /// 各文字を個別の KeyDown/KeyUp として送り、大文字の場合は Shift を付加する。
    fn send_romaji(&self, romaji: &str) {
        for ch in romaji.chars() {
            if let Some((vk, needs_shift)) = ascii_to_vk(ch) {
                if needs_shift {
                    self.send_key(0xA0, false); // LShift down
                }
                self.send_key(vk, false); // key down
                self.send_key(vk, true); // key up
                if needs_shift {
                    self.send_key(0xA0, true); // LShift up
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ascii_to_vk_lowercase() {
        assert_eq!(ascii_to_vk('a'), Some((0x41, false)));
        assert_eq!(ascii_to_vk('z'), Some((0x5A, false)));
    }

    #[test]
    fn test_ascii_to_vk_uppercase() {
        assert_eq!(ascii_to_vk('A'), Some((0x41, true)));
    }

    #[test]
    fn test_ascii_to_vk_digits() {
        assert_eq!(ascii_to_vk('0'), Some((0x30, false)));
        assert_eq!(ascii_to_vk('9'), Some((0x39, false)));
    }

    #[test]
    fn test_ascii_to_vk_unknown() {
        assert_eq!(ascii_to_vk('\u{3042}'), None); // 'あ'
    }
}
