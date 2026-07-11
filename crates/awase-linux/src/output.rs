//! uinput を使った仮想キーボード出力バックエンド。
//!
//! evdev の `VirtualDevice` を使って、キーイベントを `/dev/uinput` 経由で注入する。

use awase::engine::{Decision, Effect, InputEffect, TimerEffect};
use awase::types::{KeyAction, KeyEventType, SpecialKey, VkCode};
use evdev::{uinput::VirtualDeviceBuilder, AttributeSet, EventType, InputEvent, Key};
use log::warn;

/// KEY_LEFTSHIFT の evdev キーコード
const KEY_LEFTSHIFT: u16 = 42;

/// `SpecialKey` を evdev キーコードに変換する。
const fn special_key_to_evdev(sk: SpecialKey) -> u16 {
    match sk {
        SpecialKey::Backspace => 14, // KEY_BACKSPACE
        SpecialKey::Enter => 28,     // KEY_ENTER
        SpecialKey::Space => 57,     // KEY_SPACE
        SpecialKey::Escape => 1,     // KEY_ESC
        SpecialKey::Delete => 111,   // KEY_DELETE
    }
}

/// ASCII 文字を evdev キーコードに変換する。
///
/// 戻り値は `(keycode, needs_shift)` のタプル。
fn ascii_to_evdev(ch: char) -> Option<(u16, bool)> {
    match ch {
        'a'..='z' => Some((30 + (ch as u16 - b'a' as u16), false)),
        'A'..='Z' => Some((30 + (ch as u16 - b'A' as u16), true)),
        '1'..='9' => Some((2 + (ch as u16 - b'1' as u16), false)),
        '0' => Some((11, false)),
        '-' => Some((12, false)),  // KEY_MINUS
        '=' => Some((13, false)),  // KEY_EQUAL
        '[' => Some((26, false)),  // KEY_LEFTBRACE
        ']' => Some((27, false)),  // KEY_RIGHTBRACE
        ';' => Some((39, false)),  // KEY_SEMICOLON
        '\'' => Some((40, false)), // KEY_APOSTROPHE
        '`' => Some((41, false)),  // KEY_GRAVE
        '\\' => Some((43, false)), // KEY_BACKSLASH
        ',' => Some((51, false)),  // KEY_COMMA
        '.' => Some((52, false)),  // KEY_DOT
        '/' => Some((53, false)),  // KEY_SLASH
        ' ' => Some((57, false)),  // KEY_SPACE
        // Shifted variants
        '!' => Some((2, true)),  // Shift+1
        '@' => Some((3, true)),  // Shift+2
        '#' => Some((4, true)),  // Shift+3
        '$' => Some((5, true)),  // Shift+4
        '%' => Some((6, true)),  // Shift+5
        '^' => Some((7, true)),  // Shift+6
        '&' => Some((8, true)),  // Shift+7
        '*' => Some((9, true)),  // Shift+8
        '(' => Some((10, true)), // Shift+9
        ')' => Some((11, true)), // Shift+0
        '_' => Some((12, true)), // Shift+-
        '+' => Some((13, true)), // Shift+=
        '{' => Some((26, true)), // Shift+[
        '}' => Some((27, true)), // Shift+]
        ':' => Some((39, true)), // Shift+;
        '"' => Some((40, true)), // Shift+'
        '~' => Some((41, true)), // Shift+`
        '|' => Some((43, true)), // Shift+\
        '<' => Some((51, true)), // Shift+,
        '>' => Some((52, true)), // Shift+.
        '?' => Some((53, true)), // Shift+/
        _ => None,
    }
}

/// uinput 仮想デバイスを使ったキー出力。
pub struct UinputOutput {
    device: evdev::uinput::VirtualDevice,
}

impl std::fmt::Debug for UinputOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UinputOutput")
            .field("device", &"VirtualDevice")
            .finish()
    }
}

impl UinputOutput {
    /// uinput 仮想キーボードデバイスを作成する。
    ///
    /// `/dev/uinput` への書き込み権限が必要（通常は root または input グループ）。
    ///
    /// # Errors
    ///
    /// デバイスの作成に失敗した場合にエラーを返す。
    pub fn new() -> anyhow::Result<Self> {
        let mut keys = AttributeSet::<Key>::new();
        // 標準的なキーコードをすべて登録（0..768）
        for code in 0..768_u16 {
            keys.insert(Key::new(code));
        }

        let device = VirtualDeviceBuilder::new()?
            .name("awase-virtual-kbd")
            .with_keys(&keys)?
            .build()?;

        Ok(Self { device })
    }

    /// `KeyAction` のスライスを順に実行し、対応するキーイベントを送信する。
    pub fn send_keys(&mut self, actions: &[KeyAction]) {
        for action in actions {
            match action {
                KeyAction::SpecialKey(sk) => {
                    let code = special_key_to_evdev(*sk);
                    self.send_key_press_release(code);
                }
                KeyAction::Key(vk) => {
                    self.send_key_press_release(vk.0);
                }
                KeyAction::KeyUp(vk) => {
                    self.emit_key(vk.0, 0);
                }
                KeyAction::Char(ch) => {
                    warn!(
                        "Char('{}') output is not yet supported on Linux (Unicode direct input requires xdotool or IM protocol)",
                        ch
                    );
                }
                KeyAction::Romaji(s) => {
                    self.send_romaji(s);
                }
                KeyAction::KeySequence(s) => {
                    for ch in s.chars() {
                        if let Some((code, needs_shift)) = ascii_to_evdev(ch) {
                            if needs_shift {
                                self.emit_key(KEY_LEFTSHIFT, 1);
                            }
                            self.send_key_press_release(code);
                            if needs_shift {
                                self.emit_key(KEY_LEFTSHIFT, 0);
                            }
                        } else {
                            warn!("KeySequence char '{ch}' has no evdev keycode mapping, skipping");
                        }
                    }
                }
                KeyAction::Suppress => {}
            }
        }
    }

