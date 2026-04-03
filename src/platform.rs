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
    fn send_vk(&self, vk: crate::types::VkCode);

    /// Send a virtual key event (`KeyDown` or `KeyUp`)
    fn send_vk_event(&self, vk: crate::types::VkCode, is_keyup: bool);
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

    /// IME が未確定文字列を持っているか（変換中か）
    fn is_composing(&self) -> bool;
}

// ─── PlatformRuntime Trait ──────────────────────────────────

use std::time::Duration;

use crate::types::{FocusKind, KeyAction, RawKeyEvent};

/// フォアグラウンドウィンドウ情報（プラットフォーム非依存）
#[derive(Debug, Clone)]
pub struct ForegroundInfo {
    pub process_id: u32,
    pub class_name: String,
}

/// プラットフォーム固有の副作用実行インターフェース。
///
/// `DecisionExecutor` がこのトレイトを通じて OS 操作を行う。
/// Windows/macOS/Linux でそれぞれ実装を提供する。
pub trait PlatformRuntime {
    // ── キー出力 ──

    /// `KeyAction` のスライスを順に実行する
    fn send_keys(&mut self, actions: &[KeyAction]);

    /// 元のキーイベントを再注入する（IME OFF 時の遅延キー再生用）
    fn reinject_key(&mut self, event: &RawKeyEvent);

    // ── タイマー ──

    /// 指定 ID のタイマーを開始する
    fn set_timer(&mut self, id: usize, duration: Duration);

    /// 指定 ID のタイマーを停止する
    fn kill_timer(&mut self, id: usize);

    // ── IME 制御 ──

    /// IME の ON/OFF を設定する。成功時 true を返す。
    fn set_ime_open(&mut self, open: bool) -> bool;

    /// IME 状態キャッシュの非同期リフレッシュを要求する
    fn post_ime_refresh(&mut self);

    // ── トレイ ──

    /// エンジン有効/無効に応じてトレイアイコンを更新する
    fn update_tray(&mut self, enabled: bool);

    /// バルーン通知を表示する
    fn show_balloon(&mut self, title: &str, message: &str);

    /// 配列名をトレイに表示する
    fn set_tray_layout_name(&mut self, name: &str);

    // ── フォーカス ──

    /// 現在のフォーカス種別を設定する
    fn update_focus_kind(&mut self, kind: FocusKind);

    /// IME 信頼度をリセットする
    fn reset_ime_reliability(&mut self);

    /// フォーカスキャッシュにエントリを挿入する
    fn insert_focus_cache(&mut self, process_id: u32, class_name: String, kind: FocusKind);

    /// UIA 非同期判定をリクエストする
    fn request_uia_classification(&mut self);

    /// 最終フォーカス情報を更新する
    fn update_last_focus_info(&mut self, process_id: u32, class_name: String);

    // ── IME キャッシュ ──

    /// IME 状態キャッシュを更新する
    fn update_ime_cache(&mut self, ime_on: bool);

    /// IME 状態キャッシュを無効化する（Unknown にする）
    fn invalidate_ime_cache(&mut self);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ime_mode_is_kana_input() {
        assert!(!ImeMode::Off.is_kana_input());
        assert!(ImeMode::Hiragana.is_kana_input());
        assert!(ImeMode::Katakana.is_kana_input());
        assert!(ImeMode::HalfKatakana.is_kana_input());
        assert!(!ImeMode::Alphanumeric.is_kana_input());
    }

    #[test]
    fn ime_mode_debug_and_clone() {
        let mode = ImeMode::Hiragana;
        let cloned = mode;
        assert_eq!(mode, cloned);
        // Verify Debug is implemented
        let _debug = format!("{:?}", mode);
    }

    #[test]
    fn foreground_info_fields() {
        let info = ForegroundInfo {
            process_id: 42,
            class_name: "Notepad".to_string(),
        };
        assert_eq!(info.process_id, 42);
        assert_eq!(info.class_name, "Notepad");
    }
}
