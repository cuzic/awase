//! GJI cold-start warmup 専用ステートマシン。
#![allow(private_interfaces)]
//!
//! [`GjiWarmupFsm`] は `TsfProbeMachine` から GJI 固有のフェーズを切り出した FSM。
//! GJI の静止待ち・FreshF2 送信・NameChangeWait を担当し、
//! transmit 準備が整ったら [`ProbeAction::Transmit`]（Tsf）または
//! [`ProbeAction::StartLiteralDetect`] を emit して終了する。
//!
//! ## フェーズ遷移
//!
//! ```text
//! Probing(GjiInitial) ─[settle 不要]──► WaitingForCallback(TransmitDone) ─► emit Transmit or StartLiteralDetect
//!                     └─[settle 必要]─► WaitingForCallback(FreshF2Sent)   ─[apply_fresh_f2_sent]─► NameChangeWait
//!                                                                                        ├─[nc_fired && !settled]─► Probing(GjiSecondary) ─► WaitingForCallback
//!                                                                                        └─[その他]────────────────────────────────────────────────────────────────► emit Transmit or StartLiteralDetect
//! ```

use crate::tsf::observer::NamechangeBaseline;
use crate::tsf::output::ColdReason;
use crate::tsf::probe::TsfReadinessProbe;
use crate::tsf::probe_fsm::{
    decide_transmit_plan, DeferredVk, LiteralDetectConfig, ProbeAction, ProbeObservations,
    SendState, TransmitTarget, TsfEnvSnapshot, WaitingFor,
};
use crate::tsf::tickable_fsm::TickableFsm;
use awase::types::VkCode;
use timed_fsm::Clock;

use crate::tsf::probe_fsm::SystemClock;

// ── GjiProbePhase ────────────────────────────────────────────────────────────

/// GJI warmup FSM の現在フェーズ。
pub(crate) enum GjiProbePhase {
    /// GJI 静止待ち（`kind` で初回 / 二次を区別）。
    Probing {
        probe: TsfReadinessProbe,
        total_max_ms: u64,
        kind: GjiProbeKind,
        send: SendState,
    },
    /// dispatcher コールバック待ち。
    WaitingForCallback(WaitingFor),
    /// OBJ_NAMECHANGE 待ち。
    NameChangeWait {
        nc_baseline: NamechangeBaseline,
        deadline_ms: u64,
        fresh_f2_ms: u64,
        probe_settled: bool,
        /// fresh F2 送信直前の `gji_last_write_ms()` スナップショット。
        /// タイムアウト時に現在値と比較して GJI が F2 に I/O 応答したか判定する。
        gji_write_baseline: u64,
        send: SendState,
    },
    /// 終端フェーズ（transmit 完了後）。
    Done,
}

/// GJI probe の種類。
enum GjiProbeKind {
    /// 初回 GJI probe。settle check が必要な場合あり。
    Initial {
        needs_settle_check: bool,
        cold_reason: ColdReason,
    },
    /// OBJ_NAMECHANGE 後の二次 GJI probe。settle check なし。
    Secondary,
}

// ── GjiProbeContext ──────────────────────────────────────────────────────────

/// GJI probe 全体で不変の送信コンテキスト。
#[derive(Debug, Clone, Copy)]
struct GjiProbeContext {
    prepend_f2_warmup: bool,
    used_eager_path: bool,
    /// NameChangeWait フェーズの deadline budget (ms)。GjiFsm の ColdKind 由来。
    ncwait_budget_ms: u64,
    /// F2 をバッチに強制同梱するか（GjiFsm の ColdKind::Medium/Long で true）。
    forces_prepend_f2: bool,
    /// GjiFsm の ColdKind::Long（≥10s idle）か。
    is_long_cold: bool,
    /// プローブ開始前に VK_DBE_HIRAGANA pair が送信済みか（ReWarmup / FreshF2 / non-eager）。
    fresh_f2_at_probe_start: bool,
    /// `EmitSendFreshF2` を経由して fresh F2 を送信済みか。
    ///
    /// `apply_fresh_f2_sent` 呼び出し時に true になる。
    /// `enter_transmit_tsf` で `suppress_f2_in_batch` の判定に使い、
    /// TSF モードで F2 を再送するとリテラル化レース（"kお" バグ）が起きるのを防ぐ。
    fresh_f2_sent: bool,
}

// ── NextStep (FSM 内部遷移) ───────────────────────────────────────────────────

enum NextStep {
    Wait,
    EmitSendFreshF2 { probe_settled: bool },
    TransmitTsf { nc_fired: bool, gji_resumed: bool },
    StartSecondaryProbe { fresh_f2_ms: u64 },
}

