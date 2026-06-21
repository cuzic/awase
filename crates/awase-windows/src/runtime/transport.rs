use awase::types::{KeyEventType, RawKeyEvent};

use crate::focus::class_names::AppImeProfile;
use crate::hook::CallbackResult;

/// 元の物理キーイベントを OS に届けるかどうかの配送判断。
/// `Decision`（意味論）とは独立した配送機構上の判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PhysicalKeyDisposition {
    /// 元の物理キーイベントをそのまま OS に通す
    Allow,
    /// 元の物理キーイベントを消費（OS に届けない）
    Suppress,
}

impl PhysicalKeyDisposition {
    /// Decision と PhysicalKeyDisposition から CallbackResult を導出する。
    pub(crate) const fn to_callback(self, engine_consumed: bool) -> CallbackResult {
        match self {
            Self::Suppress => CallbackResult::Consumed,
            Self::Allow => {
                if engine_consumed {
                    CallbackResult::Consumed
                } else {
                    CallbackResult::PassThrough
                }
            }
        }
    }

    /// KANJI 関連キーの物理配送判断を計算する純粋関数。
    ///
    /// - ImmCross プロファイル: Down/Up 共に Suppress（spurious 連鎖を構造的に遮断）
    /// - Imm32Unavailable: shadow_toggle 発火時 KeyDown と全 KeyUp を Suppress
    /// - TsfNative: 物理キーを通す（従来通り）
    pub(crate) fn plan(
        event: &RawKeyEvent,
        profile: AppImeProfile,
        shadow_toggled: bool,
    ) -> Self {
        let is_kanji_event = event.ime_relevance.shadow_action.is_some();
        if !is_kanji_event {
            return Self::Allow;
        }
        let suppress = if profile.can_use_imm32_cross_process() {
            // ImmCross: KANJI 関連 VK は Down/Up 共に Suppress
            true
        } else {
            // Imm32Unavailable: shadow_toggle 発火時 KeyDown + 全 KeyUp を Suppress
            !profile.should_pass_physical_key()
                && (shadow_toggled || matches!(event.event_type, KeyEventType::KeyUp))
        };
        if suppress {
            Self::Suppress
        } else {
            Self::Allow
        }
    }
}
