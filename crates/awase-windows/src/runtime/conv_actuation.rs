//! conv-mode を変更する唯一の窓口（ADR-084 P1/INV-1、第一弾）。
//!
//! [ADR-084](../../../../docs/adr/084-conv-mode-single-ownership-and-width-ssot.md) は
//! conv-mode（`ImmSetConversionStatus`/`VK_DBE_*`）を変更しうるすべての経路を単一の
//! actuator へ集約し、書き込みと belief 無効化を同一トランザクションにすることを
//! 求めている（INV-1/INV-2）。本モジュールはその第一弾で、`Runtime::actuate_conv_mode`
//! を新設し、既存の呼び出し元のうち `kp_shift_conv_guard_key_down` の MS-IME entry
//! 書き込み**のみ**をこの関数経由に移行した。
//!
//! **未移行（次段のスコープ）**: `kp_restore_kana_from_half_width` の復元リトライ
//! ループ（`shift_conv_guard_gen`/`confirm_gate_deadline_override_ms` と密結合し、
//! BUG-49 で複数回のレビューを経て確立した挙動のため本コミットでは触れていない）、
//! `tsf/warmup/cold_warmup.rs::preamble`、`runtime/executor.rs`、
//! `kp_stage_idle_conv_check` のローマ字復元経路（`runtime/key_pipeline.rs` 内複数箇所）。
//! これらは `set_ime_romaji_mode_with_target_async` を直接呼び続けている
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
        win32_async::spawn_local(async move {
            let ok = crate::ime::set_ime_romaji_mode_with_target_async(Some(raw_target)).await;
            log::info!("[conv-actuate] IMC write 結果: ok={ok}");
        });

        ConvActuationOutcome::Actuated
    }
}
