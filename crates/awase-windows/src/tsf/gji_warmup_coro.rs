//! GJI cold-start warmup コルーチン実装。
//!
//! [`GjiWarmupCoro`] は [`GjiWarmupFsm`] + [`LiteralDetectFsm`] を `StepCoro` で置き換えた。
//! フェーズ遷移が単一の async 関数本体に直線的に記述されており、
//! `StartLiteralDetect` → `SwitchMachine` → `LiteralDetectFsm` の機械切り替えが不要。
//!
//! ## フェーズ遷移（コルーチン本体）
//!
//! ```text
//! probe ループ
//!   ├─[settle 不要 / 完了]─► transmit へ
//!   └─[settle 必要]──────► [SendFreshF2] → NameChangeWait
//!                               └─[nc + !settled]─► SecondaryProbe ──┐
//!                               └─[その他]──────────────────────────► transmit へ
//!
//! transmit
//!   ├─[needs_literal=false]──────────────────────────────► Done（single yield）
//!   ├─[long_cold + is_tsf_mode]──► [StartSacrificialWarmup] → SacrificialWarmupFsm
//!   └─[その他 needs_literal]──► [Transmit { needs_literal }] → inline LiteralDetect
//!         LiteralDetect ループ
//!           ├─[composition 確認]─► Done
//!           ├─[partial literal]───► [SendRecoveryBs + StartSacrificialWarmup + Done]
//!           └─[suspected literal]─► [RawTsfLiteralRecovery + Done]
//! ```

use std::rc::Rc;

use crate::tsf::observer::NamechangeBaseline;
use crate::tsf::output::ColdReason;
use crate::tsf::probe::{LiteralDetector, TsfReadinessProbe};
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::probe_fsm::{
    decide_transmit_plan, LiteralDetectConfig, ProbeAction, ProbeObservations,
    TransmitPlan, TransmitTarget, TsfEnvSnapshot,
};
use timed_fsm::coro::{yield_step, Channel, CoroStep, StepCoro};
use crate::tsf::tickable_fsm::TickableFsm;

// ── FreshF2Callback ──────────────────────────────────────────────────────────

struct FreshF2Callback {
    nc_baseline: NamechangeBaseline,
    fresh_f2_ms: u64,
    /// FreshF2 送信直前の `gji_last_write_ms()` スナップショット（NameChangeWait 判定用）。
    gji_write_baseline: u64,
}

// ── TransmitDonePayload ──────────────────────────────────────────────────────

/// `apply_transmit_done(Some(detector))` のペイロード。
/// 次 tick の TickInput に載り、コルーチン本体が inline LiteralDetect フェーズに入る。
struct TransmitDonePayload {
    romaji: String,
    ze_bs_count: usize,
    detector: LiteralDetector,
    /// `apply_transmit_done` 呼び出し時点の `current_tick_ms() + literal_detect_ms`。
    deadline_ms: u64,
}

// ── TickInput ────────────────────────────────────────────────────────────────

struct TickInput {
    env: TsfEnvSnapshot,
    fresh_f2_callback: Option<FreshF2Callback>,
    transmit_done: Option<TransmitDonePayload>,
}

// ── GjiProbeCtx ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct GjiProbeCtx {
    cold_seq: u32,
    prepend_f2_warmup: bool,
    used_eager_path: bool,
    ncwait_budget_ms: u64,
    forces_prepend_f2: bool,
    is_long_cold: bool,
    fresh_f2_at_probe_start: bool,
    consecutive: u32,
}

// ── コルーチン本体 ────────────────────────────────────────────────────────────

/// TSF path での部分リテラル判定に使う BS 数（LiteralDetectFsm と同値）。
const PARTIAL_LITERAL_BS: usize = 2;

