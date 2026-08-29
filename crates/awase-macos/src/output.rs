//! macOS キー出力 (CGEventPost)

use awase::types::SpecialKey;

/// SpecialKey を macOS keycode に変換する
#[must_use]
pub const fn special_key_to_keycode(sk: SpecialKey) -> u16 {
    match sk {
        SpecialKey::Backspace => 0x33,
        SpecialKey::Escape => 0x35,
        SpecialKey::Enter => 0x24,
        SpecialKey::Space => 0x31,
        SpecialKey::Delete => 0x75, // Forward Delete
        SpecialKey::Insert => 0x72, // Help/Insert
        SpecialKey::Up => 0x7E,
        SpecialKey::Down => 0x7D,
        SpecialKey::Left => 0x7B,
        SpecialKey::Right => 0x7C,
        SpecialKey::Home => 0x73,
        SpecialKey::End => 0x77,
        SpecialKey::PageUp => 0x74,
        SpecialKey::PageDown => 0x79,
    }
}

/// ASCII 文字を macOS keycode に変換する
#[must_use]
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

/// awase 自身が注入したイベントを tap 側で識別するためのマーカー。
/// `EVENT_SOURCE_USER_DATA` (kCGEventSourceUserData) に載せ、フック側は
/// この値を見て自分の注入イベントを Engine に通さず素通しする。
pub const INJECT_MARKER: i64 = 0x0A0A_5E00;

