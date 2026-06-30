//! TSF cold-start 犠牲キーウォームアップ コルーチン実装。
//!
//! [`SacrificialWarmupCoro`] は [`SacrificialWarmupFsm`] + [`ChromeGjiReinitFsm`] を
//! `StepCoro` で置き換えた。フェーズ遷移が単一の async 関数本体に直線的に記述されており、
//! `StartChromeGjiReinit` → `SwitchMachine` → `ChromeGjiReinitFsm` の機械切り替えが不要。
//!
//! ## フェーズ遷移（コルーチン本体）
//!
//! ```text
//! Phase 1: VK_A composition 確認
//!   ├─[confirmed_warm + Chrome]──► Phase 2（HIDE 待機）
//!   │                                ├─[早期 HIDE]──► Phase 3（IPC settle）──► SacrificialResend
//!   │                                └─[timeout]────────────────────────────► SacrificialResend
//!   ├─[!confirmed_warm + Chrome]─► [SendChromeGjiReinit] → IME 確認待機 ──► SacrificialResend
//!   └─[TSF/WezTerm]─────────────────────────────────────────────────────────► SacrificialResend
//! ```

use std::rc::Rc;

use crate::tsf::ime_mode_fsm::ImeModeState;
use crate::tsf::probe::LiteralDetector;
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::probe_fsm::{
    DeferredVk, ProbeAction, SacrificialResend, TransmitTarget, TsfEnvSnapshot,
};
use timed_fsm::coro::{yield_step, Channel, CoroStep, StepCoro};
use crate::tsf::tickable_fsm::TickableFsm;
use crate::tuning::{
    CHROME_GJI_REINIT_CONFIRM_MS, SACR_WARMUP_CHROME_HIDE_WAIT_MS,
    SACR_WARMUP_CHROME_IPC_SETTLE_MS,
};
use awase::types::VkCode;

// ── TickInput ─────────────────────────────────────────────────────────────────

struct TickInput {
    env: TsfEnvSnapshot,
    /// `notify_start_composition()` で蓄積された composition 観測フラグ。
    composition_seen: bool,
    /// `push_deferred()` で蓄積された後続 VK。
    new_deferred: Vec<DeferredVk>,
}

// ── コルーチン本体 ─────────────────────────────────────────────────────────────

