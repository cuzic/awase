#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! TSF cold-start ウォームアップシーケンス。
//!
//! [`ColdWarmupSequence::run_start`] は IMM32 ローマ字モード復元・診断ログ・
//! `cold_seq` 発行のみを行い、即座に [`WarmupStarted`] を返す。
//!
//! 予防的な F2 (VK_DBE_HIRAGANA) warmup 送信・`TsfReadinessProbe` の事前待機
//! （旧 `WarmupKind::FreshF2`/`ReWarmup`/`ProbeWithSettle` 行列、`ColdReason`×
//! `long_idle` で決まる `eager_settle_ms`/`probe_min_ms`）は 2026-07-18 に撤去した。
//! per-VK confirm（`probe_fsm.rs::run_per_vk_confirm`）が送信後の confirm/recovery を
//! 担うため、送信前に GJI の準備を待つ予防は二重の保険だった（実機ソーク数日、
//! 無破損を確認。`docs/known-bugs.md` BUG-24 追補参照）。

/// `ColdWarmupSequence::run_start` の戻り値。
///
/// 即座に実行できる部分（IMM32 設定等）は完了済み。
/// 残りの待機はタイマー（TIMER_TSF_PROBE）で `TsfReadinessProbe::check_now` を
/// ポーリングすることで行う。
pub(crate) struct WarmupStarted {
    /// GJI 静止プローブ
    pub probe: crate::tsf::probe::TsfReadinessProbe,
    /// probe の最大待機時間 (ms, warmup_sent_ms 起点)。予防的待機を撤去したため常に 0。
    pub total_max_ms: u64,
    /// cold になった理由（NAMECHANGE フェーズの判断に使用）
    pub cold_reason: crate::output::ColdReason,
}

/// TSF cold-start ウォームアップシーケンスを管理する構造体。
pub(crate) struct ColdWarmupSequence<'a> {
    output: &'a crate::output::Output,
}

impl<'a> ColdWarmupSequence<'a> {
    /// 新しいシーケンスを生成する。
    pub(crate) const fn new(output: &'a crate::output::Output) -> Self {
        Self { output }
    }

    /// ノンブロッキング版ウォームアップ開始。
    ///
    /// IMM32 ローマ字モード復元のみ即座に行い、F2 warmup 送信・
    /// `TsfReadinessProbe` の事前待機は行わない（即座に per-VK confirm へ進む
    /// [`WarmupStarted`] を返す）。
    pub(crate) fn run_start(&self, session_expired: bool, elapsed_ms: u64) -> WarmupStarted {
        // 診断ログ (get_ime_conversion_mode_raw) と IMM32 ローマ字モード復元
        // (set_ime_romaji_mode) は SendMessageTimeoutW を呼ぶため、メインスレッドで
        // 同期実行すると `with_app` 再入の原因になる。ワーカースレッドに offload する
        // async ラッパーを spawn_local で起動して退避する。
        //
        // カタカナ/英数への明示的復元（KATAKANA/FULLSHAPE ビット等）は BUG-19 の
        // ロックイン事故を受けて撤去した。`conv_mode_policy = observe`（デフォルト）
        // では常に None（ROMAN ビット確保のみ）を書き戻す（`docs/known-bugs.md`
        // BUG-19 参照）。
        //
        // `conv_mode_policy = force`（オプトイン設定）のときのみ、awase トレイで
        // 選択した `desired_mode` を毎回 cold 転換時に冪等に強制する。BUG-19 と
        // 異なり「観測したカタカナに追従する」のではなく「observation に関係なく
        // 常に同じ目標へ引き戻す」一方向の書き込みのため、自己増幅ループにはならない。
        let conv_mutation_allowed = self.output.conv_mutation_allowed.get();
        let forced_target = match self.output.conv_mode.policy() {
            crate::state::ConvModePolicy::Observe => None,
            crate::state::ConvModePolicy::Force => {
                Some(self.output.conv_mode.desired_mode().to_conv_bits())
            }
        };
        // ADR-086 INV-14: 起案時点の hwnd を capture してから spawn_local へ渡す。
        // フォーカス世代（ime_mode_focus_gen）は `self.output`（借用、'static でない）
        // をそのまま async へ持ち込めないため、`with_app` 経由で毎回読み直す
        // （`runtime::conv_actuation::actuate_conv_mode` と同じ手法）。
        let focus_gen = self.output.ime_mode_focus_gen.get();
        win32_async::spawn_local(async move {
            let conv_pre = crate::ime::get_ime_conversion_mode_raw_timeout_async(50).await;
            log::debug!(
                "[cold-diag] pre-send conv={} NATIVE={} ROMAN={} KATAKANA={} write={conv_mutation_allowed} \
                 forced_target={}",
                conv_pre.map_or_else(|| "none".to_string(), |v| format!("0x{v:08X}")),
                conv_pre.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_NATIVE)),
                conv_pre.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_ROMAN)),
                conv_pre.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_KATAKANA)),
                forced_target.map_or_else(|| "none".to_string(), |v| format!("0x{v:08X}")),
            );
            if conv_mutation_allowed {
                let Some(target) = crate::ime::ActuationTarget::capture(focus_gen).await else {
                    log::debug!("[cold-diag] capture 失敗（フォーカス無し） → 書き込み中止");
                    return;
                };
                let outcome = crate::ime::set_ime_conv_for_target(target, forced_target, || {
                    crate::with_app(|runtime| runtime.platform.output.ime_mode_focus_gen.get())
                        .unwrap_or_else(|| focus_gen.wrapping_add(1))
                })
                .await;
                log::debug!("[cold-diag] 結果: {outcome:?}");
            }
        });

        let cold_seq = self.output.composition.increment_cold_start_count();
        // SAFETY: Win32 GetForegroundWindow + GetClassName; returns empty string on failure.
        let win_class = unsafe { crate::ime::get_foreground_window_class() };
        let cold_reason = self.output.composition.last_cold_reason();
        log::debug!(
            "[h1-warmup] cold={cold_seq} class={win_class} session_expired={session_expired} \
             elapsed={elapsed_ms}ms reason={cold_reason:?} → F2/probe待機省略、per-VK confirm へ",
            cold_seq = cold_seq.value(),
        );

        WarmupStarted {
            probe: crate::tsf::probe::TsfReadinessProbe::new(
                crate::hook::current_tick_ms(),
                cold_seq,
                0,
            ),
            total_max_ms: 0,
            cold_reason,
        }
    }
}
