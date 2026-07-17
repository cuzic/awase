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
//! この2経路（per-VK・inline LiteralDetect）双方から撤去した。`RawTsfLiteralRecovery`
//! の dispatcher が `consecutive_count()==0` のときだけ romaji 再送を自然に次の
//! cold パス（per-VK confirm）へ委ねる（`literal_detect_fsm::emit_recovery_actions`
//! 参照）。Chrome 側の long-cold 予防的 `StartSacrificialWarmup`（`probe_fsm.rs`）
//! はまだ `DIAG_CHROME_SKIP_SACRIFICIAL_WARMUP` で flag-gate されているのみで、
//! コード自体は残っている。
//!
//! Phase 6 の LiteralDetect 判定は `literal_detect_fsm::LiteralDetectCore` に集約されており、
//! warm パスの `LiteralDetectFsm` と同一ロジックを共有する（重複排除）。

use std::rc::Rc;

use awase::types::VkCode;

use crate::tsf::output::ColdReason;
use crate::tsf::probe::{LiteralDetector, TsfReadinessProbe};
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::warmup::probe_fsm::{
    decide_transmit_plan, ProbeAction, ProbeObservations, TransmitTarget, TsfEnvSnapshot,
};
use crate::tsf::warmup::tickable_fsm::TickableFsm;
use timed_fsm::coro::{yield_step, Channel, CoroStep, StepCoro};

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

/// `apply_vk_sent` のペイロード（BUG-24 追補: IME セッション最初の1文字の per-VK confirm ループ）。
/// 次 tick の TickInput に載り、コルーチン本体がそのVK専用の detector をポーリングする。
struct VkSentPayload {
    detector: LiteralDetector,
    deadline_ms: u64,
}

// ── TickInput ────────────────────────────────────────────────────────────────

struct TickInput {
    env: TsfEnvSnapshot,
    transmit_done: Option<TransmitDonePayload>,
    vk_sent: Option<VkSentPayload>,
}

