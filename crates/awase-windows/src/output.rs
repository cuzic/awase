use std::collections::HashMap;

use awase::config::OutputMode;
use awase::types::{AppKind, KeyAction, SpecialKey};

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, VIRTUAL_KEY,
};

/// 自己注入マーカー（"KEYM" = 0x4B45_594D）
pub const INJECTED_MARKER: usize = 0x4B45_594D;

/// VK_LSHIFT の仮想キーコード
const VK_LSHIFT: u16 = 0xA0;

/// ASCII 文字を対応する VK コードに変換する。
const fn ascii_to_vk(ch: char) -> Option<(u16, bool)> {
    match ch {
        'a'..='z' => Some((0x41 + (ch as u16 - 'a' as u16), false)),
        'A'..='Z' => Some((0x41 + (ch as u16 - 'A' as u16), true)),
        '0'..='9' => Some((0x30 + (ch as u16 - '0' as u16), false)),
        '-' => Some((0xBD, false)),
        '.' => Some((0xBE, false)),
        ',' => Some((0xBC, false)),
        '/' => Some((0xBF, false)),
        _ => None,
    }
}

/// 半角 ASCII 文字をキーシーケンス用に VK コードに変換する。
/// `ascii_to_vk` より広い範囲の記号を対応する。JIS キーボード前提。
const fn ascii_to_vk_extended(ch: char) -> Option<(u16, bool)> {
    match ch {
        'a'..='z' => Some((0x41 + (ch as u16 - 'a' as u16), false)),
        'A'..='Z' => Some((0x41 + (ch as u16 - 'A' as u16), true)),
        '0' => Some((0x30, false)),
        '1' => Some((0x31, false)),
        '2' => Some((0x32, false)),
        '3' => Some((0x33, false)),
        '4' => Some((0x34, false)),
        '5' => Some((0x35, false)),
        '6' => Some((0x36, false)),
        '7' => Some((0x37, false)),
        '8' => Some((0x38, false)),
        '9' => Some((0x39, false)),
        // Shifted digits (JIS keyboard)
        '!' => Some((0x31, true)),  // Shift+1
        '"' => Some((0x32, true)),  // Shift+2
        '#' => Some((0x33, true)),  // Shift+3
        '$' => Some((0x34, true)),  // Shift+4
        '%' => Some((0x35, true)),  // Shift+5
        '&' => Some((0x36, true)),  // Shift+6
        '\'' => Some((0x37, true)), // Shift+7
        '(' => Some((0x38, true)),  // Shift+8
        ')' => Some((0x39, true)),  // Shift+9
        // Symbols (JIS keyboard)
        '-' => Some((0xBD, false)),  // VK_OEM_MINUS
        '=' => Some((0xBD, true)),   // Shift+- (JIS: =)
        '^' => Some((0xDE, false)),  // VK_OEM_7 (JIS: ^)
        '~' => Some((0xDE, true)),   // Shift+^ (JIS: ~)
        '\\' => Some((0xE2, false)), // VK_OEM_102 (JIS: ＼)
        '|' => Some((0xDC, true)),   // Shift+¥ (JIS: |)
        '@' => Some((0xC0, false)),  // VK_OEM_3 (JIS: @)
        '`' => Some((0xC0, true)),   // Shift+@ (JIS: `)
        '[' => Some((0xDB, false)),  // VK_OEM_4
        '{' => Some((0xDB, true)),   // Shift+[
        ']' => Some((0xDD, false)),  // VK_OEM_6
        '}' => Some((0xDD, true)),   // Shift+]
        ';' => Some((0xBB, false)),  // VK_OEM_PLUS (JIS: ;)
        '+' => Some((0xBB, true)),   // Shift+; (JIS: +)
        ':' => Some((0xBA, false)),  // VK_OEM_1 (JIS: :)
        '*' => Some((0xBA, true)),   // Shift+: (JIS: *)
        ',' => Some((0xBC, false)),  // VK_OEM_COMMA
        '<' => Some((0xBC, true)),   // Shift+,
        '.' => Some((0xBE, false)),  // VK_OEM_PERIOD
        '>' => Some((0xBE, true)),   // Shift+.
        '/' => Some((0xBF, false)),  // VK_OEM_2
        '?' => Some((0xBF, true)),   // Shift+/
        '_' => Some((0xE2, true)),   // Shift+＼ (JIS: _)
        _ => None,
    }
}

