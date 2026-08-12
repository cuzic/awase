//! conv-mode を変更する唯一の窓口（ADR-084 P1/INV-1、第一弾）。
//!
//! [ADR-084](../../../../docs/adr/084-conv-mode-single-ownership-and-width-ssot.md) は
//! conv-mode（`ImmSetConversionStatus`/`VK_DBE_*`）を変更しうるすべての経路を単一の
//! actuator へ集約し、書き込みと belief 無効化を同一トランザクションにすることを
//! 求めている（INV-1/INV-2）。本モジュールはその第一弾で、`Output::actuate_conv_mode`
//! を新設し、既存の呼び出し元のうち `kp_shift_conv_guard_key_down` の MS-IME entry
//! 書き込み**のみ**をこの関数経由に移行した。
//!
//! [ADR-086](../../../../docs/adr/086-force-write-trigger-and-target-identity.md)
//! INV-14（ターゲット同一性）に従い、実際の書き込みは
//! `ActuationTarget::capture` → `set_ime_conv_for_target` 経由で行う
//! （起案時点の hwnd を確定し、実行直前に再検証してから書く。BUG-59 追補が
//! 実機で踏んだ「別ウィンドウへの誤爆」を構造的に防ぐ）。
//!
//! **INV-14: 全 7 経路が移行完了（2026-08-08）。** 旧
//! `set_ime_romaji_mode_with_target`/`_async`（宛先をライブクエリで自己決定する
//! target 非対応の低レベル API）は `ime.rs` から削除済み。移行済み経路:
//! `actuate_conv_mode`（本モジュール）、`cold_warmup.rs::run_start`、
//! `executor.rs::dispatch_ime_set_open`（`set_ime_open_then_conv_for_target`、
//! open と conv を同一 hwnd に対して行う特殊版）、`key_pipeline.rs` の
//! `kp_stage_idle_conv_check`（BUG-08 ローマ字復元）・
//! `kp_reset_to_hiragana_romaji_capsoff`（read-modify-write、read 側は
//! `get_ime_conv_for_target`）・`kp_restore_kana_from_half_width`
//! （shift-conv-guard 復元リトライループ）・`apply_focus_probe`
//! （`ImmCrossProbe` かなモード補正書き込み）。
//!
//! **INV-1 は依然未達（本モジュールが単一窓口になっていない経路が残る）。**
//! `kp_reset_to_hiragana_romaji_capsoff` は `actuate_conv_mode` を経由しない
//! （`conv_mutation_allowed` ゲートも `unconfirm()` も通らない）。
//! `ConvModeTarget` に read-modify-write 用の variant が無く、
//! `actuate_conv_mode` 経由にすると `conv_mutation_allowed` 却下が新たに
//! 効いて挙動が変わってしまうため、意図的に見送っている
//! （`key_pipeline.rs` 該当箇所のコメント参照）。
//!
//! **移行済み（ADR-089 §6 Phase C item 12、2026-08-12）:**
//! `ime_controller.rs` の `ImmCrossProcessStrategy::apply` と
//! `MsImeDirectStrategy::apply` が別々に呼んでいた
//! `crate::ime::set_ime_romaji_mode()`（宛先をライブクエリで自己決定する同期
//! IMC write。ADR-086 Phase 1〜2 の「7 経路」の数え漏れ）は、
//! `ime_controller::apply_mechanism` の ROMAN 補完ステップ
//! （`romaji_pre_write`）1 箇所へ統合し、
//! `ActuationTarget::capture_blocking` → `set_ime_romaji_mode_for_target_blocking`
//! 経由へ移した。低レベル API（`set_ime_romaji_mode` / `_async`）は削除済み。
//!
//! ADR-086 Phase 3 は「`ActuationTarget` 化には `ImeOpenStrategy::apply` 自体の
//! 非同期化が必要」として見送っていたが、これは `verify_still_current` の
//! hwnd 再クエリ（`async`）まで必須と読んだ場合にのみ正しい。**捕獲自体は
//! `get_focused_hwnd()` 1 回**であり、旧 `set_ime_romaji_mode()` が内部で
//! やっていたライブクエリと同一なので、「捕獲を write の外へ出す」だけなら
//! 同期のままできる。再検証は focus 世代の照合のみで行う
//! （`ActuationTarget::verify_gen_only` の doc 参照）。
//!
//! 到達可能性の裏取り（ADR-089 §6 Phase C 実施記録 C-7）: 全呼び出し元を
//! 追跡した結果、**`ImmCrossProcessStrategy::apply` は同期経路から到達しない**
//! ことが分かった（`executor.rs`/`key_pipeline.rs` は
//! `imm_cross_is_first_applicable` で async 分岐、`apply_force_on_for_imm_broken`
//! と `arm_force_open_pending` は `!can_use_imm32_cross_process()` を要求、
//! `ir_apply_drift_correction` は ImmCross なら `set_ime_open` を使う分岐、
//! `ime_refresh.rs` の GJI TsfNative 強制 ON と `key_pipeline.rs` の
//! idle-conv-check は TsfNative 限定）。実際に到達するのは
//! `MsImeDirectStrategy::apply`（force-ON 系）だけである。
//!
//! **残る穴**: 同期 ImmCross の open write（`set_ime_open_cross_process`）は
//! 依然として自分でライブクエリする（150ms、フォールバック無し）。ROMAN 補完の
//! 捕獲（30ms + `GetForegroundWindow` フォールバック）と hwnd 解決の意味論が
//! 異なるため、両者を 1 回の捕獲へ寄せるには実機実測が要る
//! （ADR-089 §9-18）。上記のとおり現時点で到達しないため潜在的な穴である。
//!
//! **撤去済み（2026-08-09 BUG-61 対応で追加 → BUG-61/BUG-62 実機確認により
//! 無用と判明し撤去）:** `Runtime::tray_inject_romaji_mode_vk`
//! （Ctrl+Alt+R/K デバッグホットキー、`VK_DBE_ROMAN`/`VK_DBE_NOROMAN` 直接
//! SendInput）は、scan コードを実機で効くと確認済みの値（`0x70`）に固定した
//! 上で再検証しても conv が一切変化しないことが実機で確認された
//! （`docs/known-bugs.md` BUG-61 追補）。SendInput 経由の DBE 系 VK 注入で
//! ROMAN ビットを制御する経路は存在しないと確定したため、本モジュールの
//! ADR-084 INV-1 例外としての記載も含め撤去した。
//!
//! **`Runtime` から `Output` への移設（2026-08-08、ADR-086 Phase 2 設計調査）**:
//! 当初 `impl Runtime` に置かれていたが、本体は `self.platform.output.*` しか
//! 読んでおらず（`with_app` を呼ぶのは `spawn_local` 後の非同期部のみ）、実体は
//! 既に `Output` のメソッドだった。ADR-086 Phase 2 が `Output` 層に新設する
//! `force_pending`（武装フラグ）・消費点（`Output::send_romaji`）と同じ層に
//! 揃えるため本モジュールへ移設した。`Runtime::actuate_conv_mode`
//! （`runtime/conv_actuation.rs`）は 1 行 delegate として残す
//! （ADR-084 INV-1 が指す関数名を変えないため、および既存呼び出し元
//! `key_pipeline.rs::kp_shift_conv_guard_key_down` を触らないため）。