// ── GjiProbeCtx ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct GjiProbeCtx {
    cold_seq: u32,
    prepend_f2_warmup: bool,
    used_eager_path: bool,
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

    // ── Phase 1: GJI probe ───────────────────────────────────────────────────
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
                break 'initial (false, false);
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
    let suppress_f2 = ctx.fresh_f2_at_probe_start && env.is_tsf_mode;
    let mut effective_prepend_f2 = ctx.prepend_f2_warmup && !suppress_f2;

    // BUG-24 検証（DIAG_DISABLE_PROACTIVE_TSF_WARMUP）: Phase 2/5a を止めても、この
    // 「romajiバッチにF2を直接同梱する」第3の防御層が生きていると、実質的に予防が
    // 効いたままになり実験にならない（2026-07-11 実機ログで確認済み: idle=1188ms の
    // "ko" 送信でも vks=[F2,4B,4F] のようにF2先頭同梱が発生し、無防備な状態を
    // 検証できていなかった）。ここも強制的に無効化し、reactiveなLiteralDetectのみに
    // 完全に委ねる。
    if crate::tuning::DIAG_DISABLE_PROACTIVE_TSF_WARMUP && effective_prepend_f2 {
        log::debug!(
            "[gji-coro-diag] cold={} DIAG_DISABLE_PROACTIVE_TSF_WARMUP: \
             force effective_prepend_f2=false (reason={cold_reason:?} \
             real_gji_idle_ms={})",
            ctx.cold_seq,
            crate::tsf::observer::gji_idle_ms(),
        );
        effective_prepend_f2 = false;
    }

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

    // ── Phase 5a: long cold + TSF の proactive StartSacrificialWarmup は撤去 ────
    // `DIAG_DISABLE_PROACTIVE_TSF_WARMUP`（常時 true）下で、以前ここにあった
    // 「long_cold && is_tsf_mode → 犠牲キー(VK_A+BS/VK_IME_OFF→ON) escalation を
    // 即発行」という分岐は無条件に到達不能だった。Phase 5b（直接Transmit + inline
    // LiteralDetect）に完全にフォールスルーする（`docs/known-bugs.md` BUG-24 参照。
    // 再度有効化する場合はこのコミットの revert が必要）。
    //
    // なお `ProbeAction::StartSacrificialWarmup` 自体は撤去していないが、2026-07-16
    // 時点で本番コードから到達する経路は Chrome 側の cold-start パス
    // （`probe_fsm.rs` の `TsfProbeCoro`、`DIAG_CHROME_SKIP_SACRIFICIAL_WARMUP` で
    // flag-gate 済み・デフォルト到達不能）のみ。TSF mode の partial/suspected-literal
    // 回収パス（`literal_detect_fsm.rs::emit_recovery_actions`、per-VK confirm 双方）は
    // 常に `RawTsfLiteralRecovery`（backspace のみ）に一本化し、捨て駒キーには
    // 一切倒れないようにした（ユーザー方針、2026-07-16）。
    if plan.needs_literal && ctx.is_long_cold && env.is_tsf_mode {
        log::debug!(
            "[gji-coro-diag] cold={} long_cold + TSF: proactive StartSacrificialWarmup は \
             撤去済み → 直接Transmit (reason={cold_reason:?} real_gji_idle_ms={})",
            ctx.cold_seq,
            crate::tsf::observer::gji_idle_ms(),
        );
    }

    // ── Phase 5b (IME セッション最初の1文字専用, BUG-24 追補): per-VK send+confirm ──
    // is_partial_literal() は「今回送った romaji 自身の確認信号」ではなく、送信前に
    // 確定していた無関係な代理指標 nc_fired/gji_resumed（別の F2 warmup キーへの応答
    // 有無）で判定しており、cold 直後の最初の1文字はほぼ確実に誤検知する（BUG-24）。
    // このセッションでまだ literal-detect を確認済みでない場合に限り、romaji の VK を
    // 1つずつ送信し、「送信した VK 自身」への CompositionConfirmed/SuspectedLiteral を
    // 都度確認してから次の VK を送る。F2 prepend・unicode eager path は対象外とし
    // （それ以外の経路は下の通常 Transmit にフォールスルーする）、全 VK 確認できたら
    // このセッションの残りは literal-detect 自体をスキップする
    // （`tsf::observer::mark_literal_session_confirmed`、`literal_detect_fsm.rs` 参照）。
    if crate::tuning::DIAG_LITERAL_SESSION_SKIP
        && plan.needs_literal
        && !crate::tsf::observer::literal_session_confirmed()
        && !plan.should_prepend_f2
        && !plan.used_eager_path
        && env.is_tsf_mode
    {
        use crate::tsf::probe::DetectionResult;

        let vk_chars: Vec<(VkCode, bool)> = romaji
            .chars()
            .filter_map(crate::output::resolve_ascii_to_vk)
            .collect();
        if vk_chars.is_empty() {
            yield_step(ch.clone(), vec![ProbeAction::Done]).await;
            return;
        }
        let last_idx = vk_chars.len() - 1;
        for (idx, &(vk, needs_shift)) in vk_chars.iter().enumerate() {
            let is_last = idx == last_idx;
            let vk_input = yield_step(
                ch.clone(),
                vec![ProbeAction::TransmitSingleVk {
                    cold_seq: ctx.cold_seq,
                    vk,
                    needs_shift,
                    timeout_ms: plan.literal_detect_ms,
                    is_last,
                    observations,
                    plan,
                    target: TransmitTarget::Tsf,
                }],
            )
            .await;
            let Some(sent) = vk_input.vk_sent else {
                log::warn!(
                    "[gji-coro] cold={} per-VK[{idx}/{last_idx}] vk_sent 未設定 → 中断",
                    ctx.cold_seq
                );
                return;
            };

            let detection = loop {
                let poll_input = yield_step(ch.clone(), vec![]).await;
                // per-VK 分岐は常にこの for ループ内で return するため（全確認 or
                // SuspectedLiteral 回収のいずれか）、以降の Phase で使う外側の `env` を
                // 更新する必要はない。
                let _ = poll_input;
                if let Some(d) = sent.detector.check_now(sent.deadline_ms) {
                    break d;
                }
            };

            match detection {
                DetectionResult::CompositionConfirmed => {
                    log::debug!(
                        "[gji-coro] cold={} per-VK[{idx}/{last_idx}] confirmed (vk=0x{:02X})",
                        ctx.cold_seq,
                        vk.0,
                    );
                }
                DetectionResult::SuspectedLiteral => {
                    let (backs, escape_composition) =
                        crate::tsf::warmup::literal_detect_fsm::per_vk_recovery_params(idx);
                    log::debug!(
                        "[gji-coro] cold={} per-VK[{idx}/{last_idx}] suspected literal \
                         (vk=0x{:02X} escape={escape_composition})",
                        ctx.cold_seq,
                        vk.0,
                    );
                    crate::ime_diagnostic::log_composition_probe(
                        ctx.cold_seq,
                        "setopen-per-vk-literal",
                    );
                    let actions = crate::tsf::warmup::literal_detect_fsm::emit_recovery_actions(
                        ctx.cold_seq,
                        romaji.clone(),
                        backs,
                        escape_composition,
                    );
                    yield_step(ch.clone(), actions).await;
                    return;
                }
            }
        }

        log::debug!(
            "[gji-coro] cold={} per-VK: 全 {} VK 確認済み → セッション確認 (is_partial_literal bypass)",
            ctx.cold_seq,
            vk_chars.len(),
        );
        crate::tsf::observer::mark_literal_session_confirmed();
        crate::ime_diagnostic::log_composition_probe(ctx.cold_seq, "setopen-per-vk-confirmed");
        yield_step(ch.clone(), vec![ProbeAction::Done]).await;
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
        needs_settle_check: bool,
        cold_reason: ColdReason,
        prepend_f2_warmup: bool,
        used_eager_path: bool,
        forces_prepend_f2: bool,
        is_long_cold: bool,
        fresh_f2_at_probe_start: bool,
        consecutive: u32,
    ) -> Self {
        let ctx = GjiProbeCtx {
            cold_seq,
            prepend_f2_warmup,
            used_eager_path,
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

    /// BUG-24 追補: IME セッション最初の1文字の per-VK confirm ループが1 VK 送信するたびに呼ぶ。
    /// 次 tick の TickInput に `vk_sent` として載せ、コルーチン本体がそのVK専用の
    /// detector をポーリングする。
    fn apply_vk_sent(&mut self, detector: LiteralDetector, deadline_ms: u64) {
        if self.literal_detect_guard.is_none() {
            self.literal_detect_guard = Some(OutputActiveGuard::begin());
        }
        self.pending_vk_sent = Some(VkSentPayload {
            detector,
            deadline_ms,
        });
    }
}