async fn gji_coro_body(
    ch: Rc<Channel<TickInput, Vec<ProbeAction>>>,
    romaji: String,
    probe: TsfReadinessProbe,
    total_max_ms: u64,
    needs_settle_check: bool,
    cold_reason: ColdReason,
    ctx: GjiProbeCtx,
) {
    // env は 'initial ループの最初の yield で必ず設定される（loop は常に 1 回以上実行）。
    let mut env: TsfEnvSnapshot;
    let mut fresh_f2_sent = false;

    // ── Phase 1 / 2 / 3: GJI probe ──────────────────────────────────────────
    let (nc_fired, gji_resumed) = 'initial: loop {
        // 最初の step() で input は消費されないため、env は 2 tick 目から更新される。
        // 10ms タイマー駆動の 1 tick ズレは動作に影響しない
        // （`GjiWarmupCoro::new` が construction 時に self-priming tick を行うため、
        //   外部から見た最初の tick は既にこの「捨てられる」1回を消費済みになっている）。
        let input = yield_step(ch.clone(), vec![]).await;
        env = input.env;

        let Some(outcome) = probe.check_outcome(total_max_ms) else {
            continue;
        };

        log::debug!(
            "[gji-coro] cold={} GjiProbe 完了 ({}ms gji_idle={}ms settled={})",
            ctx.cold_seq,
            outcome.elapsed_ms,
            outcome.gji_idle_ms,
            outcome.settled,
        );

        if needs_settle_check {
            let is_ime_init_cold = cold_reason.requires_settle();
            if (!outcome.settled || is_ime_init_cold) && outcome.monitor_healthy {
                // pre-idle: F2 送信前から GJI が長期静止 + forces_prepend_f2
                // → F2 を送っても GJI は応答しないので FreshF2 をスキップして transmit へ
                let pre_idle = outcome.gji_idle_ms
                    >= outcome.elapsed_ms.saturating_add(crate::tuning::GJI_IDLE_MS);
                if !outcome.settled && pre_idle && ctx.forces_prepend_f2 {
                    log::debug!(
                        "[gji-coro] cold={} GJI pre-idle (idle={}ms elapsed={}ms) → skip FreshF2",
                        ctx.cold_seq,
                        outcome.gji_idle_ms,
                        outcome.elapsed_ms,
                    );
                    break 'initial (false, false);
                }

                // ── Phase 2: SendFreshF2 ─────────────────────────────────────
                let f2_input = yield_step(
                    ch.clone(),
                    vec![ProbeAction::SendFreshF2 {
                        cold_seq: ctx.cold_seq,
                        probe_settled: outcome.settled,
                    }],
                )
                .await;
                fresh_f2_sent = true;

                let cb = f2_input
                    .fresh_f2_callback
                    .expect("[gji-coro] FreshF2 tick に fresh_f2_callback が設定されていません");

                let deadline_ms = cb.fresh_f2_ms + ctx.ncwait_budget_ms;
                let probe_settled = outcome.settled;

                // ── Phase 3: NameChangeWait ──────────────────────────────────
                let result = 'ncwait: loop {
                    let nc_input = yield_step(ch.clone(), vec![]).await;
                    env = nc_input.env;

                    if env.gji_candidate_visible {
                        log::debug!(
                            "[gji-coro] cold={} NameChangeWait: candidate visible → 即 transmit",
                            ctx.cold_seq
                        );
                        break 'ncwait (false, false);
                    }

                    let nc_fired_now = cb.nc_baseline.fired();
                    let timed_out = crate::hook::current_tick_ms() >= deadline_ms;
                    if !nc_fired_now && !timed_out {
                        continue;
                    }

                    let gji_wrote_after_f2 =
                        crate::tsf::observer::gji_last_write_ms() > cb.gji_write_baseline;
                    log::debug!(
                        "[gji-coro] cold={} NameChangeWait 完了: nc={nc_fired_now} timeout={timed_out} gji_after_f2={gji_wrote_after_f2}",
                        ctx.cold_seq
                    );

                    // OBJ_NAMECHANGE 後 + unsettled → 二次 GJI probe
                    if nc_fired_now && !probe_settled {
                        log::debug!(
                            "[gji-coro] cold={} OBJ_NAMECHANGE 後 二次 GJI probe (max {}ms)",
                            ctx.cold_seq,
                            crate::tuning::GJI_POST_NAMECHANGE_MS,
                        );
                        let secondary = TsfReadinessProbe::new(cb.fresh_f2_ms, ctx.cold_seq, 0);
                        loop {
                            let sp_input = yield_step(ch.clone(), vec![]).await;
                            env = sp_input.env;
                            if secondary.check_outcome(crate::tuning::GJI_POST_NAMECHANGE_MS).is_some() {
                                log::debug!(
                                    "[gji-coro] cold={} SecondaryGjiProbe 完了",
                                    ctx.cold_seq
                                );
                                break;
                            }
                        }
                        break 'ncwait (true, false);
                    }

                    break 'ncwait (nc_fired_now, gji_wrote_after_f2);
                };
                break 'initial result;
            }
        }

        break 'initial (true, false);
    };

    // ── Phase 4: transmit plan 決定 ───────────────────────────────────────────
    // ReinjectConfirmKey/PassthroughConfirmKey + TSF mode（WezTerm等）:
    // Enter/Space 後に WezTerm は NameChange を発火しないが GJI は VKs を正常にコンポジション中。
    // nc_fired=false のまま decide_transmit_plan に渡すと needs_literal 第2項が true になり
    // LiteralDetect が誤検出して backspace を送る（composited 'な' が消えるバグ）。
    // → is_confirm_key && TSF mode の場合は nc_fired を true に昇格して第2項を抑制する。
    let nc_for_plan = nc_fired
        || (cold_reason.is_confirm_key() && env.is_tsf_mode && !gji_resumed);
    let observations = ProbeObservations { nc_fired: nc_for_plan, gji_resumed };

    // fresh F2 を送信済みかつ TSF mode → バッチへの F2 再同梱を抑制（"kお" バグ防止）
    let already_sent_f2 = ctx.fresh_f2_at_probe_start || fresh_f2_sent;
    let suppress_f2 = already_sent_f2 && env.is_tsf_mode;
    let effective_prepend_f2 = ctx.prepend_f2_warmup && !suppress_f2;

    let plan = decide_transmit_plan(
        effective_prepend_f2,
        ctx.used_eager_path,
        observations,
        &env,
        !env.deferred_pending,
        ctx.forces_prepend_f2,
        ctx.is_long_cold,
    );

    // ── Phase 5a: long cold + TSF → StartSacrificialWarmup（SwitchMachine） ────
    if plan.needs_literal && ctx.is_long_cold && env.is_tsf_mode {
        log::debug!(
            "[gji-coro] cold={} long_cold + TSF → StartSacrificialWarmup",
            ctx.cold_seq
        );
        yield_step(
            ch.clone(),
            vec![ProbeAction::StartSacrificialWarmup(LiteralDetectConfig {
                cold_seq: ctx.cold_seq,
                romaji,
                plan,
                observations,
                literal_detect_ms: plan.literal_detect_ms,
                target: TransmitTarget::Tsf,
                from_literal_recovery: false,
            })],
        )
        .await;
        return; // SacrificialWarmupFsm が SwitchMachine で引き継ぐ → このコルーチンは破棄
    }

    // ── Phase 5b: Transmit ───────────────────────────────────────────────────
    // needs_literal=false: dispatcher が apply_transmit_done(None) → true → Done
    // needs_literal=true:  dispatcher が apply_transmit_done(Some(det)) → false → Continue
    let transmit_input = yield_step(
        ch.clone(),
        vec![ProbeAction::Transmit {
            cold_seq: ctx.cold_seq,
            plan,
            observations,
            romaji: romaji.clone(),
            target: TransmitTarget::Tsf,
        }],
    )
    .await;

    if !plan.needs_literal {
        return;
    }

    // ── Phase 6: Inline LiteralDetect ────────────────────────────────────────
    let Some(td) = transmit_input.transmit_done else {
        return;
    };

    let detector = td.detector;
    let deadline_ms = td.deadline_ms;
    let ze_bs_count = td.ze_bs_count;
    let recovery_romaji = td.romaji;

    loop {
        use crate::tsf::probe::DetectionResult;

        let detect_input = yield_step(ch.clone(), vec![]).await;
        env = detect_input.env;

        let Some(detection) = detector.check_now(deadline_ms) else {
            continue;
        };

        // CompositionConfirmed でも 2 文字以上の TSF mode で nc/gji_resumed なし → partial literal
        let partial_literal = matches!(detection, DetectionResult::CompositionConfirmed)
            && !observations.nc_fired
            && !observations.gji_resumed
            && env.is_tsf_mode
            && recovery_romaji.chars().count() >= 2;

        let final_actions = if matches!(detection, DetectionResult::SuspectedLiteral)
            || partial_literal
        {
            let backs = if partial_literal { PARTIAL_LITERAL_BS } else { ze_bs_count };
            let label = if partial_literal { "partial-literal" } else { "suspected" };
            log::debug!(
                "[gji-coro] cold={} LiteralDetect: {label} (backs={backs} consecutive={})",
                ctx.cold_seq,
                ctx.consecutive,
            );
            crate::ime_diagnostic::log_composition_probe(ctx.cold_seq, label);

            emit_literal_recovery_actions(
                ctx.cold_seq,
                &recovery_romaji,
                backs,
                observations,
                ctx.consecutive,
                &env,
            )
        } else {
            log::debug!(
                "[gji-coro] cold={} LiteralDetect: composition confirmed",
                ctx.cold_seq
            );
            crate::ime_diagnostic::log_composition_probe(ctx.cold_seq, "confirmed");
            vec![ProbeAction::Done]
        };

        yield_step(ch.clone(), final_actions).await;
        return;
    }
}

