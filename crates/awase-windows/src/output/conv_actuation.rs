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
//! 到達可能性（ADR-089 §6 Phase C 実施記録 C-7 と §9-21 の訂正）:
//! `executor.rs`/`key_pipeline.rs` は `imm_cross_is_first_applicable` で async
//! 分岐、`apply_force_on_for_imm_broken` と `arm_force_open_pending` は
//! `!can_use_imm32_cross_process()` を要求、`ir_apply_drift_correction` は
//! ImmCross なら `set_ime_open` を使う分岐、`ime_refresh.rs` の GJI TsfNative
//! 強制 ON と `key_pipeline.rs` の idle-conv-check は TsfNative 限定——
//! **ただしこれで全部ではない**。
//!
//! **`runtime/mod.rs::try_force_on_bootstrap`（`:892`）から
//! `ImmCrossProcessStrategy::apply` に同期で到達する。**
//! 同関数のガードは `detect_miss_count()` / `is_user_enabled()` /
//! `is_eligible_for_ime_force_on()`（`is_japanese_ime() && effective_open()`）/
//! `!is_force_on_guard_active()` だけで、上記 2 経路が持つ
//! `!can_use_imm32_cross_process()` の**プロファイルガードを持たない**。
//! したがって Standard（LINE / Qt 等）で IME 検出ミスが閾値回連続したときの
//! bootstrap force-ON は `caps` chain の先頭 `ImmCross` に入る
//! （`state/open_warrant.rs:1166`/`:1187` のテストコメントも
//! 「`try_force_on_bootstrap` 呼び出し元は ImmCross プロファイル側で
//! 到達する」と記録している）。2026-08-12 の Phase C 記録は当初これを
//! 数え落として「同期経路からは到達しない」と書いていた（ADR-089 §9-21 で訂正）。
//!
//! **残る穴**: 同期 ImmCross の open write（`set_ime_open_cross_process`）は
//! 依然として自分でライブクエリする（150ms、フォールバック無し）。ROMAN 補完の
//! 捕獲（30ms + `GetForegroundWindow` フォールバック）と hwnd 解決の意味論が
//! 異なるため、両者を 1 回の捕獲へ寄せるには実機実測が要る
//! （ADR-089 §9-18）。上記のとおり**この穴は現に到達しうる**が、
//! **Phase C 以前から同じ挙動**である——旧実装でも同じ呼び出し元から
//! `ImmCrossProcessStrategy::apply` に入り、その中の `set_ime_romaji_mode()`
//! と `set_ime_open_cross_process()` が別々に宛先をライブクエリしていた。
//! Phase C は前者を捕獲済み `ActuationTarget` へ移しただけで、2 つの hwnd
//! 解決が別物である点は変えていない（新規の回帰ではない）。実機での確認は
//! ADR-089 §9-17 の 17-h。
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
//! 既に `Output` のメソッドだった。`Output` 層へ揃えるため本モジュールへ移設した。
//! `Runtime::actuate_conv_mode`（`runtime/conv_actuation.rs`）は 1 行 delegate として
//! 残す（ADR-084 INV-1 が指す関数名を変えないため、および既存呼び出し元
//! `key_pipeline.rs::kp_shift_conv_guard_key_down` を触らないため）。
//!
//! **`force_pending`（force-write の武装フラグ）・`consume_force_pending_and_actuate`
//! は 2026-08-17、ADR-094 で `conv_mode_policy = force` ポリシー自体を撤去した
//! のに伴い削除した。** ADR-086 Phase 2/3 が本モジュールに導入した force-write
//! 機構（conv 軸・open 軸の両方）は本 ADR で全撤去されている。詳細は ADR-094 参照。

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
        _tick_ms: TickMs,
    ) -> ConvActuationOutcome {
        if !self.conv_mutation_allowed.get() {
            tracing::debug!(
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
        tracing::info!("[conv-actuate] {reason:?} → target=0x{raw_target:08X} 書き込み (spawn)");
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
                tracing::debug!("[conv-actuate] {reason:?} → capture 失敗（フォーカス無し）");
                return;
            };
            let outcome = crate::ime::set_ime_conv_for_target(target, Some(raw_target), || {
                crate::with_app(|runtime| runtime.platform.output.ime_mode_focus_gen.get())
                    .unwrap_or_else(|| focus_gen.wrapping_add(1))
            })
            .await;
            tracing::info!("[conv-actuate] {reason:?} → 結果: {outcome:?}");
        });

        ConvActuationOutcome::Actuated
    }
}