    /// キーを押して離す（press + release）。
    fn send_key_press_release(&mut self, code: u16) {
        self.emit_key(code, 1); // key down
        self.emit_key(code, 0); // key up
    }

    /// 単一のキーイベント + SYN_REPORT を送信する。
    fn emit_key(&mut self, code: u16, value: i32) {
        let ev = InputEvent::new(EventType::KEY, code, value);
        if let Err(e) = self.device.emit(&[ev]) {
            warn!("uinput emit failed (code={code}, value={value}): {e}");
        }
    }

    /// ローマ字文字列の各文字を evdev キーイベントとして送信する。
    fn send_romaji(&mut self, romaji: &str) {
        for ch in romaji.chars() {
            if let Some((code, needs_shift)) = ascii_to_evdev(ch) {
                if needs_shift {
                    self.emit_key(KEY_LEFTSHIFT, 1);
                }
                self.send_key_press_release(code);
                if needs_shift {
                    self.emit_key(KEY_LEFTSHIFT, 0);
                }
            } else {
                warn!("Romaji char '{ch}' has no evdev keycode mapping, skipping");
            }
        }
    }

    /// `Decision` を実行し、副作用を uinput デバイスに反映する。
    pub fn execute_decision(&mut self, decision: &Decision, vk: VkCode, event_type: KeyEventType) {
        match decision {
            Decision::PassThrough => {
                // Device is grabbed, so we must re-inject the key via uinput
                self.reinject(vk, event_type);
            }
            Decision::PassThroughWith { effects } => {
                self.reinject(vk, event_type);
                self.execute_effects(effects);
            }
            Decision::Consume { effects } => {
                self.execute_effects(effects);
            }
        }
    }

    /// パススルーキーを uinput 経由で再注入する。
    fn reinject(&mut self, vk: VkCode, event_type: KeyEventType) {
        match event_type {
            KeyEventType::KeyDown => {
                self.send_keys(&[KeyAction::Key(vk)]);
            }
            KeyEventType::KeyUp => {
                self.send_keys(&[KeyAction::KeyUp(vk)]);
            }
        }
    }

    /// `Effect` リストを順に実行する。
    pub fn execute_effects(&mut self, effects: &[Effect]) {
        for effect in effects {
            match effect {
                Effect::Input(InputEffect::SendKeys(actions)) => {
                    self.send_keys(actions);
                }
                Effect::Input(InputEffect::ReinjectKey(raw_event)) => {
                    self.reinject(raw_event.vk_code, raw_event.event_type);
                }
                Effect::Timer(TimerEffect::Set { id, duration }) => {
                    log::debug!(
                        "Timer set request: id={id}, duration={duration:?} (not yet implemented)"
                    );
                }
                Effect::Timer(TimerEffect::Kill(id)) => {
                    log::debug!("Timer kill request: id={id} (not yet implemented)");
                }
                Effect::Ime(ime_effect) => {
                    log::debug!("IME effect: {ime_effect:?} (not yet implemented)");
                }
                Effect::Ui(ui_effect) => {
                    log::debug!("UI effect: {ui_effect:?}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_special_key_to_evdev() {
        assert_eq!(special_key_to_evdev(SpecialKey::Backspace), 14);
        assert_eq!(special_key_to_evdev(SpecialKey::Enter), 28);
        assert_eq!(special_key_to_evdev(SpecialKey::Space), 57);
        assert_eq!(special_key_to_evdev(SpecialKey::Escape), 1);
        assert_eq!(special_key_to_evdev(SpecialKey::Delete), 111);
    }

    #[test]
    fn test_ascii_to_evdev_lowercase() {
        // a=30, b=31, ..., z=55
        assert_eq!(ascii_to_evdev('a'), Some((30, false)));
        assert_eq!(ascii_to_evdev('z'), Some((55, false)));
    }

    #[test]
    fn test_ascii_to_evdev_uppercase() {
        assert_eq!(ascii_to_evdev('A'), Some((30, true)));
        assert_eq!(ascii_to_evdev('Z'), Some((55, true)));
    }

    #[test]
    fn test_ascii_to_evdev_digits() {
        assert_eq!(ascii_to_evdev('1'), Some((2, false)));
        assert_eq!(ascii_to_evdev('9'), Some((10, false)));
        assert_eq!(ascii_to_evdev('0'), Some((11, false)));
    }

    #[test]
    fn test_ascii_to_evdev_punctuation() {
        assert_eq!(ascii_to_evdev('-'), Some((12, false)));
        assert_eq!(ascii_to_evdev('.'), Some((52, false)));
        assert_eq!(ascii_to_evdev(','), Some((51, false)));
        assert_eq!(ascii_to_evdev('/'), Some((53, false)));
    }

    #[test]
    fn test_ascii_to_evdev_shifted() {
        assert_eq!(ascii_to_evdev('!'), Some((2, true)));
        assert_eq!(ascii_to_evdev('?'), Some((53, true)));
    }

    #[test]
    fn test_ascii_to_evdev_unknown() {
        assert_eq!(ascii_to_evdev('\u{3042}'), None); // 'あ'
    }
}