fn emit_literal_recovery_actions(
    cold_seq: u32,
    romaji: &str,
    backs: usize,
    observations: ProbeObservations,
    consecutive: u32,
    env: &TsfEnvSnapshot,
) -> Vec<ProbeAction> {
    if env.is_tsf_mode && consecutive == 0 {
        vec![
            ProbeAction::SendRecoveryBs { cold_seq, backs },
            ProbeAction::StartSacrificialWarmup(LiteralDetectConfig {
                cold_seq,
                romaji: romaji.to_string(),
                plan: TransmitPlan {
                    should_prepend_f2: false,
                    used_eager_path: false,
                    needs_literal: true,
                    literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                },
                observations,
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                target: TransmitTarget::Tsf,
                from_literal_recovery: true,
            }),
            ProbeAction::Done,
        ]
    } else {
        vec![
            ProbeAction::RawTsfLiteralRecovery {
                cold_seq,
                backs,
                romaji: romaji.to_string(),
            },
            ProbeAction::Done,
        ]
    }
}

// ── GjiWarmupCoro ─────────────────────────────────────────────────────────────

/// GJI cold-start warmup コルーチン。`GjiWarmupFsm` + `LiteralDetectFsm` の後継。
///
/// [`TickableFsm`] を実装し `pending_tsf` に格納される。
/// `tick()` ごとに `StepCoro::step()` を呼び、コルーチン本体を 1 yield 点進める。
///
/// ## OutputActiveGuard の管理
///
/// `gji_probe_guard`（`Output` が保持）は probe 開始時から Done まで active を維持する。
/// inline LiteralDetect に入ったとき `_literal_detect_guard` を追加で活性化する。
/// `OutputActiveGuard` は参照カウント (`depth`) 方式なので 2 つのガードが重複しても安全。
pub(crate) struct GjiWarmupCoro {
    coro: StepCoro<TickInput, Vec<ProbeAction>>,
    pending_fresh_f2: Option<FreshF2Callback>,
    pending_transmit_done: Option<TransmitDonePayload>,
    cold_seq: u32,
    forces_prepend_f2: bool,
    /// inline LiteralDetect フェーズ中に OUTPUT_GATE を active に保つ追加ガード。
    _literal_detect_guard: Option<OutputActiveGuard>,
}

