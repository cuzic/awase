//! MS-IME confirm-then-transmit コルーチン実装。
//!
//! # 背景（BUG-13: MS-IME cold start「を」→「wお」）
//!
//! `MsImeStrategy` は「MS-IME の TSF context は常にウォーム」という前提で
//! `is_warm()=true` / `needs_f2_probe()=false` を固定しており、cold-start 保護が
//! 一切なかった。しかし IME OFF→ON 遷移直後は OS 側の準備に実測 ~130-300ms かかり
//! （2026-07-06 WT×MS-IME: 遷移 +122ms で conv=0x00000000 のまま "wo" を送信 →
//! 'w' がリテラル化して「wお」）、この窓に VK を送ると先頭文字が化ける。
//!
//! # 方式: 固定待ちではなく IMC 観測で「準備完了」を確信してから送信する
//!
//! GJI の F2 probe（プロセス I/O 観測）と同型の confirm-then-transmit。MS-IME には
//! 変換専用プロセスがないため、観測シグナルには `IMC_GETCONVERSIONMODE`
//! （フォーカス先の conversion mode。準備完了で NATIVE ビットが立つ）を使う。
//! Chrome 経路の `send_chrome_gji_reinit_and_poll` で運用実績のあるシグナル。
//!
//! ```text
//! send_romaji_as_tsf（MS-IME + TSF mode + ImeModeFsm 未確認）
//!   ├─ romaji を保持して MsImeReadyCoro を pending_tsf に設置
//!   ├─ Output::start_ms_ime_ready_poll が IMC ポーリング開始（10ms 間隔、async）
//!   │    └─ ImeModeFsm を on_conversion_mode_read で確定させる
//!   └─ コルーチン: env.ime_mode が NATIVE 確認されるまで tick 待機
//!        ├─[確認]────────► Transmit(Tsf) → deferred VK flush → Done
//!        └─[期限切れ]────► 強制 Transmit（安全弁。give-up latch はポーリング側が設定）
//! ```
//!
//! probe 中に届いた後続キーは既存の deferred VK 機構
//! （`defer_if_probe_in_flight` / `defer_vk_if_probe_in_flight`）に積まれ、
//! dispatcher の `Transmit` アームが送信直後に flush する（順序保証）。

use std::rc::Rc;

use crate::tsf::ime_mode_fsm::ImeModeState;
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::warmup::probe_fsm::{ProbeAction, TransmitPlan, TransmitTarget, TsfEnvSnapshot};
use crate::tsf::warmup::tickable_fsm::TickableFsm;
use timed_fsm::coro::{yield_step, Channel, CoroStep, StepCoro};

/// env の IME mode が「かな VK を受け付けられる」状態として確認済みか。
///
/// Hiragana / Katakana はどちらも NATIVE ビット確認済みで romaji VK が compose される。
/// 純粋関数（`ImeModeFsm::is_native_ready` の env 版）。
fn env_native_ready(env: TsfEnvSnapshot) -> bool {
    env.ime_mode_confirmed
        && matches!(
            env.ime_mode,
            ImeModeState::Hiragana | ImeModeState::Katakana
        )
}

// `Rc` を使うため生成される future は `!Send`。これはタイマー駆動の単一スレッド設計
// による意図的な制約（crates/timed-fsm/src/coro.rs::yield_step 参照）。
#[expect(clippy::future_not_send)]
async fn ms_ime_ready_coro_body(
    ch: Rc<Channel<TsfEnvSnapshot, Vec<ProbeAction>>>,
    cold_seq: u32,
    romaji: String,
    deadline_ms: u64,
) {
    // ── Phase 1: ImeModeFsm の NATIVE 確認待ち ─────────────────────────────
    // 確認の実体は Output::start_ms_ime_ready_poll の async IMC ポーリング。
    // ここでは env 経由で結果を観測するだけ（tick = TIMER_TSF_PROBE 10ms 間隔）。
    let start_ms = crate::hook::current_tick_ms();
    loop {
        let env = yield_step(ch.clone(), vec![]).await;
        if env_native_ready(env) {
            log::info!(
                "[msime-ready] cold={cold_seq} IME mode NATIVE 確認 (+{}ms) → 送信 {romaji:?}",
                crate::hook::current_tick_ms().saturating_sub(start_ms),
            );
            break;
        }
        if crate::hook::current_tick_ms() >= deadline_ms {
            // 安全弁: IMC が読めない環境でタイピングを止めない。
            // give-up latch（連続発動の抑止）は start_ms_ime_ready_poll 側が設定する。
            log::warn!(
                "[msime-ready] cold={cold_seq} 期限切れ (mode={:?} confirmed={}) → 強制送信 {romaji:?}",
                env.ime_mode,
                env.ime_mode_confirmed,
            );
            break;
        }
    }

    // ── Phase 2: Transmit → Done ──────────────────────────────────────────
    // dispatcher が romaji 送信 → deferred VK flush → warm マークまで行う。
    // F2 前置は不要（MS-IME は VK_DBE_HIRAGANA warmup を必要としない）。
    // LiteralDetect は GJI 観測（candidate window / write_bytes）前提のため使わない。
    yield_step(
        ch,
        vec![
            ProbeAction::Transmit {
                cold_seq,
                plan: TransmitPlan {
                    used_eager_path: false,
                    needs_literal: false,
                    literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                },
                romaji,
                target: TransmitTarget::Tsf,
            },
            ProbeAction::Done,
        ],
    )
    .await;
}

