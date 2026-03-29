//! Platform abstraction traits for future cross-platform support.
//!
//! This module defines platform-independent traits and types that
//! abstract over OS-specific keyboard hook, key injection, and
//! IME detection mechanisms.

// ─── IME Types (platform-independent) ────────────────────────

/// IME の変換モード（プラットフォーム非依存）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeMode {
    /// IME OFF（直接入力）
    Off,
    /// ひらがなモード
    Hiragana,
    /// カタカナモード
    Katakana,
    /// 半角カタカナモード
    HalfKatakana,
    /// 英数モード
    Alphanumeric,
}

impl ImeMode {
    /// かな入力モード（ひらがな / カタカナ / 半角カタカナ）かどうかを返す
    #[must_use]
    pub const fn is_kana_input(self) -> bool {
        matches!(self, Self::Hiragana | Self::Katakana | Self::HalfKatakana)
    }
}

// ─── Platform Abstraction Traits ─────────────────────────────

/// Abstraction for keyboard hook installation.
///
/// Platform implementations:
/// - Windows: `WH_KEYBOARD_LL` via `SetWindowsHookExW`
/// - macOS: `CGEventTap` (future)
/// - Linux: libinput / evdev (future)
pub trait KeyboardHook {
    /// Raw key event from the platform hook
    type RawEvent;

    /// Install the hook. The callback returns `true` if the event was consumed.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform hook installation fails.
    fn install(&mut self, callback: Box<dyn FnMut(Self::RawEvent) -> bool>) -> anyhow::Result<()>;

    /// Uninstall the hook.
    fn uninstall(&mut self);
}

/// Abstraction for key injection.
///
/// Platform implementations:
/// - Windows: `SendInput`
/// - macOS: `CGEventPost` (future)
/// - Linux: uinput (future)
pub trait KeySender {
    /// Send a sequence of VK code key events (for IME romaji input)
    fn send_romaji(&self, romaji: &str);

    /// Send a Unicode character directly (for kana input mode)
    fn send_unicode(&self, ch: char);

    /// Send a literal string (each char as Unicode)
    fn send_literal(&self, s: &str);

    /// Send a virtual key code (`KeyDown` + `KeyUp`)
    fn send_vk(&self, vk: u16);

    /// Send a virtual key event (`KeyDown` or `KeyUp`)
    fn send_vk_event(&self, vk: u16, is_keyup: bool);
}

/// Abstraction for IME state detection.
///
/// Platform implementations:
/// - Windows: TSF + IMM32 hybrid
/// - macOS: `TISGetInputSourceProperty` (future)
/// - Linux: IBus / Fcitx D-Bus (future)
pub trait ImeDetector {
    /// Get the current IME mode
    fn get_mode(&self) -> ImeMode;

    /// Check if IME is active (Japanese input mode)
    fn is_active(&self) -> bool {
        let mode = self.get_mode();
        !matches!(mode, ImeMode::Off | ImeMode::Alphanumeric)
    }
}