impl GjiWarmupCoro {
    /// `GjiWarmupFsm::new` と同等のシグネチャ。`consecutive` は LiteralDetect 判定用。
    #[expect(clippy::too_many_arguments)]
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
        consecutive: u32,
    ) -> Self {
        let ctx = GjiProbeCtx {
            cold_seq,
            prepend_f2_warmup,
            used_eager_path,
            ncwait_budget_ms,
            forces_prepend_f2,
            is_long_cold,
            fresh_f2_at_probe_start,
            consecutive,
        };
        let romaji = romaji.to_string();
        let coro = StepCoro::new(async move |ch| {
            gji_coro_body(ch, romaji, probe, total_max_ms, needs_settle_check, cold_reason, ctx).await
        });
        let mut this = Self {
            coro,
            pending_fresh_f2: None,
            pending_transmit_done: None,
            cold_seq,
            forces_prepend_f2,
            _literal_detect_guard: None,
        };
        // Self-priming: StepCoro の最初の step() は input を消費しない
        // （timed_fsm::coro のドキュメント参照）。construction 直後・pending_tsf に
        // 格納される前にこの「捨てられる1回」を消費しておくことで、install 後に
        // 外部から届く最初の tick の入力（deferred VK 等）が握り潰されるのを防ぐ。
        // construction 時点では何も届いていないため、捨てても安全。
        let primed = this.tick(&TsfEnvSnapshot::default());
        debug_assert!(
            primed.is_empty(),
            "GjiWarmupCoro self-priming tick は空の ProbeAction を返すはず: {primed:?}"
        );
        this
    }
}

