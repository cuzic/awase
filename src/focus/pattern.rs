//! タイピングパターン検出によるフォーカス推定

use std::time::Instant;

use awase::types::{FocusKind, KeyEventType, RawKeyEvent};
use awase::vk;

use crate::focus::cache::DetectionSource;

/// タイピングパターン検出用トラッカー
///
/// 直近のキー入力パターンからテキスト入力コンテキストを推定する。
/// - 1 秒以内に修飾なし文字キーが 5 回 → TextInput に昇格
/// - 文字キー後 2 秒以内に BS が 2 回 → TextInput に昇格（テキスト編集パターン）
pub struct KeyPatternTracker {
    /// ��近の修飾なし文字キーのタイムスタンプ
    char_timestamps: Vec<Instant>,
    /// 直近の BS キーのタイムスタンプ（文字キー���のみ追跡）
    bs_timestamps: Vec<Instant>,
    /// 最近文字キーが押されたか（BS 追跡用）
    had_recent_chars: bool,
}

impl KeyPatternTracker {
    pub const fn new() -> Self {
        Self {
            char_timestamps: Vec::new(),
            bs_timestamps: Vec::new(),
            had_recent_chars: false,
        }
    }

    #[allow(dead_code)] // 簡略化コールバックからは未使用だが、将来再有効化予定
    /// キーイベントを追跡し、パ���ーンが検出された場合は理由文字列を返す。
    pub fn on_key(&mut self, vk_code: u16, is_modifier_free_char: bool) -> Option<&'static str> {
        let now = Instant::now();

        if is_modifier_free_char {
            self.char_timestamps.push(now);
            self.char_timestamps
                .retain(|t| now.duration_since(*t).as_millis() < 1000);
            self.had_recent_chars = true;

            if self.char_timestamps.len() >= 5 {
                self.char_timestamps.clear();
                self.bs_timestamps.clear();
                return Some("5 char keys in 1s");
            }
        }

        if vk_code == 0x08 && self.had_recent_chars {
            // VK_BACK
            self.bs_timestamps.push(now);
            self.bs_timestamps
                .retain(|t| now.duration_since(*t).as_millis() < 2000);

            if self.bs_timestamps.len() >= 2 {
                self.char_timestamps.clear();
                self.bs_timestamps.clear();
                return Some("2 BS after chars in 2s");
            }
        }

        // 文字キーでも BS でもないキー → 2 秒経過で recent chars リセット
        if !is_modifier_free_char && vk_code != 0x08 {
            if let Some(last) = self.char_timestamps.last() {
                if now.duration_since(*last).as_millis() > 2000 {
                    self.had_recent_chars = false;
                    self.bs_timestamps.clear();
                }
            }
        }

        None
    }

    /// トラッカーをリセットする（昇格後やフォーカス変更時）
    pub fn clear(&mut self) {
        self.char_timestamps.clear();
        self.bs_timestamps.clear();
        self.had_recent_chars = false;
    }
}

/// OS レベルで Ctrl/Alt が押されているかを判定する。
///
/// `GetAsyncKeyState` を使用してリアルタイムの修飾キー状態を取得する。
#[allow(dead_code)] // 簡略化コールバックからは未使用だが、将来再有効化予定
pub fn is_os_modifier_held() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    unsafe {
        let ctrl = GetAsyncKeyState(0x11); // VK_CONTROL
        let alt = GetAsyncKeyState(0x12); // VK_MENU
        (ctrl & (1 << 15) as i16) != 0 || (alt & (1 << 15) as i16) != 0
    }
}

/// FocusKind を TextInput に昇格させる共通ヘルパー。
///
/// キャッシュとログの更新を一元化する。
pub unsafe fn promote_to_text_input(source: DetectionSource, reason: &str) {
    let current = FocusKind::load(&crate::FOCUS_KIND);
    if current == FocusKind::TextInput {
        return;
    }
    FocusKind::TextInput.store(&crate::FOCUS_KIND);
    if let Some(f) = crate::FOCUS.get_mut() {
        if let Some((pid, cls)) = f.last_focus_info.as_ref() {
            f.cache
                .insert(*pid, cls.clone(), FocusKind::TextInput, source);
        }
    }
    log::info!("Promoting to TextInput: {reason} (source={source:?})",);
}

/// キー入力パターンを観察し、テキスト入力コンテキストを推定する。
///
/// すべてのキーイベントに対して、FOCUS_KIND バイパスチェックの **前** に呼び出す。
/// パターンが検出されると `promote_to_text_input` で昇格する。
#[allow(dead_code)] // 簡略化コールバックからは未使用だが、将来再有効化予定
pub unsafe fn observe_key_pattern(event: &RawKeyEvent) {
    let is_key_down = matches!(
        event.event_type,
        KeyEventType::KeyDown | KeyEventType::SysKeyDown
    );
    if !is_key_down {
        return;
    }

    let current = FocusKind::load(&crate::FOCUS_KIND);
    if current == FocusKind::TextInput {
        return; // 既に TextInput なら追跡不要
    }

    let is_char = vk::is_modifier_free_char(event.vk_code, is_os_modifier_held());

    if let Some(f) = crate::FOCUS.get_mut() {
        if let Some(reason) = f.pattern_tracker.on_key(event.vk_code.0, is_char) {
            promote_to_text_input(DetectionSource::TypingPatternInferred, reason);
            f.pattern_tracker.clear();

            // IME OFF + Undetermined で PassThrough 済みキーがある場合、
            // BS で取り消して再処理する
            crate::key_buffer::retract_passthrough_memory();
        }
    }
}