/// MS-IME confirm-then-transmit コルーチン。
///
/// [`TickableFsm`] を実装し `pending_tsf` に格納される。
/// 設置は `Output::ms_ime_gate_defer`（`send_romaji_as_tsf` のゲート）。
pub(crate) struct MsImeReadyCoro {
    coro: StepCoro<TsfEnvSnapshot, Vec<ProbeAction>>,
    cold_seq: u32,
    /// RAII guard。drop で `OUTPUT_GATE.active=false`。
    _guard: OutputActiveGuard,
}

impl MsImeReadyCoro {
    pub(crate) fn new(romaji: &str, cold_seq: u32, deadline_ms: u64) -> Self {
        let guard = OutputActiveGuard::begin();
        let romaji = romaji.to_string();
        let coro = StepCoro::new(async move |ch| {
            ms_ime_ready_coro_body(ch, cold_seq, romaji, deadline_ms).await;
        });
        let mut this = Self {
            coro,
            cold_seq,
            _guard: guard,
        };
        // Self-priming: StepCoro の最初の step() は input を消費しない。construction 直後・
        // pending_tsf に格納される前にこの「捨てられる1回」を消費しておく
        // （詳細は `GjiWarmupCoro::new` のコメント参照）。
        let primed = this.tick(TsfEnvSnapshot::default());
        debug_assert!(
            primed.is_empty(),
            "MsImeReadyCoro self-priming tick は空の ProbeAction を返すはず: {primed:?}"
        );
        this
    }
}

impl TickableFsm for MsImeReadyCoro {
    fn tick(&mut self, env: TsfEnvSnapshot) -> Vec<ProbeAction> {
        match self.coro.step(env) {
            CoroStep::Yielded(actions) => actions,
            CoroStep::Complete => vec![ProbeAction::Done],
        }
    }

    fn cold_seq_hint(&self) -> u32 {
        self.cold_seq
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsf::warmup::probe_fsm::TsfEnvSnapshot;

    fn env(mode: ImeModeState, confirmed: bool) -> TsfEnvSnapshot {
        TsfEnvSnapshot {
            ime_mode: mode,
            ime_mode_confirmed: confirmed,
            ..Default::default()
        }
    }

    #[test]
    fn native_ready_requires_confirmation() {
        // belief だけ（unconfirmed）では準備完了と見なさない — BUG-13 はまさに
        // belief=ON のまま OS 未準備の窓で送信したことが原因。
        assert!(!env_native_ready(env(ImeModeState::Hiragana, false)));
        assert!(env_native_ready(env(ImeModeState::Hiragana, true)));
    }

    #[test]
    fn katakana_counts_as_ready() {
        // ユーザーが意図的にカタカナモードの場合も NATIVE 確認済みなら送信してよい
        // （MsImeDirectStrategy の KATAKANA スキップと同じ扱い）。
        assert!(env_native_ready(env(ImeModeState::Katakana, true)));
    }

    #[test]
    fn off_and_unknown_are_not_ready() {
        assert!(!env_native_ready(env(ImeModeState::Off, true)));
        assert!(!env_native_ready(env(ImeModeState::Unknown, true)));
        assert!(!env_native_ready(env(ImeModeState::Unknown, false)));
    }

    #[test]
    fn coro_waits_until_confirmed_then_transmits() {
        let deadline = crate::hook::current_tick_ms() + 60_000;
        let mut coro = MsImeReadyCoro::new("wo", 7, deadline);

        // 未確認の間は待機（アクションなし）
        for _ in 0..3 {
            let actions = coro.tick(env(ImeModeState::Hiragana, false));
            assert!(actions.is_empty(), "未確認中は待機するはず: {actions:?}");
        }

        // NATIVE 確認 → Transmit + Done
        let actions = coro.tick(env(ImeModeState::Hiragana, true));
        assert_eq!(actions.len(), 2);
        assert!(matches!(
            &actions[0],
            ProbeAction::Transmit { romaji, target: TransmitTarget::Tsf, plan, .. }
                if romaji == "wo" && !plan.needs_literal
        ));
        assert!(matches!(actions[1], ProbeAction::Done));
    }

    #[test]
    fn coro_transmits_on_deadline_even_without_confirmation() {
        // 安全弁: IMC が読めない環境でも期限でタイピングを止めない。
        let deadline = crate::hook::current_tick_ms(); // 即座に期限切れ
        let mut coro = MsImeReadyCoro::new("ka", 8, deadline);

        let actions = coro.tick(env(ImeModeState::Unknown, false));
        assert_eq!(actions.len(), 2);
        assert!(matches!(
            &actions[0],
            ProbeAction::Transmit { romaji, .. } if romaji == "ka"
        ));
        assert!(matches!(actions[1], ProbeAction::Done));
    }
}