#[cfg(target_os = "macos")]
mod imp {
    use awase::kana_table::KanaTable;
    use awase::types::{KeyAction, KeyEventType, VkCode};
    use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, EventField};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    use log::warn;

    use super::{ascii_to_keycode, special_key_to_keycode, INJECT_MARKER};

    /// kVK_Shift（Romaji/KeySequence の大文字送出用）
    const KEYCODE_SHIFT: u16 = 0x38;

    /// CGEventPost によるキー出力。
    ///
    /// 注入イベントはすべて `INJECT_MARKER` 付きで `HID` タップ位置に post する。
    /// HID 位置に注入することで IME を含む通常の入力パイプラインを通る
    /// （ローマ字キーストロークを IME に変換させるために必要）。
    /// かな記号 → IME ローマ字入力で同じ文字に変換されるキーストローク。
    /// `KanaTable` はかな文字のみを収録するため、.yab の Literal 由来で
    /// `Char` に載ってくる記号類はここで補う。
    const fn kana_symbol_to_ascii(ch: char) -> Option<&'static str> {
        match ch {
            'ー' => Some("-"),
            '、' => Some(","),
            '。' => Some("."),
            '・' => Some("/"),
            '「' => Some("["),
            '」' => Some("]"),
            _ => None,
        }
    }

    pub struct Output {
        source: CGEventSource,
        /// `Char(かな)` をローマ字キーストロークへ逆引きするためのテーブル
        /// （Windows 版 VK モードの `send_char_as_vk` と同じ方針。macOS では
        /// Unicode 直接注入だと IME が未確定文字列を持たず漢字変換不能になる）。
        kana: KanaTable,
    }

    impl std::fmt::Debug for Output {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Output").finish_non_exhaustive()
        }
    }

    impl Output {
        /// イベントソースを作成する。
        ///
        /// # Errors
        ///
        /// `CGEventSource` の作成に失敗した場合。
        pub fn new() -> anyhow::Result<Self> {
            let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                .map_err(|()| anyhow::anyhow!("Failed to create CGEventSource"))?;
            Ok(Self {
                source,
                kana: KanaTable::build(),
            })
        }

        /// 単一のキーイベントを post する。
        fn post_key(&self, keycode: u16, down: bool, shift: bool) {
            let Ok(event) = CGEvent::new_keyboard_event(self.source.clone(), keycode, down)
            else {
                warn!("Failed to create keyboard event (keycode=0x{keycode:02X})");
                return;
            };
            if shift {
                event.set_flags(CGEventFlags::CGEventFlagShift);
            }
            event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, INJECT_MARKER);
            event.post(CGEventTapLocation::HID);
        }

        /// キーを押して離す。
        fn post_press_release(&self, keycode: u16, shift: bool) {
            self.post_key(keycode, true, shift);
            self.post_key(keycode, false, shift);
        }

        /// Unicode 文字を CGEvent の文字列ペイロードで直接注入する。
        ///
        /// キーコードに依存しないため任意のかな文字を出力できるが、
        /// IME のかな漢字変換は経由しない（Unicode 直接注入モード用）。
        fn post_char(&self, ch: char) {
            let Ok(event) = CGEvent::new_keyboard_event(self.source.clone(), 0, true) else {
                warn!("Failed to create keyboard event for Char('{ch}')");
                return;
            };
            event.set_string(&ch.to_string());
            event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, INJECT_MARKER);
            event.post(CGEventTapLocation::HID);
            // 対応する KeyUp（文字列ペイロードなし）
            if let Ok(up) = CGEvent::new_keyboard_event(self.source.clone(), 0, false) {
                up.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, INJECT_MARKER);
                up.post(CGEventTapLocation::HID);
            }
        }

        /// ASCII 文字列をキーストロークとして送出する（IME に変換させる用途）。
        fn send_ascii_sequence(&self, s: &str, kind: &str) {
            for ch in s.chars() {
                if let Some((keycode, needs_shift)) = ascii_to_keycode(ch) {
                    if needs_shift {
                        self.post_key(KEYCODE_SHIFT, true, false);
                    }
                    self.post_press_release(keycode, needs_shift);
                    if needs_shift {
                        self.post_key(KEYCODE_SHIFT, false, false);
                    }
                } else {
                    warn!("{kind} char '{ch}' has no macOS keycode mapping, skipping");
                }
            }
        }

        /// `KeyAction` のリストを順に post する。
        pub fn send_keys(&mut self, actions: &[KeyAction]) {
            for action in actions {
                match action {
                    KeyAction::SpecialKey(sk) => {
                        self.post_press_release(special_key_to_keycode(*sk), false);
                    }
                    KeyAction::Key(vk) => self.post_key(vk.0, true, false),
                    KeyAction::KeyUp(vk) => self.post_key(vk.0, false, false),
                    KeyAction::Char(ch) => {
                        // IME に変換させるため、かなはローマ字キーストロークへ
                        // 逆引きして送出する。逆引き不能な文字のみ Unicode 直接注入
                        // へフォールバック（IME 変換対象にはならない）。
                        if let Some(romaji) = self.kana.romaji_for_kana(*ch) {
                            let romaji = romaji.to_owned();
                            self.send_ascii_sequence(&romaji, "Char");
                        } else if let Some(ascii) = kana_symbol_to_ascii(*ch) {
                            self.send_ascii_sequence(ascii, "Char");
                        } else {
                            self.post_char(*ch);
                        }
                    }
                    KeyAction::Romaji(s) => self.send_ascii_sequence(s, "Romaji"),
                    KeyAction::KeySequence(s) => self.send_ascii_sequence(s, "KeySequence"),
                    KeyAction::Suppress => {}
                }
            }
        }

        /// 握りつぶしたキーを合成イベントとして再注入する
        /// （親指キー単独打鍵の英数/かな送出等）。
        pub fn reinject(&mut self, vk: VkCode, event_type: KeyEventType) {
            let down = matches!(event_type, KeyEventType::KeyDown);
            self.post_key(vk.0, down, false);
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::Output;

/// 非 macOS ビルド用スタブ（ワークスペース全体のクロスチェック用）。
#[cfg(not(target_os = "macos"))]
#[derive(Debug)]
pub struct Output;

#[cfg(not(target_os = "macos"))]
impl Output {
    /// スタブ生成。
    ///
    /// # Errors
    ///
    /// スタブのため常に成功する。
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self)
    }

    pub fn send_keys(&mut self, actions: &[awase::types::KeyAction]) {
        for action in actions {
            log::trace!("macOS output stub: {action:?}");
        }
    }

    pub fn reinject(&mut self, vk: awase::types::VkCode, event_type: awase::types::KeyEventType) {
        log::trace!("macOS output stub: reinject 0x{:02X} {event_type:?}", vk.0);
    }
}