// ── GjiWarmupFsm ─────────────────────────────────────────────────────────────

/// GJI cold-start warmup 専用 FSM。
///
/// `TsfProbeMachine` の GJI フェーズ（`Probing(GjiInitial/GjiSecondary)` +
/// `WaitingForCallback(FreshF2Sent)` + `NameChangeWait`）を担当する。
/// transmit 準備が整ったら [`ProbeAction::Transmit`] または
/// [`ProbeAction::StartLiteralDetect`] を emit して終了する。
///
/// LiteralDetect フェーズ自体は `LiteralDetectFsm`（別ファイル予定）が担当する。
pub(crate) struct GjiWarmupFsm {
    cold_seq: u32,
    ctx: GjiProbeContext,
    phase: GjiProbePhase,
}

impl GjiWarmupFsm {
    /// GJI cold-start warmup 用コンストラクタ。
    ///
    /// `TsfProbeMachine::new_gji` と同等のパラメータを受け取る。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        romaji: &str,
        cold_seq: u32,
        probe: TsfReadinessProbe,
        total_max_ms: u64,
        needs_settle_check: bool,
        cold_reason: ColdReason,
        prepend_f2_warmup: bool,
        used_eager_path: bool,
        ncwait_budget_ms: u64,
        forces_prepend_f2: bool,
        is_long_cold: bool,
        fresh_f2_at_probe_start: bool,
    ) -> Self {
        Self {
            cold_seq,
            ctx: GjiProbeContext {
                prepend_f2_warmup,
                used_eager_path,
                ncwait_budget_ms,
                forces_prepend_f2,
                is_long_cold,
                fresh_f2_at_probe_start,
                fresh_f2_sent: false,
            },
            phase: GjiProbePhase::Probing {
                probe,
                total_max_ms,
                kind: GjiProbeKind::Initial {
                    needs_settle_check,
                    cold_reason,
                },
                send: SendState::new(romaji),
            },
        }
    }

    /// ログ用 cold_seq 参照。
    pub(crate) const fn cold_seq_hint(&self) -> u32 {
        self.cold_seq
    }

    /// `SendFreshF2` dispatch 時に追加 F2 (F2×2) を送るかを示す。
    pub(crate) fn forces_prepend_f2_for_extra_f2(&self) -> bool {
        self.ctx.forces_prepend_f2
    }

    /// probe 進行中に後続 VK を 1 つ蓄積する。
    pub(crate) fn push_deferred(&mut self, vk: VkCode, needs_shift: bool) {
        if let Some(send) = self.current_send_mut() {
            send.deferred_vks.push(DeferredVk { vk, needs_shift });
        } else {
            log::warn!(
                "[gji-warmup] cold={} push_deferred dropped: phase has no SendState (label={})",
                self.cold_seq,
                self.phase_label_internal()
            );
        }
    }

    /// probe 進行中に後続 VK を複数蓄積する。
    #[allow(dead_code)]
    pub(crate) fn extend_deferred(&mut self, vks: impl IntoIterator<Item = DeferredVk>) {
        let collected: Vec<DeferredVk> = vks.into_iter().collect();
        if let Some(send) = self.current_send_mut() {
            send.deferred_vks.extend(collected);
        } else {
            log::warn!(
                "[gji-warmup] cold={} extend_deferred dropped {} VK(s): phase has no SendState (label={})",
                self.cold_seq,
                collected.len(),
                self.phase_label_internal()
            );
        }
    }

    /// TIMER_TSF_PROBE ハンドラから 10ms ごとに呼ぶ。
    pub(crate) fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        self.tick_impl(&SystemClock, env)
    }

    /// `Clock` と `TsfEnvSnapshot` を注入できる `tick` の内部実装（テスト用）。
    #[cfg(test)]
    pub(crate) fn tick_with_clock_env<C: Clock>(
        &mut self,
        clock: &C,
        env: &TsfEnvSnapshot,
    ) -> Vec<ProbeAction> {
        self.tick_impl(clock, env)
    }

    fn tick_impl<C: Clock>(&mut self, clock: &C, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        match self.inspect_phase(clock, env) {
            NextStep::Wait => vec![],
            NextStep::EmitSendFreshF2 { probe_settled } => {
                let send = self.take_send_for_fresh_f2();
                self.phase = GjiProbePhase::WaitingForCallback(WaitingFor::FreshF2Sent {
                    probe_settled,
                    budget_ms: self.ctx.ncwait_budget_ms,
                    send,
                });
                vec![ProbeAction::SendFreshF2 {
                    cold_seq: self.cold_seq,
                    probe_settled,
                }]
            }
            NextStep::TransmitTsf { nc_fired, gji_resumed } => {
                self.enter_transmit_tsf(nc_fired, gji_resumed, env)
            }
            NextStep::StartSecondaryProbe { fresh_f2_ms } => {
                let send = self.take_send_for_secondary_probe();
                let probe = TsfReadinessProbe::new(fresh_f2_ms, self.cold_seq, 0);
                self.phase = GjiProbePhase::Probing {
                    probe,
                    total_max_ms: crate::tuning::GJI_POST_NAMECHANGE_MS,
                    kind: GjiProbeKind::Secondary,
                    send,
                };
                vec![]
            }
        }
    }

    /// 現フェーズを検査して次の遷移先を返す。副作用なし（`&self` のみ使用）。
    fn inspect_phase<C: Clock>(&self, clock: &C, env: &TsfEnvSnapshot) -> NextStep {
        match &self.phase {
            GjiProbePhase::Probing {
                probe,
                total_max_ms,
                kind,
                ..
            } => {
                let Some(outcome) = probe.check_outcome(*total_max_ms) else {
                    return NextStep::Wait;
                };
                match kind {
                    GjiProbeKind::Initial {
                        needs_settle_check,
                        cold_reason,
                    } => {
                        log::debug!(
                            "[gji-warmup] cold={} GjiProbe 完了 ({}ms, gji_idle={}ms, settled={})",
                            self.cold_seq,
                            outcome.elapsed_ms,
                            outcome.gji_idle_ms,
                            outcome.settled,
                        );
                        if *needs_settle_check {
                            let is_ime_init_cold = cold_reason.requires_settle();
                            if (!outcome.settled || is_ime_init_cold) && outcome.monitor_healthy {
                                let pre_idle = outcome.gji_idle_ms
                                    >= outcome
                                        .elapsed_ms
                                        .saturating_add(crate::tuning::GJI_IDLE_MS);
                                if !outcome.settled
                                    && outcome.monitor_healthy
                                    && pre_idle
                                    && self.ctx.forces_prepend_f2
                                {
                                    log::debug!(
                                        "[gji-warmup] cold={} GJI pre-idle (idle={}ms elapsed={}ms forces_f2=true) → skip fresh F2",
                                        self.cold_seq,
                                        outcome.gji_idle_ms,
                                        outcome.elapsed_ms,
                                    );
                                    return NextStep::TransmitTsf {
                                        nc_fired: false,
                                        gji_resumed: false,
                                    };
                                }
                                return NextStep::EmitSendFreshF2 {
                                    probe_settled: outcome.settled,
                                };
                            }
                        }
                        NextStep::TransmitTsf {
                            nc_fired: true,
                            gji_resumed: false,
                        }
                    }
                    GjiProbeKind::Secondary => {
                        log::debug!(
                            "[gji-warmup] cold={} SecondaryGjiProbe 完了 ({}ms)",
                            self.cold_seq,
                            outcome.elapsed_ms
                        );
                        NextStep::TransmitTsf {
                            nc_fired: true,
                            gji_resumed: false,
                        }
                    }
                }
            }

            GjiProbePhase::WaitingForCallback(_) => NextStep::Wait,

            GjiProbePhase::NameChangeWait {
                nc_baseline,
                deadline_ms,
                fresh_f2_ms,
                probe_settled,
                gji_write_baseline,
                ..
            } => {
                let now = clock.now_ms();

                // candidate 窓が表示中 = composition active → OBJ_NAMECHANGE を待たず即 transmit。
                if env.gji_candidate_visible {
                    let elapsed = now.saturating_sub(*fresh_f2_ms);
                    log::debug!(
                        "[gji-warmup] cold={} NameChangeWait: candidate visible → composition active ({elapsed}ms)",
                        self.cold_seq
                    );
                    return NextStep::TransmitTsf {
                        nc_fired: false,
                        gji_resumed: false,
                    };
                }

                let nc_fired = nc_baseline.fired();
                let timed_out = now >= *deadline_ms;
                if !nc_fired && !timed_out {
                    return NextStep::Wait;
                }
                let elapsed = now.saturating_sub(*fresh_f2_ms);
                // fresh F2 送信後に GJI が I/O 書き込みを行ったか確認する。
                // 書き込みあり → GJI が F2 に応答して composition を開始した可能性が高い
                // (gji_resumed=true)。書き込みなし → TSF が cold のまま (gji_resumed=false)。
                let gji_wrote_after_f2 =
                    crate::tsf::observer::gji_last_write_ms() > *gji_write_baseline;
                log::debug!(
                    "[gji-warmup] cold={} NameChangeWait → nc_fired={nc_fired} timed_out={timed_out} gji_wrote_after_f2={gji_wrote_after_f2} ({elapsed}ms)",
                    self.cold_seq
                );
                if nc_fired && !probe_settled {
                    log::debug!(
                        "[gji-warmup] cold={} OBJ_NAMECHANGE後 GJI 二次プローブ (max {}ms)",
                        self.cold_seq,
                        crate::tuning::GJI_POST_NAMECHANGE_MS
                    );
                    NextStep::StartSecondaryProbe {
                        fresh_f2_ms: *fresh_f2_ms,
                    }
                } else {
                    NextStep::TransmitTsf {
                        nc_fired,
                        gji_resumed: gji_wrote_after_f2,
                    }
                }
            }

            GjiProbePhase::Done => NextStep::Wait,
        }
    }

    /// dispatcher が `SendFreshF2` を実行した後に呼ぶ。
    /// `WaitingForCallback(FreshF2Sent { .. })` → `NameChangeWait` へ遷移する。
    pub(crate) fn apply_fresh_f2_sent(
        &mut self,
        nc_baseline: NamechangeBaseline,
        fresh_f2_ms: u64,
    ) {
        // TSF モードで transmit バッチに F2 を再送すると reinit レースで先頭 VK がリテラル化する。
        // EmitSendFreshF2 で F2 送信済みとしてフラグを立て、enter_transmit_tsf で抑制する。
        self.ctx.fresh_f2_sent = true;
        let phase = std::mem::replace(
            &mut self.phase,
            GjiProbePhase::WaitingForCallback(WaitingFor::TransmitDone),
        );
        let (probe_settled, budget_ms, send) = match phase {
            GjiProbePhase::WaitingForCallback(WaitingFor::FreshF2Sent {
                probe_settled,
                budget_ms,
                send,
            }) => (probe_settled, budget_ms, send),
            other => {
                log::warn!(
                    "[gji-warmup] cold={} apply_fresh_f2_sent: unexpected phase",
                    self.cold_seq
                );
                self.phase = other;
                return;
            }
        };
        let deadline_ms = fresh_f2_ms + budget_ms;
        let gji_write_baseline = crate::tsf::observer::gji_last_write_ms();
        log::debug!(
            "[gji-warmup] cold={} NameChangeWait deadline {}ms (probe_settled={probe_settled}, budget={budget_ms}ms forces_f2={})",
            self.cold_seq, budget_ms, self.ctx.forces_prepend_f2
        );
        self.phase = GjiProbePhase::NameChangeWait {
            nc_baseline,
            deadline_ms,
            fresh_f2_ms,
            probe_settled,
            gji_write_baseline,
            send,
        };
    }

    /// transmit 実行後に呼ぶ（LiteralDetect 不要の場合）。Done フェーズへ遷移する。
    pub(crate) fn apply_transmit_done_no_literal(&mut self) {
        self.phase = GjiProbePhase::Done;
    }

    /// フェーズ名文字列（内部用）。
    const fn phase_label_internal(&self) -> &'static str {
        match &self.phase {
            GjiProbePhase::Probing { .. } => "Probing",
            GjiProbePhase::WaitingForCallback(WaitingFor::FreshF2Sent { .. }) => {
                "WaitingForCallback(FreshF2Sent)"
            }
            GjiProbePhase::WaitingForCallback(WaitingFor::TransmitDone) => {
                "WaitingForCallback(TransmitDone)"
            }
            GjiProbePhase::NameChangeWait { .. } => "NameChangeWait",
            GjiProbePhase::Done => "Done",
        }
    }

    /// テスト専用: 現在フェーズの文字列ラベルを返す。
    #[cfg(test)]
    pub(crate) fn phase_label(&self) -> &'static str {
        self.phase_label_internal()
    }

    /// テスト専用: 任意のフェーズに強制遷移する。
    #[cfg(test)]
    pub(crate) fn force_phase_for_test(&mut self, phase: GjiProbePhase) {
        self.phase = phase;
    }

    // ── 内部ヘルパー ──────────────────────────────────────────────────────────

    /// 現フェーズの `SendState` への可変参照を返す。`TransmitDone` / `Done` フェーズは `None`。
    fn current_send_mut(&mut self) -> Option<&mut SendState> {
        match &mut self.phase {
            GjiProbePhase::Probing { send, .. }
            | GjiProbePhase::NameChangeWait { send, .. }
            | GjiProbePhase::WaitingForCallback(WaitingFor::FreshF2Sent { send, .. }) => {
                Some(send)
            }
            GjiProbePhase::WaitingForCallback(WaitingFor::TransmitDone)
            | GjiProbePhase::Done => None,
        }
    }

    /// `EmitSendFreshF2` 遷移時に `Probing` フェーズから `SendState` を取り出す。
    fn take_send_for_fresh_f2(&mut self) -> SendState {
        if let GjiProbePhase::Probing { send, .. } = &mut self.phase {
            std::mem::take(send)
        } else {
            log::warn!(
                "[gji-warmup] cold={} take_send_for_fresh_f2 unexpected phase {}",
                self.cold_seq,
                self.phase_label_internal()
            );
            SendState::default()
        }
    }

    /// `StartSecondaryProbe` 遷移時に `NameChangeWait` フェーズから `SendState` を取り出す。
    fn take_send_for_secondary_probe(&mut self) -> SendState {
        if let GjiProbePhase::NameChangeWait { send, .. } = &mut self.phase {
            std::mem::take(send)
        } else {
            log::warn!(
                "[gji-warmup] cold={} take_send_for_secondary_probe unexpected phase {}",
                self.cold_seq,
                self.phase_label_internal()
            );
            SendState::default()
        }
    }

    /// transmit 時に現フェーズから `SendState` を取り出す。
    fn take_current_send_for_transmit(&mut self) -> SendState {
        match &mut self.phase {
            GjiProbePhase::Probing { send, .. }
            | GjiProbePhase::NameChangeWait { send, .. } => std::mem::take(send),
            _ => {
                log::warn!(
                    "[gji-warmup] cold={} enter_transmit_tsf called from unexpected phase {}",
                    self.cold_seq,
                    self.phase_label_internal()
                );
                SendState::default()
            }
        }
    }

    /// transmit フェーズへ進み `ProbeAction` を返す。
    ///
    /// `plan.needs_literal == true` の場合は [`ProbeAction::StartLiteralDetect`]、
    /// それ以外は [`ProbeAction::Transmit`]（Tsf）を emit して Done へ遷移する。
    fn enter_transmit_tsf(
        &mut self,
        nc_fired: bool,
        gji_resumed: bool,
        env: &TsfEnvSnapshot,
    ) -> Vec<ProbeAction> {
        let send = self.take_current_send_for_transmit();
        let cold_seq = self.cold_seq;
        let ctx = self.ctx;

        // ReWarmup / FreshF2 / non-eager パスではプローブ開始前に F2 を送信済み。
        // EmitSendFreshF2 パスでもプローブ中に F2 を送信済み。
        // TSF モード（WezTerm）でバッチに F2 を再送すると reinit が起きて先頭 VK がリテラル化する
        // ("kお" バグ)。いずれかの経路で F2 を送信済みの場合はバッチへの同梱を抑制する。
        let already_sent_f2 = ctx.fresh_f2_at_probe_start || ctx.fresh_f2_sent;
        let suppress_f2_in_batch = already_sent_f2 && env.is_tsf_mode;
        if suppress_f2_in_batch {
            log::debug!(
                "[gji-warmup] cold={} F2 送信済み (probe_start={} sent={}) + TSF → F2 バッチ重複を抑制",
                cold_seq, ctx.fresh_f2_at_probe_start, ctx.fresh_f2_sent
            );
        }
        let effective_prepend_f2 = ctx.prepend_f2_warmup && !suppress_f2_in_batch;
        let observations = ProbeObservations { nc_fired, gji_resumed };
        let plan = decide_transmit_plan(
            effective_prepend_f2,
            ctx.used_eager_path,
            observations,
            env,
            send.deferred_vks.is_empty(),
            ctx.forces_prepend_f2,
            ctx.is_long_cold,
        );

        if plan.needs_literal {
            // LiteralDetect フェーズが必要 → StartLiteralDetect を emit し、Done へ遷移する。
            // LiteralDetect FSM は caller 側で起動する（#15 以降）。
            let literal_detect_ms = plan.literal_detect_ms;
            self.phase = GjiProbePhase::Done;
            vec![ProbeAction::StartLiteralDetect(LiteralDetectConfig {
                cold_seq,
                romaji: send.romaji,
                deferred_vks: send.deferred_vks,
                plan,
                observations,
                literal_detect_ms,
            })]
        } else {
            self.phase = GjiProbePhase::WaitingForCallback(WaitingFor::TransmitDone);
            vec![ProbeAction::Transmit {
                cold_seq,
                plan,
                observations,
                romaji: send.romaji,
                deferred_vks: send.deferred_vks,
                target: TransmitTarget::Tsf,
            }]
        }
    }
}

