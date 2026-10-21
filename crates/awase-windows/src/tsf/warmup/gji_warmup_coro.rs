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
//! ```
//!
//! 捨て駒キー（`StartSacrificialWarmup`、VK_A+BS/VK_IME_OFF→ON）は 2026-07-16 に
//! この2経路（per-VK・当時あった inline LiteralDetect）双方から撤去し、2026-07-18 に
//! Chrome 側の long-cold 予防的 `StartSacrificialWarmup`（`probe_fsm.rs`）・
//! `SacrificialWarmupCoro`/`ImeOffOnWarmupFsm` 自体も物理削除した。
//! `RawTsfLiteralRecovery` の dispatcher が `consecutive_count()==0` のときだけ
//! romaji 再送を自然に次の cold パス（per-VK confirm）へ委ねる
//! （`literal_detect_fsm::emit_recovery_actions` 参照）。
//!
//! inline LiteralDetect(旧 Phase 6)は到達しないので撤去した(ECRS MB-01)。`LiteralDetectCore` は
//! warm パスの `LiteralDetectFsm` だけが使う。

use std::rc::Rc;

use crate::state::event_origin::Generation;
use crate::tsf::output::ColdReason;
use crate::tsf::probe::{LiteralDetector, TsfReadinessProbe};
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::warmup::probe_coro_state::ProbeCoroState;
use crate::tsf::warmup::probe_fsm::{
    decide_transmit_plan, run_per_vk_confirm, ProbeAction, ProbeObservations, ProbeTickInput,
    TransmitTarget, TsfEnvSnapshot,
};
use crate::tsf::warmup::tickable_fsm::TickableFsm;
use timed_fsm::coro::{yield_step, Channel, StepCoro};

// ── GjiProbeCtx ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct GjiProbeCtx {
    cold_seq: Generation,
    used_eager_path: bool,
    forces_prepend_f2: bool,
    is_long_cold: bool,
}

// ── コルーチン本体 ────────────────────────────────────────────────────────────