/// 全角文字を半角に変換する。
/// 全角 ASCII 範囲 (U+FF01..U+FF5E) に該当する場合、対応する半角文字を返す。
const fn fullwidth_to_halfwidth(ch: char) -> Option<char> {
    let cp = ch as u32;
    // 全角 ASCII: U+FF01 ('！') .. U+FF5E ('～')
    // 対応する半角: U+0021 ('!') .. U+007E ('~')
    if cp >= 0xFF01 && cp <= 0xFF5E {
        // const fn では char::from_u32 が使えないため直接変換
        let half_cp = cp - 0xFEE0;
        if half_cp <= 0x7F {
            Some(half_cp as u8 as char)
        } else {
            None
        }
    } else {
        None
    }
}

/// 文字をキーシーケンス用の VK コードに変換する。
/// 全角文字は半角に変換してから `ascii_to_vk_extended` で対応する。
fn char_to_key_sequence(ch: char) -> Option<(u16, bool)> {
    // まず全角→半角変換を試みる
    let half = fullwidth_to_halfwidth(ch).unwrap_or(ch);
    ascii_to_vk_extended(half)
}

/// SpecialKey を Windows VK コードに変換する
const fn special_key_to_vk(sk: SpecialKey) -> u16 {
    match sk {
        SpecialKey::Backspace => 0x08, // VK_BACK
        SpecialKey::Escape => 0x1B,    // VK_ESCAPE
        SpecialKey::Enter => 0x0D,     // VK_RETURN
        SpecialKey::Space => 0x20,     // VK_SPACE
        SpecialKey::Delete => 0x2E,    // VK_DELETE
    }
}

/// SendInput によるキー注入を行うモジュール
pub struct Output {
    mode: OutputMode,
    /// Unicode モード用: ローマ字→ひらがな変換テーブル
    romaji_to_kana: Option<HashMap<String, char>>,
}

impl Output {
    pub fn new(mode: OutputMode) -> Self {
        let romaji_to_kana = if mode == OutputMode::Unicode {
            Some(awase::kana_table::build_romaji_to_kana())
        } else {
            None
        };
        Self {
            mode,
            romaji_to_kana,
        }
    }

    /// 出力モードを変更する
    pub fn set_mode(&mut self, mode: OutputMode) {
        self.mode = mode;
        if mode == OutputMode::Unicode {
            self.romaji_to_kana
                .get_or_insert_with(awase::kana_table::build_romaji_to_kana);
        }
    }

    /// アクション列を順に実行する
    ///
    /// `AppKind` に応じて `Char` と `KeySequence` の出力方式を適応的に切り替える:
    /// - Chrome: Char/KeySequence ともに VK キーストローク（全角→半角変換問題の回避）
    /// - Win32/Uwp 等: Unicode 直接送信
    pub fn send_keys(&self, actions: &[KeyAction]) {
        let use_vk = AppKind::load(&crate::APP_KIND) == AppKind::Chrome;

        for action in actions {
            match action {
                KeyAction::SpecialKey(sk) => self.send_key(special_key_to_vk(*sk), false),
                KeyAction::Key(vk) => self.send_key(vk.0, false),
                KeyAction::KeyUp(vk) => self.send_key(vk.0, true),
                KeyAction::Char(ch) => {
                    if use_vk {
                        self.send_key_sequence(&ch.to_string());
                    } else {
                        self.send_unicode_char(*ch);
                    }
                }
                KeyAction::Suppress => {}
                KeyAction::Romaji(s) => self.send_romaji(s),
                KeyAction::KeySequence(s) => {
                    if use_vk {
                        self.send_key_sequence(s);
                    } else {
                        for ch in s.chars() {
                            self.send_unicode_char(ch);
                        }
                    }
                }
            }
        }
    }

    /// 仮想キーコードを使って即座に KeyDown/KeyUp を送信する
    #[allow(clippy::unused_self)]
    fn send_key(&self, vk: u16, is_keyup: bool) {
        let input = make_key_input(vk, is_keyup);
        unsafe {
            SendInput(
                &[input],
                i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
            );
        }
    }

