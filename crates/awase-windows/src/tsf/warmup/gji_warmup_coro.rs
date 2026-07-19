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
//!   └─[settle 不要 / 完了]─► transmit へ
//!
//! transmit
//!   ├─[needs_literal=false]──────────────────────────────► Done（single yield）
//!   ├─[needs_literal かつセッション未確認]──► per-VK confirm ループ（Phase 5b、BUG-24 追補）
//!   │     romaji の VK を1つずつ送信し CompositionConfirmed/SuspectedLiteral を都度確認
//!   │       ├─[全 VK 確認]─────────────► セッション確認 (literal_session_confirmed) → Done
//!   │       └─[いずれか SuspectedLiteral]─► [RawTsfLiteralRecovery + Done]
//!   └─[その他 needs_literal]──► [Transmit { needs_literal }] → inline LiteralDetect
//!         LiteralDetect ループ（warm パスと共有の `LiteralDetectCore` に委譲）
//!           ├─[composition 確認]─► Done
//!           └─[partial/suspected literal]─► [RawTsfLiteralRecovery + Done]
//! ```
//!
//! 捨て駒キー（`StartSacrificialWarmup`、VK_A+BS/VK_IME_OFF→ON）は 2026-07-16 に
//! この2経路（per-VK・inline LiteralDetect）双方から撤去し、2026-07-18 に
//! Chrome 側の long-cold 予防的 `StartSacrificialWarmup`（`probe_fsm.rs`）・
//! `SacrificialWarmupCoro`/`ImeOffOnWarmupFsm` 自体も物理削除した。
//! `RawTsfLiteralRecovery` の dispatcher が `consecutive_count()==0` のときだけ
//! romaji 再送を自然に次の cold パス（per-VK confirm）へ委ねる
//! （`literal_detect_fsm::emit_recovery_actions` 参照）。
//!
//! Phase 6 の LiteralDetect 判定は `literal_detect_fsm::LiteralDetectCore` に集約されており、
//! warm パスの `LiteralDetectFsm` と同一ロジックを共有する（重複排除）。

use std::rc::Rc;

use crate::tsf::output::ColdReason;
use crate::tsf::probe::{LiteralDetector, TsfReadinessProbe};
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::warmup::probe_fsm::{
    decide_transmit_plan, run_per_vk_confirm, ProbeAction, ProbeObservations, ProbeTickInput,
    TransmitDonePayload, TransmitTarget, TsfEnvSnapshot, VkSentPayload,
};
use crate::tsf::warmup::tickable_fsm::TickableFsm;
use timed_fsm::coro::{yield_step, Channel, CoroStep, StepCoro};

// ── GjiProbeCtx ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct GjiProbeCtx {
    cold_seq: u32,
    used_eager_path: bool,
    forces_prepend_f2: bool,
    is_long_cold: bool,
    consecutive: u32,
}

// ── コルーチン本体 ────────────────────────────────────────────────────────────

