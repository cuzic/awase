use std::collections::VecDeque;

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

/// 非同期出力キューつき SendInput ラッパー
///
/// ローマ字の送信は即座に行わず、内部キューに積む。
/// メッセージループの `WM_TIMER`（`TIMER_OUTPUT_DRAIN`）で
/// キューから 1 イベントずつ取り出して `SendInput` する。
/// これにより他のキーボードフック（PowerToys 等）に処理時間を与え、
/// 文字の取りこぼしを防ぐ。
///
/// `Key`, `KeyUp`, `Char` は即座に送信する（遅延不要）。
pub struct Output {
    /// 遅延送信用キュー（ローマ字 INPUT イベント）
    queue: VecDeque<INPUT>,
}

impl Output {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
        }
    }

    /// アクション列を処理する。
    ///
    /// `Key`, `KeyUp`, `Char` は即座に送信。
    /// `Romaji` はキューに積み、タイマーでドリップフィードする。
    pub fn send_keys(&mut self, actions: &[KeyAction]) {
        for action in actions {
            match action {
                KeyAction::Key(vk) => {
                    // キューに溜まっている分を先にフラッシュ
                    self.flush_queue();
                    self.send_key_immediate(*vk, false);
                }
                KeyAction::KeyUp(vk) => {
                    self.flush_queue();
                    self.send_key_immediate(*vk, true);
                }
                KeyAction::Char(ch) => {
                    self.flush_queue();
                    self.send_unicode_char(*ch);
                }
                KeyAction::Suppress => {
                    // 何もしない
                }
                KeyAction::Romaji(s) => {
                    self.enqueue_romaji(s);
                }
            }
        }
        // キューにイベントがあればドレインタイマーを開始
        if !self.queue.is_empty() {
            self.start_drain_timer();
        }
    }

    /// キューから 1 イベントを取り出して SendInput する。
    ///
    /// メッセージループの `WM_TIMER(TIMER_OUTPUT_DRAIN)` から呼ばれる。
    /// キューが空になったらタイマーを停止する。
    pub fn drain_one(&mut self) {
        if let Some(input) = self.queue.pop_front() {
            unsafe {
                SendInput(
                    &[input],
                    i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                );
            }
        }
        if self.queue.is_empty() {
            self.stop_drain_timer();
        }
    }

    /// キューに残っているイベントを全て即座に送信する。
    ///
    /// `Key`/`KeyUp`/`Char`（即時送信が必要なアクション）の前に呼ぶ。
    /// BS（投機出力の取り消し）等が正しい順序で送信されることを保証する。
    pub fn flush_queue(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        let inputs: Vec<INPUT> = self.queue.drain(..).collect();
        unsafe {
            SendInput(
                &inputs,
                i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
            );
        }
        self.stop_drain_timer();
    }

    /// キューが空かどうか
    pub fn is_queue_empty(&self) -> bool {
        self.queue.is_empty()
    }

    // ── 内部メソッド ──

    /// ローマ字文字列をキューに積む
    fn enqueue_romaji(&mut self, romaji: &str) {
        for ch in romaji.chars() {
            if let Some((vk, needs_shift)) = ascii_to_vk(ch) {
                if needs_shift {
                    self.queue.push_back(make_key_input(0xA0, false)); // LShift down
                }
                self.queue.push_back(make_key_input(vk, false)); // key down
                self.queue.push_back(make_key_input(vk, true)); // key up
                if needs_shift {
                    self.queue.push_back(make_key_input(0xA0, true)); // LShift up
                }
            }
        }
    }

    /// 仮想キーコードを使って即座に KeyDown/KeyUp を送信する
    fn send_key_immediate(&self, vk: u16, is_keyup: bool) {
        let input = make_key_input(vk, is_keyup);
        unsafe {
            SendInput(
                &[input],
                i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
            );
        }
    }

    /// Unicode 文字を直接送信する（`KEYEVENTF_UNICODE`）
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

    /// ドレインタイマーを開始する
    fn start_drain_timer(&self) {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::SetTimer;
        unsafe {
            let _ = SetTimer(HWND::default(), TIMER_OUTPUT_DRAIN, DRAIN_INTERVAL_MS, None);
        }
    }

    /// ドレインタイマーを停止する
    fn stop_drain_timer(&self) {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::KillTimer;
        unsafe {
            let _ = KillTimer(HWND::default(), TIMER_OUTPUT_DRAIN);
        }
    }
}

/// ドレインタイマー ID
pub const TIMER_OUTPUT_DRAIN: usize = 102;

/// ドレイン間隔（ミリ秒）
const DRAIN_INTERVAL_MS: u32 = 10;

/// INPUT 構造体を作成するヘルパー
fn make_key_input(vk: u16, is_keyup: bool) -> INPUT {
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