impl TickableFsm for GjiWarmupFsm {
    fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        GjiWarmupFsm::tick(self, env)
    }

    fn cold_seq_hint(&self) -> u32 {
        GjiWarmupFsm::cold_seq_hint(self)
    }

    fn forces_prepend_f2_for_extra_f2(&self) -> bool {
        GjiWarmupFsm::forces_prepend_f2_for_extra_f2(self)
    }

    fn apply_fresh_f2_sent(&mut self, nc_baseline: NamechangeBaseline, fresh_f2_ms: u64) {
        GjiWarmupFsm::apply_fresh_f2_sent(self, nc_baseline, fresh_f2_ms);
    }

    fn apply_transmit_done(
        &mut self,
        _romaji: String,
        _ze_bs_count: usize,
        _detector: Option<crate::tsf::probe::LiteralDetector>,
        _literal_detect_ms: u64,
        _expected_kana: Option<char>,
    ) -> bool {
        self.apply_transmit_done_no_literal();
        true
    }

    fn push_deferred(&mut self, vk: VkCode, needs_shift: bool) {
        GjiWarmupFsm::push_deferred(self, vk, needs_shift);
    }
}

// ── テスト ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use timed_fsm::ManualClock;

    fn make_fsm() -> GjiWarmupFsm {
        make_fsm_with_cold(crate::tuning::SETTLE_TIMEOUT_MS, false)
    }

    fn make_fsm_with_cold(ncwait_budget_ms: u64, forces_prepend_f2: bool) -> GjiWarmupFsm {
        let is_long_cold = ncwait_budget_ms == crate::tuning::GJI_LONG_IDLE_PROBE_TOTAL_MS;
        let probe = TsfReadinessProbe::new(0, 0, 0);
        GjiWarmupFsm::new(
            "ka",
            0,
            probe,
            0,
            false,
            ColdReason::FocusChange,
            false,
            false,
            ncwait_budget_ms,
            forces_prepend_f2,
            is_long_cold,
            false,
        )
    }

    fn make_namechange_wait(deadline_ms: u64, probe_settled: bool) -> GjiProbePhase {
        make_namechange_wait_with_gji_baseline(deadline_ms, probe_settled, 0)
    }

    fn make_namechange_wait_with_gji_baseline(
        deadline_ms: u64,
        probe_settled: bool,
        gji_write_baseline: u64,
    ) -> GjiProbePhase {
        let baseline = crate::tsf::observer::namechange_baseline();
        GjiProbePhase::NameChangeWait {
            nc_baseline: baseline,
            deadline_ms,
            fresh_f2_ms: 0,
            probe_settled,
            gji_write_baseline,
            send: SendState::default(),
        }
    }

    // ── NameChangeWait フェーズ遷移テスト ─────────────────────────────────────

    #[test]
    fn namechange_wait_before_deadline_stays_waiting() {
        let mut fsm = make_fsm();
        fsm.force_phase_for_test(make_namechange_wait(1000, false));

        let actions = fsm.tick_with_clock_env(&ManualClock(500), &TsfEnvSnapshot::default());
        assert!(actions.is_empty(), "待機中は空 Vec を返すべき");
        assert_eq!(fsm.phase_label(), "NameChangeWait");
    }

    #[test]
    fn namechange_wait_candidate_visible_exits_immediately() {
        let mut fsm = make_fsm();
        fsm.force_phase_for_test(make_namechange_wait(10_000, true));

        let env = TsfEnvSnapshot {
            gji_candidate_visible: true,
            is_tsf_mode: true,
            gji_active: true,
            ..Default::default()
        };
        let actions = fsm.tick_with_clock_env(&ManualClock(500), &env);

        assert!(!actions.is_empty(), "candidate visible → action を emit するべき");
        // TSF mode + gji_active + nc_fired=false → needs_literal=true → StartLiteralDetect
        assert!(
            matches!(actions[0], ProbeAction::StartLiteralDetect(_)),
            "candidate visible (TSF mode, gji_active): StartLiteralDetect を emit するべき: {actions:?}",
        );
        if let ProbeAction::StartLiteralDetect(cfg) = &actions[0] {
            assert!(!cfg.observations.nc_fired, "candidate visible 経路: nc_fired=false");
            assert!(
                !cfg.plan.should_prepend_f2,
                "candidate visible 経路: 余分な F2 はバッチに含めない"
            );
            assert!(
                cfg.plan.needs_literal,
                "candidate visible 経路: IME mode 未確認 → LiteralDetect 有効"
            );
        }
    }

    #[test]
    fn namechange_wait_timeout_settled_emits_transmit() {
        let mut fsm = make_fsm();
        fsm.force_phase_for_test(make_namechange_wait(500, true)); // settled=true

        let actions = fsm.tick_with_clock_env(&ManualClock(1000), &TsfEnvSnapshot::default());
        assert!(
            !actions.is_empty(),
            "タイムアウト後は action を emit するべき"
        );
        // non-TSF mode, gji_active=false → needs_literal=false → Transmit
        assert!(
            matches!(actions[0], ProbeAction::Transmit { target: TransmitTarget::Tsf, .. }),
            "タイムアウト時は Transmit(Tsf) を emit するべき: {actions:?}",
        );
    }

    #[test]
    fn namechange_wait_timeout_unsettled_emits_transmit() {
        let mut fsm = make_fsm();
        fsm.force_phase_for_test(make_namechange_wait(500, false));

        let actions = fsm.tick_with_clock_env(&ManualClock(1000), &TsfEnvSnapshot::default());
        assert!(
            matches!(actions[0], ProbeAction::Transmit { target: TransmitTarget::Tsf, .. }),
            "タイムアウト(unsettled)でも Transmit(Tsf) を emit するべき: {actions:?}",
        );
        if let ProbeAction::Transmit { observations, .. } = &actions[0] {
            assert!(!observations.nc_fired, "タイムアウトなので nc_fired=false が必須");
        }
    }

    #[test]
    fn apply_fresh_f2_sent_medium_cold_uses_extended_budget() {
        let baseline = crate::tsf::observer::namechange_baseline();
        let mut fsm = make_fsm_with_cold(crate::tuning::MEDIUM_IDLE_PROBE_TOTAL_MS, true);
        fsm.force_phase_for_test(GjiProbePhase::WaitingForCallback(WaitingFor::FreshF2Sent {
            probe_settled: false,
            budget_ms: crate::tuning::MEDIUM_IDLE_PROBE_TOTAL_MS,
            send: SendState::default(),
        }));
        let fresh_f2_ms: u64 = 1000;
        fsm.apply_fresh_f2_sent(baseline, fresh_f2_ms);

        assert_eq!(fsm.phase_label(), "NameChangeWait");
        if let GjiProbePhase::NameChangeWait { deadline_ms, .. } = &fsm.phase {
            let expected_deadline = fresh_f2_ms + crate::tuning::MEDIUM_IDLE_PROBE_TOTAL_MS;
            assert_eq!(
                *deadline_ms,
                expected_deadline,
                "medium cold タイムアウト = MEDIUM_IDLE_PROBE_TOTAL_MS ({}ms) が必須",
                crate::tuning::MEDIUM_IDLE_PROBE_TOTAL_MS
            );
        } else {
            panic!("NameChangeWait フェーズになるべき: {:?}", fsm.phase_label());
        }
    }

    #[test]
    fn apply_fresh_f2_sent_long_cold_uses_long_probe_timeout() {
        let baseline = crate::tsf::observer::namechange_baseline();
        let mut fsm = make_fsm_with_cold(crate::tuning::GJI_LONG_IDLE_PROBE_TOTAL_MS, true);
        fsm.force_phase_for_test(GjiProbePhase::WaitingForCallback(WaitingFor::FreshF2Sent {
            probe_settled: false,
            budget_ms: crate::tuning::GJI_LONG_IDLE_PROBE_TOTAL_MS,
            send: SendState::default(),
        }));
        let fresh_f2_ms: u64 = 1000;
        fsm.apply_fresh_f2_sent(baseline, fresh_f2_ms);

        if let GjiProbePhase::NameChangeWait { deadline_ms, .. } = &fsm.phase {
            let expected_deadline = fresh_f2_ms + crate::tuning::GJI_LONG_IDLE_PROBE_TOTAL_MS;
            assert_eq!(
                *deadline_ms,
                expected_deadline,
                "long cold タイムアウト = GJI_LONG_IDLE_PROBE_TOTAL_MS ({}ms) が必須",
                crate::tuning::GJI_LONG_IDLE_PROBE_TOTAL_MS
            );
        } else {
            panic!("NameChangeWait フェーズになるべき: {:?}", fsm.phase_label());
        }
    }

    #[test]
    fn waiting_for_callback_is_no_op() {
        let mut fsm = make_fsm();
        fsm.force_phase_for_test(GjiProbePhase::WaitingForCallback(WaitingFor::TransmitDone));

        let actions = fsm.tick_with_clock_env(&ManualClock(0), &TsfEnvSnapshot::default());
        assert!(actions.is_empty(), "WaitingForCallback は空 Vec を返すべき");
        assert_eq!(fsm.phase_label(), "WaitingForCallback(TransmitDone)");
    }

    #[test]
    fn push_deferred_does_not_panic() {
        let mut fsm = make_fsm();
        fsm.push_deferred(VkCode(0x41), false);
        fsm.push_deferred(VkCode(0x42), true);
    }

    #[test]
    fn extend_deferred_does_not_panic() {
        let mut fsm = make_fsm();
        fsm.extend_deferred(vec![
            DeferredVk {
                vk: VkCode(0x41),
                needs_shift: false,
            },
            DeferredVk {
                vk: VkCode(0x42),
                needs_shift: true,
            },
        ]);
    }

    // FSM 統合: Medium cold NameChangeWait タイムアウト → F2 バッチ同梱
    #[test]
    fn fsm_ncwait_medium_cold_timeout_emits_f2_in_batch() {
        let mut fsm = {
            let probe = TsfReadinessProbe::new(0, 0, 0);
            GjiWarmupFsm::new(
                "ka",
                0,
                probe,
                0,
                false,
                ColdReason::FocusChange,
                true,  // prepend_f2_warmup=true
                false,
                crate::tuning::MEDIUM_IDLE_PROBE_TOTAL_MS,
                true,  // forces_prepend_f2
                false, // is_long_cold: Medium は false
                false,
            )
        };
        fsm.force_phase_for_test(make_namechange_wait(0, false));

        let env = TsfEnvSnapshot {
            is_tsf_mode: true,
            gji_active: true,
            ..Default::default()
        };
        let actions = fsm.tick_with_clock_env(&ManualClock(1000), &env);

        // Medium cold + TSF mode + gji_active → needs_literal=true → StartLiteralDetect
        if let ProbeAction::StartLiteralDetect(cfg) = &actions[0] {
            assert!(
                cfg.plan.should_prepend_f2,
                "Medium cold + nc_fired=false: forces_prepend_f2=true → F2 をバッチに含める"
            );
            assert!(cfg.plan.needs_literal, "GJI 未応答: LiteralDetect 有効");
            assert!(!cfg.observations.nc_fired, "タイムアウト: nc_fired=false");
        } else {
            panic!(
                "StartLiteralDetect を emit するべき (Medium cold, TSF mode): {actions:?}"
            );
        }
    }

    #[test]
    fn fsm_ncwait_short_idle_timeout_skips_f2_emits_literal_detect() {
        let mut fsm = {
            let probe = TsfReadinessProbe::new(0, 0, 0);
            GjiWarmupFsm::new(
                "sa",
                0,
                probe,
                0,
                false,
                ColdReason::FocusChange,
                true,
                false,
                crate::tuning::SETTLE_TIMEOUT_MS,
                false, // forces_prepend_f2=false (Short cold)
                false,
                false,
            )
        };
        fsm.force_phase_for_test(make_namechange_wait(0, false));

        let env = TsfEnvSnapshot {
            is_tsf_mode: true,
            gji_active: true,
            ..Default::default()
        };
        let actions = fsm.tick_with_clock_env(&ManualClock(1000), &env);

        // Short cold + TSF + gji_active → needs_literal=true → StartLiteralDetect
        if let ProbeAction::StartLiteralDetect(cfg) = &actions[0] {
            assert!(
                !cfg.plan.should_prepend_f2,
                "Short cold (forces_prepend_f2=false) + nc_fired=false: F2 をバッチに含めない"
            );
            assert!(cfg.plan.needs_literal, "TSF + gji_active: LiteralDetect 有効");
        } else {
            panic!(
                "StartLiteralDetect を emit するべき (Short cold, TSF mode): {actions:?}"
            );
        }
    }

    #[test]
    fn fsm_ncwait_timeout_non_tsf_no_gji_emits_transmit() {
        let mut fsm = make_fsm();
        fsm.force_phase_for_test(make_namechange_wait(0, false));

        let env = TsfEnvSnapshot {
            is_tsf_mode: false,
            gji_active: false,
            ..Default::default()
        };
        let actions = fsm.tick_with_clock_env(&ManualClock(1000), &env);

        // non-TSF mode, gji_active=false → needs_literal=false → Transmit
        assert!(
            matches!(actions[0], ProbeAction::Transmit { target: TransmitTarget::Tsf, .. }),
            "non-TSF mode + gji_active=false → Transmit(Tsf): {actions:?}",
        );
    }
}