// `Rc` を使うため生成される future は `!Send`。これはタイマー駆動の単一スレッド設計
// による意図的な制約（crates/timed-fsm/src/coro.rs::yield_step 参照）。
#[expect(clippy::future_not_send)]
async fn gji_coro_body(
    ch: Rc<Channel<ProbeTickInput, Vec<ProbeAction>>>,
    romaji: String,
    probe: TsfReadinessProbe,
    total_max_ms: u64,
    cold_reason: ColdReason,
    ctx: GjiProbeCtx,
) {
    // env は 'initial ループの最初の yield で必ず設定される（loop は常に 1 回以上実行）。
    let mut env: TsfEnvSnapshot;

    // ── Phase 1: GJI probe ───────────────────────────────────────────────────
    let nc_fired = 'initial: loop {
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

        let is_ime_init_cold = cold_reason.requires_settle();
        if (!outcome.settled || is_ime_init_cold) && outcome.monitor_healthy {
            // Phase 2 (SendFreshF2) + Phase 3 (NameChangeWait/SecondaryProbe) は
            // DIAG_DISABLE_PROACTIVE_TSF_WARMUP（常時 true）下で無条件に到達不能
            // だったため撤去した。reactive な LiteralDetect のみに委ねる
            // （`docs/known-bugs.md` BUG-24 参照。再度有効化する場合はこのコミット
            // の revert が必要）。
            log::debug!(
                "[gji-coro] cold={} settle 必要 (reason={cold_reason:?} gji_idle_ms={} \
                 probe_elapsed={}ms settled={}) → skip FreshF2, reactive LiteralDetect のみ",
                ctx.cold_seq,
                outcome.gji_idle_ms,
                outcome.elapsed_ms,
                outcome.settled,
            );
            break 'initial false;
        }

        break 'initial true;
    };

    // ── Phase 4: transmit plan 決定 ───────────────────────────────────────────
    // ReinjectConfirmKey/PassthroughConfirmKey + TSF mode（WezTerm等）:
    // Enter/Space 後に WezTerm は NameChange を発火しないが GJI は VKs を正常にコンポジション中。
    // nc_fired=false のまま decide_transmit_plan に渡すと needs_literal 第2項が true になり
    // LiteralDetect が誤検出して backspace を送る（composited 'な' が消えるバグ）。
    // → is_confirm_key && TSF mode の場合は nc_fired を true に昇格して第2項を抑制する。
    let nc_for_plan = nc_fired || (cold_reason.is_confirm_key() && env.is_tsf_mode);
    let observations = ProbeObservations {
        nc_fired: nc_for_plan,
    };

    let plan = decide_transmit_plan(
        ctx.used_eager_path,
        observations,
        env,
        !env.deferred_pending,
        ctx.forces_prepend_f2,
        ctx.is_long_cold,
    );

    // transmit plan の判定結果（needs_literal になるかどうか、その入力になった
    // nc_fired）は従来ログに一切出ておらず、部分リテラル発生時に
    // 「なぜ LiteralDetect に入った/入らなかったか」を事後診断できなかった
    // （切り分けログ強化、2026-07-09 の "kお" 系調査で判明）。
    log::debug!(
        "[gji-coro] cold={} transmit-plan needs_literal={} nc_fired={} is_tsf_mode={}",
        ctx.cold_seq,
        plan.needs_literal,
        observations.nc_fired,
        env.is_tsf_mode,
    );

    // ── Phase 5b (IME セッション最初の1文字専用, BUG-24 追補): per-VK send+confirm ──
    // is_partial_literal() は「今回送った romaji 自身の確認信号」ではなく、送信前に
    // 確定していた無関係な代理指標 nc_fired（別の F2 warmup キーへの応答
    // 有無）で判定しており、cold 直後の最初の1文字はほぼ確実に誤検知する（BUG-24）。
    // このセッションでまだ literal-detect を確認済みでない場合に限り、Chrome 側
    // Phase 2c と共通実装（`run_per_vk_confirm`、2026-07-17 統合）に romaji の VK 単位
    // 送信+確認を委ねる。F2 prepend・unicode eager path は対象外とし（それ以外の経路は
    // 下の通常 Transmit にフォールスルーする）、全 VK 確認できたらこのセッションの残りは
    // literal-detect 自体をスキップする
    // （`tsf::observer::mark_literal_session_confirmed`、`literal_detect_fsm.rs` 参照）。
    if plan.needs_literal
        && !crate::tsf::observer::literal_session_confirmed()
        && !plan.used_eager_path
        && env.is_tsf_mode
    {
        run_per_vk_confirm(ch.clone(), ctx.cold_seq, &romaji, plan, TransmitTarget::Tsf).await;
        return;
    }

    // ── Phase 5b: Transmit ───────────────────────────────────────────────────
    // needs_literal=false: dispatcher が apply_transmit_done(None) → true → Done
    // needs_literal=true:  dispatcher が apply_transmit_done(Some(det)) → false → Continue
    let transmit_input = yield_step(
        ch.clone(),
        vec![ProbeAction::Transmit {
            cold_seq: ctx.cold_seq,
            plan,
            romaji: romaji.clone(),
            target: TransmitTarget::Tsf,
        }],
    )
    .await;

    if !plan.needs_literal {
        // 診断用（副作用なし）: nc_fired ヒューリスティックで LiteralDetect を
        // スキップした際、実際に composition が確定したか（部分リテラル false-negative の疑い）を
        // 非同期に事後確認する。gate は保持しない fire-and-forget のため後続入力に影響しない。
        // 参照: 「起動直後、GJIがローマ字をそのまま全角英数として通す」報告の再現条件切り分け。
        let cold_seq = ctx.cold_seq;
        let nc_fired = observations.nc_fired;
        let gji_active = env.gji_active;
        let used_eager_path = plan.used_eager_path;
        win32_async::spawn_local(async move {
            win32_async::sleep_ms(crate::tuning::RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE as u32).await;
            log::debug!(
                "[gji-coro-diag] cold={cold_seq} skip-verify nc_fired={nc_fired} \
                 gji_active={gji_active} used_eager_path={used_eager_path}"
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

        if let Some(final_actions) = core.poll(env) {
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
    coro: StepCoro<ProbeTickInput, Vec<ProbeAction>>,
    pending_transmit_done: Option<TransmitDonePayload>,
    pending_vk_sent: Option<VkSentPayload>,
    cold_seq: u32,
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
        cold_reason: ColdReason,
        used_eager_path: bool,
        forces_prepend_f2: bool,
        is_long_cold: bool,
        consecutive: u32,
    ) -> Self {
        let ctx = GjiProbeCtx {
            cold_seq,
            used_eager_path,
            forces_prepend_f2,
            is_long_cold,
            consecutive,
        };
        let romaji = romaji.to_string();
        let coro = StepCoro::new(async move |ch| {
            gji_coro_body(ch, romaji, probe, total_max_ms, cold_reason, ctx).await;
        });
        let mut this = Self {
            coro,
            pending_transmit_done: None,
            pending_vk_sent: None,
            cold_seq,
            literal_detect_guard: None,
        };
        // Self-priming: StepCoro の最初の step() は input を消費しない
        // （timed_fsm::coro のドキュメント参照）。construction 直後・pending_tsf に
        // 格納される前にこの「捨てられる1回」を消費しておくことで、install 後に
        // 外部から届く最初の tick の入力（deferred VK 等）が握り潰されるのを防ぐ。
        // construction 時点では何も届いていないため、捨てても安全。
        let primed = this.tick(TsfEnvSnapshot::default());
        debug_assert!(
            primed.is_empty(),
            "GjiWarmupCoro self-priming tick は空の ProbeAction を返すはず: {primed:?}"
        );
        this
    }
}

// ── TickableFsm impl ─────────────────────────────────────────────────────────

impl TickableFsm for GjiWarmupCoro {
    fn tick(&mut self, env: TsfEnvSnapshot) -> Vec<ProbeAction> {
        // BUG-27 調査用ログ: probe_fsm.rs::TsfProbeCoro::tick と同じ意図。
        // 通常 tick は毎回 None のため、どちらかが Some の場合のみ出す。
        if self.pending_vk_sent.is_some() || self.pending_transmit_done.is_some() {
            log::debug!(
                "[gji-coro-vk-sent-trace] cold={} tick consuming pending_vk_sent={} \
                 pending_transmit_done={} t={}ms",
                self.cold_seq,
                self.pending_vk_sent.is_some(),
                self.pending_transmit_done.is_some(),
                crate::hook::current_tick_ms(),
            );
        }
        let input = ProbeTickInput {
            env,
            transmit_done: self.pending_transmit_done.take(),
            vk_sent: self.pending_vk_sent.take(),
        };
        match self.coro.step(input) {
            CoroStep::Yielded(actions) => actions,
            CoroStep::Complete => vec![ProbeAction::Done],
        }
    }

    fn cold_seq_hint(&self) -> u32 {
        self.cold_seq
    }

    /// `needs_literal=true` のとき `false`（Continue）を返してコルーチンに LiteralDetect を委譲する。
    ///
    /// `false` → `dispatch_probe_actions` が `DispatchResult::Continue` を返す
    /// → 次 tick の `ProbeTickInput` に `transmit_done` が乗り、コルーチン本体が Phase 6 に入る。
    fn apply_transmit_done(
        &mut self,
        romaji: String,
        ze_bs_count: usize,
        detector: Option<LiteralDetector>,
        literal_detect_ms: u64,
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

    /// BUG-24 追補: IME セッション最初の1文字の per-VK confirm ループが1 VK 送信するたびに呼ぶ。
    /// 次 tick の `ProbeTickInput` に `vk_sent` として載せ、コルーチン本体がそのVK専用の
    /// detector をポーリングする。
    fn apply_vk_sent(&mut self, detector: LiteralDetector, deadline_ms: u64) {
        // BUG-27 調査用ログ: probe_fsm.rs::TsfProbeCoro::apply_vk_sent と同じ意図。
        let overwritten = self.pending_vk_sent.is_some();
        log::debug!(
            "[gji-coro-vk-sent-trace] cold={} apply_vk_sent SET deadline_ms={deadline_ms} \
             overwritten_unconsumed={overwritten} t={}ms",
            self.cold_seq,
            crate::hook::current_tick_ms(),
        );
        if self.literal_detect_guard.is_none() {
            self.literal_detect_guard = Some(OutputActiveGuard::begin());
        }
        self.pending_vk_sent = Some(VkSentPayload {
            detector,
            deadline_ms,
        });
    }
}
