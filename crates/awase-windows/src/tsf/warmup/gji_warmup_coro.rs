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
//!         LiteralDetect ループ（warm パスと共有の `LiteralDetectCore` に委譲）
//!           ├─[composition 確認]─► Done
//!           ├─[partial literal]───► [SendRecoveryBs + StartSacrificialWarmup + Done]
//!           └─[suspected literal]─► [RawTsfLiteralRecovery + Done]
//! ```
//!
//! Phase 6 の LiteralDetect 判定は `literal_detect_fsm::LiteralDetectCore` に集約されており、
//! warm パスの `LiteralDetectFsm` と同一ロジックを共有する（重複排除）。

use std::rc::Rc;

use crate::tsf::observer::NamechangeBaseline;
use crate::tsf::output::ColdReason;
use crate::tsf::probe::{LiteralDetector, TsfReadinessProbe};
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::warmup::probe_fsm::{
    decide_transmit_plan, LiteralDetectConfig, ProbeAction, ProbeObservations, TransmitTarget,
    TsfEnvSnapshot,
};
use crate::tsf::warmup::tickable_fsm::TickableFsm;
use timed_fsm::coro::{yield_step, Channel, CoroStep, StepCoro};

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

// `Rc` を使うため生成される future は `!Send`。これはタイマー駆動の単一スレッド設計
// による意図的な制約（crates/timed-fsm/src/coro.rs::yield_step 参照）。
// probe 分岐（Phase 1/2/3・eager/cold・consecutive 判定）が本質的に多いディスパッチャの
// ため複雑度警告も抑制する。
#[expect(clippy::future_not_send)]
#[expect(clippy::cognitive_complexity)]
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

        // probe 完了時の実測値（elapsed/gji_idle/settled）は「予算に対して実際どれだけ
        // かかったか」を事後ログから直接確認するために必要（切り分けログ強化、2026-07-09）。
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
                    >= outcome
                        .elapsed_ms
                        .saturating_add(crate::tuning::GJI_IDLE_MS);
                if !outcome.settled && pre_idle && ctx.forces_prepend_f2 {
                    // このヒューリスティックは「F2 を送っても GJI は応答しない」と判断して
                    // 即 transmit（nc_fired=false, gji_resumed=false）に倒すため、GJI が
                    // 実際には数百ms後に応答するケースでは partial literal の疑い経路
                    // （is_partial_literal）を直接誘発しうる。発火有無を事後ログから
                    // 確認できるようにする（切り分けログ強化、2026-07-09）。
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
                            if secondary
                                .check_outcome(crate::tuning::GJI_POST_NAMECHANGE_MS)
                                .is_some()
                            {
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
    let nc_for_plan = nc_fired || (cold_reason.is_confirm_key() && env.is_tsf_mode && !gji_resumed);
    let observations = ProbeObservations {
        nc_fired: nc_for_plan,
        gji_resumed,
    };

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

    // transmit plan の判定結果（needs_literal になるかどうか、その入力になった
    // nc_fired/gji_resumed/suppress_f2）は従来ログに一切出ておらず、部分リテラル発生時に
    // 「なぜ LiteralDetect に入った/入らなかったか」を事後診断できなかった
    // （切り分けログ強化、2026-07-09 の "kお" 系調査で判明）。
    log::debug!(
        "[gji-coro] cold={} transmit-plan needs_literal={} nc_fired={} gji_resumed={} \
         suppress_f2={suppress_f2} effective_prepend_f2={effective_prepend_f2} is_tsf_mode={}",
        ctx.cold_seq,
        plan.needs_literal,
        observations.nc_fired,
        observations.gji_resumed,
        env.is_tsf_mode,
    );

    // ── Phase 5a: long cold + TSF → StartSacrificialWarmup（SwitchMachine） ────
    // `ctx.is_long_cold` は `ColdKind::is_long()` 由来（cold 突入時点の gji_idle_ms() 実IO
    // 観測から分類済み）で、Chrome 側の自己参照タイマーとは異なり既に実IOに基づいている。
    // そのため `tuning::DIAG_SKIP_PROACTIVE_SACRIFICIAL_WARMUP`（Chrome 専用の診断フラグ、
    // `output/vk_send.rs` 参照）はここには適用しない。
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
        // 診断用（副作用なし）: gji_resumed/nc_fired ヒューリスティックで LiteralDetect を
        // スキップした際、実際に composition が確定したか（部分リテラル false-negative の疑い）を
        // 非同期に事後確認する。gate は保持しない fire-and-forget のため後続入力に影響しない。
        // 参照: 「起動直後、GJIがローマ字をそのまま全角英数として通す」報告の再現条件切り分け。
        let cold_seq = ctx.cold_seq;
        let nc_fired = observations.nc_fired;
        let gji_resumed = observations.gji_resumed;
        let gji_active = env.gji_active;
        let should_prepend_f2 = plan.should_prepend_f2;
        let used_eager_path = plan.used_eager_path;
        win32_async::spawn_local(async move {
            win32_async::sleep_ms(crate::tuning::RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE as u32).await;
            log::debug!(
                "[gji-coro-diag] cold={cold_seq} skip-verify nc_fired={nc_fired} \
                 gji_resumed={gji_resumed} gji_active={gji_active} \
                 should_prepend_f2={should_prepend_f2} used_eager_path={used_eager_path}"
            );
            crate::ime_diagnostic::log_composition_probe(cold_seq, "skip-verify");
        });
        return;
    }

    // ── Phase 6: Inline LiteralDetect（warm パスと共有の LiteralDetectCore） ────
    // literal 検出の判定は warm パス（LiteralDetectFsm）と同一の LiteralDetectCore に
    // 委譲する（ロジックの単一所在地: literal_detect_fsm.rs）。
    let Some(td) = transmit_input.transmit_done else {
        return;
    };

    let mut core = crate::tsf::warmup::literal_detect_fsm::LiteralDetectCore::new(
        ctx.cold_seq,
        td.romaji,
        observations,
        td.detector,
        td.deadline_ms,
        td.ze_bs_count,
        ctx.consecutive,
    );

    loop {
        let detect_input = yield_step(ch.clone(), vec![]).await;
        env = detect_input.env;

        if let Some(final_actions) = core.poll(&env) {
            yield_step(ch.clone(), final_actions).await;
            return;
        }
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
/// inline LiteralDetect に入ったとき `literal_detect_guard` を追加で活性化する。
/// `OutputActiveGuard` は参照カウント (`depth`) 方式なので 2 つのガードが重複しても安全。
pub(crate) struct GjiWarmupCoro {
    coro: StepCoro<TickInput, Vec<ProbeAction>>,
    pending_fresh_f2: Option<FreshF2Callback>,
    pending_transmit_done: Option<TransmitDonePayload>,
    cold_seq: u32,
    forces_prepend_f2: bool,
    /// inline LiteralDetect フェーズ中に OUTPUT_GATE を active に保つ追加ガード。
    literal_detect_guard: Option<OutputActiveGuard>,
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
            gji_coro_body(
                ch,
                romaji,
                probe,
                total_max_ms,
                needs_settle_check,
                cold_reason,
                ctx,
            )
            .await;
        });
        let mut this = Self {
            coro,
            pending_fresh_f2: None,
            pending_transmit_done: None,
            cold_seq,
            forces_prepend_f2,
            literal_detect_guard: None,
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
                self.literal_detect_guard = Some(OutputActiveGuard::begin());
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
