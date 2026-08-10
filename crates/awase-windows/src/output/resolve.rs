pub(crate) use crate::vk::ascii_to_vk;
use awase::types::{SpecialKey, VkCode};

/// SpecialKey を Windows VK コードに変換する
#[must_use]
pub(super) const fn special_key_to_vk(sk: SpecialKey) -> VkCode {
    match sk {
        SpecialKey::Backspace => crate::vk::VK_BACK,
        SpecialKey::Escape => crate::vk::VK_ESCAPE,
        SpecialKey::Enter => crate::vk::VK_RETURN,
        SpecialKey::Space => crate::vk::VK_SPACE,
        SpecialKey::Delete => crate::vk::VK_DELETE,
        SpecialKey::Insert => crate::vk::VK_INSERT,
        SpecialKey::Up => crate::vk::VK_UP,
        SpecialKey::Down => crate::vk::VK_DOWN,
        SpecialKey::Left => crate::vk::VK_LEFT,
        SpecialKey::Right => crate::vk::VK_RIGHT,
        SpecialKey::Home => crate::vk::VK_HOME,
        SpecialKey::End => crate::vk::VK_END,
        SpecialKey::PageUp => crate::vk::VK_PRIOR,
        SpecialKey::PageDown => crate::vk::VK_NEXT,
    }
}

/// `send_char_as_tsf` / `send_char_as_vk` 共通の文字解決結果。
pub(super) enum CharResolution<'a> {
    /// かな → ローマ字（VK / TSF 経由で IME に渡す）
    Romaji(&'a str),
    /// 記号 → (VK コード, Shift 要否)
    Vk(VkCode, bool),
    /// フォールバック（Unicode 直接出力）
    Unicode(char),
}
