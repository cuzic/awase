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

use awase::engine::InputModeState;

/// IME 補助状態 (input_mode / is_japanese_ime / prev_conversion_mode)。
///
/// IME ON/OFF 自体は [`crate::state::ime_model::ImeModel`] の `desired_open` が SSOT。
#[derive(Debug)]
pub struct ImeBelief {
    /// 入力モード（ローマ字 / かな / 不明）
    ///
    /// `hook.rs` がフックコールバック内で直接読み取るため `pub(crate)` とする。
    /// 書き込みは `PlatformState::set_input_mode()` 経由で行うこと。
    pub(crate) input_mode: InputModeState,
    /// 日本語 IME がアクティブか
    pub(in crate::state) is_japanese_ime: bool,
    /// 直前の conversion_mode（ROMAN ビット消失によるかな切替検出用）
    /// None = まだ一度も取得できていない
    pub(in crate::state) prev_conversion_mode: Option<u32>,
}

impl ImeBelief {
    /// 入力モードを返す。
    #[inline]
    pub(crate) const fn input_mode(&self) -> InputModeState {
        self.input_mode
    }

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