// ── TickableFsm impl ─────────────────────────────────────────────────────────

impl TickableFsm for GjiWarmupCoro {
    fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        let input = TickInput {
            env: *env,
            fresh_f2_callback: self.pending_fresh_f2.take(),
            transmit_done: self.pending_transmit_done.take(),
        };
        match self.coro.step(input) {
            CoroStep::Yielded(actions) => actions,
            CoroStep::Complete => vec![ProbeAction::Done],
        }
    }

    fn cold_seq_hint(&self) -> u32 {
        self.cold_seq
    }

    fn forces_prepend_f2_for_extra_f2(&self) -> bool {
        self.forces_prepend_f2
    }

    fn apply_fresh_f2_sent(&mut self, nc_baseline: NamechangeBaseline, fresh_f2_ms: u64) {
        let gji_write_baseline = crate::tsf::observer::gji_last_write_ms();
        self.pending_fresh_f2 = Some(FreshF2Callback {
            nc_baseline,
            fresh_f2_ms,
            gji_write_baseline,
        });
    }

    /// `needs_literal=true` のとき `false`（Continue）を返してコルーチンに LiteralDetect を委譲する。
    ///
    /// `false` → `dispatch_probe_actions` が `DispatchResult::Continue` を返す
    /// → 次 tick の TickInput に `transmit_done` が乗り、コルーチン本体が Phase 6 に入る。
    fn apply_transmit_done(
        &mut self,
        romaji: String,
        ze_bs_count: usize,
        detector: Option<LiteralDetector>,
        literal_detect_ms: u64,
        _expected_kana: Option<char>,
    ) -> bool {
        match detector {
            Some(det) => {
                let deadline_ms = crate::hook::current_tick_ms() + literal_detect_ms;
                self._literal_detect_guard = Some(OutputActiveGuard::begin());
                self.pending_transmit_done = Some(TransmitDonePayload {
                    romaji,
                    ze_bs_count,
                    detector: det,
                    deadline_ms,
                });
                false
            }
            None => true,
        }
    }
}
