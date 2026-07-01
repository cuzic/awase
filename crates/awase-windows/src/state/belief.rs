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

use awase::engine::{AssumedReason, InputModeState};

/// IME 補助状態 (input_mode / is_japanese_ime / prev_conversion_mode)。
///
/// IME ON/OFF 自体は [`crate::state::ime_model::ImeModel`] の `desired_open` が SSOT。
#[derive(Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
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

#[cfg_attr(not(windows), allow(dead_code))]
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

    /// IMM-broken アプリ（Chrome 等）で IME-ON が確認されたとき、`input_mode` を補正すべき
    /// 新しい値を返す純粋関数。Win32 呼び出しを一切含まないため実機なしでテスト可能。
    ///
    /// | 現在の `input_mode`   | 戻り値                                    | 理由                            |
    /// |-----------------------|-------------------------------------------|---------------------------------|
    /// | `ObservedRomaji`      | `None`                                    | すでに romaji-capable            |
    /// | `AssumedRomaji { .. }`| `None`                                    | すでに romaji-capable            |
    /// | `ObservedEisu`        | `None`                                    | 英数モード確定済み（補正不要）   |
    /// | `ObservedKana`        | `Some(AssumedRomaji { ImmBridgeBroken })` | stale なかな → AssumedRomaji     |
    /// | `Unknown`             | `Some(AssumedRomaji { ImmBridgeBroken })` | 不明 → 保守的に AssumedRomaji   |
    pub(crate) fn correction_for_imm_broken(&self) -> Option<InputModeState> {
        if self.input_mode.is_romaji_capable()
            || matches!(self.input_mode, InputModeState::ObservedEisu)
        {
            return None;
        }
        Some(InputModeState::AssumedRomaji {
            reason: AssumedReason::ImmBridgeBroken,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use awase::engine::{AssumedReason, InputModeState};

    fn belief(mode: InputModeState) -> ImeBelief {
        ImeBelief {
            input_mode: mode,
            is_japanese_ime: true,
            prev_conversion_mode: None,
        }
    }

    const ASSUMED: InputModeState = InputModeState::AssumedRomaji {
        reason: AssumedReason::ImmBridgeBroken,
    };

    // ── correction_for_imm_broken ────────────────────────────────────────────
    //
    // IMM-broken アプリ（Chrome 等）入場時の belief 補正ロジック。
    // Win32 呼び出しゼロの純粋関数なので実機なしでテスト可能。

    // romaji-capable な状態は補正しない。
    #[test]
    fn imm_broken_romaji_noop() {
        assert_eq!(belief(InputModeState::ObservedRomaji).correction_for_imm_broken(), None);
    }

    // AssumedRomaji は reason によらず補正しない。
    #[test]
    fn imm_broken_assumed_romaji_noop() {
        for reason in [
            AssumedReason::ImmBridgeBroken,
            AssumedReason::FocusTransition,
            AssumedReason::AppKindExcluded,
            AssumedReason::ForceOnGuardActive,
        ] {
            assert_eq!(
                belief(InputModeState::AssumedRomaji { reason }).correction_for_imm_broken(),
                None,
                "reason={reason:?}"
            );
        }
    }

    // ObservedEisu は英数モード確定済みのため補正しない（GJI tray 誤起動防止）。
    #[test]
    fn imm_broken_eisu_noop() {
        assert_eq!(
            belief(InputModeState::ObservedEisu).correction_for_imm_broken(),
            None,
        );
    }

    // ObservedKana は stale なかなとみなし AssumedRomaji に補正する。
    #[test]
    fn imm_broken_kana_yields_assumed() {
        assert_eq!(
            belief(InputModeState::ObservedKana).correction_for_imm_broken(),
            Some(ASSUMED),
        );
    }

    // Unknown も保守的に AssumedRomaji に補正する。
    #[test]
    fn imm_broken_unknown_yields_assumed() {
        assert_eq!(
            belief(InputModeState::Unknown).correction_for_imm_broken(),
            Some(ASSUMED),
        );
    }
}
