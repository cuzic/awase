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
//! | Blacklist typing 中の GJI I/O 観測（`ir_stage_observe`） | `ObservationSource::GjiIoInference`（こちらは真正の外部観測なので `InputModeObserved`） | [`gji_io_eisu_correction`] |
//! | TurnOn 系キー（ひらがな/かな等）受信、IME は既に open で OFF→ON 遷移なし（`kp_stage_shadow_ime_toggle` の no-op 分岐） | `InputModeApplyStrategy::UserTurnOnEisuReset` | [`eisu_reset_on_turn_on_while_open`] |
//!
//! この表と実装の対称性は `tests/architecture_guard.rs` の
//! `user_ime_on_paths_are_paired_with_eisu_reset` が監視する。
//! **新しい user IME-ON 経路を追加する場合は、[`eisu_reset_on_ime_on`] による救済を
//! 対で配線し、上記の表と guard テストの期待値を更新すること。**
//!
//! ## hwnd キャッシュ復元は対応表の対象外（別ガード）
//!
//! `apply_hwnd_cache_restore`（`state/platform_state.rs`）が復元する
//! `HwndImeSnapshot::input_mode` は「ユーザーが今 IME を ON にした」観測ではなく、
//! 最大 `HWND_CACHE_MAX_AGE_MS`（1 時間）前のスナップショットに過ぎない。他の
//! `InputModeApplied` 経路と異なり confidence を持たず reduce() が無条件に上書きする
//! ため、キャッシュされた `ObservedEisu` をそのまま復元すると、実際にはとうに解消
//! している可能性が高い stale な eisu 固着を engine activation ごと再現してしまう
//! （2026-07-09 MS Edge で実発生: Uwp⇔TsfNative フォーカス往復のたびに 131 秒前の
//! `ObservedEisu` キャッシュが復元され、eisu guard に阻まれて engine が inactive の
//! まま固着し続けた）。[`cache_restore_eisu_guard`] がこの経路専用の防御。
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
pub fn eisu_reset_on_ime_on(ime_turned_on: bool, mode: InputModeState) -> Option<InputModeState> {
    (ime_turned_on && mode == InputModeState::ObservedEisu).then_some(
        InputModeState::AssumedRomaji {
            reason: AssumedReason::AppKindExcluded,
        },
    )
}

/// フォーカス後の GJI I/O 観測による stale `ObservedEisu` 救済判定。
///
/// Blacklist アプリで GJI がフォーカス後に実際に変換 I/O をしている
/// （= 英数モードではあり得ない）ことが確認できた場合のみ、
/// `AssumedRomaji { ImmBridgeBroken }` への訂正値を返す。
/// これは awase 自身の先読みではなく真正の外部観測なので、呼び出し元は
/// `InputModeObserved { source: GjiIoInference, confidence: Medium }` で dispatch すること。
/// 方向は `ObservedEisu → AssumedRomaji` の一方通行のみ（他モードには触れない）。
///
/// # 引数
/// - `gji_io_after_focus`: フォーカス変更より後の GJI I/O が確認できたか
///   （`observe_gji_after_focus` の observer_poll=true と同じ条件）。
/// - `mode`: 現在の `input_mode` belief。
#[must_use]
pub fn gji_io_eisu_correction(
    gji_io_after_focus: bool,
    mode: InputModeState,
) -> Option<InputModeState> {
    (gji_io_after_focus && mode == InputModeState::ObservedEisu).then_some(
        InputModeState::AssumedRomaji {
            reason: AssumedReason::ImmBridgeBroken,
        },
    )
}

/// TurnOn 系キー（ひらがな/かな 等）受信時の stale `ObservedEisu` 救済判定。
///
/// [`eisu_reset_on_ime_on`] は OFF→ON 遷移でのみ発火するため、IME が既に open な
/// 状態でユーザーが「ひらがなに戻す」キー（`ShadowImeAction::TurnOn` に分類される
/// VK_DBE_HIRAGANA / VK_KANA 等）を押しても遷移が起きず救済されない。この関数は
/// その OFF→ON 遷移を伴わないケースを別に救済する。
///
/// `ShadowImeAction::Toggle`（VK_KANJI）は ON/OFF どちらへ向かうか一意に決まらない
/// ため対象外。TurnOn 系のみが「ひらがなへ戻す」という意図を一意に持つ。
///
/// # 引数
/// - `action_is_turn_on`: 呼び出し元の経路で `ShadowImeAction::TurnOn` が確定したか。
/// - `mode`: 現在の `input_mode` belief。
#[must_use]
pub fn eisu_reset_on_turn_on_while_open(
    action_is_turn_on: bool,
    mode: InputModeState,
) -> Option<InputModeState> {
    (action_is_turn_on && mode == InputModeState::ObservedEisu).then_some(
        InputModeState::AssumedRomaji {
            reason: AssumedReason::AppKindExcluded,
        },
    )
}