// `Rc` を使うため生成される future は `!Send`。これはタイマー駆動の単一スレッド設計
// による意図的な制約（crates/timed-fsm/src/coro.rs::yield_step 参照）。
#[expect(clippy::future_not_send)]
async fn gji_coro_body(
    ch: Rc<Channel<ProbeTickInput, Vec<ProbeAction>>>,
    romaji: String,
    probe: TsfReadinessProbe,
    cold_reason: ColdReason,
    ctx: GjiProbeCtx,
) {
    // ── Phase 1: GJI probe ───────────────────────────────────────────────────
    let (env, nc_fired, gji_settled) = {
        // 最初の step() で input は消費されないため、env は 2 tick 目から更新される。
        // 10ms タイマー駆動の 1 tick ズレは動作に影響しない
        // （`GjiWarmupCoro::new` が construction 時に self-priming tick を行うため、
        //   外部から見た最初の tick は既にこの「捨てられる」1回を消費済みになっている）。
        let input = yield_step(ch.clone(), vec![]).await;
        let env = input.env;

        let outcome = probe.outcome();

        // probe 完了時の実測値（elapsed/gji_idle/settled）は「予算に対して実際どれだけ
        // かかったか」を事後ログから直接確認するために必要（切り分けログ強化、2026-07-09）。
        tracing::debug!(
            "[gji-coro] cold={} GjiProbe 完了 ({}ms gji_idle={}ms settled={})",
            ctx.cold_seq.value(),
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
            tracing::debug!(
                "[gji-coro] cold={} settle 必要 (reason={cold_reason:?} gji_idle_ms={} \
                 probe_elapsed={}ms settled={}) → skip FreshF2, reactive LiteralDetect のみ",
                ctx.cold_seq.value(),
                outcome.gji_idle_ms,
                outcome.elapsed_ms,
                outcome.settled,
            );
            (env, false, outcome.settled)
        } else {
            (env, true, outcome.settled)
        }
    };

    // ── Phase 4: transmit plan 決定 ───────────────────────────────────────────
    // ReinjectConfirmKey/PassthroughConfirmKey + TSF mode（WezTerm等）:
    // Enter/Space 後に WezTerm は NameChange を発火しないが GJI は VKs を正常にコンポジション中。
    // nc_fired=false のまま decide_transmit_plan に渡すと needs_literal 第2項が true になり
    // LiteralDetect が誤検出して backspace を送る（composited 'な' が消えるバグ、`3ffbe66`）。
    // → confirm_key_tsf_hint として decide_transmit_plan に渡し、gji_settled（実測）と
    // 併せて救済してよいかを判定させる。nc_fired 自体はここで書き換えない
    // （BUG-40: 以前は is_confirm_key && is_tsf_mode だけで nc_fired を無条件に true へ
    // 昇格していたため、88s idle 後の genuinely cold なセッション（gji_settled=false）
    // まで誤救済され、reactive LiteralDetect が丸ごとスキップされて漏れた romaji が
    // 無補正で出力された。`docs/known-bugs.md` 参照）。
    let observations = ProbeObservations {
        nc_fired,
        gji_settled,
        confirm_key_tsf_hint: cold_reason.is_confirm_key() && env.is_tsf_mode,
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
    // nc_fired/gji_settled/confirm_key_tsf_hint）は従来ログに一切出ておらず、部分
    // リテラル発生時に「なぜ LiteralDetect に入った/入らなかったか」を事後診断
    // できなかった（切り分けログ強化、2026-07-09 の "kお" 系調査で判明。
    // gji_settled/confirm_key_tsf_hint は BUG-40 追加）。
    tracing::debug!(
        "[gji-coro] cold={} transmit-plan needs_literal={} nc_fired={} gji_settled={} \
         confirm_key_tsf_hint={} is_tsf_mode={}",
        ctx.cold_seq.value(),
        plan.needs_literal,
        observations.nc_fired,
        observations.gji_settled,
        observations.confirm_key_tsf_hint,
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
        && env.literal_session_confirmed_gen != Some(ctx.cold_seq)
        && !plan.used_eager_path
        && env.is_tsf_mode
    {
        run_per_vk_confirm(ch.clone(), ctx.cold_seq, &romaji, plan, TransmitTarget::Tsf).await;
        return;
    }

    // ── Phase 5b: Transmit ───────────────────────────────────────────────────
    // ここへ来るのは `needs_literal=false` のときだけ。dispatcher が `apply_transmit_done(None)` → true で
    // machine を drop するので、コルーチンは再開されない。
    //
    // `needs_literal=true` は上の per-VK confirm が必ず先に取る(ECRS MB-01、2026-10-21): `needs_literal` は
    // `is_tsf_mode` のときだけ真で、そのとき `used_eager_path` は偽になる(`decide_transmit_plan`)。per-VK を外れるのは
    // `env.literal_session_confirmed_gen == Some(cold_seq)` のときだけだが、`cold_seq` は cold start のたびに新しく採番され
    // (`increment_cold_start_count` は `vk_send.rs`・`cold_warmup.rs` のコルーチン生成直前の 2 か所)、
    // `mark_literal_session_confirmed` に渡るのは完了した per-VK/warm LiteralDetect 自身の `cold_seq` なので、
    // 生成直後のコルーチンでは成り立たない。旧 Phase 6(inline LiteralDetect、`LiteralDetectCore` を warm パスと共有)は撤去した。
    yield_step(
        ch.clone(),
        vec![ProbeAction::Transmit {
            cold_seq: ctx.cold_seq,
            plan,
            romaji: romaji.clone(),
            target: TransmitTarget::Tsf,
        }],
    )
    .await;
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
    /// `tick`/`apply_vk_sent`/prime のラッパー配線
    /// （`TsfProbeCoro` と共有、`probe_coro_state.rs` 参照）。
    state: ProbeCoroState,
    /// inline LiteralDetect フェーズ中に OUTPUT_GATE を active に保つ追加ガード。
    literal_detect_guard: Option<OutputActiveGuard>,
}

impl GjiWarmupCoro {
    /// `GjiWarmupFsm::new` と同等のシグネチャ。
    pub(crate) fn new(
        romaji: &str,
        cold_seq: Generation,
        probe: TsfReadinessProbe,
        cold_reason: ColdReason,
        used_eager_path: bool,
        forces_prepend_f2: bool,
        is_long_cold: bool,
    ) -> Self {
        let ctx = GjiProbeCtx {
            cold_seq,
            used_eager_path,
            forces_prepend_f2,
            is_long_cold,
        };
        let romaji = romaji.to_string();
        let coro = StepCoro::new(async move |ch| {
            gji_coro_body(ch, romaji, probe, cold_reason, ctx).await;
        });
        Self {
            state: ProbeCoroState::new(coro, cold_seq, "GjiWarmupCoro"),
            literal_detect_guard: None,
        }
    }
}

// ── TickableFsm impl ─────────────────────────────────────────────────────────

impl TickableFsm for GjiWarmupCoro {
    fn tick(&mut self, env: TsfEnvSnapshot) -> Vec<ProbeAction> {
        self.state.tick(env, "gji-coro-vk-sent-trace")
    }

    fn cold_seq_hint(&self) -> Generation {
        self.state.cold_seq()
    }

    /// BUG-24 追補: IME セッション最初の1文字の per-VK confirm ループが1 VK 送信するたびに呼ぶ。
    /// 次 tick の `ProbeTickInput` に `vk_sent` として載せ、コルーチン本体がそのVK専用の
    /// detector をポーリングする。
    fn apply_vk_sent(&mut self, detector: LiteralDetector, deadline_ms: u64) {
        if self.literal_detect_guard.is_none() {
            self.literal_detect_guard = Some(OutputActiveGuard::begin());
        }
        self.state
            .apply_vk_sent(detector, deadline_ms, "gji-coro-vk-sent-trace");
    }
}
