//! conv-mode を変更する唯一の窓口（ADR-084 P1/INV-1、第一弾）。
//!
//! [ADR-084](../../../../docs/adr/084-conv-mode-single-ownership-and-width-ssot.md) は
//! conv-mode（`ImmSetConversionStatus`/`VK_DBE_*`）を変更しうるすべての経路を単一の
//! actuator へ集約し、書き込みと belief 無効化を同一トランザクションにすることを
//! 求めている（INV-1/INV-2）。本モジュールはその第一弾で、`Runtime::actuate_conv_mode`
//! を新設し、既存の呼び出し元のうち `kp_shift_conv_guard_key_down` の MS-IME entry
//! 書き込み**のみ**をこの関数経由に移行した。
//!
//! [ADR-086](../../../../docs/adr/086-force-write-trigger-and-target-identity.md)
//! INV-14（ターゲット同一性）に従い、実際の書き込みは
//! `ActuationTarget::capture` → `set_ime_conv_for_target` 経由で行う
//! （起案時点の hwnd を確定し、実行直前に再検証してから書く。BUG-59 追補が
//! 実機で踏んだ「別ウィンドウへの誤爆」を構造的に防ぐ）。`kp_stage_idle_conv_check`
//! の BUG-08 ローマ字復元・`kp_reset_to_hiragana_romaji_capsoff`
//! （read-modify-write、read 側は `get_ime_conv_for_target`）も移行済み
//! （いずれも `runtime/key_pipeline.rs`）。
//!
//! **未移行（次段のスコープ）**: `kp_restore_kana_from_half_width`
//! （`runtime/key_pipeline.rs`）の復元リトライループ（`shift_conv_guard_gen`/
//! `confirm_gate_deadline_override_ms` と密結合し、BUG-49 で複数回のレビューを
//! 経て確立した挙動のため本コミットでは触れていない）。これは
//! `set_ime_romaji_mode_with_target_async` を直接呼び続けている
//! （`docs/known-bugs.md` ADR-084 追補参照）。よって INV-1 が求める「低レベル API を
//! private にしてこの関数だけが呼べるようにする」というコンパイラ強制は、これら全ての
//! 移行が完了するまで導入できない。

use super::Runtime;
use crate::state::{ConvActuationOutcome, ConvModeTarget, ConvMutationReason, TickMs};

impl Runtime {
    /// conv-mode を変更する唯一の窓口。
    ///
    /// 成功・却下にかかわらず、実際の書き込みを行う前に必ず次を行う:
    ///
    /// 1. ADR-064 の `conv_mutation_allowed`（`Output::conv_mutation_allowed`、
    ///    `UserManaged`/エンジン非活性中は false）を確認する。false なら
    ///    `Rejected` を返し何も書き込まない。
    /// 2. `ImeModeFsm::unconfirm(reason)` を**同期的に**呼ぶ（INV-2）。async タスク
    ///    内で行うと、その完了前に送信ゲート（`Output::ms_ime_gate_defer`）が stale
    ///    な `is_native_ready()==true` を信じて素通ししてしまう
    ///    （BUG-49 の根本原因、`docs/known-bugs.md` 参照）。
    /// 3. `ms_ime_gate_give_up` ラッチを解除する。新たに conv actuation が起きた
    ///    以上、過去の期限切れ判定（IMC 不可読環境へのフォールバック）を持ち越す
    ///    理由がない。
    ///
    /// 実際の Win32 書き込み（`set_ime_romaji_mode_with_target_async`）は非同期
    /// （`spawn_local`）のまま行う。1〜3 が「書き込みと同一トランザクション」で
    /// 完結しさえすれば、実際の OS 反映が遅延すること自体は問題ない
    /// （ADR-084 §2 P1 の doc コメント参照）。
    ///
    /// `_tick_ms` は現状未使用。`InputModeApplied` dispatch（INV-6）は、この
    /// 関数が扱う `ConvMutationReason::ShiftSoloTapCounter` が投機的な安全網の
    /// 書き込み（P5 で明示的に許容された既存例外）であり、対応する確定した
    /// `InputModeState` の主張を伴わないため今回は行っていない（`kp_shift_conv_guard_key_up`
    /// が本物の単独タップと確定した時点で別途 `ObservedEisu` を dispatch する、
    /// 既存の分離を維持）。確定した mode 主張を伴う呼び出し元を移行する際は、
    /// 引数に `Option<(InputModeState, InputModeApplyStrategy)>` を追加し
    /// `Runtime::apply_input_mode_correction` を呼ぶこと。
    pub(crate) fn actuate_conv_mode(
        &self,
        target: ConvModeTarget,
        reason: ConvMutationReason,
        _tick_ms: TickMs,
    ) -> ConvActuationOutcome {
        if !self.platform.output.conv_mutation_allowed.get() {
            log::debug!(
                "[conv-actuate] {reason:?} → conv_mutation_allowed=false のため却下 \
                 (target={target:?})"
            );
            return ConvActuationOutcome::Rejected;
        }

        self.platform
            .output
            .ime_mode_fsm
            .borrow_mut()
            .unconfirm(reason.as_unconfirm_label());
        self.platform.output.ms_ime_gate_give_up.set(false);

        let raw_target = target.imm_conv_value();
        log::info!("[conv-actuate] {reason:?} → target=0x{raw_target:08X} 書き込み (spawn)");
        // ADR-086 INV-14: 起案時点（＝今、unconfirm と同一トランザクション内）の
        // hwnd を capture してから spawn_local へ渡す。フォーカス世代
        // （ime_mode_focus_gen）は `with_app` 経由でしか読めない（`ime.rs` は
        // Runtime/Output に依存しないため、verify_still_current へは
        // クロージャとして渡す）。`with_app` が None を返す場合（再入不可）は
        // 安全側に倒して起案時点の gen と一致しない値を返し、書き込みを
        // 中止させる（BUG-59 追補が使っていたのと同じフォールバック手法:
        // `wrapping_add(1)`）。
        let focus_gen = self.platform.output.ime_mode_focus_gen.get();
        win32_async::spawn_local(async move {
            let Some(target) = crate::ime::ActuationTarget::capture(focus_gen).await else {
                log::debug!("[conv-actuate] {reason:?} → capture 失敗（フォーカス無し）");
                return;
            };
            let outcome = crate::ime::set_ime_conv_for_target(target, Some(raw_target), || {
                crate::with_app(|runtime| runtime.platform.output.ime_mode_focus_gen.get())
                    .unwrap_or_else(|| focus_gen.wrapping_add(1))
            })
            .await;
            log::info!("[conv-actuate] {reason:?} → 結果: {outcome:?}");
        });

        ConvActuationOutcome::Actuated
    }
}