/// hwnd キャッシュ復元時の stale `ObservedEisu` 救済判定。
///
/// キャッシュされた `input_mode` が `ObservedEisu` の場合のみ `AssumedRomaji` に
/// 訂正する。キャッシュは生の観測ではなく最大 1 時間前のスナップショットのため、
/// 他の `InputModeApplied` 経路と同じ強さで engine activation を塞がせない。
/// `ObservedEisu` 以外はそのままキャッシュ値を信頼する（キャッシュの本来の目的を
/// 損なわないため、訂正は eisu 固着の解除のみに限定する）。
#[must_use]
pub fn cache_restore_eisu_guard(cached_mode: InputModeState) -> InputModeState {
    if cached_mode == InputModeState::ObservedEisu {
        InputModeState::AssumedRomaji {
            reason: AssumedReason::AppKindExcluded,
        }
    } else {
        cached_mode
    }
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
        assert_eq!(
            eisu_reset_on_ime_on(true, InputModeState::ObservedKana),
            None
        );
        assert_eq!(eisu_reset_on_ime_on(true, InputModeState::Unknown), None);
    }

    // ── gji_io_eisu_correction ──

    #[test]
    fn gji_io_corrects_eisu_with_imm_bridge_broken_reason() {
        assert_eq!(
            gji_io_eisu_correction(true, EISU),
            Some(InputModeState::AssumedRomaji {
                reason: AssumedReason::ImmBridgeBroken
            })
        );
    }

    #[test]
    fn gji_io_correction_requires_confirmed_io() {
        assert_eq!(gji_io_eisu_correction(false, EISU), None);
    }

    #[test]
    fn gji_io_correction_is_one_way_eisu_only() {
        // ObservedEisu 以外には触れない（逆方向・他モードの推定はしない）
        assert_eq!(
            gji_io_eisu_correction(true, InputModeState::ObservedRomaji),
            None
        );
        assert_eq!(
            gji_io_eisu_correction(true, InputModeState::ObservedKana),
            None
        );
        assert_eq!(gji_io_eisu_correction(true, InputModeState::Unknown), None);
    }

    // ── eisu_reset_on_turn_on_while_open ──

    #[test]
    fn turn_on_while_open_resets_eisu() {
        assert_eq!(
            eisu_reset_on_turn_on_while_open(true, EISU),
            Some(InputModeState::AssumedRomaji {
                reason: AssumedReason::AppKindExcluded
            })
        );
    }

    #[test]
    fn turn_on_while_open_requires_turn_on_action() {
        // Toggle (VK_KANJI) は ON/OFF どちらへ向かうか一意に決まらないため対象外。
        // 呼び出し元は action_is_turn_on=false として渡す。
        assert_eq!(eisu_reset_on_turn_on_while_open(false, EISU), None);
    }

    #[test]
    fn turn_on_while_open_is_one_way_eisu_only() {
        assert_eq!(
            eisu_reset_on_turn_on_while_open(true, InputModeState::ObservedRomaji),
            None
        );
        assert_eq!(
            eisu_reset_on_turn_on_while_open(true, InputModeState::ObservedKana),
            None
        );
        assert_eq!(
            eisu_reset_on_turn_on_while_open(true, InputModeState::Unknown),
            None
        );
    }

    // ── cache_restore_eisu_guard ──

    #[test]
    fn cache_restore_guard_corrects_stale_eisu() {
        assert_eq!(
            cache_restore_eisu_guard(EISU),
            InputModeState::AssumedRomaji {
                reason: AssumedReason::AppKindExcluded
            }
        );
    }

    #[test]
    fn cache_restore_guard_trusts_non_eisu_modes() {
        assert_eq!(
            cache_restore_eisu_guard(InputModeState::ObservedRomaji),
            InputModeState::ObservedRomaji
        );
        assert_eq!(
            cache_restore_eisu_guard(InputModeState::ObservedKana),
            InputModeState::ObservedKana
        );
        assert_eq!(
            cache_restore_eisu_guard(InputModeState::AssumedRomaji {
                reason: AssumedReason::ImmBridgeBroken
            }),
            InputModeState::AssumedRomaji {
                reason: AssumedReason::ImmBridgeBroken
            }
        );
        assert_eq!(
            cache_restore_eisu_guard(InputModeState::Unknown),
            InputModeState::Unknown
        );
    }
}