async fn sacr_warmup_coro_body(
    ch: Rc<Channel<TickInput, Vec<ProbeAction>>>,
    cold_seq: u32,
    romaji: String,
    mut deferred_vks: Vec<DeferredVk>,
    detector: LiteralDetector,
    deadline_ms: u64,
    target: TransmitTarget,
) {
    use crate::tsf::probe::DetectionResult;

    let mut composition_was_seen = false;

    // ── Phase 1: VK_A composition 確認 ──────────────────────────────────────
    let (detection, detection_env) = loop {
        let input = yield_step(ch.clone(), vec![]).await;
        composition_was_seen |= input.composition_seen;
        deferred_vks.extend(input.new_deferred);

        let Some(detection) = detector.check_now(deadline_ms) else {
            continue;
        };

        let confirmed = matches!(detection, DetectionResult::CompositionConfirmed);
        log::debug!(
            "[sacr-warmup] cold={cold_seq} VK_A 判定={} → 実ローマ字 {romaji:?} 再送",
            if confirmed { "composition-confirmed (TSF warm)" } else { "timeout (TSF still cold)" },
        );
        crate::ime_diagnostic::log_composition_probe(
            cold_seq,
            if confirmed { "sacr-warm" } else { "sacr-timeout" },
        );
        break (detection, input.env);
    };

    let confirmed_warm = matches!(detection, DetectionResult::CompositionConfirmed);

    // ── Chrome cold: VK_IME_OFF→VK_IME_ON + IME 確認待機（旧 ChromeGjiReinitFsm の内容）──
    if !confirmed_warm && target == TransmitTarget::Chrome {
        let reinit_deadline_ms = crate::hook::current_tick_ms() + CHROME_GJI_REINIT_CONFIRM_MS;
        log::debug!(
            "[sacr-warmup] cold={cold_seq} Chrome cold → SendChromeGjiReinit (timeout={}ms)",
            CHROME_GJI_REINIT_CONFIRM_MS,
        );
        // VK_IME_OFF→VK_IME_ON 送信 + async IMC ポーリング開始を dispatcher に委譲する。
        yield_step(ch.clone(), vec![ProbeAction::SendChromeGjiReinit { cold_seq }]).await;

        // IME mode が Hiragana に確定するまで待機する。
        loop {
            let input = yield_step(ch.clone(), vec![]).await;
            deferred_vks.extend(input.new_deferred);
            let ime_ready =
                input.env.ime_mode == ImeModeState::Hiragana && input.env.ime_mode_confirmed;
            if ime_ready {
                log::info!(
                    "[sacr-warmup] cold={cold_seq} Chrome reinit: IME Hiragana 確認 → 再送 {romaji:?}",
                );
                break;
            }
            if crate::hook::current_tick_ms() >= reinit_deadline_ms {
                log::warn!(
                    "[sacr-warmup] cold={cold_seq} Chrome reinit: タイムアウト → 強制再送 {romaji:?}",
                );
                break;
            }
        }

        yield_step(
            ch.clone(),
            vec![
                ProbeAction::SacrificialResend(SacrificialResend {
                    cold_seq,
                    romaji,
                    deferred_vks,
                    target,
                    confirmed_warm: false,
                    skip_cleanup_bs: false,
                }),
                ProbeAction::Done,
            ],
        )
        .await;
        return;
    }

    // ── Chrome warm: HIDE 待機 + IPC settle ────────────────────────────────
    if confirmed_warm && target == TransmitTarget::Chrome {
        if detection_env.gji_candidate_visible {
            // Phase 2: candidate window HIDE 待機（VK_A+BS の EndComposition IPC 到達の代理指標）
            let hide_deadline = crate::hook::current_tick_ms() + SACR_WARMUP_CHROME_HIDE_WAIT_MS;
            let early_hide = loop {
                let input = yield_step(ch.clone(), vec![]).await;
                deferred_vks.extend(input.new_deferred);
                let candidate_gone = !input.env.gji_candidate_visible;
                let timed_out = crate::hook::current_tick_ms() >= hide_deadline;
                if candidate_gone || timed_out {
                    log::debug!(
                        "[sacr-warmup] cold={cold_seq} Chrome HIDE 待機完了: \
                         candidate_gone={candidate_gone} timed_out={timed_out}",
                    );
                    break candidate_gone;
                }
            };

            if early_hide {
                // 早期 HIDE: EndComposition IPC がまだ Chrome に到達していない（~200ms かかる）。
                let settle_deadline =
                    crate::hook::current_tick_ms() + SACR_WARMUP_CHROME_IPC_SETTLE_MS;
                log::debug!(
                    "[sacr-warmup] cold={cold_seq} Chrome HIDE 後 IPC settle 待機 ({}ms)",
                    SACR_WARMUP_CHROME_IPC_SETTLE_MS,
                );
                loop {
                    let input = yield_step(ch.clone(), vec![]).await;
                    deferred_vks.extend(input.new_deferred);
                    if crate::hook::current_tick_ms() >= settle_deadline {
                        log::debug!("[sacr-warmup] cold={cold_seq} IPC settle 完了 → 再送");
                        break;
                    }
                }
            }
            // timed_out: 300ms 経過 → IPC settle 済みと見なして即再送
        } else if composition_was_seen {
            // VK_A+BS atomic batch で SHOW+HIDE が最初の tick 前に完了した場合。
            // candidate は既に非表示だが EndComposition IPC は ~200ms かかる。
            let settle_deadline =
                crate::hook::current_tick_ms() + SACR_WARMUP_CHROME_IPC_SETTLE_MS;
            log::debug!(
                "[sacr-warmup] cold={cold_seq} Chrome warm: 早期 HIDE (composition 観測済み) \
                 IPC settle 待機 ({}ms)",
                SACR_WARMUP_CHROME_IPC_SETTLE_MS,
            );
            loop {
                let input = yield_step(ch.clone(), vec![]).await;
                deferred_vks.extend(input.new_deferred);
                if crate::hook::current_tick_ms() >= settle_deadline {
                    log::debug!("[sacr-warmup] cold={cold_seq} IPC settle 完了 → 再送");
                    break;
                }
            }
        } else {
            // candidate window が全く出なかった → 即再送
            log::debug!("[sacr-warmup] cold={cold_seq} Chrome warm: candidate 非表示、即再送");
        }
    }

    // ── TSF/WezTerm または Chrome warm 完了: SacrificialResend ──────────────
    yield_step(
        ch.clone(),
        vec![
            ProbeAction::SacrificialResend(SacrificialResend {
                cold_seq,
                romaji,
                deferred_vks,
                target,
                confirmed_warm,
                skip_cleanup_bs: false,
            }),
            ProbeAction::Done,
        ],
    )
    .await;
}

// ── SacrificialWarmupCoro ─────────────────────────────────────────────────────

/// TSF cold-start 犠牲キーウォームアップ コルーチン。
///
/// `SacrificialWarmupFsm` + `ChromeGjiReinitFsm` の後継。
/// [`TickableFsm`] を実装し `pending_tsf` に格納される。
pub(crate) struct SacrificialWarmupCoro {
    coro: StepCoro<TickInput, Vec<ProbeAction>>,
    cold_seq: u32,
    _guard: OutputActiveGuard,
    pending_composition_seen: bool,
    pending_deferred: Vec<DeferredVk>,
}

impl SacrificialWarmupCoro {
    pub(crate) fn new(
        cold_seq: u32,
        romaji: String,
        deferred_vks: Vec<DeferredVk>,
        detector: LiteralDetector,
        deadline_ms: u64,
        target: TransmitTarget,
    ) -> Self {
        let guard = OutputActiveGuard::begin();
        let coro = StepCoro::new(async move |ch| {
            sacr_warmup_coro_body(ch, cold_seq, romaji, deferred_vks, detector, deadline_ms, target).await
        });
        Self {
            coro,
            cold_seq,
            _guard: guard,
            pending_composition_seen: false,
            pending_deferred: vec![],
        }
    }
}

impl TickableFsm for SacrificialWarmupCoro {
    fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        let input = TickInput {
            env: *env,
            composition_seen: std::mem::take(&mut self.pending_composition_seen),
            new_deferred: std::mem::take(&mut self.pending_deferred),
        };
        match self.coro.step(input) {
            CoroStep::Yielded(actions) => actions,
            CoroStep::Complete => vec![ProbeAction::Done],
        }
    }

    fn cold_seq_hint(&self) -> u32 {
        self.cold_seq
    }

    fn push_deferred(&mut self, vk: VkCode, needs_shift: bool) {
        self.pending_deferred.push(DeferredVk { vk, needs_shift });
    }

    fn notify_start_composition(&mut self) {
        self.pending_composition_seen = true;
    }
}
