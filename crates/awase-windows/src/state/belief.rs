//! IME 補助状態（input_mode / is_japanese_ime / prev_conversion_mode）。
//!
//! # IME 状態の 3 層モデル（Phase 3e 以降）
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │ Layer 1: 生観測 event (ImeEvent::ObserverReported)           │
//! │  各ソース (ObserverPoll / FocusProbe / Gji / Tsf / HwndCache) │
//! │  は ImeEvent を dispatch する。shadow_model.reduce() が記録。  │
//! └────────────────────┬────────────────────────────────────────┘
//!                      │ reduce() → observations.record()
//! ┌────────────────────▼────────────────────────────────────────┐
//! │ Layer 2: shadow_model.desired_open / effective_open()       │
//! │  Engine が前提とすべき IME 状態の SSOT。                     │
//! │  UserImeSetIntent / UserImeToggleIntent のみが書き換え可能。 │
//! └────────────────────┬────────────────────────────────────────┘
//!                      │ apply_ime_open() → OS に送信
//! ┌────────────────────▼────────────────────────────────────────┐
//! │ Layer 3: 制御ログ (ImeModel.applied_open / applied_at_ms)   │
//! │  最後に OS に送ったコマンド値。VK_KANJI 重複送信防止専用。   │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! `ImeBelief` は IME ON/OFF 自体は持たず、補助的な属性（input_mode 等）のみを保持する。

/// IME 補助状態 (is_japanese_ime / prev_conversion_mode)。
///
/// IME ON/OFF 自体は [`crate::state::ime_model::ImeModel`] の `desired_open` が SSOT。
#[derive(Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct ImeBelief {
    /// 日本語 IME がアクティブか
    pub(in crate::state) is_japanese_ime: bool,
    /// 直前の conversion_mode（ROMAN ビット消失によるかな切替検出用）
    /// None = まだ一度も取得できていない
    pub(in crate::state) prev_conversion_mode: Option<u32>,
}

impl Default for ImeBelief {
    fn default() -> Self {
        Self {
            is_japanese_ime: true,
            prev_conversion_mode: None,
        }
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
impl ImeBelief {
    /// 日本語 IME がアクティブかを返す。
    #[inline]
    pub(crate) const fn is_japanese_ime(&self) -> bool {
        self.is_japanese_ime
    }

    /// 直前の conversion_mode を返す。
    #[inline]
    pub(crate) const fn prev_conversion_mode(&self) -> Option<u32> {
        self.prev_conversion_mode
    }
}