use super::Output;
use crate::state::{ConvActuationOutcome, ConvModeTarget, ConvMutationReason, TickMs};

impl Output {
    /// conv-mode を変更する唯一の窓口。
    ///
    /// 成功・却下にかかわらず、実際の書き込みを行う前に必ず次を行う:
    ///
    /// 1. ADR-064 の `conv_mutation_allowed`（`UserManaged`/エンジン非活性中は false）を
    ///    確認する。false なら `Rejected` を返し何も書き込まない。
    /// 2. `ImeModeFsm::unconfirm(reason)` を**同期的に**呼ぶ（INV-2）。async タスク
    ///    内で行うと、その完了前に送信ゲート（`ms_ime_gate_defer`）が stale な
    ///    `is_native_ready()==true` を信じて素通ししてしまう
    ///    （BUG-49 の根本原因、`docs/known-bugs.md` 参照）。
    /// 3. `ms_ime_gate_give_up` ラッチを解除する。新たに conv actuation が起きた
    ///    以上、過去の期限切れ判定（IMC 不可読環境へのフォールバック）を持ち越す
    ///    理由がない。
    ///
    /// 実際の Win32 書き込み（`ActuationTarget::capture` → `set_ime_conv_for_target`）は非同期
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
        tick_ms: TickMs,
    ) -> ConvActuationOutcome {
        self.actuate_conv_mode_with_completion(target, reason, tick_ms, |_outcome| {})
    }

    /// [`Self::actuate_conv_mode`] の内部実装。非同期書き込みの完了時に
    /// `on_async_result` を呼ぶ点だけが異なる（`consume_force_pending_and_actuate` の
    /// 再武装判定専用、ADR-086 Phase 2 item 5）。
    ///
    /// `on_async_result` は書き込みが実際に起きた場合のみ呼ばれる
    /// （`Rejected` で早期 return した場合は呼ばれない —— 呼び出し元は同期の
    /// 戻り値 `ConvActuationOutcome::Rejected` で判定できるため）。`None` は
    /// `ActuationTarget::capture` 失敗（フォーカス無し）を表す。
    fn actuate_conv_mode_with_completion(
        &self,
        target: ConvModeTarget,
        reason: ConvMutationReason,
        _tick_ms: TickMs,
        on_async_result: impl FnOnce(Option<crate::ime::ActuationOutcome>) + 'static,
    ) -> ConvActuationOutcome {
        if !self.conv_mutation_allowed.get() {
            log::debug!(
                "[conv-actuate] {reason:?} → conv_mutation_allowed=false のため却下 \
                 (target={target:?})"
            );
            return ConvActuationOutcome::Rejected;
        }

        self.ime_mode_fsm
            .borrow_mut()
            .unconfirm(reason.as_unconfirm_label());
        self.ms_ime_gate_give_up.set(false);

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
        let focus_gen = self.ime_mode_focus_gen.get();
        win32_async::spawn_local(async move {
            let Some(target) = crate::ime::ActuationTarget::capture(focus_gen).await else {
                log::debug!("[conv-actuate] {reason:?} → capture 失敗（フォーカス無し）");
                on_async_result(None);
                return;
            };
            let outcome = crate::ime::set_ime_conv_for_target(target, Some(raw_target), || {
                crate::with_app(|runtime| runtime.platform.output.ime_mode_focus_gen.get())
                    .unwrap_or_else(|| focus_gen.wrapping_add(1))
            })
            .await;
            log::info!("[conv-actuate] {reason:?} → 結果: {outcome:?}");
            on_async_result(Some(outcome));
        });

        ConvActuationOutcome::Actuated
    }

    /// `force_pending`（ADR-086 Phase 2 の武装フラグ）を消費し、武装済みなら
    /// `actuate_conv_mode` で force-write を起こす。
    ///
    /// `Output::send_romaji`/`send_kana_char`（送信要求という入力意図に紐づく
    /// 唯一の消費点、INV-15 item 3）から呼ぶこと。武装を**同期的に**消費してから
    /// 実際の書き込みを非同期で起案する —— 同期消費でないと同一バッチ内の後続
    /// 送信要求が二重に消費してしまう。
    ///
    /// **再武装（item 5）**: 消費後の非同期書き込みが `ActuationTarget::capture`
    /// 失敗または `Aborted` だった場合、「消費済み・未書き込み」のまま次の
    /// `FocusChange` まで force が永久に発火しない穴ができる。これを防ぐため、
    /// 完了時に **武装した時点からフォーカス世代が変わっていない**（＝同一フォーカス
    /// 内での一時的な失敗であり、`on_ime_mode_focus_changed` による新しい武装が
    /// 割り込んでいない）ことを確認してから再武装する。世代が変わっていれば、
    /// 既にその新しいフォーカスに対する正規の武装が（あるいは force policy が
    /// 有効なら）別途行われているはずなので、ここでは何もしない —— 古い世代の
    /// 値で再武装すると、新しい正規の武装を誤って上書きしてしまう。
    ///
    /// `Failed`（Win32 呼び出し自体の失敗、ターゲット不一致ではない）は再武装
    /// 対象に含めない。持続的に失敗する環境で毎回の入力ごとに書き込みを再試行
    /// することは入力意図に紐づく限り自己駆動（INV-16）ではないが、スコープを
    /// 「消費済み・未書き込みの穴を塞ぐ」に絞るため、Aborted/capture 失敗のみを
    /// 対象とする。
    pub(crate) fn consume_force_pending_and_actuate(&self) {
        let Some(armed_gen) = self.force_pending.take() else {
            return;
        };
        let target = ConvModeTarget::Desired(self.conv_mode.desired_mode());
        let tick_ms = TickMs(crate::hook::current_tick_ms());
        let outcome = self.actuate_conv_mode_with_completion(
            target,
            ConvMutationReason::ForcePolicy,
            tick_ms,
            move |async_result| {
                let should_rearm = matches!(
                    async_result,
                    None | Some(crate::ime::ActuationOutcome::Aborted(_))
                );
                if !should_rearm {
                    return;
                }
                let _ = crate::with_app(|runtime| {
                    let output = &runtime.platform.output;
                    if output.ime_mode_focus_gen.get() == armed_gen {
                        log::debug!(
                            "[force-pending] 消費済み・未書き込み（{async_result:?}）→ \
                             同一フォーカス内のため再武装 (gen={armed_gen})"
                        );
                        output.force_pending.set(Some(armed_gen));
                    } else {
                        log::debug!(
                            "[force-pending] 消費済み・未書き込み（{async_result:?}）だが \
                             フォーカス世代が武装時から変化（armed={armed_gen}, \
                             current={}）→ 再武装しない（別の正規の武装を上書きしないため）",
                            output.ime_mode_focus_gen.get()
                        );
                    }
                });
            },
        );
        if matches!(outcome, ConvActuationOutcome::Rejected) {
            log::debug!(
                "[force-pending] 消費 (gen={armed_gen}) → conv_mutation_allowed=false の \
                 ため actuate は却下（再武装しない — エンジン非活性中に書くべきでないため）"
            );
        }
    }
}
