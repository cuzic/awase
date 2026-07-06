//! stale `ObservedEisu` belief からの回復判定を集約する純粋関数群。
//!
//! ## 背景: ObservedEisu 循環デッドロック（2026-07-06 MS Edge で実発生）
//!
//! engine の activation 条件は `ime_on && input_mode.is_romaji_capable()` であり、
//! `ObservedEisu` は `NotRomajiInput` として activation を塞ぐ。一方
//! `transition_activation` は `NotRomajiInput` の場合 `SetOpen(true)` を抑制するため、
//! Decision 経由の救済 (`PostSetOpenEisuReset`) は原理的に発火できない。さらに
//! Imm32Unavailable（Chrome/Edge 等ブラックリスト）アプリでは IMM query がスキップされ
//! idle-conv-check も TsfNative 限定のため、**stale な ObservedEisu を訂正する観測経路が
//! 存在せず、engine が永久に inactive のまま**になる。
//!
//! この状態から抜けるには「IME を ON にする経路」ごとに ObservedEisu 救済を対で
//! 配線する必要がある。判定ロジックをこのモジュールの純粋関数に集約し、経路ごとの
//! 実装ドリフトを防ぐ。
//!
//! ## user IME-ON 経路 × ObservedEisu 救済の対応表
//!
//! | IME-ON 経路 | 救済 (strategy / source) | 判定関数 |
//! |---|---|---|
//! | Decision 経由 `SetOpen(true)`（`kp_stage_post_decision`） | `InputModeApplyStrategy::PostSetOpenEisuReset` | [`eisu_reset_on_ime_on`] |
//! | 物理 IME キー / SyncKey shadow toggle OFF→ON（`kp_stage_shadow_ime_toggle`） | `InputModeApplyStrategy::UserImeOnEisuReset` | [`eisu_reset_on_ime_on`] |
//! | refresh force-ON（`apply_force_on_for_imm_broken`） | `InputModeApplyStrategy::ImmBrokenCorrection`（ObservedEisu は eisu guard で意図的に対象外 — 受動的経路がユーザーの英数選択を踏み潰さないため） | `correction_for_imm_broken` |
//!
//! この表と実装の対称性は `tests/architecture_guard.rs` の
//! `user_ime_on_paths_are_paired_with_eisu_reset` が監視する。
//! **新しい user IME-ON 経路を追加する場合は、[`eisu_reset_on_ime_on`] による救済を
//! 対で配線し、上記の表と guard テストの期待値を更新すること。**
//!
//! ## eisu guard との関係
//!
//! `correction_for_imm_broken` の eisu guard は「ユーザーが意図的に英数モードを選んだ
//! 状態を、awase の**受動的な** force-ON（周期 refresh・フォーカス変更）が踏み潰さない」
//! ための保護。ここの救済は「ユーザーが**たった今**明示的に IME を ON にした」瞬間のみ
//! 発火するため、保護対象と衝突しない（IME-ON 直後の GJI/MS-IME はひらがなモードで
//! 再開するため、過去の英数観測は必ず stale）。

use awase::engine::{AssumedReason, InputModeState};

/// ユーザー起点で IME が ON になった直後の stale `ObservedEisu` 救済判定。
///
/// `ime_turned_on` が真（呼び出し元の経路で IME が実際に ON へ遷移した）かつ
/// belief が `ObservedEisu` の場合のみ、`AssumedRomaji` への訂正値を返す。
/// 訂正は `InputModeApplied`（awase 自身の能動的訂正）として dispatch すること。
/// 実際の入力モードは後続の観測（idle-conv-check / GJI 観測等）が再確認・再訂正する。
///
/// # 引数
/// - `ime_turned_on`: 経路固有の「IME が ON に遷移した」条件。
///   - Decision 経由: `applied && new_ime_on`
///   - shadow toggle: `!was_open && now_open`
/// - `mode`: 現在の `input_mode` belief。
#[must_use]
pub fn eisu_reset_on_ime_on(
    ime_turned_on: bool,
    mode: InputModeState,
) -> Option<InputModeState> {
    (ime_turned_on && mode == InputModeState::ObservedEisu).then_some(
        InputModeState::AssumedRomaji {
            reason: AssumedReason::AppKindExcluded,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const EISU: InputModeState = InputModeState::ObservedEisu;

    #[test]
    fn resets_eisu_when_ime_turned_on() {
        assert_eq!(
            eisu_reset_on_ime_on(true, EISU),
            Some(InputModeState::AssumedRomaji {
                reason: AssumedReason::AppKindExcluded
            })
        );
    }

    #[test]
    fn no_reset_when_ime_not_turned_on() {
        // OFF→OFF / ON→ON / ON→OFF はすべて ime_turned_on=false になる
        assert_eq!(eisu_reset_on_ime_on(false, EISU), None);
    }

    #[test]
    fn no_reset_for_romaji_capable_modes() {
        assert_eq!(
            eisu_reset_on_ime_on(true, InputModeState::ObservedRomaji),
            None
        );
        assert_eq!(
            eisu_reset_on_ime_on(
                true,
                InputModeState::AssumedRomaji {
                    reason: AssumedReason::ImmBridgeBroken
                }
            ),
            None
        );
    }

    #[test]
    fn no_reset_for_kana_and_unknown() {
        // ObservedKana / Unknown は correction_for_imm_broken (ImmBrokenCorrection) の
        // 担当領域。この関数は ObservedEisu 固着の救済に限定する。
        assert_eq!(eisu_reset_on_ime_on(true, InputModeState::ObservedKana), None);
        assert_eq!(eisu_reset_on_ime_on(true, InputModeState::Unknown), None);
    }
}