    /// Unicode 文字を直接送信する（`KEYEVENTF_UNICODE`）
    #[allow(clippy::unused_self)]
    fn send_unicode_char(&self, ch: char) {
        let mut utf16_buf = [0u16; 2];
        let utf16 = ch.encode_utf16(&mut utf16_buf);

        let mut inputs = Vec::with_capacity(utf16.len() * 2);
        for &code_unit in utf16.iter() {
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
            SendInput(
                &inputs,
                i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
            );
        }
    }

    /// ローマ字文字列を送信する（モードに応じて方式を切り替え）
    fn send_romaji(&self, romaji: &str) {
        match self.mode {
            OutputMode::PerKey => self.send_romaji_per_key(romaji),
            OutputMode::Batched => self.send_romaji_batched(romaji),
            OutputMode::Unicode => self.send_romaji_as_unicode(romaji),
        }
    }

    /// PerKey モード: 1文字ずつ個別の SendInput 呼び出し
    ///
    /// 各文字の KeyDown+KeyUp は1回の SendInput にまとめるが、
    /// 文字間は別の SendInput 呼び出しに分離する。
    /// 他のキーボードフックに処理時間を与える。
    #[allow(clippy::unused_self)]
    fn send_romaji_per_key(&self, romaji: &str) {
        for ch in romaji.chars() {
            if let Some((vk, needs_shift)) = ascii_to_vk(ch) {
                let mut inputs = Vec::with_capacity(4);
                if needs_shift {
                    inputs.push(make_key_input(VK_LSHIFT, false));
                }
                inputs.push(make_key_input(vk, false));
                inputs.push(make_key_input(vk, true));
                if needs_shift {
                    inputs.push(make_key_input(VK_LSHIFT, true));
                }
                unsafe {
                    SendInput(
                        &inputs,
                        i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                    );
                }
            }
        }
    }

    /// Batched モード: 全文字を1回の SendInput にまとめて送信
    ///
    /// 最も高速。SendInput のアトミック性により他の入力が割り込めない。
    #[allow(clippy::unused_self)]
    fn send_romaji_batched(&self, romaji: &str) {
        let mut inputs = Vec::with_capacity(romaji.len() * 4);
        for ch in romaji.chars() {
            if let Some((vk, needs_shift)) = ascii_to_vk(ch) {
                if needs_shift {
                    inputs.push(make_key_input(VK_LSHIFT, false));
                }
                inputs.push(make_key_input(vk, false));
                inputs.push(make_key_input(vk, true));
                if needs_shift {
                    inputs.push(make_key_input(VK_LSHIFT, true));
                }
            }
        }
        if !inputs.is_empty() {
            unsafe {
                SendInput(
                    &inputs,
                    i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                );
            }
        }
    }

    /// Unicode モード: ローマ字→ひらがなに変換して Unicode 文字として直接送信
    ///
    /// IME を経由せず、ひらがなを直接テキストフィールドに挿入する。
    /// 変換テーブルにないローマ字は PerKey モードでフォールバック送信する。
    fn send_romaji_as_unicode(&self, romaji: &str) {
        if let Some(&kana) = self.romaji_to_kana.as_ref().and_then(|t| t.get(romaji)) {
            self.send_unicode_char(kana);
            return;
        }
        // テーブルにない場合はフォールバック
        self.send_romaji_per_key(romaji);
    }

    /// キーシーケンスを送信する（IME がキーストロークを変換する）
    ///
    /// 各文字について対応するキーストローク（VK コード + Shift）を送信する。
    /// マッピングが見つからない文字は Unicode 直接出力でフォールバックする。
    fn send_key_sequence(&self, s: &str) {
        for ch in s.chars() {
            if let Some((vk, needs_shift)) = char_to_key_sequence(ch) {
                let mut inputs = Vec::with_capacity(4);
                if needs_shift {
                    inputs.push(make_key_input(VK_LSHIFT, false));
                }
                inputs.push(make_key_input(vk, false));
                inputs.push(make_key_input(vk, true));
                if needs_shift {
                    inputs.push(make_key_input(VK_LSHIFT, true));
                }
                unsafe {
                    SendInput(
                        &inputs,
                        i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                    );
                }
            } else {
                // マッピングが見つからない場合は Unicode 直接出力
                self.send_unicode_char(ch);
            }
        }
    }
}

/// INPUT 構造体を作成するヘルパー
const fn make_key_input(vk: u16, is_keyup: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: if is_keyup {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: INJECTED_MARKER,
            },
        },
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
